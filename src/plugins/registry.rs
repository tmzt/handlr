//! The built-in plugins, a fixed list built at start
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.1, items 1–3).
//!
//! Every entry's name is checked: a well-formed package name, under
//! `dev.flankr.plugins.` (other names are reserved for later), and unique.
//! Its general agent tools (the plan's 02:10 EDT amendment of 2026-10-29,
//! item 2) are named `<short name>_<tool>` in `[a-z][a-z0-9_]{0,47}`,
//! unique across every plugin, and none is one of the general agent's
//! own. A bad entry is logged and left out at run time; a test fails on
//! one.

use std::collections::HashSet;
use std::sync::Arc;

use super::{Plugin, agent_tool_name, is_agent_tool_name};
use crate::llm::ReadPattern;

/// flankr's own plugins are named under this.
pub const OWN_PREFIX: &str = "dev.flankr.plugins.";

/// The built-in plugins, checked.
pub fn builtin() -> Vec<Arc<dyn Plugin>> {
    let (kept, problems) = check(declared());
    for problem in &problems {
        log::warn!("plugins: {problem}; left out");
    }
    kept
}

/// The list as written, unchecked.
fn declared() -> Vec<Arc<dyn Plugin>> {
    vec![Arc::new(super::planning::Planning)]
}

/// `plugins` with the bad entries left out, and why each was.
pub fn check(plugins: Vec<Arc<dyn Plugin>>) -> (Vec<Arc<dyn Plugin>>, Vec<String>) {
    let mut seen = HashSet::new();
    let mut tools: HashSet<String> = HashSet::new();
    let mut kept = Vec::new();
    let mut problems = Vec::new();
    for plugin in plugins {
        let names: Vec<String> = plugin
            .tools()
            .iter()
            .map(|t| agent_tool_name(plugin.ns(), t.name))
            .collect();
        match problem(&*plugin) {
            Some(problem) => problems.push(problem),
            None if !seen.insert(plugin.ns()) => {
                problems.push(format!("{} is registered twice", plugin.ns()));
            }
            None => match names.iter().find(|n| tools.contains(*n)) {
                Some(taken) => problems.push(format!(
                    "{}: the tool {taken} is another plugin's",
                    plugin.ns()
                )),
                None => {
                    tools.extend(names);
                    kept.push(plugin);
                }
            },
        }
    }
    (kept, problems)
}

/// An icon name: 1 to 32 of `a-z`, `0-9` and `_`, starting with a letter.
/// It names one of the app's own icons; a plugin cannot send a glyph.
fn is_icon_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// What is wrong with one entry, if anything.
fn problem(plugin: &dyn Plugin) -> Option<String> {
    let ns = plugin.ns();
    if !libflankr_proto::companion::is_plugin_ns(ns) {
        return Some(format!("{ns:?} is not a package name"));
    }
    if !ns.starts_with(OWN_PREFIX) || ns[OWN_PREFIX.len()..].contains('.') {
        return Some(format!(
            "{ns} is not dev.flankr.plugins.<name>; other names are reserved"
        ));
    }
    if !plugin.version().valid() {
        return Some(format!("{ns} has a bad version {:?}", plugin.version()));
    }
    let mut ids = HashSet::new();
    for item in plugin.menu() {
        if !libflankr_proto::companion::is_plugin_method(item.id) || item.id.contains('/') {
            return Some(format!("{ns} has a bad menu id {:?}", item.id));
        }
        if !ids.insert(item.id) {
            return Some(format!("{ns} has the menu id {} twice", item.id));
        }
        if let Some(icon) = item.icon
            && !is_icon_name(icon)
        {
            return Some(format!("{ns} has a bad icon name {icon:?}"));
        }
    }
    for pattern in plugin.read_patterns() {
        if let Err(e) = ReadPattern::parse(pattern) {
            return Some(format!("{ns}: {e}"));
        }
    }
    let mut names = HashSet::new();
    for spec in plugin.commands() {
        if !names.insert(spec.name) {
            return Some(format!("{ns} declares the command {} twice", spec.name));
        }
        if spec.binary.is_empty() || spec.binary.contains('/') {
            return Some(format!(
                "{ns}: the command {} names {:?}, not a bare binary name",
                spec.name, spec.binary
            ));
        }
        if spec.max_output > libflankr_proto::companion::PLUGIN_MAX_BYTES {
            return Some(format!(
                "{ns}: the command {} may print more than 256 KiB",
                spec.name
            ));
        }
    }
    let mut tools = HashSet::new();
    for tool in plugin.tools() {
        let name = agent_tool_name(ns, tool.name);
        if tool.name.is_empty() || !is_agent_tool_name(&name) {
            return Some(format!(
                "{ns}: the tool {:?} would be {name:?}, not [a-z][a-z0-9_]{{0,47}}",
                tool.name
            ));
        }
        if !tools.insert(name.clone()) {
            return Some(format!("{ns} declares the tool {} twice", tool.name));
        }
        if crate::reasoning::OWN_TOOL_NAMES.contains(&name.as_str()) {
            return Some(format!("{ns}: the tool {name} is the general agent's own"));
        }
        if tool.description.trim().is_empty() {
            return Some(format!("{ns}: the tool {} has no description", tool.name));
        }
        if !tool.params.is_object() {
            return Some(format!(
                "{ns}: the tool {}'s params are not a JSON schema object",
                tool.name
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::host::tests::{TestPlugin, tool};

    #[test]
    fn the_builtin_list_is_clean() {
        let (kept, problems) = check(declared());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(kept.len(), declared().len());
        assert_eq!(builtin()[0].ns(), "dev.flankr.plugins.planning");
    }

    #[test]
    fn bad_names_duplicates_and_other_namespaces_are_left_out() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(TestPlugin::named("dev.flankr.plugins.test")),
            Arc::new(TestPlugin::named("dev.flankr.plugins.test")),
            Arc::new(TestPlugin::named("Dev.Flankr.plugins.bad")),
            Arc::new(TestPlugin::named("com.example.plugins.other")),
            Arc::new(TestPlugin::named("dev.flankr.plugins.a.b")),
            Arc::new(TestPlugin::named("dev.flankr.plugins")),
        ];
        let (kept, problems) = check(plugins);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].ns(), "dev.flankr.plugins.test");
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert!(problems[0].contains("twice"));
        assert!(problems[2].contains("reserved"));
    }

    #[test]
    fn icon_names() {
        for good in ["plans", "a", "agent_menu", "x2"] {
            assert!(is_icon_name(good), "{good}");
        }
        for bad in ["", "Plans", "2x", "_a", "pl-ans", "☑", &"a".repeat(33)] {
            assert!(!is_icon_name(bad), "{bad}");
        }
    }

    #[test]
    fn tool_names_are_checked_and_unique_across_plugins() {
        let long: &'static str = Box::leak("a".repeat(44).into_boxed_str());
        let ok: &'static str = Box::leak("a".repeat(43).into_boxed_str());
        let mut no_params = tool("x", true);
        no_params.params = serde_json::json!("none");
        let mut no_description = tool("x", true);
        no_description.description = " ";
        let cases: Vec<(Vec<_>, Option<&str>)> = vec![
            (vec![tool("echo", true)], None),
            // `test_` and 43 letters is 48: the most.
            (vec![tool(ok, true)], None),
            (vec![tool(long, true)], Some("not [a-z]")),
            (vec![tool("Echo", true)], Some("not [a-z]")),
            (vec![tool("", true)], Some("not [a-z]")),
            (vec![tool("a-b", true)], Some("not [a-z]")),
            (vec![tool("x", true), tool("x", false)], Some("twice")),
            (vec![no_params], Some("JSON schema")),
            (vec![no_description], Some("no description")),
        ];
        for (tools, problem) in cases {
            let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
            let plugin: Arc<dyn Plugin> =
                Arc::new(TestPlugin::with_tools("dev.flankr.plugins.test", tools));
            let (kept, problems) = check(vec![plugin]);
            match problem {
                None => assert!(problems.is_empty(), "{names:?}: {problems:?}"),
                Some(p) => {
                    assert!(kept.is_empty(), "{names:?}");
                    assert!(problems[0].contains(p), "{names:?}: {problems:?}");
                }
            }
        }
        // The general agent's own names are refused: `read` + `_file`.
        let plugin: Arc<dyn Plugin> = Arc::new(TestPlugin::with_tools(
            "dev.flankr.plugins.read",
            vec![tool("file", true)],
        ));
        let (_, problems) = check(vec![plugin]);
        assert!(problems[0].contains("general agent's own"), "{problems:?}");
        // `a_b` + `c` and `a` + `b_c` are both `a_b_c`: the second is left out.
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(TestPlugin::with_tools(
                "dev.flankr.plugins.a_b",
                vec![tool("c", true)],
            )),
            Arc::new(TestPlugin::with_tools(
                "dev.flankr.plugins.a",
                vec![tool("b_c", true)],
            )),
        ];
        let (kept, problems) = check(plugins);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].ns(), "dev.flankr.plugins.a_b");
        assert!(problems[0].contains("another plugin's"), "{problems:?}");
    }
}
