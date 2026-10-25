//! `ADDITIONAL_PROMPTS.md`: prompts other than the main system prompt, read
//! with libtypedmd (docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md,
//! decision 7).
//!
//! Each prompt is a `# [Prompt] <name>` object. Its members are its
//! settings; `system` and `user` are literal blocks whose
//! `{{ [sigil] name }}` slots are filled by [`Prompt::fill`]. libtypedmd
//! takes a literal block verbatim, slots included, so the slots are found
//! here.

use std::collections::BTreeMap;

use libtypedmd::{Node, NodeKind};

/// The shipped prompts, compiled in as `PROMPT.md` is.
pub const ADDITIONAL_PROMPTS_RAW: &str = include_str!("../ADDITIONAL_PROMPTS.md");

/// The tag that marks a prompt object.
pub const PROMPT_TAG: &str = "Prompt";

/// The spoken summary of a complex reply (plan 20261023_05, decision 8).
pub const SPOKEN_SUMMARY: &str = "spoken_summary";

/// One prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub name: String,
    /// Every member but `system` and `user`, by name, its value as written.
    /// Members the daemon does not use are kept and ignored.
    pub settings: BTreeMap<String, String>,
    /// The system text, with its slots unfilled.
    pub system: String,
    /// The user message, with its slots unfilled.
    pub user: String,
}

/// The prompts of one file, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompts {
    by_name: BTreeMap<String, Prompt>,
}

impl Prompts {
    /// Every `[Prompt]` object in `text`. Errors are per prompt: a prompt
    /// that has one is left out and the rest load.
    pub fn parse(text: &str) -> (Prompts, Vec<String>) {
        let stream = libtypedmd::parse(text);
        let mut prompts = Prompts::default();
        let mut errors = Vec::new();
        for document in &stream.documents {
            collect(&document.nodes, &mut prompts, &mut errors);
        }
        (prompts, errors)
    }

    /// The shipped `ADDITIONAL_PROMPTS.md`.
    pub fn shipped() -> (Prompts, Vec<String>) {
        Self::parse(ADDITIONAL_PROMPTS_RAW)
    }

    /// `path` in place of the shipped file (`--prompts <path>`).
    pub fn load(path: &std::path::Path) -> Result<(Prompts, Vec<String>), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read prompts file {}: {e}", path.display()))?;
        Ok(Self::parse(&text))
    }

    pub fn get(&self, name: &str) -> Option<&Prompt> {
        self.by_name.get(name)
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Prompt objects among `nodes`, at any depth.
fn collect(nodes: &[Node], prompts: &mut Prompts, errors: &mut Vec<String>) {
    for node in nodes {
        let Some(object) = node.as_object() else {
            continue;
        };
        if object.tag.as_deref() != Some(PROMPT_TAG) {
            collect(&node.nodes, prompts, errors);
            continue;
        }
        let name = object.head.render().trim().to_string();
        match prompt(&name, &node.nodes) {
            Ok(_) if prompts.by_name.contains_key(&name) => {
                errors.push(format!("prompt `{name}`: defined twice; the first is kept"));
            }
            Ok(p) => {
                prompts.by_name.insert(name, p);
            }
            Err(e) => errors.push(format!("prompt `{name}`: {e}")),
        }
    }
}

fn prompt(name: &str, children: &[Node]) -> Result<Prompt, String> {
    if name.is_empty() {
        return Err("it has no name".to_string());
    }
    let mut settings = BTreeMap::new();
    let mut system = None;
    let mut user = None;
    for child in children {
        let NodeKind::Member(member) = &child.kind else {
            continue;
        };
        let value = member.rhs.as_ref().map(|t| t.render()).unwrap_or_default();
        match member.name.as_str() {
            "system" => system = Some(value),
            "user" => user = Some(value),
            other => {
                settings.insert(other.to_string(), value.trim().to_string());
            }
        }
    }
    Ok(Prompt {
        name: name.to_string(),
        settings,
        system: system.ok_or("it has no `system`")?,
        user: user.ok_or("it has no `user`")?,
    })
}

impl Prompt {
    /// `text` with each `{{ [sigil] name }}` replaced from `values`, then
    /// from the prompt's settings; an unknown name is an error.
    pub fn fill(&self, text: &str, values: &BTreeMap<&str, String>) -> Result<String, String> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after
                .find("}}")
                .ok_or_else(|| format!("prompt `{}`: a `{{{{` is not closed", self.name))?;
            let name = slot_name(&after[..end]);
            let value = values
                .get(name)
                .map(String::as_str)
                .or_else(|| self.settings.get(name).map(String::as_str))
                .ok_or_else(|| format!("prompt `{}`: unknown slot `{name}`", self.name))?;
            out.push_str(value);
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// A setting read as a number (`- [#] max_words: 60`).
    pub fn number(&self, name: &str) -> Option<f64> {
        self.settings.get(name)?.parse().ok()
    }
}

/// A slot's expression without its sigil: `[abc] agent` is `agent`.
fn slot_name(inner: &str) -> &str {
    let inner = inner.trim();
    match inner.strip_prefix('[').and_then(|s| s.split_once(']')) {
        Some((_, name)) => name.trim(),
        None => inner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&'static str, &str)]) -> BTreeMap<&'static str, String> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn the_shipped_file_parses() {
        let (prompts, errors) = Prompts::shipped();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(prompts.len(), 1);
        let p = prompts.get(SPOKEN_SUMMARY).expect("spoken_summary");
        assert_eq!(p.settings.get("max_words").map(String::as_str), Some("60"));
        assert_eq!(p.number("max_words"), Some(60.0));
        assert_eq!(p.number("temperature"), Some(0.2));
        assert!(p.system.starts_with("You turn a coding agent's reply"));
        assert!(p.system.contains("{{ [#] max_words }}"));
        assert!(p.user.ends_with("{{ [abc] reply }}"));
    }

    #[test]
    fn spoken_summary_fills() {
        let (prompts, _) = Prompts::shipped();
        let p = prompts.get(SPOKEN_SUMMARY).unwrap();
        let system = p.fill(&p.system, &BTreeMap::new()).unwrap();
        assert!(system.contains("at most 60 words."), "{system}");
        assert!(!system.contains("{{"));
        let v = values(&[
            ("agent", "claude"),
            ("project", "flankr"),
            ("reply", "Done: `src/prompts.rs` added."),
        ]);
        let user = p.fill(&p.user, &v).unwrap();
        assert_eq!(
            user,
            "The claude agent in flankr replied:\n\nDone: `src/prompts.rs` added."
        );
    }

    #[test]
    fn values_come_before_settings() {
        let (prompts, _) = Prompts::shipped();
        let p = prompts.get(SPOKEN_SUMMARY).unwrap();
        let v = values(&[("max_words", "20")]);
        assert!(p.fill(&p.system, &v).unwrap().contains("at most 20 words."));
    }

    #[test]
    fn a_prompt_missing_user_is_reported_and_the_rest_load() {
        let text = "# [Prompt] broken\n\n- system: |\n    hi\n\n# [Prompt] fine\n\n- system: |\n    a\n- user: |\n    b\n";
        let (prompts, errors) = Prompts::parse(text);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("broken") && errors[0].contains("user"));
        assert!(prompts.get("broken").is_none());
        assert_eq!(prompts.get("fine").unwrap().user, "b");
    }

    #[test]
    fn an_unknown_slot_name_is_an_error() {
        let (prompts, _) = Prompts::shipped();
        let p = prompts.get(SPOKEN_SUMMARY).unwrap();
        let err = p
            .fill(&p.user, &values(&[("agent", "claude")]))
            .unwrap_err();
        assert!(err.contains("unknown slot `project`"), "{err}");
        assert!(p.fill("{{ [abc] reply", &BTreeMap::new()).is_err());
    }

    #[test]
    fn an_unknown_member_is_kept() {
        let text = "# [Prompt] p\n\n- [abc] voice: low\n- system: |\n    s {{ voice }}\n- user: |\n    u\n";
        let (prompts, errors) = Prompts::parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        let p = prompts.get("p").unwrap();
        assert_eq!(p.settings.get("voice").map(String::as_str), Some("low"));
        assert_eq!(p.fill(&p.system, &BTreeMap::new()).unwrap(), "s low");
    }

    #[test]
    fn untagged_objects_are_not_prompts() {
        let text = "# Notes\n\n- system: |\n    x\n- user: |\n    y\n";
        let (prompts, errors) = Prompts::parse(text);
        assert!(prompts.is_empty() && errors.is_empty());
    }
}
