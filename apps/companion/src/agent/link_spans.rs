//! Links in the agent view (plan 20261022_03, the 21:57 EDT amendment, and
//! the owner's "we should also detect other http/https links and make them
//! clickable"): the daemon gives each entry and question its links (a
//! `docs/…` path's repowiki page, or an `http(s)://` URL); where a link's
//! text is in a block's rows it is drawn underlined in the link colour,
//! and a tap on it is [`AgentHit::Link`] with its index in the screen's
//! links.
//!
//! A long link may be broken across rows by the wrap, mid-word or at a
//! space: the block's rows are searched joined both ways, so it is found
//! either way, as a span in each row it covers.

use libcontrols::{DrawContext, Rect};
use libhandlr_proto::common::Link;
use libmsdf::drawlist::DrawList;

use super::{CELL_EM, Flow, Row, Terminal, draw_row, fill, line_h, term_h};

use crate::theme::theme;

/// A link's text in one row: `start..end` in characters, and the link's
/// index in the screen's links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    /// The row's index in its flow's rows.
    pub row: usize,
    pub start: usize,
    pub end: usize,
    pub link: usize,
}

/// The distinct links of `lists`, in order: the screen's links, which a
/// [`LinkSpan`] and [`super::AgentHit::Link`] index.
pub fn targets<'a>(lists: impl IntoIterator<Item = &'a [Link]>) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    for list in lists {
        for l in list {
            if !out.contains(l) {
                out.push(l.clone());
            }
        }
    }
    out
}

/// The spans of `links` (the block's own) in `rows`, a block's rows
/// `first..` of `flow`, each link's index taken from `targets`.
pub fn mark(flow: &mut Flow, first: usize, links: &[Link], targets: &[Link]) {
    if links.is_empty() || first >= flow.rows.len() {
        return;
    }
    let texts: Vec<&str> = flow.rows[first..].iter().map(|r| r.text.as_str()).collect();
    let mut spans = Vec::new();
    for link in links {
        let Some(index) = targets.iter().position(|t| t == link) else {
            continue;
        };
        for (row, start, end) in find(&texts, &link.text) {
            let span = LinkSpan {
                row: first + row,
                start,
                end,
                link: index,
            };
            if !spans.contains(&span) {
                spans.push(span);
            }
        }
    }
    flow.links.extend(spans);
}

/// Where `needle` is in `rows`: `(row, start, end)` in characters, for
/// each row an occurrence covers. The rows are joined with nothing (a word
/// broken mid-way) and with a space (a break at a space).
pub fn find(rows: &[&str], needle: &str) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let needle: Vec<char> = needle.chars().collect();
    for sep in [None, Some(' ')] {
        // The joined text, and for each of its characters the row and the
        // character in it (none for a separator).
        let mut joined: Vec<char> = Vec::new();
        let mut at: Vec<Option<(usize, usize)>> = Vec::new();
        for (r, text) in rows.iter().enumerate() {
            if r > 0
                && let Some(c) = sep
            {
                joined.push(c);
                at.push(None);
            }
            for (i, c) in text.chars().enumerate() {
                joined.push(c);
                at.push(Some((r, i)));
            }
        }
        let n = needle.len();
        let mut i = 0;
        while i + n <= joined.len() {
            if joined[i..i + n] != needle[..] {
                i += 1;
                continue;
            }
            // One span per row the occurrence covers.
            let mut current: Option<(usize, usize, usize)> = None;
            for p in at[i..i + n].iter().flatten() {
                match &mut current {
                    Some((row, _, end)) if *row == p.0 => *end = p.1 + 1,
                    _ => {
                        if let Some(done) = current.take() {
                            out.push(done);
                        }
                        current = Some((p.0, p.1, p.1 + 1));
                    }
                }
            }
            out.extend(current);
            i += n;
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// A row's character width: the terminal grid's cell, or roughly half an
/// em in the screen's font, as [`super::columns`] counts.
fn char_w(row: &Row) -> f32 {
    if row.mono {
        row.px * CELL_EM
    } else {
        row.px * 0.5
    }
}

/// Where `span` of `row` is, with the row drawn at `(row.x, row.y + dy)`:
/// its tap area, a little taller than the line.
pub fn rect(row: &Row, span: &LinkSpan, dy: f32) -> Rect {
    let w = char_w(row);
    let h = if row.mono {
        term_h(row.px)
    } else {
        line_h(row.px)
    };
    Rect::new(
        row.x + span.start as f32 * w,
        row.y + dy - h * 0.15,
        (span.end - span.start) as f32 * w,
        h * 1.3,
    )
}

/// Draws `row` at `pos` with its `spans` in the link colour and
/// underlined; a row with none is drawn as it is.
pub fn draw(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    row: &Row,
    spans: &[&LinkSpan],
    pos: [f32; 2],
) {
    if spans.is_empty() {
        return draw_row(list, ctx, term, row, pos);
    }
    let chars: Vec<char> = row.text.chars().collect();
    let mono = row.mono && term.is_some();
    // The x of character `i`: on the grid by cells, else by the font's
    // advance of what comes before it.
    let x_of = |i: usize| -> f32 {
        if mono {
            pos[0] + i as f32 * row.px * CELL_EM
        } else {
            let before: String = chars[..i.min(chars.len())].iter().collect();
            pos[0] + ctx.text_advance(&before, row.px)
        }
    };
    let mut sorted: Vec<&LinkSpan> = spans.to_vec();
    sorted.sort_by_key(|s| s.start);
    let mut from = 0;
    let piece = |list: &mut DrawList, a: usize, b: usize, color: [f32; 4]| {
        if a >= b {
            return;
        }
        let text: String = chars[a..b.min(chars.len())].iter().collect();
        let part = Row {
            text,
            x: 0.0,
            y: 0.0,
            color,
            ..row.clone()
        };
        draw_row(list, ctx, term, &part, [x_of(a), pos[1]]);
    };
    for s in sorted {
        let start = s.start.max(from);
        piece(list, from, start, row.color);
        piece(list, start, s.end, theme().link);
        let h = if row.mono {
            term_h(row.px)
        } else {
            line_h(row.px)
        };
        let (x0, x1) = (x_of(start), x_of(s.end));
        let thick = (row.px * 0.07).max(1.0);
        fill(
            list,
            Rect::new(x0, pos[1] + h * 0.92 - thick, (x1 - x0).max(0.0), thick),
            theme().link,
            0.0,
        );
        from = from.max(s.end);
    }
    piece(list, from, chars.len(), row.color);
}

/// The spans of `flow` in row `row`.
pub fn of_row(flow: &Flow, row: usize) -> Vec<&LinkSpan> {
    flow.links.iter().filter(|s| s.row == row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_found_in_one_row() {
        assert_eq!(find(&["see docs/a.md now"], "docs/a.md"), vec![(0, 4, 13)]);
        assert!(find(&["nothing"], "docs/a.md").is_empty());
    }

    #[test]
    fn a_link_broken_mid_word_is_found_in_both_rows() {
        // wrap_hard broke the path at the row's end.
        let rows = ["Read docs/design/2026", "1023_04_X_PLAN.md today"];
        assert_eq!(
            find(&rows, "docs/design/20261023_04_X_PLAN.md"),
            vec![(0, 5, 21), (1, 0, 17)]
        );
    }

    #[test]
    fn a_link_with_a_space_broken_there_is_found() {
        let rows = ["See the", "docs here"];
        assert_eq!(find(&rows, "the docs"), vec![(0, 4, 7), (1, 0, 4)]);
    }

    #[test]
    fn targets_are_distinct_and_in_order() {
        let a = Link {
            text: "a".into(),
            url: "u".into(),
        };
        let b = Link {
            text: "b".into(),
            url: "v".into(),
        };
        let one = [a.clone(), b.clone()];
        let two = [b.clone()];
        assert_eq!(targets([&one[..], &two[..]]), vec![a, b]);
    }
}
