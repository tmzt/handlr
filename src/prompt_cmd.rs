//! `flankr prompt [--use general_agent] [--agent <name>] [--prompts <path>]`
//! (docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md, decision 8):
//! the general agent's system prompt as the daemon would assemble it from
//! this config, its tools' names, and, with `--agent`, the `instructions`
//! block for that agent's project. It reads the config, the keys' presence
//! and herdr; it calls no LLM.

use std::fmt::Write as _;
use std::path::Path;

use crate::config::AppConfig;
use crate::core::naming::Names;
use crate::llm::{self, AgentProfile, KeyEnv, PlanningPrompt, Use, UseState};
use crate::project::ProjectFiles;
use crate::reasoning::{declare_gemini_tools_for, tool_names};
use crate::transport::HerdrClient;

/// What `flankr prompt` was asked for.
#[derive(Debug, Clone, Default)]
pub struct PromptOptions<'a> {
    /// `--use`: only `general_agent` has an assembled prompt.
    pub use_name: Option<&'a str>,
    /// `--agent`: a name, a tab label or a pane id.
    pub agent: Option<&'a str>,
    /// `--prompts`: in place of the shipped `ADDITIONAL_PROMPTS.md`.
    pub prompts_file: Option<&'a Path>,
}

/// The report `flankr prompt` prints.
pub async fn report(
    config: &AppConfig,
    env: &KeyEnv,
    client: &HerdrClient,
    options: &PromptOptions<'_>,
) -> Result<String, String> {
    let use_name = options.use_name.unwrap_or(Use::GeneralAgent.name());
    if use_name != Use::GeneralAgent.name() {
        return Err(format!(
            "`{use_name}`: only general_agent's prompt is assembled (the other uses keep \
             their own prompts in ADDITIONAL_PROMPTS.md)"
        ));
    }
    let settings = llm::resolve(config, env);
    let prompt = PlanningPrompt::load(options.prompts_file);
    let planning = match (settings.state(Use::Planning), &prompt) {
        (UseState::Available(_), Ok(prompt)) => Ok(prompt),
        (UseState::Available(_), Err(e)) => Err(format!("planning is off: {e}")),
        (UseState::Unavailable { reason, .. }, _) => Err(reason.clone()),
    };
    let profile = match &planning {
        Ok(prompt) => AgentProfile::planning(prompt, &settings.planning)?,
        Err(_) => AgentProfile::base(),
    };

    let mut out = String::new();
    let _ = writeln!(out, "# flankr prompt: {use_name}");
    match &planning {
        Ok(_) => {
            let _ = writeln!(
                out,
                "# planning: on (read: {}; agents_prompt: {})",
                if settings.planning.file_tools() {
                    settings.planning.read_list()
                } else {
                    "none, so no file tools".to_string()
                },
                if settings.planning.agents_prompt {
                    "on"
                } else {
                    "off"
                }
            );
        }
        Err(reason) => {
            let _ = writeln!(out, "# planning: off ({reason})");
        }
    }
    let _ = writeln!(out, "\n## System prompt\n\n{}", profile.system_prompt);
    let tools = tool_names(&declare_gemini_tools_for(profile.tools));
    let _ = writeln!(out, "\n## Tools\n\n{}", tools.join(", "));

    if let Some(agent) = options.agent {
        let prompt = prompt.as_ref().map_err(|e| e.clone())?;
        let _ = writeln!(
            out,
            "\n{}",
            instructions(config, client, prompt, agent).await?
        );
        if !(planning.is_ok() && settings.planning.agents_prompt) {
            let _ = writeln!(
                out,
                "\n(agents_prompt is not on with planning, so turns do not get this block.)"
            );
        }
    }
    Ok(out)
}

/// The `instructions` block for `agent`'s project, under a heading.
async fn instructions(
    config: &AppConfig,
    client: &HerdrClient,
    prompt: &PlanningPrompt,
    agent: &str,
) -> Result<String, String> {
    let snapshot = client
        .list_panes()
        .await
        .map_err(|e| format!("herdr is not answering: {e}"))?;
    let names = Names::from_snapshot(&snapshot, &config.naming);
    let named = names.resolve(agent).map_err(|e| e.to_string())?;
    let cwd = named
        .agent
        .cwd
        .clone()
        .ok_or_else(|| format!("herdr does not say where {} works", named.shown))?;
    let project = named.agent.project.clone().unwrap_or_else(|| cwd.clone());
    let heading = format!("## Instructions for {} ({cwd})", named.shown);
    let files = ProjectFiles::open(&cwd, Vec::new())?;
    let max = prompt.max_instructions_bytes();
    let Some((file, mut text, cut)) = files.instructions(max) else {
        return Ok(format!(
            "{heading}\n\nNo AGENTS.md, CLAUDE.md or GEMINI.md in {cwd}."
        ));
    };
    if cut {
        text.push_str(&format!("\n\n(cut at {} KiB)", max / 1024));
    }
    let agent_name = crate::core::naming::capitalise(&named.agent.agent);
    let block = prompt.instructions(&project, &agent_name, &file, &text)?;
    Ok(format!("{heading}\n\n{block}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HerdrTransport;

    fn write_key(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(llm::gemini::KEY_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "test-key").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    const PLANNING: &str = r#"
[llm]
enabled = true
[llm.uses.planning]
enabled = true
agents_prompt = true
read = ["*.md", "docs/**/*.md"]
"#;

    #[tokio::test]
    async fn it_prints_the_assembled_prompt_tools_and_instructions() {
        let dir = tempfile::tempdir().unwrap();
        write_key(dir.path());
        let project = dir.path().join("example");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("AGENTS.md"), "Keep plans frozen.").unwrap();
        let client = HerdrClient::mock();
        if let HerdrTransport::Mock { state } = client.transport() {
            state.write().await.snapshot.workspaces[0].tabs[0].panes[0].cwd =
                Some(project.display().to_string());
        }
        let config: AppConfig = toml::from_str(PLANNING).unwrap();
        let env = KeyEnv::in_dir(dir.path());

        let out = report(
            &config,
            &env,
            &client,
            &PromptOptions {
                agent: Some("claude"),
                ..PromptOptions::default()
            },
        )
        .await
        .unwrap();
        assert!(out.starts_with("# flankr prompt: general_agent\n# planning: on"));
        assert!(out.contains("reasoning engine for `flankr`"));
        assert!(out.contains("plan work for the coding agents"));
        assert!(out.contains("matching \"*.md\", \"docs/**/*.md\" can be read"));
        assert!(out.contains(
            "## Tools\n\nfocus, focus_project, send_agent_command, watch_pane, unwatch_pane, \
             list_files, read_file"
        ));
        assert!(out.contains("## Instructions for Claude"), "{out}");
        assert!(
            out.contains("from AGENTS.md:\n\nKeep plans frozen."),
            "{out}"
        );
        assert!(!out.contains("turns do not get this block"));
        assert!(!out.contains("test-key"));

        // Without planning: PROMPT.md and the base tools only.
        let off = report(
            &toml::from_str("[llm]\nenabled = true\n").unwrap(),
            &env,
            &client,
            &PromptOptions::default(),
        )
        .await
        .unwrap();
        assert!(off.contains("# planning: off ([llm.uses.planning] is off"));
        assert!(!off.contains("plan work for the coding agents"));
        assert!(off.ends_with("watch_pane, unwatch_pane\n"), "{off}");

        // Only the general agent's prompt is assembled.
        let err = report(
            &config,
            &env,
            &client,
            &PromptOptions {
                use_name: Some("spoken_summaries"),
                ..PromptOptions::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("only general_agent"));
    }
}
