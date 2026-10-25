//! Names the agents in a captured herdr snapshot, so the naming ladder
//! (plan 20261022_05) can be checked against a real session shape rather
//! than only fixtures.
//!
//! Capture one with `herdr api snapshot > snap.json`, then:
//! `FLANKR_SNAPSHOT=snap.json cargo test --test live_naming -- --nocapture`
use flankr::config::NamingConfig;
use flankr::core::naming::Names;

/// Whether `name` contains `pane_id`, fully qualified or as a short id word.
fn contains_pane_id(name: &str, pane_id: &str) -> bool {
    let short = pane_id.rsplit(':').next().unwrap_or(pane_id);
    name.contains(pane_id)
        || name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == short)
}

#[test]
fn names_agents_in_a_captured_snapshot() {
    let Ok(path) = std::env::var("FLANKR_SNAPSHOT") else {
        eprintln!("FLANKR_SNAPSHOT not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read snapshot");
    let snap: serde_json::Value = serde_json::from_str(&text).expect("parse snapshot");
    let names = Names::from_snapshot(&snap, &NamingConfig::default());
    for a in &names.agents {
        println!(
            "{:<10} agent={:<8} tab={:<8} shown=\"{}\" spoken=\"{}\"",
            a.agent.pane_id,
            a.agent.agent,
            a.agent.tab_label.as_deref().unwrap_or("-"),
            a.shown,
            a.spoken
        );
    }
    println!("vocabulary: {:?}", names.vocabulary());
    assert!(!names.agents.is_empty(), "snapshot contained no agents");
    for a in &names.agents {
        for other in &names.agents {
            for name in [&a.shown, &a.spoken] {
                assert!(
                    !contains_pane_id(name, &other.agent.pane_id),
                    "{name:?} contains pane id {}",
                    other.agent.pane_id
                );
            }
        }
    }
}
