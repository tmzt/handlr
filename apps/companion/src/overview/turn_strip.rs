//! The one line of the Gemini turns left on the overview (plan 20261023_04,
//! the 18:07 EDT amendment, 2): just above the bottom row, and above the
//! confirmation and dictation cards' place, the newest Gemini line, dim;
//! while the user is speaking, what STT is hearing instead. A tap on it
//! opens the overlay. With nothing to show, it is hidden and takes no room.

use flankr_companion::Speaker;
use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{PAD, columns, fill, one_line};
use crate::theme::theme;

/// The strip: what it shows and where.
pub struct TurnStrip {
    /// The newest line Gemini said.
    gemini: Option<String>,
    /// What STT is hearing, before it is an utterance.
    partial: Option<String>,
    /// Whether voice recognition is active.
    listening: bool,
    rect: Rect,
    scale: f32,
}

impl Default for TurnStrip {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnStrip {
    /// The pinned card's height and its type size, in points.
    pub const HEIGHT: f32 = 48.0;
    pub const PT: f32 = 13.0;

    pub fn new() -> Self {
        TurnStrip {
            gemini: None,
            partial: None,
            listening: false,
            rect: Rect::default(),
            scale: 1.0,
        }
    }

    /// A line of the conversation: Gemini's is the one shown. Returns
    /// whether what is shown changed.
    pub fn push(&mut self, speaker: Speaker, text: &str) -> bool {
        let text = text.trim();
        if speaker != Speaker::Gemini || text.is_empty() {
            return false;
        }
        let before = self.line();
        self.gemini = Some(text.to_string());
        self.line() != before
    }

    /// What STT is hearing, shown in place of Gemini's line until it is
    /// an utterance. Returns whether what is shown changed.
    pub fn set_partial(&mut self, partial: Option<String>) -> bool {
        let partial = partial.filter(|p| !p.trim().is_empty());
        if partial == self.partial {
            return false;
        }
        let before = self.line();
        self.partial = partial;
        self.line() != before
    }

    /// Whether voice recognition is active: shows "Listening…" if no partial
    /// is heard yet.
    pub fn set_listening(&mut self, listening: bool) -> bool {
        if self.listening == listening {
            return false;
        }
        let before = self.line();
        self.listening = listening;
        self.line() != before
    }

    /// The line shown, as the turn log writes it, uncut; none while hidden.
    pub fn line(&self) -> Option<String> {
        if let Some(p) = &self.partial {
            Some(format!("> {}\u{2026}", p.trim()))
        } else if self.listening {
            Some("Listening\u{2026}".to_string())
        } else {
            self.gemini.as_ref().map(|g| format!("< {g}"))
        }
    }

    /// Whether it shows, taking its room above the bottom row.
    pub fn visible(&self) -> bool {
        self.line().is_some()
    }

    /// Its place on the screen, at scale `s`.
    pub fn set_rect(&mut self, rect: Rect, s: f32) {
        self.rect = rect;
        self.scale = s;
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The card within the horizontal area.
    pub fn card_rect(&self) -> Rect {
        let s = self.scale;
        let pad = PAD * s;
        let r = self.rect;
        Rect::new(r.x + pad, r.y + 2.0 * s, r.w - 2.0 * pad, r.h - 4.0 * s)
    }

    /// Whether `(x, y)` is on it while it shows.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.visible() && self.card_rect().contains([x, y])
    }

    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let Some(line) = self.line() else {
            return;
        };
        let s = self.scale;
        let card = self.card_rect();
        // Card background and left accent edge.
        fill(list, card, theme().card, 10.0 * s);
        fill(
            list,
            Rect::new(card.x, card.y, 4.0 * s, card.h),
            theme().accent,
            3.0 * s,
        );
        // Header: "✦ Gemini" at top left, "^" at top right.
        let header_px = 11.0 * s;
        ctx.draw_text(
            list,
            "\u{2726} Gemini",
            [card.x + 12.0 * s, card.y + 6.0 * s],
            header_px,
            theme().accent,
        );
        ctx.draw_text(
            list,
            "^",
            [card.x + card.w - 18.0 * s, card.y + 5.0 * s],
            header_px,
            theme().dim,
        );
        // Body text: latest turn, partial transcript, or listening indicator.
        let px = Self::PT * s;
        let text = one_line(&line, columns(card.w - 28.0 * s, px));
        let color = if self.listening && self.partial.is_none() {
            theme().muted
        } else {
            theme().fg
        };
        ctx.draw_text(
            list,
            &text,
            [card.x + 12.0 * s, card.y + 22.0 * s],
            px,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_until_gemini_speaks_then_its_newest_line() {
        let mut t = TurnStrip::new();
        assert!(!t.visible());
        // The user's words alone show nothing: the strip is Gemini's.
        assert!(!t.push(Speaker::User, "what is claude doing"));
        assert!(!t.visible());
        assert!(t.push(Speaker::Gemini, " Claude is running the tests. "));
        assert_eq!(t.line().as_deref(), Some("< Claude is running the tests."));
        assert!(t.push(Speaker::Gemini, "They pass."));
        assert_eq!(t.line().as_deref(), Some("< They pass."));
        assert!(!t.push(Speaker::Gemini, "They pass."));
    }

    #[test]
    fn the_live_words_while_speaking() {
        let mut t = TurnStrip::new();
        // Heard before any turn: it shows.
        assert!(t.set_partial(Some("and co".into())));
        assert_eq!(t.line().as_deref(), Some("> and co\u{2026}"));
        assert!(!t.set_partial(Some("and co".into())));
        assert!(t.set_partial(Some("  ".into())));
        assert!(!t.visible());
        t.push(Speaker::Gemini, "Done.");
        t.set_partial(Some("next".into()));
        assert_eq!(t.line().as_deref(), Some("> next\u{2026}"));
        // The words heard become an utterance: Gemini's line is back.
        assert!(t.set_partial(None));
        assert_eq!(t.line().as_deref(), Some("< Done."));
    }

    #[test]
    fn it_is_one_line_and_hit_only_while_shown() {
        let mut t = TurnStrip::new();
        let r = Rect::new(0.0, 2000.0, 1080.0, TurnStrip::HEIGHT * 2.0);
        t.set_rect(r, 2.0);
        assert!(!t.contains(540.0, 2010.0));
        t.push(Speaker::Gemini, &"a long reply ".repeat(40));
        assert!(t.contains(540.0, 2010.0));
        assert!(!t.contains(540.0, 1990.0));
        let mut list = DrawList::new();
        t.draw(&mut list, &DrawContext::default());
    }

    #[test]
    fn card_rect_has_padding_and_hit_inside_only() {
        let mut t = TurnStrip::new();
        let r = Rect::new(0.0, 2000.0, 1080.0, TurnStrip::HEIGHT * 2.0);
        t.set_rect(r, 2.0);
        t.push(Speaker::Gemini, "Hello");
        let card = t.card_rect();
        assert!(card.x > r.x);
        assert!(card.w < r.w);
        assert!(t.contains(card.x + 10.0, card.y + 10.0));
        assert!(!t.contains(r.x + 2.0, card.y + 10.0));
    }

    #[test]
    fn listening_shows_immediately_and_switches_to_partial() {
        let mut t = TurnStrip::new();
        assert!(!t.visible());
        // Immediate activation:
        assert!(t.set_listening(true));
        assert!(t.visible());
        assert_eq!(t.line().as_deref(), Some("Listening\u{2026}"));

        // Partial speech arrives:
        assert!(t.set_partial(Some("hello gemini".into())));
        assert_eq!(t.line().as_deref(), Some("> hello gemini\u{2026}"));

        // Partial speech ends, Gemini replies:
        t.set_partial(None);
        t.set_listening(false);
        assert!(!t.visible());
        t.push(Speaker::Gemini, "Hello! How can I help?");
        assert!(t.visible());
        assert_eq!(t.line().as_deref(), Some("< Hello! How can I help?"));
    }
}
