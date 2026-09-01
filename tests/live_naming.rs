//! Names the agents in a captured herdr snapshot, so the disambiguation ladder
//! can be checked against a real session shape rather than only fixtures.
//!
//! Capture one with `herdr api snapshot > snap.json`, then:
//! `HANDLR_SNAPSHOT=snap.json cargo test --test live_naming -- --nocapture`
use handlr::notify::{agent_identities, speakable_name};

#[test]
fn names_agents_in_a_captured_snapshot() {
    let Ok(path) = std::env::var("HANDLR_SNAPSHOT") else {
        eprintln!("HANDLR_SNAPSHOT not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read snapshot");
    let snap: serde_json::Value = serde_json::from_str(&text).expect("parse snapshot");
    let ids = agent_identities(&snap);
    for id in &ids {
        println!(
            "{:<10} agent={:<8} project={:<12} -> \"{}\"",
            id.pane_id,
            id.agent,
            id.project.as_deref().unwrap_or("-"),
            speakable_name(id, &ids)
        );
    }
    assert!(!ids.is_empty(), "snapshot contained no agents");
}
