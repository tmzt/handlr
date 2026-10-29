//! The planning plugin's daemon half, `dev.flankr.plugins.planning@0.1.2`
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.8).
//!
//! Read-only and off by default. `plans.list` lists the focused project's
//! plans and specs in `docs/design/` with their status and the README's
//! decision; `plans.show` gives one plan's front matter, headings and
//! amendments. Both answer in tpt's schema-1 JSON shapes, from
//! `read_file` and [`reader`]. tpt's query commands are declared from this
//! version, but only the `read_file` source is switched on until tpt has
//! them.
//!
//! From 0.1.2 it offers the general agent two read-only tools (the plan's
//! 02:10 EDT amendment of 2026-10-29, item 6): `planning_plans_list {}`,
//! the plans newest first by last update with their NN, date, title and
//! abbreviated status, and `planning_plan_show { name }`, one plan as
//! `plans.show` gives it.

pub mod reader;

use std::sync::LazyLock;

use serde_json::{Value, json};

use super::command::{ArgShape, CommandSpec, is_plan_name};
use super::{
    MenuItem, Opens, Plugin, PluginCtx, PluginError, PluginVersion, Screens, ToolSpec, codes,
};

/// The plugin's package name.
pub const NS: &str = "dev.flankr.plugins.planning";
/// Where the plans are.
pub const DESIGN: &str = "docs/design";
/// No `docs/design/` plan files (decision 3.8, item 4).
pub const NO_DESIGN: i64 = -32110;
/// No plan of the name asked for.
pub const NO_PLAN: i64 = -32111;
/// Whether `[plugins."<ns>"] source = "tpt"` may be used: not until tpt's
/// query commands exist (decision 3.8, item 6). Turning it on is a patch
/// release of the plugin.
const TPT_SOURCE: bool = false;

const MENU: [MenuItem; 1] = [MenuItem {
    id: "plans",
    label: "Plans in this project",
    opens: Opens::Menu,
    screens: Screens::Both,
    icon: Some("plans"),
}];

const READ: [&str; 3] = ["README.md", "CHANGES.md", "docs/design/*.md"];

const COMMANDS: [CommandSpec; 3] = [
    CommandSpec {
        name: "plans_list",
        binary: "tpt",
        root_flag: Some("-C"),
        fixed: &["plans", "list", "--json"],
        args: &[],
        max_output: 256 * 1024,
    },
    CommandSpec {
        name: "plans_show",
        binary: "tpt",
        root_flag: Some("-C"),
        fixed: &["plans", "show", "--json"],
        args: &[ArgShape::PlanName],
        max_output: 256 * 1024,
    },
    CommandSpec {
        name: "check",
        binary: "tpt",
        root_flag: Some("-C"),
        fixed: &["check", "--json"],
        args: &[],
        max_output: 256 * 1024,
    },
];

/// Its general agent tools, both read-only.
static TOOLS: LazyLock<[ToolSpec; 2]> = LazyLock::new(|| {
    [
        ToolSpec {
            name: "plans_list",
            description: "Lists the current project's plans and specs in docs/design, newest first by last update: each one's name, its sequence number within its day (nn), its date, title and abbreviated status (Draft, Hold, InImpl, Impl, Succd, Rtrd), and when it was last updated.",
            params: json!({"type": "object", "properties": {}}),
            read_only: true,
        },
        ToolSpec {
            name: "plan_show",
            description: "Shows one of the current project's plans: its front matter (title, date, status, covers, sources, related documents), its headings and its amendments' timestamps and subjects. Never the text itself.",
            params: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The plan's base name, as planning_plans_list gives it (e.g. 20261028_04_PLUGINS_PLAN)."
                    }
                },
                "required": ["name"]
            }),
            read_only: true,
        },
    ]
});

/// The planning plugin.
pub struct Planning;

#[async_trait::async_trait]
impl Plugin for Planning {
    fn ns(&self) -> &'static str {
        NS
    }

    fn name(&self) -> &'static str {
        "Planning"
    }

    fn description(&self) -> &'static str {
        "Lists this project's plans and their amendments, from docs/design. Read-only."
    }

    fn version(&self) -> PluginVersion {
        PluginVersion::Semver("0.1.2")
    }

    fn menu(&self) -> &'static [MenuItem] {
        &MENU
    }

    fn read_patterns(&self) -> &'static [&'static str] {
        &READ
    }

    fn commands(&self) -> &'static [CommandSpec] {
        &COMMANDS
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        ctx: &PluginCtx,
    ) -> Result<Value, PluginError> {
        let name = match method {
            "plans.list" => None,
            "plans.show" => Some(plan_name(&params)?),
            other => return Err(PluginError::no_method(other)),
        };
        if source(ctx) == Source::Tpt {
            match via_tpt(ctx, name.as_deref()).await {
                Ok(value) => return Ok(value),
                Err(e) => log::warn!("plugin {NS}: tpt: {e}; reading the files instead"),
            }
        }
        match name {
            None => list(ctx).await,
            Some(name) => show(ctx, &name).await,
        }
    }

    fn tools(&self) -> &[ToolSpec] {
        &*TOOLS
    }

    /// The tools answer from `plans.list` and `plans.show`, from the same
    /// source.
    async fn run_tool(
        &self,
        name: &str,
        args: Value,
        ctx: &PluginCtx,
    ) -> Result<Value, PluginError> {
        match name {
            "plans_list" => Ok(reader::agent_list(
                &self.call("plans.list", json!({}), ctx).await?,
            )),
            "plan_show" => self.call("plans.show", args, ctx).await,
            other => Err(PluginError::new(
                libflankr_proto::envelope::codes::METHOD_NOT_FOUND,
                format!("This plugin has no tool {other}."),
            )),
        }
    }
}

/// Where the answers come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    ReadFile,
    Tpt,
}

fn source(ctx: &PluginCtx) -> Source {
    match ctx.config().get("source").and_then(|v| v.as_str()) {
        None | Some("read_file") => Source::ReadFile,
        Some("tpt") if TPT_SOURCE => Source::Tpt,
        Some("tpt") => {
            log::warn!(
                "[plugins.\"{NS}\"] source = \"tpt\" is not switched on yet; reading the files"
            );
            Source::ReadFile
        }
        Some(other) => {
            log::warn!("[plugins.\"{NS}\"] source = {other:?} is not known; reading the files");
            Source::ReadFile
        }
    }
}

/// `plans.show`'s `name`: a plan's base name.
fn plan_name(params: &Value) -> Result<String, PluginError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !is_plan_name(name) {
        return Err(PluginError::invalid(
            "plans.show needs a plan's base name, e.g. 20261028_04_PLUGINS_PLAN.",
        ));
    }
    Ok(name.to_string())
}

/// The README's rows, by the plan file they link; none when it can't be
/// read.
async fn readme(ctx: &PluginCtx) -> Vec<reader::ReadmeRow> {
    match ctx.read_file("README.md").await {
        Ok(read) => reader::readme_rows(&read.text),
        Err(e) => {
            log::info!("plugin {NS}: README.md: {}", e.message);
            Vec::new()
        }
    }
}

fn with_readme(plan: &mut reader::Plan, rows: &[reader::ReadmeRow]) {
    plan.readme = rows
        .iter()
        .find(|r| reader::root_relative(&r.target).as_deref() == Some(plan.file.as_str()))
        .cloned();
}

/// `plans.list {}`: every plan and spec, oldest first.
async fn list(ctx: &PluginCtx) -> Result<Value, PluginError> {
    let listing = ctx.list_files(Some("docs/design/*.md")).await?;
    let mut files: Vec<(String, reader::FileName)> = listing
        .paths
        .iter()
        .filter_map(|path| {
            let name = path.strip_prefix("docs/design/")?;
            Some((path.clone(), reader::file_name(name)?))
        })
        .collect();
    if files.is_empty() {
        return Err(PluginError::new(
            NO_DESIGN,
            "No docs/design folder in this project",
        ));
    }
    files.sort_by(|a, b| (&a.1.date, a.1.nn, &a.1.base).cmp(&(&b.1.date, b.1.nn, &b.1.base)));
    let rows = readme(ctx).await;
    let mut plans = Vec::with_capacity(files.len());
    for (path, name) in files {
        let plan = match ctx.read_file(&path).await {
            Ok(read) => {
                let mut plan = reader::read_plan(&path, name, &read.text, read.cut);
                with_readme(&mut plan, &rows);
                plan
            }
            Err(e) => {
                let mut plan = reader::read_plan(&path, name, "", false);
                plan.error = Some(e.message);
                with_readme(&mut plan, &rows);
                plan
            }
        };
        plans.push(reader::plan_json(&plan, false));
    }
    let root = ctx.project().map(|p| p.name.clone()).unwrap_or_default();
    Ok(json!({ "schema": 1, "root": root, "plans": plans }))
}

/// `plans.show { name }`: one plan in full.
async fn show(ctx: &PluginCtx, name: &str) -> Result<Value, PluginError> {
    let path = format!("{DESIGN}/{name}.md");
    let Some(file_name) = reader::file_name(&format!("{name}.md")) else {
        return Err(PluginError::new(NO_PLAN, format!("No plan named {name}")));
    };
    let read = ctx.read_file(&path).await.map_err(|e| {
        if e.code == codes::READ_REFUSED && e.message.contains("does not exist") {
            PluginError::new(NO_PLAN, format!("No plan named {name}"))
        } else {
            e
        }
    })?;
    let mut plan = reader::read_plan(&path, file_name, &read.text, read.cut);
    with_readme(&mut plan, &readme(ctx).await);
    Ok(json!({ "schema": 1, "plan": reader::plan_json(&plan, true) }))
}

/// The `tpt` source: its JSON passed through once it says `"schema": 1`.
async fn via_tpt(ctx: &PluginCtx, name: Option<&str>) -> Result<Value, PluginError> {
    let out = match name {
        None => ctx.run_command("plans_list", &[]).await?,
        Some(name) => ctx.run_command("plans_show", &[name]).await?,
    };
    let value: Value = serde_json::from_str(&out.stdout)
        .map_err(|e| PluginError::new(codes::COMMAND_FAILED, format!("tpt's JSON: {e}")))?;
    if value.get("schema") != Some(&json!(1)) {
        return Err(PluginError::new(
            codes::COMMAND_FAILED,
            "tpt's JSON is not schema 1",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
