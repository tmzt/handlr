//! An entry that is context, not the user's words nor the agent's (an
//! `Entry::origin`: a finished task, another session's or a subagent's
//! message, a recap, a compaction), drawn in the scrollback as one dim
//! line, two at most, after a small amber dot (the CHANGES.md addendum of
//! 2026-10-23 22:31 EDT): `task finished: <summary>`, `<peer>: <first
//! line>`, `subagent: <first line>`, `※ <recap>`, `※ conversation
//! compacted`. It never sits on the user's band and never makes a card.

use libhandlr_proto::common::Entry;

use super::input_box::AMBER;
use super::{CELL_EM, DIM, Flow, Row, TOOL_PT, link_spans, one_line, term_h, wrap_hard};

/// The dot before a notice, from the terminal face's symbol page.
pub const DOT: &str = "\u{25CF}";
/// The recap and compaction mark.
pub const MARK: &str = "\u{203B}";
/// The most lines a notice takes.
const LINES: usize = 2;

pub struct NoticeLine;

impl NoticeLine {
    /// What a notice says for an entry with `origin`.
    pub fn text(origin: &str, summary: &str) -> String {
        let first = summary
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let all = summary.split_whitespace().collect::<Vec<_>>().join(" ");
        if origin == "task" {
            format!("task finished: {first}")
        } else if let Some(name) = origin.strip_prefix("peer:") {
            format!("{name}: {first}")
        } else if origin.starts_with("agent:") {
            format!("subagent: {first}")
        } else if origin == "recap" {
            format!("{MARK} {all}")
        } else if origin == "compact" {
            format!("{MARK} conversation compacted")
        } else {
            format!("{origin}: {first}")
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
            color: AMBER,
            mono: true,
        });
        let width = cols.saturating_sub(2).max(1);
        let mut lines = wrap_hard(&Self::text(origin, &e.summary), width);
        if lines.len() > LINES {
            let rest = lines[LINES - 1..].join(" ");
            lines.truncate(LINES - 1);
            lines.push(one_line(&rest, width));
        }
        let first_row = flow.rows.len();
        let mut ly = y;
        for line in lines {
            flow.rows.push(Row {
                text: line,
                x: x + indent,
                y: ly,
                px,
                color: DIM,
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
            "task finished: Build finished"
        );
        assert_eq!(
            NoticeLine::text("peer:docs-writer", "\nRebased on main.\n\nDone."),
            "docs-writer: Rebased on main."
        );
        assert_eq!(
            NoticeLine::text("agent:a0123", "Added the test.\nIt passes."),
            "subagent: Added the test."
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
