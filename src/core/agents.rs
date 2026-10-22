//! Agents as herdr's snapshot describes them (plan 20261022_01, decision 5).
//!
//! herdr reports each pane twice, flat under `panes` and nested under
//! workspaces -> tabs, so everything here works from `collect_panes` and
//! keys on `pane_id`.

use std::collections::HashMap;

use libherdr::protocol::AgentStatus;

/// One pane and the agent running in it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRef {
    /// Canonical, as `resolve_pane_id` returns it (`w1:p6`).
    pub pane_id: String,
    /// Lowercased, as `extract_agent_name_for_pane` resolves it; `shell`
    /// when the pane runs no known agent.
    pub agent: String,
    /// The workspace label, else the cwd's basename.
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub status: AgentStatus,
    pub focused: bool,
}

/// Every pane in the snapshot, once each, as an `AgentRef`.
pub fn agents(snapshot: &serde_json::Value) -> Vec<AgentRef> {
    let labels = workspace_labels(snapshot);
    let mut refs: Vec<AgentRef> = Vec::new();
    for pane in collect_panes(snapshot) {
        let Some(pane_id) = pane.get("pane_id").and_then(|v| v.as_str()) else {
            continue;
        };
        if refs.iter().any(|r| r.pane_id == pane_id) {
            continue;
        }
        let text = |key: &str| pane.get(key).and_then(|v| v.as_str()).map(str::to_string);
        refs.push(AgentRef {
            pane_id: pane_id.to_string(),
            agent: agent_name_of(pane).unwrap_or_else(|| "shell".to_string()),
            project: project_for(pane, &labels),
            cwd: text("cwd"),
            workspace_id: text("workspace_id"),
            tab_id: text("tab_id"),
            status: pane
                .get("agent_status")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            focused: pane
                .get("focused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }
    refs
}

/// Workspace id -> label, for workspaces that have a non-empty label.
pub fn workspace_labels(snapshot: &serde_json::Value) -> HashMap<String, String> {
    let root = snapshot_root(snapshot);
    let mut labels: HashMap<String, String> = HashMap::new();
    if let Some(workspaces) = root.get("workspaces").and_then(|w| w.as_array()) {
        for ws in workspaces {
            let id = ws.get("workspace_id").and_then(|v| v.as_str());
            let label = ws
                .get("label")
                .or_else(|| ws.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let (Some(id), Some(label)) = (id, label) {
                labels.insert(id.to_string(), label.to_string());
            }
        }
    }
    labels
}

/// The project a pane sits in: its workspace's label, or the basename of its
/// working directory when the workspace is unlabelled.
pub fn project_for(pane: &serde_json::Value, labels: &HashMap<String, String>) -> Option<String> {
    pane.get("workspace_id")
        .and_then(|v| v.as_str())
        .and_then(|ws| labels.get(ws).cloned())
        .or_else(|| {
            // Unlabelled workspace: the repo directory names it well enough.
            pane.get("cwd")
                .and_then(|v| v.as_str())
                .and_then(|cwd| cwd.trim_end_matches('/').rsplit('/').next())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

/// Unwrap the session snapshot from whatever envelope it arrived in.
pub fn snapshot_root(snapshot: &serde_json::Value) -> &serde_json::Value {
    snapshot
        .get("result")
        .and_then(|r| r.get("snapshot").or(Some(r)))
        .or_else(|| snapshot.get("snapshot"))
        .unwrap_or(snapshot)
}

/// Collect every pane object in a session snapshot, from both the flat `panes`
/// list and the nested workspaces -> tabs -> panes hierarchy.
pub fn collect_panes(snapshot: &serde_json::Value) -> Vec<&serde_json::Value> {
    let root = snapshot_root(snapshot);
    let mut panes = Vec::new();

    if let Some(flat) = root.get("panes").and_then(|p| p.as_array()) {
        panes.extend(flat.iter());
    }
    if let Some(workspaces) = root.get("workspaces").and_then(|w| w.as_array()) {
        for ws in workspaces {
            if let Some(tabs) = ws.get("tabs").and_then(|t| t.as_array()) {
                for tab in tabs {
                    if let Some(tab_panes) = tab.get("panes").and_then(|p| p.as_array()) {
                        panes.extend(tab_panes.iter());
                    }
                }
            }
            if let Some(ws_panes) = ws.get("panes").and_then(|p| p.as_array()) {
                panes.extend(ws_panes.iter());
            }
        }
    }
    panes
}

/// Whether a pane object is the one the caller meant, tolerating the short
/// (`p6`) and fully-qualified (`w1:p6`) spellings of an id.
fn pane_matches(pane: &serde_json::Value, target: &str) -> bool {
    pane.get("pane_id")
        .and_then(|id| id.as_str())
        .is_some_and(|pid| crate::watch::pane_ids_match(pid, target))
}

/// Resolve a pane id to the fully-qualified form herdr uses (`w1:p6`).
///
/// Event subscriptions are matched by exact pane id server-side, so a watch
/// registered as "p6" would silently never fire.
pub fn resolve_pane_id(snapshot: &serde_json::Value, target: &str) -> Option<String> {
    let clean = target.trim();
    if clean.is_empty() {
        return None;
    }
    collect_panes(snapshot)
        .into_iter()
        .find(|pane| pane_matches(pane, clean))
        .and_then(|pane| pane.get("pane_id"))
        .and_then(|id| id.as_str())
        .map(|id| id.to_string())
}

/// Extracts the lowercased agent name for a target pane from Herdr's session snapshot.
pub fn extract_agent_name_for_pane(snapshot: &serde_json::Value, target_pane_id: &str) -> String {
    let clean_target = target_pane_id.trim();

    for pane in collect_panes(snapshot) {
        if !pane_matches(pane, clean_target) {
            continue;
        }
        if let Some(agent) = agent_name_of(pane) {
            return agent;
        }
    }

    "shell".to_string()
}

/// The lowercased agent name of one pane object: its `agent`, else
/// `extra.agent`, else a known agent named in its terminal title.
fn agent_name_of(pane: &serde_json::Value) -> Option<String> {
    if let Some(agent) = pane.get("agent").and_then(|a| a.as_str()) {
        let lower = agent.trim().to_lowercase();
        if !lower.is_empty() {
            return Some(lower);
        }
    }
    if let Some(extra) = pane.get("extra").and_then(|e| e.as_object()) {
        if let Some(agent) = extra.get("agent").and_then(|a| a.as_str()) {
            let lower = agent.trim().to_lowercase();
            if !lower.is_empty() {
                return Some(lower);
            }
        }
    }
    if let Some(title) = pane.get("terminal_title").and_then(|t| t.as_str()) {
        let lower_title = title.to_lowercase();
        for known in ["claude", "omp", "agy"] {
            if lower_title.contains(known) {
                return Some(known.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_agent_name_for_pane() {
        let nested_snap = json!({
            "snapshot": {
                "panes": [
                    { "pane_id": "w2:p1", "agent": "claude", "cwd": "/Users/tmeade/src/highbay" },
                    { "pane_id": "w2:pA", "agent": "agy", "cwd": "/Users/tmeade/src/highbay" }
                ],
                "workspaces": [
                    {
                        "workspace_id": "w2",
                        "tabs": [
                            {
                                "tab_id": "w2:t1",
                                "panes": [
                                    { "pane_id": "w2:p1", "agent": "claude" }
                                ]
                            }
                        ]
                    }
                ]
            }
        });

        assert_eq!(extract_agent_name_for_pane(&nested_snap, "w2:p1"), "claude");
        assert_eq!(extract_agent_name_for_pane(&nested_snap, "p1"), "claude");
        assert_eq!(extract_agent_name_for_pane(&nested_snap, "w2:pA"), "agy");
        assert_eq!(
            extract_agent_name_for_pane(&nested_snap, "unknown"),
            "shell"
        );
    }

    #[test]
    fn test_resolve_pane_id_canonicalizes() {
        let snap = json!({"snapshot": {"panes": [
            {"pane_id": "w1:p6", "agent": "claude"},
            {"pane_id": "w1:p1", "agent": "agy"}
        ]}});
        assert_eq!(resolve_pane_id(&snap, "p6").as_deref(), Some("w1:p6"));
        assert_eq!(resolve_pane_id(&snap, "w1:p6").as_deref(), Some("w1:p6"));
        assert_eq!(resolve_pane_id(&snap, "  P1 ").as_deref(), Some("w1:p1"));
        assert_eq!(resolve_pane_id(&snap, "p9"), None);
        assert_eq!(resolve_pane_id(&snap, ""), None);
    }

    #[test]
    fn agents_lists_each_pane_once_with_its_project() {
        let snap = json!({"snapshot": {
            "panes": [
                {"pane_id": "w1:p1", "agent": "Claude", "workspace_id": "w1", "tab_id": "w1:t1",
                 "cwd": "/src/handlr", "agent_status": "blocked", "focused": true},
                {"pane_id": "w2:p2", "workspace_id": "w2", "cwd": "/src/highbay/"}
            ],
            "workspaces": [
                {"workspace_id": "w1", "label": "orchestration",
                 "tabs": [{"tab_id": "w1:t1", "panes": [{"pane_id": "w1:p1", "agent": "claude"}]}]},
                {"workspace_id": "w2"}
            ]
        }});
        let refs = agents(&snap);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].pane_id, "w1:p1");
        assert_eq!(refs[0].agent, "claude");
        assert_eq!(refs[0].project.as_deref(), Some("orchestration"));
        assert_eq!(refs[0].status, AgentStatus::Blocked);
        assert!(refs[0].focused);
        assert_eq!(refs[0].tab_id.as_deref(), Some("w1:t1"));
        // No agent: a shell. Unlabelled workspace: the cwd's basename.
        assert_eq!(refs[1].agent, "shell");
        assert_eq!(refs[1].project.as_deref(), Some("highbay"));
        assert_eq!(refs[1].status, AgentStatus::Unknown);
    }
}
