//! An entry that is context, not the user's words nor the agent's (an
//! `Entry::origin`: a finished task, another session's or a subagent's
//! message, a recap, a compaction), drawn in the scrollback as dim lines
//! after a small amber dot (the CHANGES.md addendum of 2026-10-23 22:31
//! EDT): `task finished: <text>`, `<peer>: <text>`, `subagent: <text>`,
//! `※ <recap>`, `※ conversation compacted`. Its whole text shows, never
//! cut (plan 20261022_03, the L3 amendment of 2026-10-24 11:54 EDT). It
//! never sits on the user's band and never makes a card.

use libhandlr_proto::common::Entry;

use super::{CELL_EM, Flow, Row, TOOL_PT, full_text, link_spans, term_h, wrap_hard};

use crate::theme::theme;

/// The dot before a notice, from the terminal face's symbol page.
pub const DOT: &str = "\u{25CF}";
/// The recap and compaction mark.
pub const MARK: &str = "\u{203B}";

pub struct NoticeLine;

impl NoticeLine {
    /// What a notice says for an entry with `origin`: its whole text, on
    /// one line before wrapping.
    pub fn text(origin: &str, text: &str) -> String {
        let all = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if origin == "task" {
            format!("task finished: {all}")
        } else if let Some(name) = origin.strip_prefix("peer:") {
            format!("{name}: {all}")
        } else if origin.starts_with("agent:") {
            format!("subagent: {all}")
        } else if origin == "recap" {
            format!("{MARK} {all}")
        } else if origin == "compact" {
            format!("{MARK} conversation compacted")
        } else {
            format!("{origin}: {all}")
        }
    }

    /// Whether `e` is a compaction's line.
    pub fn compacts(e: &Entry) -> bool {
        e.origin.as_deref() == Some("compact")
    }

    /// Whether `e`, after an entry that `previous_compacts`, says nothing
    /// new: a compaction is marked by two lines in a row (the boundary and
    /// its notice), and shown once.
    pub fn repeats(e: &Entry, previous_compacts: bool) -> bool {
        Self::compacts(e) && previous_compacts
    }

    /// The notice for `e` (which has an origin) at `y`, from `x`, `cols`
    /// cells of tool-sized type wide; its height. `targets` are the flow's
    /// links, for the entry's own.
    #[allow(clippy::too_many_arguments)]
    pub fn lay_out(
        e: &Entry,
        origin: &str,
        x: f32,
        y: f32,
        cols: usize,
        s: f32,
        targets: &[libhandlr_proto::common::Link],
        flow: &mut Flow,
    ) -> f32 {
        let px = TOOL_PT * s;
        let lh = term_h(px);
        let indent = 2.0 * px * CELL_EM;
        flow.rows.push(Row {
            text: DOT.to_string(),
            x,
            y,
            px,
            color: theme().accent,
            mono: true,
        });
        let width = cols.saturating_sub(2).max(1);
        let lines = wrap_hard(&Self::text(origin, full_text(e)), width);
        let first_row = flow.rows.len();
        let mut ly = y;
        for line in lines {
            flow.rows.push(Row {
                text: line,
                x: x + indent,
                y: ly,
                px,
                color: theme().dim,
                mono: true,
            });
            ly += lh;
        }
        link_spans::mark(flow, first_row, &e.links, targets);
        ly - y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_origin_says_where_it_is_from() {
        assert_eq!(
            NoticeLine::text("task", "Build finished\nmore"),
            "task finished: Build finished more"
        );
        assert_eq!(
            NoticeLine::text("peer:docs-writer", "\nRebased on main.\n\nDone."),
            "docs-writer: Rebased on main. Done."
        );
        assert_eq!(
            NoticeLine::text("agent:a0123", "Added the test.\nIt passes."),
            "subagent: Added the test. It passes."
        );
        assert_eq!(
            NoticeLine::text("recap", "You asked for the build.\nIt passed."),
            "\u{203B} You asked for the build. It passed."
        );
        assert_eq!(
            NoticeLine::text("compact", "Conversation compacted"),
            "\u{203B} conversation compacted"
        );
        assert_eq!(NoticeLine::text("other", "x"), "other: x");
    }
}
