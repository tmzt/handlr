//! A prompt card's history as plain text (plan 20261027_03, decision 3),
//! shown in its expanded overlay ([`PromptOverlay`]):
//!
//! - *General agent*: the conversation with Gemini, the user's words and
//!   Gemini's replies, with what STT is hearing last (as `TurnLog` held it,
//!   plan 20261023_04's 17:04 EDT amendment, A5);
//! - *Message*: only the messages sent to the session, each under its
//!   time (the amendment of 2026-10-27 01:14 EDT keeps them per session
//!   for the app's run).
//!
//! Oldest at the top; it follows the newest until scrolled back, and
//! scrolls on its own.
//!
//! [`PromptOverlay`]: super::overlay::PromptOverlay

use std::time::Duration;

use flankr_companion::Speaker;
use flankr_markdown::{DEFAULT_MARKDOWN_THRESHOLD_PCT, Style, is_markdown, lay_out};
use libcontrols::{DrawContext, Rect};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::{CELL_EM, PAD, Row, line_h};
use crate::controls::scroll::{Scroll, Touched};
use crate::theme::theme;
use crate::ui::wrap;

/// The lines kept, the oldest dropped beyond it.
pub const KEPT: usize = 200;

/// A message sent to a session: its text and the time it went, as shown
/// ("14:05").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    pub text: String,
    pub time: String,
}

/// One line of a history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A turn of the conversation with Gemini.
    Turn(Speaker, String),
    /// A message sent to the session.
    Sent(Sent),
}

/// The history, laid out in its window.
pub struct History {
    lines: Vec<Line>,
    /// What STT is hearing, before it is an utterance.
    partial: Option<String>,
    /// Shown dim while there are no lines.
    empty: Option<String>,
    viewport: Rect,
    scale: f32,
    /// In content coordinates (y from the history's top).
    rows: Vec<Row>,
    height: f32,
    scroll: Scroll,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// Type sizes, in points: the user's words and Gemini's, as the main
    /// screen had them, and a sent message's time.
    pub const USER_PT: f32 = 18.0;
    pub const GEMINI_PT: f32 = 17.0;
    pub const TIME_PT: f32 = 12.0;

    pub fn new() -> Self {
        History {
            lines: Vec::new(),
            partial: None,
            empty: None,
            viewport: Rect::default(),
            scale: 1.0,
            rows: Vec::new(),
            height: 0.0,
            scroll: Scroll::new(true),
        }
    }

    /// A line of the conversation, at the bottom.
    pub fn push(&mut self, speaker: Speaker, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.add(Line::Turn(speaker, text));
    }

    /// A message sent, at the bottom.
    pub fn push_sent(&mut self, sent: Sent) {
        if sent.text.trim().is_empty() {
            return;
        }
        self.add(Line::Sent(sent));
    }

    fn add(&mut self, line: Line) {
        self.lines.push(line);
        let drop = self.lines.len().saturating_sub(KEPT);
        self.lines.drain(..drop);
        self.relayout();
    }

    /// Every line at once, as kept elsewhere (a session's messages, or the
    /// conversation, given to a screen as it opens). Returns whether they
    /// changed.
    pub fn set_lines(&mut self, lines: Vec<Line>) -> bool {
        let skip = lines.len().saturating_sub(KEPT);
        let lines: Vec<Line> = lines.into_iter().skip(skip).collect();
        if lines == self.lines {
            return false;
        }
        self.lines = lines;
        self.relayout();
        true
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The dim line shown while there are none.
    pub fn set_empty(&mut self, empty: Option<String>) {
        if empty != self.empty {
            self.empty = empty;
            self.relayout();
        }
    }

    /// What STT is hearing, shown dim after the last line until it is an
    /// utterance. Returns whether it changed.
    pub fn set_partial(&mut self, partial: Option<String>) -> bool {
        let partial = partial.filter(|p| !p.trim().is_empty());
        if partial == self.partial {
            return false;
        }
        self.partial = partial;
        self.relayout();
        true
    }

    /// The history's window on the screen, at scale `s`.
    pub fn set_viewport(&mut self, viewport: Rect, s: f32) {
        if viewport == self.viewport && s == self.scale {
            return;
        }
        self.viewport = viewport;
        self.scale = s;
        self.relayout();
    }

    /// The theme changed: the lines are laid out again in its colours.
    pub fn restyle(&mut self) {
        self.relayout();
    }

    fn relayout(&mut self) {
        let s = self.scale;
        let pad = PAD * s;
        let w = self.viewport.w - 2.0 * pad;
        let x = self.viewport.x + pad;
        let mut rows = Vec::new();
        let mut y = 4.0 * s;
        let plain = |rows: &mut Vec<Row>, y: &mut f32, text: &str, px: f32, color| {
            for t in wrap(text, (w / (px * 0.5)).max(8.0) as usize) {
                rows.push(Row {
                    text: t,
                    x,
                    y: *y,
                    px,
                    color,
                    mono: false,
                });
                *y += line_h(px);
            }
        };

        if self.lines.is_empty()
            && self.partial.is_none()
            && let Some(empty) = &self.empty
        {
            y += 8.0 * s;
            plain(&mut rows, &mut y, empty, Self::TIME_PT * s, theme().dim);
        }

        for line in &self.lines {
            y += 8.0 * s;
            match line {
                Line::Sent(sent) => {
                    plain(
                        &mut rows,
                        &mut y,
                        &sent.time,
                        Self::TIME_PT * s,
                        theme().dim,
                    );
                    let text = format!("> {}", sent.text);
                    plain(&mut rows, &mut y, &text, Self::USER_PT * s, theme().text);
                }
                Line::Turn(Speaker::User, text) => {
                    let text = format!("> {text}");
                    plain(&mut rows, &mut y, &text, Self::USER_PT * s, theme().text);
                }
                Line::Turn(Speaker::Gemini, text) => {
                    let px = Self::GEMINI_PT * s;
                    let lh = line_h(px);
                    if is_markdown(text, DEFAULT_MARKDOWN_THRESHOLD_PCT) {
                        let width_of = |t: &str, mono: bool| -> f32 {
                            let count = t.chars().count() as f32;
                            if mono {
                                count * px * CELL_EM
                            } else {
                                count * px * 0.55
                            }
                        };
                        let measure = |t: &str, style: Style| width_of(t, style == Style::Code);
                        let laid = lay_out(text, w, &measure);
                        for line in laid.lines {
                            y += line.gap_before * lh;
                            let mut rx = x + line.indent;
                            for run in line.runs {
                                let mono = run.style == Style::Code;
                                let run_w = width_of(&run.text, mono);
                                let color = if run.dim {
                                    theme().muted
                                } else if mono {
                                    theme().code
                                } else {
                                    theme().spoken
                                };
                                rows.push(Row {
                                    text: run.text,
                                    x: rx,
                                    y,
                                    px,
                                    color,
                                    mono,
                                });
                                rx += run_w;
                            }
                            y += lh;
                        }
                    } else {
                        let text = format!("< {text}");
                        plain(&mut rows, &mut y, &text, px, theme().spoken);
                    }
                }
            }
        }

        if let Some(partial) = &self.partial {
            y += 8.0 * s;
            let text = format!("> {}\u{2026}", partial.trim());
            plain(&mut rows, &mut y, &text, Self::USER_PT * s, theme().muted);
        }

        self.height = y + 8.0 * s;
        self.rows = rows;
        self.scroll.set_extent(self.height, self.viewport.h);
    }

    /// The rows as laid out, in content coordinates.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn scroll(&self) -> &Scroll {
        &self.scroll
    }

    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, now: Duration) -> Touched {
        self.scroll.touch(id, phase, x, y, now, self.scale)
    }

    pub fn holds(&self, id: u64) -> bool {
        self.scroll.holds(id)
    }

    /// Carries a fling on. Returns whether it moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        self.scroll.tick(dt, self.scale)
    }

    /// Draws it, window and all, moved down by `down` pixels: as it slides
    /// in with the overlay.
    pub fn draw_moved(&self, list: &mut DrawList, ctx: &DrawContext, down: f32) {
        let v = self.viewport;
        let v = Rect::new(v.x, v.y + down, v.w, v.h);
        list.push_clip(v.pos(), v.size());
        let dy = v.y - self.scroll.offset();
        for r in &self.rows {
            let y = r.y + dy;
            if y + line_h(r.px) >= v.y && y <= v.y + v.h {
                ctx.draw_text(list, &r.text, [r.x, y], r.px, r.color);
            }
        }
        list.push_clip_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window on the phone.
    const HALF: Rect = Rect {
        x: 0.0,
        y: 1120.0,
        w: 1080.0,
        h: 700.0,
    };

    fn history() -> History {
        let mut l = History::new();
        l.set_viewport(HALF, 2.0);
        l
    }

    fn turn(l: &mut History, i: usize) {
        l.push(Speaker::User, format!("what is claude doing {i}"));
        l.push(
            Speaker::Gemini,
            format!("Claude is running the tests for the {i}th time, and they pass."),
        );
    }

    #[test]
    fn the_users_words_and_geminis_oldest_first() {
        let mut l = history();
        turn(&mut l, 1);
        let texts: Vec<&str> = l.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts[0], "> what is claude doing 1");
        assert!(texts[1].starts_with("< Claude is running"));
        assert_eq!(l.rows()[0].color, theme().text);
        assert_eq!(l.rows().last().unwrap().color, theme().spoken);
        // What STT hears comes last, dim, until it is an utterance.
        assert!(l.set_partial(Some("and co".into())));
        assert!(!l.set_partial(Some("and co".into())));
        let last = l.rows().last().unwrap();
        assert_eq!(
            (last.text.as_str(), last.color),
            ("> and co\u{2026}", theme().muted)
        );
        l.set_partial(None);
        assert_eq!(l.rows().last().unwrap().color, theme().spoken);
        // Nothing runs past the window.
        for r in l.rows() {
            assert!(r.text.chars().count() as f32 * r.px * 0.5 <= HALF.w - 2.0 * PAD * 2.0 + r.px);
        }
    }

    #[test]
    fn sent_messages_each_under_its_time_and_no_replies() {
        let mut l = history();
        l.set_empty(Some("No messages yet".into()));
        assert_eq!(l.rows().len(), 1);
        assert_eq!(l.rows()[0].color, theme().dim);
        l.push_sent(Sent {
            text: "please run the tests".into(),
            time: "14:05".into(),
        });
        l.push_sent(Sent {
            text: "  ".into(),
            time: "14:06".into(),
        });
        l.push_sent(Sent {
            text: "and push".into(),
            time: "14:07".into(),
        });
        let texts: Vec<&str> = l.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            ["14:05", "> please run the tests", "14:07", "> and push"]
        );
        assert_eq!(l.rows()[0].color, theme().dim);
        assert_eq!(l.rows()[1].color, theme().text);
        // Given all at once, as kept for the session: unchanged, no work.
        let lines = l.lines().to_vec();
        assert!(!l.set_lines(lines));
        assert!(l.set_lines(Vec::new()));
        assert_eq!(l.rows()[0].text, "No messages yet");
    }

    #[test]
    fn it_follows_the_newest_until_scrolled_back() {
        let mut l = history();
        for i in 0..20 {
            turn(&mut l, i);
        }
        assert!(l.scroll().max() > 0.0);
        assert!(l.scroll().following());
        assert_eq!(l.scroll().offset(), l.scroll().max());
        // Back by 300 px: new lines leave it where it is.
        l.touch(1, TouchPhase::Started, 540.0, 1200.0, Duration::ZERO);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += Duration::from_millis(16);
            l.touch(1, TouchPhase::Moved, 540.0, 1200.0 + 30.0 * i as f32, now);
        }
        l.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1500.0,
            now + Duration::from_millis(300),
        );
        let back = l.scroll().offset();
        assert!((l.scroll().max() - back - 300.0).abs() < 1.0);
        turn(&mut l, 21);
        assert_eq!(l.scroll().offset(), back);
        assert!(l.scroll().offset() < l.scroll().max());
    }

    #[test]
    fn only_the_newest_lines_are_kept() {
        let mut l = history();
        for i in 0..KEPT {
            turn(&mut l, i);
        }
        assert_eq!(l.lines.len(), KEPT);
        assert!(l.rows()[0].text.contains(&format!("{}", KEPT / 2)));
        l.push(Speaker::User, "  ".into());
        assert_eq!(l.lines.len(), KEPT);
    }

    #[test]
    fn gemini_markdown_turn_renders_markdown_styling() {
        let mut l = history();
        l.push(
            Speaker::Gemini,
            "Here is the plan:\n\n```rust\nfn main() {}\n```\n- Step 1\n- Step 2".into(),
        );
        let rows = l.rows();
        assert!(
            rows.iter().any(|r| r.mono),
            "code block should have mono: true"
        );
        assert!(
            rows.iter().any(|r| r.mono && r.color == theme().code),
            "code block should have theme().code color"
        );
        assert!(
            rows.iter().any(|r| r.text.contains("Step 1")),
            "bullet list should render Step 1"
        );
    }
}
