//! Who is asking (plan 20261022_01, decision 4).

/// The identity a request runs as, fixed when its connection says `hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// `local:<client>` or `paired:<name>`.
    pub id: String,
    /// Agents this principal may reach. `None` leaves `[[agents]]` as it is;
    /// a list narrows it, and can never widen it.
    pub agents: Option<Vec<String>>,
    /// Whether this principal may approve confirmations.
    pub can_approve: bool,
}

impl Principal {
    /// A local principal: every configured agent, and no approving.
    pub fn local(client: &str) -> Self {
        Self {
            id: format!("local:{client}"),
            agents: None,
            can_approve: false,
        }
    }

    /// Whether this principal may reach the agent named `agent` (lowercased).
    pub fn may_reach(&self, agent: &str) -> bool {
        match &self.agents {
            None => true,
            Some(allowed) => allowed.iter().any(|a| a.eq_ignore_ascii_case(agent)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_principals_reach_every_agent_and_cannot_approve() {
        let p = Principal::local("ctl");
        assert_eq!(p.id, "local:ctl");
        assert!(p.may_reach("claude"));
        assert!(!p.can_approve);
    }

    #[test]
    fn an_agent_list_narrows() {
        let p = Principal {
            agents: Some(vec!["Claude".to_string()]),
            ..Principal::local("mcp")
        };
        assert!(p.may_reach("claude"));
        assert!(!p.may_reach("codex"));
    }
}
