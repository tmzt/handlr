//! The preview's text bodies (docs/design/20261028_05_BINARY_FILES_PLAN.md,
//! decision 10.3, and the 23:00 EDT amendment): plain text and diffs laid
//! out as monospaced rows wrapped to the width, a dim ↩ at each soft wrap,
//! at most [`LINES_MAX`] lines; a diff's lines in the transcript's diff
//! colours; and the scrollbar's section labels: a Markdown file's `#` and
//! `##` headings, a diff's files, and a `git show`'s commit.

use crate::theme::theme;

/// The most lines a plain text or diff body shows.
pub const LINES_MAX: usize = 20_000;
/// A Markdown heading's label is cut to this many characters.
pub const LABEL_MAX: usize = 40;

/// One row of a text body.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRow {
    pub text: String,
    pub color: [f32; 4],
    /// The line goes on in the next row: a dim ↩ at its end.
    pub wraps: bool,
    /// Its line's index in the source.
    pub line: usize,
}

/// A diff line's colour (the transcript's): `diff `, `index `, `--- `
/// and `+++ ` dim, `@@` the accent, `+` added, `-` removed.
pub fn diff_colour(line: &str) -> [f32; 4] {
    let t = theme();
    if ["diff ", "index ", "--- ", "+++ "]
        .iter()
        .any(|p| line.starts_with(p))
    {
        t.dim
    } else if line.starts_with("@@") {
        t.accent
    } else if line.starts_with('+') {
        t.green
    } else if line.starts_with('-') {
        t.red
    } else {
        t.fg
    }
}

/// `text` as rows of at most `cols` characters, in the diff colours when
/// `diff`; and whether it was cut at [`LINES_MAX`] lines.
pub fn rows(text: &str, cols: usize, diff: bool) -> (Vec<TextRow>, bool) {
    let cols = cols.max(8);
    let mut out = Vec::new();
    let mut cut = false;
    // A trailing newline ends the last line; it makes none.
    let text = text.strip_suffix('\n').unwrap_or(text);
    for (i, raw) in text.split('\n').enumerate() {
        if i >= LINES_MAX {
            cut = true;
            break;
        }
        let raw = raw.trim_end_matches('\r').replace('\t', "    ");
        let color = if diff { diff_colour(&raw) } else { theme().fg };
        let chars: Vec<char> = raw.chars().collect();
        if chars.is_empty() {
            out.push(TextRow {
                text: String::new(),
                color,
                wraps: false,
                line: i,
            });
            continue;
        }
        // One column is kept for the ↩.
        let room = cols - 1;
        let mut at = 0;
        while at < chars.len() {
            let end = (at + room).min(chars.len());
            out.push(TextRow {
                text: chars[at..end].iter().collect(),
                color,
                wraps: end < chars.len(),
                line: i,
            });
            at = end;
        }
    }
    (out, cut)
}

/// A scrollbar label: its text and the source line it is at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub line: usize,
    pub text: String,
}

/// `text` cut to `max` characters with "…".
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max - 1).collect();
    format!("{kept}\u{2026}")
}

/// A Markdown file's labels: one per `#` and `##` heading outside fenced
/// code, its text cut to [`LABEL_MAX`] characters, at its source line.
pub fn markdown_labels(md: &str) -> Vec<Label> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in md.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.starts_with("    ") {
            continue;
        }
        let heading = t
            .strip_prefix("## ")
            .or_else(|| t.strip_prefix("# "))
            .map(|h| h.trim().trim_end_matches('#').trim());
        if let Some(h) = heading.filter(|h| !h.is_empty()) {
            out.push(Label {
                line: i,
                text: cut(h, LABEL_MAX),
            });
        }
    }
    out
}

/// A diff's labels: one per file at its `diff --git` line, its name (the
/// last component; with its folder when two files share a name), and for a
/// `git show` a first label `commit <hash8>` at the top.
pub fn diff_labels(text: &str) -> Vec<Label> {
    let mut files: Vec<(usize, String)> = Vec::new();
    let mut commit: Option<String> = None;
    for (i, line) in text.lines().enumerate() {
        if i == 0
            && let Some(hash) = line.strip_prefix("commit ")
        {
            commit = Some(hash.chars().take(8).collect());
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // `a/<path> b/<path>`: the new side's path.
            let path = rest
                .rsplit_once(" b/")
                .map(|(_, b)| b)
                .unwrap_or_else(|| rest.trim_start_matches("a/"));
            files.push((i, path.to_string()));
        }
    }
    let name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let mut out: Vec<Label> = commit
        .into_iter()
        .map(|h| Label {
            line: 0,
            text: format!("commit {h}"),
        })
        .collect();
    for (line, path) in &files {
        let n = name(path);
        let shared = files.iter().filter(|(_, p)| name(p) == n).count() > 1;
        let text = if shared {
            let mut parts = path.rsplit('/');
            let file = parts.next().unwrap_or(path);
            match parts.next() {
                Some(folder) => format!("{folder}/{file}"),
                None => file.to_string(),
            }
        } else {
            n
        };
        out.push(Label { line: *line, text });
    }
    out
}

/// Whether a text body's scrollbar is shown always, whatever the setting:
/// it is longer than three screens (the 23:00 EDT amendment, item 2).
pub fn always_shown(content_h: f32, window_h: f32) -> bool {
    window_h > 0.0 && content_h > 3.0 * window_h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_wrap_with_a_mark_and_keep_their_lines() {
        let (r, cut) = rows("abcdefghij\n\nxy\n", 8, false);
        assert!(!cut);
        let texts: Vec<(&str, bool, usize)> = r
            .iter()
            .map(|r| (r.text.as_str(), r.wraps, r.line))
            .collect();
        assert_eq!(
            texts,
            [
                ("abcdefg", true, 0),
                ("hij", false, 0),
                ("", false, 1),
                ("xy", false, 2)
            ]
        );
    }

    #[test]
    fn rows_stop_at_twenty_thousand_lines() {
        let text = "x\n".repeat(LINES_MAX + 5);
        let (r, cut) = rows(&text, 80, false);
        assert!(cut);
        assert_eq!(r.len(), LINES_MAX);
        let (r, cut) = rows(&"x\n".repeat(LINES_MAX), 80, false);
        assert!(!cut);
        assert_eq!(r.len(), LINES_MAX);
    }

    #[test]
    fn diff_lines_take_the_transcripts_colours() {
        let t = theme();
        assert_eq!(diff_colour("diff --git a/x b/x"), t.dim);
        assert_eq!(diff_colour("index 1..2"), t.dim);
        assert_eq!(diff_colour("--- a/x"), t.dim);
        assert_eq!(diff_colour("+++ b/x"), t.dim);
        assert_eq!(diff_colour("@@ -1 +1 @@"), t.accent);
        assert_eq!(diff_colour("+new"), t.green);
        assert_eq!(diff_colour("-old"), t.red);
        assert_eq!(diff_colour(" same"), t.fg);
    }

    #[test]
    fn markdown_labels_are_its_first_two_heading_levels() {
        let md = "# Plan\n\ntext\n## 1. Goal\n### deeper\n```\n# not\n```\n## A heading that is much longer than forty characters in all\n";
        let labels = markdown_labels(md);
        assert_eq!(
            labels,
            [
                Label {
                    line: 0,
                    text: "Plan".into()
                },
                Label {
                    line: 3,
                    text: "1. Goal".into()
                },
                Label {
                    line: 8,
                    text: "A heading that is much longer than fort\u{2026}".into()
                },
            ]
        );
        assert_eq!(labels[2].text.chars().count(), 40);
    }

    #[test]
    fn a_two_file_diff_is_labelled_by_file_and_a_show_by_its_commit() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n+x\ndiff --git a/docs/b.md b/docs/b.md\n-y\n";
        assert_eq!(
            diff_labels(diff),
            [
                Label {
                    line: 0,
                    text: "a.rs".into()
                },
                Label {
                    line: 2,
                    text: "b.md".into()
                }
            ]
        );
        let show = "commit 1a2b3c4d5e6f\nAuthor: x\n\ndiff --git a/a/mod.rs b/a/mod.rs\n+1\ndiff --git a/b/mod.rs b/b/mod.rs\n+2\n";
        assert_eq!(
            diff_labels(show),
            [
                Label {
                    line: 0,
                    text: "commit 1a2b3c4d".into()
                },
                Label {
                    line: 3,
                    text: "a/mod.rs".into()
                },
                Label {
                    line: 5,
                    text: "b/mod.rs".into()
                }
            ]
        );
        // Plain text has none: the caller asks for no labels.
        assert!(diff_labels("just text\n").is_empty());
    }

    #[test]
    fn past_three_screens_the_thumb_is_always_shown() {
        assert!(!always_shown(300.0, 100.0));
        assert!(always_shown(301.0, 100.0));
        assert!(!always_shown(1000.0, 0.0));
    }
}
