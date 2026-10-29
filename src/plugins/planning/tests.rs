//! The planning plugin on fixture documents: four of this repository's
//! plans and its README, as they were at 91b545b
//! (tests/fixtures/planning/), and hand-made projects.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::core::principal::Principal;
use crate::plugins::command::CommandEnv;
use crate::plugins::ctx::tests::{FakeLookup, patterns};
use crate::plugins::ctx::{Commands, CtxParts};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/planning")
}

async fn ctx_at(root: &Path, config: toml::Table, commands: Commands) -> PluginCtx {
    PluginCtx::build(CtxParts {
        ns: NS,
        principal: Principal::local("t"),
        pane: Some("w1:p1".into()),
        lookup: Arc::new(FakeLookup(vec![(
            "w1:p1".into(),
            "claude".into(),
            root.display().to_string(),
        )])),
        // The template's `read`.
        read: patterns(&["*.md", "docs/**/*.md"]),
        plugin_read: patterns(&READ),
        commands,
        config,
        notifier: None,
    })
    .await
}

async fn ctx(root: &Path) -> PluginCtx {
    ctx_at(root, toml::Table::new(), Commands::default()).await
}

#[tokio::test]
async fn plans_list_on_the_fixture() {
    let c = ctx(&fixture()).await;
    let v = Planning.call("plans.list", json!({}), &c).await.unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["root"], "planning");
    let plans = v["plans"].as_array().unwrap();
    let bases: Vec<&str> = plans.iter().map(|p| p["base"].as_str().unwrap()).collect();
    assert_eq!(
        bases,
        [
            "20260831_03_SPEECH_PIPELINE_PLAN",
            "20261024_02_SETTINGS_SCREEN_PLAN",
            "20261025_02_EXTRACT_LIBINFER_PLAN",
            "20261027_04_GENERAL_AGENT_PLANNING_PLAN",
        ],
        "oldest first"
    );
    for p in plans {
        assert_eq!(p["error"], Value::Null, "{p}");
        assert_eq!(p["kind"], "plan");
        let status = p["status"].as_str().unwrap();
        let readme = p["readme"]["status"].as_str().unwrap();
        assert!(reader::same_status(status, readme), "{status} / {readme}");
        assert!(p["readme"]["decision"].as_str().unwrap().len() > 10);
        assert!(p.get("amendments").is_none(), "list items leave them out");
    }
    let speech = &plans[0];
    assert_eq!(
        speech["status"],
        "Retired (by 20261022_01_ORCHESTRATION_DAEMON_PLAN)"
    );
    assert_eq!(speech["status_base"], "Retired");
    assert_eq!(
        speech["status_by"],
        json!({"ref": "20261022_01_ORCHESTRATION_DAEMON_PLAN"})
    );
    // The README's cell differs in text, and both are kept.
    assert_ne!(speech["readme"]["status"], speech["status"]);
    assert_eq!(speech["date_from_name"], "2026-08-31");
    assert_eq!(speech["nn"], 3);
    let settings = &plans[1];
    assert_eq!(
        settings["title"],
        "A settings screen, from a gear on the overview"
    );
    assert_eq!(settings["status"], "In implementation");
    assert_eq!(settings["last_amendment"], "2026-10-28 17:57 EDT");
}

#[tokio::test]
async fn plans_show_on_the_fixture() {
    let c = ctx(&fixture()).await;
    let v = Planning
        .call(
            "plans.show",
            json!({"name": "20261024_02_SETTINGS_SCREEN_PLAN"}),
            &c,
        )
        .await
        .unwrap();
    assert_eq!(v["schema"], 1);
    let p = &v["plan"];
    assert_eq!(p["file"], "docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md");
    assert_eq!(p["date"], "2026-10-24");
    let amendments = p["amendments"].as_array().unwrap();
    assert_eq!(amendments.len(), 12);
    assert_eq!(
        amendments[0],
        json!({
            "index": 1,
            "line": 195,
            "timestamp": "2026-10-24 11:32 EDT",
            "subject": "a Display section with the scrollbar's visibility",
            "living_section": null,
        })
    );
    assert_eq!(amendments[11]["timestamp"], "2026-10-28 17:57 EDT");
    // A multi-line quoted list item.
    assert_eq!(
        p["covers"][0],
        "A gear at the right of the overview's header opening a settings screen, in sections, on libcontrols 0.8.0's scroll pane"
    );
    assert!(
        p["sources"][0]
            .as_str()
            .unwrap()
            .contains("\"plan a settings screen")
    );
    let headings = p["headings"].as_array().unwrap();
    assert!(
        headings.iter().any(|h| h["text"] == "1. Goal"),
        "{headings:?}"
    );
    assert!(p["readme"]["row"].as_u64().unwrap() > 10);
    assert!(p.get("body").is_none(), "no bodies");
}

#[tokio::test]
async fn refusals_and_errors() {
    let c = ctx(&fixture()).await;
    let missing = Planning
        .call(
            "plans.show",
            json!({"name": "20261028_09_NOT_HERE_PLAN"}),
            &c,
        )
        .await
        .unwrap_err();
    assert_eq!(missing.code, NO_PLAN);
    assert_eq!(missing.message, "No plan named 20261028_09_NOT_HERE_PLAN");
    for bad in [
        json!({}),
        json!({"name": "../README"}),
        json!({"name": "README"}),
    ] {
        let err = Planning
            .call("plans.show", bad.clone(), &c)
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            libflankr_proto::envelope::codes::INVALID_PARAMS,
            "{bad}"
        );
    }
    let err = Planning
        .call("plans.write", json!({}), &c)
        .await
        .unwrap_err();
    assert_eq!(err.code, libflankr_proto::envelope::codes::METHOD_NOT_FOUND);
    // No docs/design.
    let empty = tempfile::tempdir().unwrap();
    std::fs::write(empty.path().join("README.md"), "# x\n").unwrap();
    let c = ctx(empty.path()).await;
    let err = Planning
        .call("plans.list", json!({}), &c)
        .await
        .unwrap_err();
    assert_eq!(err.code, NO_DESIGN);
    // Reading off.
    let off = PluginCtx::build(CtxParts {
        ns: NS,
        principal: Principal::local("t"),
        pane: Some("w1:p1".into()),
        lookup: Arc::new(FakeLookup(vec![(
            "w1:p1".into(),
            "claude".into(),
            fixture().display().to_string(),
        )])),
        read: vec![],
        plugin_read: patterns(&READ),
        commands: Commands::default(),
        config: toml::Table::new(),
        notifier: None,
    })
    .await;
    let err = Planning
        .call("plans.list", json!({}), &off)
        .await
        .unwrap_err();
    assert_eq!(err.message, crate::plugins::ctx::READING_OFF);
}

#[tokio::test]
async fn hand_made_files() {
    let dir = tempfile::tempdir().unwrap();
    let design = dir.path().join("docs/design");
    std::fs::create_dir_all(&design).unwrap();
    std::fs::write(
        design.join("20261028_01_NO_FRONT_PLAN.md"),
        "# No front matter\n",
    )
    .unwrap();
    std::fs::write(
        design.join("20261028_02_A_SPEC.md"),
        "---\ntitle: A spec\ndate: 2026-10-28\nstatus: Draft\n---\n",
    )
    .unwrap();
    std::fs::write(design.join("notes.md"), "not a plan's name\n").unwrap();
    let mut long = "---\ntitle: Long\ndate: 2026-10-28\nstatus: Draft\n---\n".to_string();
    while long.len() < crate::project::MAX_READ_BYTES + 100 {
        long.push_str("Some text that makes the file long.\n");
    }
    std::fs::write(design.join("20261028_03_LONG_PLAN.md"), long).unwrap();
    // README with a row for one plan only.
    std::fs::write(
        dir.path().join("README.md"),
        "# P\n\n## Architectural decisions\n\n| Document | Date | Status | Decision |\n| --- | --- | --- | --- |\n| [A spec](docs/design/20261028_02_A_SPEC.md) | 2026-10-28 | In implementation | A. |\n",
    )
    .unwrap();
    let c = ctx(dir.path()).await;
    let v = Planning.call("plans.list", json!({}), &c).await.unwrap();
    let plans = v["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 3, "notes.md is not a plan");
    assert_eq!(plans[0]["error"], "no front matter");
    assert_eq!(plans[0]["title"], Value::Null);
    assert_eq!(plans[1]["kind"], "spec");
    // Front matter and README differ: both kept, as they are.
    assert_eq!(plans[1]["status"], "Draft");
    assert_eq!(plans[1]["readme"]["status"], "In implementation");
    assert_eq!(plans[2]["error"], "cut at 64 KiB");
    assert_eq!(plans[2]["status"], "Draft");
    assert_eq!(plans[2]["readme"], Value::Null);
}

#[tokio::test]
async fn the_tpt_source_is_declared_but_off() {
    // `source = "tpt"` still reads the files.
    let mut config = toml::Table::new();
    config.insert("source".into(), "tpt".into());
    let c = ctx_at(&fixture(), config, Commands::default()).await;
    let v = Planning.call("plans.list", json!({}), &c).await.unwrap();
    assert_eq!(v["plans"].as_array().unwrap().len(), 4);
    assert_eq!(
        Planning
            .commands()
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>(),
        ["plans_list", "plans_show", "check"]
    );
}

#[tokio::test]
async fn the_tpt_path_passes_schema_1_through() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let tpt = bin.join("tpt");
    std::fs::write(
        &tpt,
        "#!/bin/sh\nif [ \"$6\" = \"20261028_09_BAD_PLAN\" ]; then echo '{\"schema\": 2}'; exit 0; fi\nprintf '{\"schema\": 1, \"args\": \"%s\"}' \"$*\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&tpt, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = CommandEnv {
        path: Some(format!("{}:/bin:/usr/bin", bin.display()).into()),
        ..CommandEnv::default()
    };
    let commands = Commands {
        on: COMMANDS.iter().map(|c| (c, env.find(c.binary))).collect(),
        env,
    };
    let root = std::fs::canonicalize(fixture()).unwrap();
    let c = ctx_at(&root, toml::Table::new(), commands).await;
    let v = via_tpt(&c, Some("20261028_04_PLUGINS_PLAN")).await.unwrap();
    assert_eq!(
        v["args"],
        format!(
            "-C {} plans show --json 20261028_04_PLUGINS_PLAN",
            root.display()
        )
    );
    let v = via_tpt(&c, None).await.unwrap();
    assert_eq!(
        v["args"],
        format!("-C {} plans list --json", root.display())
    );
    let err = via_tpt(&c, Some("20261028_09_BAD_PLAN")).await.unwrap_err();
    assert!(err.message.contains("not schema 1"), "{err}");
}
