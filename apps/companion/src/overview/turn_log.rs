//! The overview's lower half: the conversation with Gemini as plain text
//! (plan 20261023_04, the 17:04 EDT amendment, A5), the main screen's
//! conversation moved here. The user's words and Gemini's replies, oldest
//! at the top, with what STT is hearing last; it follows the newest until
//! scrolled back, as the agent view's flow does, and scrolls on its own.

use std::time::Duration;

use handlr_companion::Speaker;
use libcontrols::{DrawContext, Rect};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::{PAD, Row, line_h};
use crate::controls::scroll::{Scroll, Touched};
use crate::ui::{MUTED, SPOKEN, TEXT, wrap};

/// The lines kept, the oldest dropped beyond it.
pub const KEPT: usize = 200;

/// The conversation, laid out in its window.
pub struct TurnLog {
    lines: Vec<(Speaker, String)>,
    /// What STT is hearing, before it is an utterance.
    partial: Option<String>,
    viewport: Rect,
    scale: f32,
    /// In content coordinates (y from the log's top).
    rows: Vec<Row>,
    height: f32,
    scroll: Scroll,
}

impl Default for TurnLog {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnLog {
    /// Type sizes, in points: the user's words and Gemini's, as the main
    /// screen had them.
    pub const USER_PT: f32 = 18.0;
    pub const GEMINI_PT: f32 = 17.0;

    pub fn new() -> Self {
        TurnLog {
            lines: Vec::new(),
            partial: None,
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
        self.lines.push((speaker, text));
        let drop = self.lines.len().saturating_sub(KEPT);
        self.lines.drain(..drop);
        self.relayout();
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

    /// The log's window on the screen, at scale `s`.
    pub fn set_viewport(&mut self, viewport: Rect, s: f32) {
        if viewport == self.viewport && s == self.scale {
            return;
        }
        self.viewport = viewport;
        self.scale = s;
        self.relayout();
    }

    fn relayout(&mut self) {
        let s = self.scale;
        let pad = PAD * s;
        let w = self.viewport.w - 2.0 * pad;
        let mut rows = Vec::new();
        let mut y = 4.0 * s;
        let partial = self
            .partial
            .as_ref()
            .map(|p| (format!("> {}\u{2026}", p.trim()), Self::USER_PT, MUTED));
        let lines = self.lines.iter().map(|(speaker, text)| match speaker {
            Speaker::User => (format!("> {text}"), Self::USER_PT, TEXT),
            Speaker::Gemini => (format!("< {text}"), Self::GEMINI_PT, SPOKEN),
        });
        for (text, pt, color) in lines.chain(partial) {
            let px = pt * s;
            y += 8.0 * s;
            for t in wrap(&text, (w / (px * 0.5)).max(8.0) as usize) {
                rows.push(Row {
                    text: t,
                    x: self.viewport.x + pad,
                    y,
                    px,
                    color,
                    mono: false,
                });
                y += line_h(px);
            }
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

    /// Carries a fling on. Returns whether the log moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        self.scroll.tick(dt, self.scale)
    }

    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let v = self.viewport;
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

    /// The lower half of the phone.
    const HALF: Rect = Rect {
        x: 0.0,
        y: 1120.0,
        w: 1080.0,
        h: 700.0,
    };

    fn log() -> TurnLog {
        let mut l = TurnLog::new();
        l.set_viewport(HALF, 2.0);
        l
    }

    fn turn(l: &mut TurnLog, i: usize) {
        l.push(Speaker::User, format!("what is claude doing {i}"));
        l.push(
            Speaker::Gemini,
            format!("Claude is running the tests for the {i}th time, and they pass."),
        );
    }

    #[test]
    fn the_users_words_and_geminis_oldest_first() {
        let mut l = log();
        turn(&mut l, 1);
        let texts: Vec<&str> = l.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts[0], "> what is claude doing 1");
        assert!(texts[1].starts_with("< Claude is running"));
        assert_eq!(l.rows()[0].color, TEXT);
        assert_eq!(l.rows().last().unwrap().color, SPOKEN);
        // What STT hears comes last, dim, until it is an utterance.
        assert!(l.set_partial(Some("and co".into())));
        assert!(!l.set_partial(Some("and co".into())));
        let last = l.rows().last().unwrap();
        assert_eq!(
            (last.text.as_str(), last.color),
            ("> and co\u{2026}", MUTED)
        );
        l.set_partial(None);
        assert_eq!(l.rows().last().unwrap().color, SPOKEN);
        // Nothing runs past the window.
        for r in l.rows() {
            assert!(r.text.chars().count() as f32 * r.px * 0.5 <= HALF.w - 2.0 * PAD * 2.0 + r.px);
        }
    }

    #[test]
    fn it_follows_the_newest_until_scrolled_back() {
        let mut l = log();
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
        let mut l = log();
        for i in 0..KEPT {
            turn(&mut l, i);
        }
        assert_eq!(l.lines.len(), KEPT);
        assert!(l.rows()[0].text.contains(&format!("{}", KEPT / 2)));
        l.push(Speaker::User, "  ".into());
        assert_eq!(l.lines.len(), KEPT);
    }
}
