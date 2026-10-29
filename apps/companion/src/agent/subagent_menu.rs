//! The agent menu: the ⊙ title icon's popup on the agent view while the
//! pane has subagents (docs/design/20261023_07_SUBAGENTS_PLAN.md, the
//! 2026-10-28 20:46, 20:51 and 22:03 EDT amendments).
//!
//! A libcontrols `PopupMenu` with radio marks (`PopupMarkStyle::Radio`,
//! libcontrols 0.10.7), in the menu's own text colour, top to bottom:
//! **main**, the pane's own agent; a **Running** heading and the running
//! subagents, each by its short name, with its elapsed minutes, `(12m)`,
//! only past 5; a **Finished** heading and the finished ones. Within each,
//! newest first, as the client lists them. A heading with nothing under
//! it is left out. The session viewed has the filled radio (⦿), the
//! others the empty one (⊙). Every item is an action; none is a toggle.

use libcontrols::PopupItem;
use libflankr_proto::common::ChildInfo;

/// The main agent's item id.
pub const MAIN: &str = "app/agent/main";
/// A subagent's item id: this, then its id.
pub const CHILD: &str = "app/agent/child/";
/// The main agent's label, and the headings.
pub const MAIN_LABEL: &str = "main";
pub const RUNNING: &str = "Running";
pub const FINISHED: &str = "Finished";
/// A running subagent's minutes show only past this many.
pub const MINUTES_PAST: u64 = 5;

/// What an item id chooses: `Some(None)` the main agent, `Some(Some(id))`
/// a subagent; `None` for an id that is not the agent menu's.
pub fn parse(id: &str) -> Option<Option<String>> {
    if id == MAIN {
        return Some(None);
    }
    id.strip_prefix(CHILD)
        .filter(|c| !c.is_empty())
        .map(|c| Some(c.to_string()))
}

/// A running subagent's whole minutes since its first entry, at `now`
/// (seconds since 1970), when past [`MINUTES_PAST`]; `None` once finished
/// or while its start is not known.
pub fn minutes(child: &ChildInfo, now: u64) -> Option<u64> {
    if child.finished || child.started == 0 {
        return None;
    }
    let m = now.saturating_sub(child.started) / 60;
    (m > MINUTES_PAST).then_some(m)
}

/// A subagent's name: its short name, else its description, else its
/// type, else its id.
pub fn name(child: &ChildInfo) -> &str {
    [&child.short, &child.description, &child.kind]
        .into_iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or(&child.id)
}

/// A subagent's label in the menu: its name, and its minutes when shown.
pub fn label(child: &ChildInfo, now: u64) -> String {
    match minutes(child, now) {
        Some(m) => format!("{} ({m}m)", name(child)),
        None => name(child).to_string(),
    }
}

/// The menu's items for `children` (running first, then finished, newest
/// first within each, as `Event::AgentView` gives them), with the
/// session `viewed` (`None`: the main agent) checked, at `now`.
pub fn items(children: &[ChildInfo], viewed: Option<&str>, now: u64) -> Vec<PopupItem> {
    let mut out = vec![PopupItem::action(MAIN, MAIN_LABEL).with_checked(Some(viewed.is_none()))];
    for (heading, finished) in [(RUNNING, false), (FINISHED, true)] {
        let mut group = children
            .iter()
            .filter(|c| c.finished == finished)
            .peekable();
        if group.peek().is_none() {
            continue;
        }
        out.push(PopupItem::heading(heading));
        for c in group {
            out.push(
                PopupItem::action(format!("{CHILD}{}", c.id), label(c, now))
                    .with_checked(Some(viewed == Some(c.id.as_str()))),
            );
        }
    }
    out
}

/// Seconds since 1970 on this device's clock.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use libcontrols::PopupItemKind;

    pub(crate) const NOW: u64 = 1_761_700_000;

    pub(crate) fn child(id: &str, short: &str, finished: bool, minutes_ago: u64) -> ChildInfo {
        ChildInfo {
            id: id.into(),
            short: short.into(),
            description: format!("Build the {short} plan"),
            kind: "general-purpose".into(),
            background: true,
            finished,
            started: NOW - minutes_ago * 60,
            started_by: Some(format!("toolu_{id}")),
            turn: None,
        }
    }

    /// Two running (12 and 3 minutes in) and one finished, in the
    /// client's order.
    pub(crate) fn children() -> Vec<ChildInfo> {
        vec![
            child("a1", "overview screen", false, 12),
            child("b2", "spoken summaries", false, 3),
            child("c3", "libtypedmd licences", true, 40),
        ]
    }

    fn shown(items: &[PopupItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match (i.kind, i.checked) {
                (PopupItemKind::Heading, _) => format!("# {}", i.label),
                (_, Some(true)) => format!("(*) {}", i.label),
                (_, Some(false)) => format!("( ) {}", i.label),
                (_, None) => i.label.clone(),
            })
            .collect()
    }

    #[test]
    fn main_then_running_then_finished_with_one_filled_radio() {
        let i = items(&children(), None, NOW);
        assert_eq!(
            shown(&i),
            [
                "(*) main",
                "# Running",
                "( ) overview screen (12m)",
                "( ) spoken summaries",
                "# Finished",
                "( ) libtypedmd licences",
            ]
        );
        // Actions, not toggles; ids that choose.
        assert!(
            i.iter()
                .filter(|i| i.kind != PopupItemKind::Heading)
                .all(|i| i.kind == PopupItemKind::Action)
        );
        assert_eq!(parse(i[0].id.as_str()), Some(None));
        assert_eq!(parse(i[2].id.as_str()), Some(Some("a1".into())));
        // Viewing a subagent: its radio is the filled one, main's empty.
        let i = items(&children(), Some("c3"), NOW);
        let filled: Vec<&str> = i
            .iter()
            .filter(|i| i.checked == Some(true))
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(filled, ["libtypedmd licences"]);
        assert_eq!(i[0].checked, Some(false));
    }

    #[test]
    fn a_heading_with_nothing_under_it_is_left_out() {
        let running: Vec<ChildInfo> = children().into_iter().filter(|c| !c.finished).collect();
        assert!(!shown(&items(&running, None, NOW)).contains(&"# Finished".to_string()));
        let finished: Vec<ChildInfo> = children().into_iter().filter(|c| c.finished).collect();
        assert_eq!(
            shown(&items(&finished, None, NOW)),
            ["(*) main", "# Finished", "( ) libtypedmd licences"]
        );
    }

    #[test]
    fn the_minutes_show_past_five_and_not_before() {
        let at = |m: u64| child("a", "x", false, m);
        assert_eq!(minutes(&at(0), NOW), None);
        assert_eq!(minutes(&at(5), NOW), None);
        assert_eq!(minutes(&at(6), NOW), Some(6));
        assert_eq!(label(&at(6), NOW), "x (6m)");
        assert_eq!(label(&at(5), NOW), "x");
        // Five and a half minutes is five whole minutes: not yet.
        let mut c = at(5);
        c.started -= 30;
        assert_eq!(minutes(&c, NOW), None);
        // Finished, or with no known start: none.
        assert_eq!(minutes(&child("a", "x", true, 60), NOW), None);
        let mut unknown = at(60);
        unknown.started = 0;
        assert_eq!(minutes(&unknown, NOW), None);
    }

    #[test]
    fn a_name_falls_back_to_the_description_type_and_id() {
        let mut c = child("a1", "", false, 0);
        assert_eq!(name(&c), "Build the  plan");
        c.description.clear();
        assert_eq!(name(&c), "general-purpose");
        c.kind.clear();
        assert_eq!(name(&c), "a1");
        assert_eq!(parse("app/agent/child/"), None);
        assert_eq!(parse("app/settings"), None);
    }
}
