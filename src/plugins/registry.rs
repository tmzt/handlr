//! The built-in plugins, a fixed list built at start
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.1, items 1–3).
//!
//! Every entry's name is checked: a well-formed package name, under
//! `dev.flankr.plugins.` (other names are reserved for later), and unique.
//! A bad entry is logged and left out at run time; a test fails on one.

use std::collections::HashSet;
use std::sync::Arc;

use super::Plugin;
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
    let mut kept = Vec::new();
    let mut problems = Vec::new();
    for plugin in plugins {
        match problem(&*plugin) {
            Some(problem) => problems.push(problem),
            None if !seen.insert(plugin.ns()) => {
                problems.push(format!("{} is registered twice", plugin.ns()));
            }
            None => kept.push(plugin),
        }
    }
    (kept, problems)
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
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::host::tests::TestPlugin;

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
}
