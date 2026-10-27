//! `[llm.uses.planning]` (docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md,
//! decisions 2, 4–7, as amended at 11:20, 11:22 and 11:25 EDT): what the
//! general agent's own turns get when it is on — the `planning` prompt's
//! blocks after `PROMPT.md`, the file tools while `read` has patterns,
//! and the project's agent instructions with `agents_prompt`.
//!
//! `planning` is not a client of its own: it runs on the general agent's
//! backend and model, and changes the tools and the system prompt that
//! client is built with.

use std::collections::BTreeMap;

use crate::config::LlmUseConfig;
use crate::prompts::{PLANNING, Prompt, Prompts};
use crate::reasoning::{ToolSet, default_system_prompt};

/// `max_instructions_kb` when the prompt does not set it.
pub const DEFAULT_MAX_INSTRUCTIONS_KB: usize = 32;

/// One `read` pattern (the 11:20 EDT amendment, item 4): `/`-separated
/// segments matched against a root-relative path's segments. `**` as a
/// whole segment is zero or more segments; `*` within a segment is any run
/// of characters but `/`; everything else is literal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadPattern {
    /// As written.
    pub text: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Segment {
    /// `**`: zero or more segments.
    AnyDepth,
    /// One segment, `*` its only wildcard.
    Name(String),
}

impl ReadPattern {
    /// `text` as a pattern, or why it is refused.
    pub fn parse(text: &str) -> Result<ReadPattern, String> {
        let refuse = |why: &str| Err(format!("read pattern \"{text}\" is refused: {why}"));
        if text.is_empty() {
            return refuse("it is empty");
        }
        if let Some(c) = text
            .chars()
            .find(|c| matches!(c, '?' | '[' | ']' | '{' | '}' | '!' | '\\'))
        {
            return refuse(&format!("'{c}' is not part of the pattern syntax"));
        }
        if text.starts_with('/') {
            return refuse("it starts with '/'; patterns are relative to the project's root");
        }
        let mut segments = Vec::new();
        for segment in text.split('/') {
            match segment {
                "" => return refuse("it has an empty segment"),
                "." | ".." => return refuse("'.' and '..' are not allowed"),
                "**" => segments.push(Segment::AnyDepth),
                s if s.contains("**") => return refuse("'**' must be a whole segment"),
                s => segments.push(Segment::Name(s.to_string())),
            }
        }
        Ok(ReadPattern {
            text: text.to_string(),
            segments,
        })
    }

    /// Whether the root-relative `path` (`/`-separated) matches.
    pub fn matches(&self, path: &str) -> bool {
        let names: Vec<&str> = path.split('/').collect();
        match_segments(&self.segments, &names)
    }
}

fn match_segments(pattern: &[Segment], names: &[&str]) -> bool {
    match pattern.split_first() {
        None => names.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=names.len()).any(|skip| match_segments(rest, &names[skip..]))
        }
        Some((Segment::Name(p), rest)) => match names.split_first() {
            Some((name, names)) => match_name(p, name) && match_segments(rest, names),
            None => false,
        },
    }
}

/// One name against one segment, `*` being any run of characters.
fn match_name(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // No `*`: the whole name.
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// `[llm.uses.planning]` as resolved: what its turns may read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PlanningConfig {
    /// The project's agent instructions go in each turn's context.
    pub agents_prompt: bool,
    /// The valid `read` patterns, in order; empty turns the file tools off.
    pub read: Vec<ReadPattern>,
}

impl PlanningConfig {
    /// The table read: its valid patterns, and a warning for each refused
    /// one (left out).
    pub fn from_use(table: &LlmUseConfig) -> (PlanningConfig, Vec<String>) {
        let mut problems = Vec::new();
        let read = table
            .read
            .iter()
            .flatten()
            .filter_map(|p| {
                ReadPattern::parse(p)
                    .inspect_err(|e| problems.push(format!("[llm.uses.planning] {e}; left out")))
                    .ok()
            })
            .collect();
        (
            PlanningConfig {
                agents_prompt: table.agents_prompt.unwrap_or(false),
                read,
            },
            problems,
        )
    }

    /// `list_files` and `read_file` are declared (the 11:22 EDT amendment).
    pub fn file_tools(&self) -> bool {
        !self.read.is_empty()
    }

    /// The patterns, for the `files` block: `"*.md"`, `"docs/**/*.md"`.
    pub fn read_list(&self) -> String {
        self.read
            .iter()
            .map(|p| format!("\"{}\"", p.text))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The `planning` prompt (decision 6): `system`, `files` and
/// `instructions`, and `max_instructions_kb`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPrompt {
    prompt: Prompt,
}

impl PlanningPrompt {
    /// The `planning` prompt of `prompts`, checked: its three blocks are
    /// there and fill.
    pub fn from_prompts(prompts: &Prompts) -> Result<PlanningPrompt, String> {
        let prompt = prompts
            .get(PLANNING)
            .cloned()
            .ok_or_else(|| format!("the prompt `{PLANNING}` did not load"))?;
        let planning = PlanningPrompt { prompt };
        for block in ["files", "instructions"] {
            if !planning.prompt.settings.contains_key(block) {
                return Err(format!("the prompt `{PLANNING}` has no `{block}`"));
            }
        }
        let sample = PlanningConfig {
            agents_prompt: true,
            read: vec![ReadPattern::parse("*.md").expect("a valid pattern")],
        };
        planning.system_text(&sample)?;
        planning.instructions("p", "a", "AGENTS.md", "i")?;
        Ok(planning)
    }

    /// `file` in place of the shipped prompts, or the shipped ones; the
    /// file's errors in other prompts are not this prompt's.
    pub fn load(file: Option<&std::path::Path>) -> Result<PlanningPrompt, String> {
        let (prompts, _) = match file {
            Some(path) => Prompts::load(path)?,
            None => Prompts::shipped(),
        };
        Self::from_prompts(&prompts)
    }

    /// `system`, then `files` with `read` filled while the file tools are
    /// declared (the 11:22 EDT amendment, item 3).
    pub fn system_text(&self, config: &PlanningConfig) -> Result<String, String> {
        let p = &self.prompt;
        let mut text = p.fill(&p.system, &BTreeMap::new())?.trim().to_string();
        if config.file_tools() {
            let values = BTreeMap::from([("read", config.read_list())]);
            let files = p.fill(&p.settings["files"], &values)?;
            text.push_str("\n\n");
            text.push_str(files.trim());
        }
        Ok(text)
    }

    /// The `instructions` block for one project.
    pub fn instructions(
        &self,
        project: &str,
        agent: &str,
        file: &str,
        instructions: &str,
    ) -> Result<String, String> {
        let values = BTreeMap::from([
            ("project", project.to_string()),
            ("agent", agent.to_string()),
            ("file", file.to_string()),
            ("instructions", instructions.to_string()),
        ]);
        let p = &self.prompt;
        Ok(p.fill(&p.settings["instructions"], &values)?
            .trim()
            .to_string())
    }

    /// How much of an instructions file is read, in bytes.
    pub fn max_instructions_bytes(&self) -> usize {
        self.prompt
            .number("max_instructions_kb")
            .filter(|kb| *kb > 0.0)
            .map_or(DEFAULT_MAX_INSTRUCTIONS_KB, |kb| kb as usize)
            * 1024
    }
}

/// What the general agent's client is built with: its system prompt and
/// its tools. Part of the client's key, so a change builds a new client.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentProfile {
    pub system_prompt: String,
    pub tools: ToolSet,
}

impl AgentProfile {
    /// The general agent without planning: `PROMPT.md` and the base tools.
    pub fn base() -> AgentProfile {
        AgentProfile {
            system_prompt: default_system_prompt().unwrap_or_default(),
            tools: ToolSet::default(),
        }
    }

    /// With planning on: `PROMPT.md`, then the planning blocks (decision 6,
    /// items 1 and 2), and the file tools while `read` has patterns.
    pub fn planning(
        prompt: &PlanningPrompt,
        config: &PlanningConfig,
    ) -> Result<AgentProfile, String> {
        let base = AgentProfile::base();
        Ok(AgentProfile {
            system_prompt: assemble(&base.system_prompt, &prompt.system_text(config)?),
            tools: ToolSet {
                file_tools: config.file_tools(),
            },
        })
    }
}

/// `base`, then `extra` after a blank line.
pub fn assemble(base: &str, extra: &str) -> String {
    match (base.trim(), extra.trim()) {
        ("", extra) => extra.to_string(),
        (base, "") => base.to_string(),
        (base, extra) => format!("{base}\n\n{extra}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(read: &[&str], agents_prompt: bool) -> (PlanningConfig, Vec<String>) {
        PlanningConfig::from_use(&LlmUseConfig {
            agents_prompt: Some(agents_prompt),
            read: Some(read.iter().map(|s| s.to_string()).collect()),
            ..LlmUseConfig::default()
        })
    }

    #[test]
    fn patterns_outside_the_syntax_are_refused_and_left_out() {
        let (c, problems) = config(
            &[
                "*.md",
                "docs/**/*.md",
                "docs/**",
                "a?.md",
                "[ab].md",
                "{a,b}.md",
                "!x.md",
                "a\\b",
                "/etc/*",
                "a**/b",
                "docs/**x",
                "a//b",
                "../x.md",
                "",
            ],
            false,
        );
        let kept: Vec<&str> = c.read.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(kept, ["*.md", "docs/**/*.md", "docs/**"]);
        assert_eq!(problems.len(), 11, "{problems:?}");
        assert!(problems.iter().all(|p| p.contains("left out")));
        assert!(problems[0].contains("'?'"), "{}", problems[0]);
        assert!(problems[5].contains("starts with '/'"), "{}", problems[5]);
        assert!(problems[6].contains("whole segment"), "{}", problems[6]);
    }

    #[test]
    fn patterns_match_by_segment() {
        let p = |t: &str| ReadPattern::parse(t).unwrap();
        let top = p("*.md");
        assert!(top.matches("README.md"));
        assert!(!top.matches("docs/README.md"), "the top folder only");
        assert!(!top.matches("README.mdx"));
        let docs = p("docs/**/*.md");
        for path in ["docs/x.md", "docs/design/x.md", "docs/a/b/c.md"] {
            assert!(docs.matches(path), "{path}");
        }
        for path in ["docs/x.txt", "src/docs/x.md", "docs", "x.md"] {
            assert!(!docs.matches(path), "{path}");
        }
        let all = p("**/*.md");
        assert!(all.matches("README.md") && all.matches("src/a/README.md"));
        let tree = p("docs/**");
        assert!(tree.matches("docs/x.txt") && tree.matches("docs/a/b"));
        assert!(!tree.matches("src/x"));
        let mid = p("src/*_PLAN*.md");
        assert!(mid.matches("src/20261027_04_X_PLAN.md"));
        assert!(mid.matches("src/a_PLAN.md"));
        assert!(!mid.matches("src/a_PLA.md"));
        assert!(p("a*a").matches("aa") && !p("a*a").matches("a"));
        assert!(p("Cargo.toml").matches("Cargo.toml") && !p("Cargo.toml").matches("Cargo.lock"));
    }

    #[test]
    fn no_read_means_no_file_tools() {
        let (c, _) = PlanningConfig::from_use(&LlmUseConfig::default());
        assert!(!c.file_tools() && !c.agents_prompt);
        let (c, _) = config(&[], true);
        assert!(!c.file_tools() && c.agents_prompt);
        let (c, _) = config(&["*.md", "docs/**/*.md"], false);
        assert!(c.file_tools());
        assert_eq!(c.read_list(), "\"*.md\", \"docs/**/*.md\"");
    }

    #[test]
    fn the_shipped_prompt_assembles_with_and_without_files() {
        let prompt = PlanningPrompt::load(None).unwrap();
        assert_eq!(prompt.max_instructions_bytes(), 32 * 1024);
        let base = AgentProfile::base();
        assert!(base.system_prompt.contains("reasoning engine for `flankr`"));

        let (no_files, _) = config(&[], true);
        let p = AgentProfile::planning(&prompt, &no_files).unwrap();
        assert!(p.system_prompt.starts_with(&base.system_prompt));
        assert!(p.system_prompt.contains("plan work for the coding agents"));
        assert!(!p.system_prompt.contains("list_files"));
        assert!(!p.tools.file_tools);

        let (files, _) = config(&["*.md", "docs/**/*.md"], false);
        let p = AgentProfile::planning(&prompt, &files).unwrap();
        assert!(p.system_prompt.contains("list_files"));
        assert!(
            p.system_prompt
                .contains("Only files\nmatching \"*.md\", \"docs/**/*.md\" can be read."),
            "{}",
            p.system_prompt
        );
        assert!(!p.system_prompt.contains("{{"));
        assert!(p.tools.file_tools);

        let block = prompt
            .instructions("flankr", "claude", "AGENTS.md", "Be brief.")
            .unwrap();
        assert!(block.starts_with("The current project is flankr, where the claude"));
        assert!(block.ends_with("from AGENTS.md:\n\nBe brief."), "{block}");
    }

    #[test]
    fn a_missing_or_broken_planning_prompt_is_an_error() {
        let (prompts, _) = Prompts::parse("# [Prompt] other\n\n- system: |\n    x\n");
        let err = PlanningPrompt::from_prompts(&prompts).unwrap_err();
        assert!(err.contains("did not load"), "{err}");
        let (prompts, _) =
            Prompts::parse("# [Prompt] planning\n\n- system: |\n    x\n- files: |\n    f\n");
        let err = PlanningPrompt::from_prompts(&prompts).unwrap_err();
        assert!(err.contains("no `instructions`"), "{err}");
        let (prompts, _) = Prompts::parse(
            "# [Prompt] planning\n\n- system: |\n    x {{ [abc] nothing }}\n- files: |\n    f\n- instructions: |\n    i\n",
        );
        let err = PlanningPrompt::from_prompts(&prompts).unwrap_err();
        assert!(err.contains("unknown slot `nothing`"), "{err}");
    }
}
