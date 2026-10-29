//! What may be sent to an agent (plan 20261022_01, decision 6).

use std::fmt;

use crate::commands::{check_paths, validate_arbitrary_command, validate_message};
use crate::config::AgentConfig;
use crate::core::principal::Principal;

/// A command that passed the permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The command to type: trimmed if it was on the allow list, cleaned by
    /// `validate_arbitrary_command` otherwise.
    pub command: String,
    /// The agent's `require_confirmation`.
    pub require_confirmation: bool,
}

/// Why a command was refused. `Display` is the message a client sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    /// The agent has no `[[agents]]` entry.
    UnconfiguredAgent {
        agent: String,
        pane_id: String,
        permitted: Vec<String>,
    },
    /// The agent is configured, but this principal may not reach it.
    OutOfReach { agent: String, principal: String },
    /// Not on the allow list, and arbitrary commands are off.
    NotAllowed {
        command: String,
        agent: String,
        allowed: Vec<String>,
    },
    /// An arbitrary command that `validate_arbitrary_command` rejected, or
    /// a message that `validate_message` rejected.
    Invalid(String),
    /// The agent's `allow_messages` is off.
    MessagesOff { agent: String },
}

impl fmt::Display for Denied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Denied::UnconfiguredAgent {
                agent,
                pane_id,
                permitted,
            } => write!(
                f,
                "Agent '{agent}' (in pane '{pane_id}') is not configured in [[agents]]. Permitted agents: {permitted:?}"
            ),
            Denied::OutOfReach { agent, principal } => {
                write!(f, "Agent '{agent}' is not available to {principal}")
            }
            Denied::NotAllowed {
                command,
                agent,
                allowed,
            } => write!(
                f,
                "Command '{command}' is not permitted for agent '{agent}'. Allowed commands for {agent}: {allowed:?}"
            ),
            Denied::Invalid(reason) => f.write_str(reason),
            Denied::MessagesOff { agent } => {
                write!(
                    f,
                    "Messages are not permitted for agent '{agent}' (allow_messages is off)"
                )
            }
        }
    }
}

/// The `[[agents]]` entry for `agent`, if `principal` may reach it.
fn entry<'a>(
    principal: &Principal,
    agents: &'a [AgentConfig],
    agent: &str,
    pane_id: &str,
) -> Result<&'a AgentConfig, Denied> {
    let Some(agent_cfg) = agents.iter().find(|a| a.name.to_lowercase() == agent) else {
        return Err(Denied::UnconfiguredAgent {
            agent: agent.to_string(),
            pane_id: pane_id.to_string(),
            permitted: agents.iter().map(|a| a.name.clone()).collect(),
        });
    };
    if !principal.may_reach(agent) {
        return Err(Denied::OutOfReach {
            agent: agent.to_string(),
            principal: principal.id.clone(),
        });
    }
    Ok(agent_cfg)
}

/// The `[[agents]]` entry of the agent named `agent` (lowercased) in
/// `pane_id`, when it is configured and `principal` may reach it: the
/// check every send starts with, and all a terminal needs to open (plan
/// 20261027_07, decision 6).
pub fn reach<'a>(
    principal: &Principal,
    agents: &'a [AgentConfig],
    agent: &str,
    pane_id: &str,
) -> Result<&'a AgentConfig, Denied> {
    entry(principal, agents, agent, pane_id)
}

/// Check `command` for the agent named `agent` (lowercased) in `pane_id`,
/// as `principal`, against the `[[agents]]` entries.
pub fn check(
    principal: &Principal,
    agents: &[AgentConfig],
    agent: &str,
    pane_id: &str,
    command: &str,
) -> Result<Checked, Denied> {
    let agent_cfg = entry(principal, agents, agent, pane_id)?;

    let is_explicitly_allowed = agent_cfg
        .allowed_commands
        .iter()
        .any(|allowed| allowed.trim().eq_ignore_ascii_case(command.trim()));

    let command = if is_explicitly_allowed {
        command.trim().to_string()
    } else if agent_cfg.allow_arbitrary_commands {
        validate_arbitrary_command(command).map_err(Denied::Invalid)?
    } else {
        return Err(Denied::NotAllowed {
            command: command.to_string(),
            agent: agent.to_string(),
            allowed: agent_cfg.allowed_commands.clone(),
        });
    };

    Ok(Checked {
        command,
        require_confirmation: agent_cfg.require_confirmation,
    })
}

/// Whether `command` is on the allow list of the agent named `agent`
/// (lowercased): `send_agent_command` checks such text with [`check`], and
/// any other with [`check_message`] (plan 20261027_04, decision 1).
pub fn is_allow_listed(agents: &[AgentConfig], agent: &str, command: &str) -> bool {
    agents
        .iter()
        .find(|a| a.name.to_lowercase() == agent)
        .is_some_and(|a| {
            a.allowed_commands
                .iter()
                .any(|allowed| allowed.trim().eq_ignore_ascii_case(command.trim()))
        })
}

/// Check that a choice question's answer may be given (plan 20261029_01,
/// decision 6): the agent's entry and reach as for a command, and its
/// `allow_messages`, an answer being the user telling the agent something.
/// Each Other text is checked with [`check_message`] as well.
pub fn check_answer(
    principal: &Principal,
    agents: &[AgentConfig],
    agent: &str,
    pane_id: &str,
) -> Result<(), Denied> {
    let agent_cfg = entry(principal, agents, agent, pane_id)?;
    if !agent_cfg.allow_messages {
        return Err(Denied::MessagesOff {
            agent: agent.to_string(),
        });
    }
    Ok(())
}

/// Check a dictated message (plan 20261023_02, decision 6 as amended):
/// the agent's entry and reach as for a command, its `allow_messages`, then
/// `validate_message`, and `check_paths` against `root`, the agent's project
/// folder (herdr's `cwd` for its pane). A message is always confirmed,
/// whatever the agent's `require_confirmation`.
pub fn check_message(
    principal: &Principal,
    agents: &[AgentConfig],
    agent: &str,
    pane_id: &str,
    text: &str,
    root: Option<&str>,
) -> Result<Checked, Denied> {
    let agent_cfg = entry(principal, agents, agent, pane_id)?;
    if !agent_cfg.allow_messages {
        return Err(Denied::MessagesOff {
            agent: agent.to_string(),
        });
    }
    let command = validate_message(text).map_err(Denied::Invalid)?;
    check_paths(&command, root).map_err(Denied::Invalid)?;
    Ok(Checked {
        command,
        require_confirmation: true,
    })
}

/// A message in parts, checked (plan 20261028_05, decision 7 and the 21:29
/// EDT amendment, item 4): what [`check_parts`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedParts {
    /// The parts in order, each text part cleaned (runs of space folded,
    /// surrounding space trimmed; empty ones dropped), each image part its
    /// reference as given.
    pub parts: Vec<Part>,
    /// The user's words joined, as checked; empty for an image-only send.
    pub text: String,
    /// Always true: a message is always confirmed.
    pub require_confirmation: bool,
}

/// One part of a [`CheckedParts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Image(String),
}

/// Most parts in one send, and most images.
pub const PARTS_MAX: usize = libflankr_blob::PARTS_PER_SEND;
pub const IMAGES_MAX: usize = libflankr_blob::IMAGES_PER_SEND;

/// Check a message in parts: every send with images needs the agent's
/// entry, reach and `allow_messages`, not `allow_arbitrary_commands`; the
/// text parts, joined with one space, are checked as one message by
/// `validate_message` and `check_paths` against `root`, exactly as
/// [`check_message`] checks a typed one, unless there is no text at all
/// (an image-only send). The image paths are inserted by the caller after
/// this check and are never run through `check_paths`.
pub fn check_parts(
    principal: &Principal,
    agents: &[AgentConfig],
    agent: &str,
    pane_id: &str,
    parts: &[Part],
    root: Option<&str>,
) -> Result<CheckedParts, Denied> {
    let agent_cfg = entry(principal, agents, agent, pane_id)?;
    if !agent_cfg.allow_messages {
        return Err(Denied::MessagesOff {
            agent: agent.to_string(),
        });
    }
    if parts.len() > PARTS_MAX {
        return Err(Denied::Invalid(format!(
            "Message rejected: more than {PARTS_MAX} parts."
        )));
    }
    let images = parts.iter().filter(|p| matches!(p, Part::Image(_))).count();
    if images > IMAGES_MAX {
        return Err(Denied::Invalid(format!(
            "Message rejected: more than {IMAGES_MAX} images."
        )));
    }
    let mut cleaned = Vec::new();
    for part in parts {
        match part {
            Part::Text(t) => {
                if let Some(bad) = t.chars().find(|c| c.is_control()) {
                    return Err(Denied::Invalid(format!(
                        "Message rejected: control characters (found {bad:?}) are not permitted."
                    )));
                }
                let folded = t.split_whitespace().collect::<Vec<_>>().join(" ");
                if !folded.is_empty() {
                    cleaned.push(Part::Text(folded));
                }
            }
            Part::Image(r) => cleaned.push(Part::Image(r.clone())),
        }
    }
    let joined = cleaned
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.as_str()),
            Part::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text = if joined.is_empty() {
        if images == 0 {
            return Err(Denied::Invalid("Message cannot be empty.".to_string()));
        }
        String::new()
    } else {
        let text = validate_message(&joined).map_err(Denied::Invalid)?;
        check_paths(&text, root).map_err(Denied::Invalid)?;
        text
    };
    Ok(CheckedParts {
        parts: cleaned,
        text,
        require_confirmation: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents() -> Vec<AgentConfig> {
        vec![
            AgentConfig::new(
                "claude",
                vec!["continue".to_string(), "/compact".to_string()],
                true,
            ),
            AgentConfig::new("omp", vec!["continue".to_string()], false)
                .with_arbitrary_commands(true),
        ]
    }

    #[test]
    fn allow_listed_commands_pass_trimmed_and_case_insensitively() {
        let checked = check(
            &Principal::local("t"),
            &agents(),
            "claude",
            "p1",
            " /COMPACT ",
        )
        .unwrap();
        assert_eq!(checked.command, "/COMPACT");
        assert!(checked.require_confirmation);
    }

    #[test]
    fn arbitrary_commands_are_validated() {
        let checked = check(
            &Principal::local("t"),
            &agents(),
            "omp",
            "p2",
            "run the tests.",
        )
        .unwrap();
        // The validator strips trailing sentence punctuation.
        assert_eq!(checked.command, "run the tests");
        assert!(!checked.require_confirmation);
        let err = check(&Principal::local("t"), &agents(), "omp", "p2", "rm -rf /").unwrap_err();
        assert!(matches!(err, Denied::Invalid(_)));
    }

    #[test]
    fn messages_are_validated_and_always_confirmed() {
        let p = Principal::local("t");
        // omp does not require confirmation for commands; a message still does.
        let checked =
            check_message(&p, &agents(), "omp", "p2", "please run  the tests.", None).unwrap();
        assert_eq!(checked.command, "please run the tests");
        assert!(checked.require_confirmation);
        let err = check_message(&p, &agents(), "omp", "p2", "rm -rf /", None).unwrap_err();
        assert!(matches!(err, Denied::Invalid(_)), "{err}");
        // A command as the first word, even after spaces, needs the allow list.
        let err = check_message(&p, &agents(), "omp", "p2", "  /status", None).unwrap_err();
        assert!(err.to_string().contains("makes it a command"), "{err}");
        // Paths: inside the project, kept; outside, refused.
        let checked = check_message(
            &p,
            &agents(),
            "omp",
            "p2",
            "fix src/main.rs --all",
            Some("/work/x"),
        )
        .unwrap();
        assert_eq!(checked.command, "fix src/main.rs --all");
        let err =
            check_message(&p, &agents(), "omp", "p2", "read ../y/z", Some("/work/x")).unwrap_err();
        assert!(err.to_string().contains("outside the project"), "{err}");
        let err = check_message(&p, &agents(), "codex", "p3", "hello", None).unwrap_err();
        assert!(matches!(err, Denied::UnconfiguredAgent { .. }));
        let off = vec![AgentConfig::new("claude", vec![], true).with_messages(false)];
        let err = check_message(&p, &off, "claude", "p1", "hello", None).unwrap_err();
        assert_eq!(
            err,
            Denied::MessagesOff {
                agent: "claude".to_string()
            }
        );
        let narrowed = Principal {
            agents: Some(vec!["omp".to_string()]),
            ..Principal::local("mcp")
        };
        let err = check_message(&narrowed, &agents(), "claude", "p1", "hello", None).unwrap_err();
        assert!(matches!(err, Denied::OutOfReach { .. }));
    }

    #[test]
    fn refusals_keep_their_messages() {
        let err = check(
            &Principal::local("t"),
            &agents(),
            "claude",
            "p1",
            "rm -rf /",
        )
        .unwrap_err();
        assert!(err.to_string().contains("not permitted for agent 'claude'"));
        let err = check(&Principal::local("t"), &agents(), "codex", "p3", "continue").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("Agent 'codex' (in pane 'p3') is not configured in [[agents]]")
        );
    }

    #[test]
    fn a_principal_agent_list_narrows_but_never_widens() {
        let narrowed = Principal {
            agents: Some(vec!["omp".to_string(), "codex".to_string()]),
            ..Principal::local("mcp")
        };
        let err = check(&narrowed, &agents(), "claude", "p1", "continue").unwrap_err();
        assert_eq!(
            err,
            Denied::OutOfReach {
                agent: "claude".to_string(),
                principal: "local:mcp".to_string()
            }
        );
        // On the principal's list but not in [[agents]]: still unconfigured.
        let err = check(&narrowed, &agents(), "codex", "p3", "continue").unwrap_err();
        assert!(matches!(err, Denied::UnconfiguredAgent { .. }));
    }

    // Plan 20261028_05, step 6: a message in parts.
    #[test]
    fn parts_need_allow_messages_and_check_the_text_as_one_message() {
        let p = Principal::local("t");
        let img = |r: &str| Part::Image(r.to_string());
        let txt = |t: &str| Part::Text(t.to_string());
        // claude: no arbitrary commands; messages on (the default).
        let checked = check_parts(&p, &agents(), "claude", "p1", &[img("a")], None).unwrap();
        assert_eq!(checked.text, "");
        assert_eq!(checked.parts, [img("a")]);
        assert!(checked.require_confirmation);
        // allow_messages off: refused, image-only or not.
        let off = vec![AgentConfig::new("claude", vec![], true).with_messages(false)];
        for parts in [vec![img("a")], vec![txt("look"), img("a")]] {
            assert_eq!(
                check_parts(&p, &off, "claude", "p1", &parts, None).unwrap_err(),
                Denied::MessagesOff {
                    agent: "claude".to_string()
                }
            );
        }
        // The text, joined, is checked as a typed message is.
        let checked = check_parts(
            &p,
            &agents(),
            "claude",
            "p1",
            &[txt("  look   at "), img("a"), txt(""), txt("and src/x.rs")],
            Some("/work/x"),
        )
        .unwrap();
        assert_eq!(checked.text, "look at and src/x.rs");
        assert_eq!(
            checked.parts,
            [txt("look at"), img("a"), txt("and src/x.rs")]
        );
        // A typed path into the temp folder is still outside the project.
        let err = check_parts(
            &p,
            &agents(),
            "claude",
            "p1",
            &[txt("see /tmp/flankr-501/images/p1/x.png"), img("a")],
            Some("/work/x"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("outside the project"), "{err}");
        // The first-word rule applies to the user's words as a whole.
        let err = check_parts(
            &p,
            &agents(),
            "claude",
            "p1",
            &[img("a"), txt("/compact")],
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("makes it a command"), "{err}");
        // Control characters, limits, nothing at all.
        assert!(
            check_parts(
                &p,
                &agents(),
                "claude",
                "p1",
                &[txt("a\nb"), img("a")],
                None
            )
            .is_err()
        );
        let five: Vec<Part> = (0..5).map(|i| img(&i.to_string())).collect();
        assert!(check_parts(&p, &agents(), "claude", "p1", &five, None).is_err());
        let many: Vec<Part> = (0..17).map(|_| txt("a")).collect();
        assert!(check_parts(&p, &agents(), "claude", "p1", &many, None).is_err());
        assert!(check_parts(&p, &agents(), "claude", "p1", &[txt("  ")], None).is_err());
        // Reach and entry as for any send.
        assert!(matches!(
            check_parts(&p, &agents(), "codex", "p3", &[img("a")], None),
            Err(Denied::UnconfiguredAgent { .. })
        ));
    }
}
