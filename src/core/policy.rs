//! What may be sent to an agent (plan 20261022_01, decision 6).

use std::fmt;

use crate::commands::validate_arbitrary_command;
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
    /// An arbitrary command that `validate_arbitrary_command` rejected.
    Invalid(String),
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
        }
    }
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
        assert!(err
            .to_string()
            .starts_with("Agent 'codex' (in pane 'p3') is not configured in [[agents]]"));
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
}
