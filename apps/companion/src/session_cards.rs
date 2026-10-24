//! Other sessions' cards at the bottom of the screen (plan 20261022_03,
//! the L3 amendments of 2026-10-24 15:56, 16:00 and 16:01 EDT).
//!
//! The client gives the cards ([`handlr_companion::Event::SessionCards`]),
//! newest first. The newest is drawn: a question or approval as a card
//! whose header is in its session's colour and names it ("<agent> is
//! asking", "<agent> needs approval") over its text; a finished turn as one
//! line in its colour ("<agent> finished: <line>"). "+N" at the header's
//! right counts the rest. Text on the colour is black or white, whichever
//! contrasts more.
//!
//! The screens make room for it: under the overview's cards, above the
//! agent view's pinned card, and at the bottom of the settings. A tap opens
//! its session; a swipe down dismisses it.

use handlr_companion::{SessionCard, SessionCardKind};
use libcontrols::{DrawContext, Rect};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::{columns, fill, line_h, one_line, wrap_hard};
use crate::settings::copy;
use crate::theme::{theme, wants_dark_icons};

/// Type sizes, padding and radius, in points.
const HEADER_PT: f32 = 15.0;
const TEXT_PT: f32 = 15.0;
const PAD: f32 = 10.0;
const RADIUS: f32 = 10.0;
/// Lines of a question's or approval's text, at most.
const TEXT_LINES: usize = 2;
/// How far a finger must move down for a swipe that dismisses, and the
/// most it may move for a tap, in points.
const SWIPE: f32 = 40.0;
const TAP_SLOP: f32 = 10.0;

/// What a touch on the card did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionHit {
    /// A tap: open this pane's view.
    Open(String),
    /// A swipe down: dismiss this pane's newest card.
    Dismiss(String),
}

/// What a touch was, for the screens under the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Touched {
    /// Not on the card: the screen's.
    No,
    /// On the card, and what it did, if anything yet.
    Yes(Option<SessionHit>),
}

/// A line of the card as placed.
#[derive(Clone, Debug, PartialEq)]
pub struct CardRow {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub px: f32,
    pub color: [f32; 4],
}

/// The card as placed.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub pane_id: String,
    pub rect: Rect,
    /// The header in the session's colour (the whole card for a finished
    /// turn).
    pub header: Rect,
    pub colour: [f32; 4],
    pub rows: Vec<CardRow>,
    /// "+N", drawn at the header's right.
    pub more: Option<CardRow>,
}

/// The cards, and the newest as placed.
#[derive(Default)]
pub struct SessionCards {
    cards: Vec<SessionCard>,
    placed: Option<Placed>,
    /// The touch on the card: its id, where it started, and the most it
    /// moved.
    touch: Option<(u64, f32, f32, f32)>,
    dirty: bool,
}

/// The header's text for `c`.
pub fn header(c: &SessionCard) -> String {
    let agent = [("agent", c.agent.as_str())];
    match c.kind {
        SessionCardKind::Question => copy::SESSION_ASKING.fill(None, &agent),
        SessionCardKind::Approval => copy::SESSION_APPROVAL.fill(None, &agent),
        SessionCardKind::Finished => copy::SESSION_FINISHED.fill(
            None,
            &[("agent", c.agent.as_str()), ("line", c.text.as_str())],
        ),
    }
    .trim()
    .to_string()
}

/// "+N" for `n` more cards.
pub fn more(n: usize) -> String {
    copy::SESSION_MORE
        .fill(None, &[("count", &n.to_string())])
        .trim()
        .to_string()
}

/// Black or white, whichever reads on `c`.
fn on(c: [f32; 4]) -> [f32; 4] {
    if wants_dark_icons(c) {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    }
}

impl SessionCards {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cards, newest first, as the client gave them. Returns whether
    /// they changed.
    pub fn set(&mut self, cards: Vec<SessionCard>) -> bool {
        if cards == self.cards {
            return false;
        }
        self.cards = cards;
        self.placed = None;
        self.dirty = true;
        true
    }

    pub fn cards(&self) -> &[SessionCard] {
        &self.cards
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// The card's height at width `w` and scale `s`; 0 with none.
    pub fn height(&self, w: f32, s: f32) -> f32 {
        let Some(c) = self.cards.first() else {
            return 0.0;
        };
        let (head, text) = (line_h(HEADER_PT * s), line_h(TEXT_PT * s));
        let pad = PAD * s;
        match c.kind {
            SessionCardKind::Finished => head + pad,
            _ => {
                let lines = self.text_lines(c, w, s).len() as f32;
                head + pad + lines * text + pad
            }
        }
    }

    fn text_lines(&self, c: &SessionCard, w: f32, s: f32) -> Vec<String> {
        let cols = columns(w - 2.0 * PAD * s, TEXT_PT * s);
        let mut lines = wrap_hard(c.text.trim(), cols);
        lines.retain(|l| !l.is_empty());
        if lines.len() > TEXT_LINES {
            lines.truncate(TEXT_LINES);
            if let Some(last) = lines.last_mut() {
                *last = one_line(&format!("{last}\u{2026}"), cols);
            }
        }
        lines
    }

    /// Lays the newest card out `w` wide from `x`, its bottom at `bottom`,
    /// at scale `s`.
    pub fn place(&mut self, x: f32, bottom: f32, w: f32, s: f32) {
        let Some(c) = self.cards.first() else {
            if self.placed.take().is_some() {
                self.dirty = true;
            }
            return;
        };
        let h = self.height(w, s);
        let rect = Rect::new(x, bottom - h, w, h);
        let pad = PAD * s;
        let (head_px, text_px) = (HEADER_PT * s, TEXT_PT * s);
        let head_h = line_h(head_px) + pad;
        let colour = theme().session(c.colour);
        let header_rect = Rect::new(x, rect.y, w, head_h);
        let more_text = (self.cards.len() > 1).then(|| more(self.cards.len() - 1));
        let more_w = more_text
            .as_ref()
            .map_or(0.0, |m| (m.chars().count() as f32 + 1.0) * head_px * 0.5);
        let head_cols = columns(w - 2.0 * pad - more_w, head_px);
        let mut rows = vec![CardRow {
            text: one_line(&header(c), head_cols),
            x: x + pad,
            y: rect.y + pad * 0.5,
            px: head_px,
            color: on(colour),
        }];
        if c.kind != SessionCardKind::Finished {
            let mut y = rect.y + head_h + pad * 0.5;
            for line in self.text_lines(c, w, s) {
                rows.push(CardRow {
                    text: line,
                    x: x + pad,
                    y,
                    px: text_px,
                    color: theme().fg,
                });
                y += line_h(text_px);
            }
        }
        let more = more_text.map(|text| CardRow {
            x: x + w - pad - more_w + head_px * 0.5,
            y: rect.y + pad * 0.5,
            text,
            px: head_px,
            color: on(colour),
        });
        let placed = Placed {
            pane_id: c.pane_id.clone(),
            rect,
            header: header_rect,
            colour,
            rows,
            more,
        };
        if self.placed.as_ref() != Some(&placed) {
            self.placed = Some(placed);
            self.dirty = true;
        }
    }

    /// The newest card as placed.
    pub fn placed(&self) -> Option<&Placed> {
        self.placed.as_ref()
    }

    /// A touch: one that starts on the card is the card's to its end. A
    /// tap opens its session, a swipe down dismisses it.
    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, s: f32) -> Touched {
        if phase == TouchPhase::Started {
            let on_card = self
                .placed
                .as_ref()
                .is_some_and(|p| p.rect.contains([x, y]));
            if !on_card {
                return Touched::No;
            }
            self.touch = Some((id, x, y, 0.0));
            return Touched::Yes(None);
        }
        let Some((held, x0, y0, moved)) = self.touch else {
            return Touched::No;
        };
        if held != id {
            return Touched::No;
        }
        let moved = moved.max((x - x0).hypot(y - y0));
        self.touch = Some((held, x0, y0, moved));
        let pane = || self.placed.as_ref().map(|p| p.pane_id.clone());
        match phase {
            TouchPhase::Ended => {
                self.touch = None;
                if y - y0 >= SWIPE * s {
                    Touched::Yes(pane().map(SessionHit::Dismiss))
                } else if moved <= TAP_SLOP * s {
                    Touched::Yes(pane().map(SessionHit::Open))
                } else {
                    Touched::Yes(None)
                }
            }
            TouchPhase::Cancelled => {
                self.touch = None;
                Touched::Yes(None)
            }
            _ => Touched::Yes(None),
        }
    }

    /// Whether the card changed since this was last asked: the screen
    /// under it is to be drawn again.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Draws the card over what the screen drew.
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext, s: f32) {
        let Some(p) = &self.placed else {
            return;
        };
        let r = RADIUS * s;
        fill(list, p.rect, theme().card, r);
        fill(list, p.header, p.colour, r);
        // The header's lower corners square where the text follows.
        if p.header.h < p.rect.h {
            let half = p.header.h / 2.0;
            fill(
                list,
                Rect::new(p.header.x, p.header.y + half, p.header.w, half),
                p.colour,
                0.0,
            );
        }
        for row in p.rows.iter().chain(&p.more) {
            ctx.draw_text(list, &row.text, [row.x, row.y], row.px, row.color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    pub(crate) fn card(pane: &str, kind: SessionCardKind, colour: u8, text: &str) -> SessionCard {
        SessionCard {
            pane_id: pane.to_string(),
            session: format!("s-{pane}"),
            kind,
            agent: "Claude in libmorph".to_string(),
            colour,
            text: text.to_string(),
            at: Duration::ZERO,
        }
    }

    fn placed(cards: Vec<SessionCard>) -> SessionCards {
        let mut c = SessionCards::new();
        c.set(cards);
        c.place(24.0, 2000.0, 1032.0, 2.0);
        c
    }

    #[test]
    fn the_header_names_the_agent_in_its_sessions_colour() {
        crate::theme::testing::use_theme("dark_gray");
        let c = placed(vec![card(
            "w1:p1",
            SessionCardKind::Question,
            5,
            "Ship it?",
        )]);
        let p = c.placed().expect("placed");
        assert_eq!(p.colour, theme().session(5));
        assert_eq!(p.rows[0].text, "Claude in libmorph is asking");
        assert_eq!(p.rows[1].text, "Ship it?");
        assert!(p.more.is_none());
        assert_eq!(
            p.rect.y + p.rect.h,
            2000.0,
            "its bottom where it was placed"
        );
        let a = placed(vec![card(
            "w1:p1",
            SessionCardKind::Approval,
            0,
            "cargo publish",
        )]);
        assert_eq!(
            a.placed().unwrap().rows[0].text,
            "Claude in libmorph needs approval"
        );
    }

    #[test]
    fn a_finished_card_is_one_line() {
        let c = placed(vec![card("w1:p1", SessionCardKind::Finished, 2, "Done.")]);
        let p = c.placed().unwrap();
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.rows[0].text, "Claude in libmorph finished: Done.");
        assert_eq!(p.header, p.rect, "the colour is the whole card");
        let q = placed(vec![card(
            "w1:p1",
            SessionCardKind::Question,
            2,
            "Ship it?",
        )]);
        assert!(p.rect.h < q.placed().unwrap().rect.h);
    }

    #[test]
    fn several_collapse_to_the_newest_with_plus_n() {
        let c = placed(vec![
            card("w1:p2", SessionCardKind::Question, 1, "Two?"),
            card("w1:p1", SessionCardKind::Question, 2, "One?"),
            card("w1:p3", SessionCardKind::Approval, 3, "rm"),
        ]);
        let p = c.placed().unwrap();
        assert_eq!(p.pane_id, "w1:p2");
        assert_eq!(p.rows[1].text, "Two?");
        assert_eq!(p.more.as_ref().unwrap().text, "+2");
    }

    #[test]
    fn a_tap_opens_and_a_swipe_down_dismisses() {
        let mut c = placed(vec![card("w1:p1", SessionCardKind::Question, 1, "Ship?")]);
        let r = c.placed().unwrap().rect;
        let (x, y) = (r.x + 40.0, r.y + 10.0);
        // Not on it: the screen's.
        assert_eq!(
            c.touch(1, TouchPhase::Started, x, r.y - 50.0, 2.0),
            Touched::No
        );
        assert_eq!(
            c.touch(1, TouchPhase::Ended, x, r.y - 50.0, 2.0),
            Touched::No
        );
        assert_eq!(
            c.touch(2, TouchPhase::Started, x, y, 2.0),
            Touched::Yes(None)
        );
        assert_eq!(
            c.touch(2, TouchPhase::Ended, x + 2.0, y, 2.0),
            Touched::Yes(Some(SessionHit::Open("w1:p1".into())))
        );
        c.touch(3, TouchPhase::Started, x, y, 2.0);
        c.touch(3, TouchPhase::Moved, x, y + 50.0, 2.0);
        assert_eq!(
            c.touch(3, TouchPhase::Ended, x, y + 90.0, 2.0),
            Touched::Yes(Some(SessionHit::Dismiss("w1:p1".into())))
        );
        // A drag sideways is neither.
        c.touch(4, TouchPhase::Started, x, y, 2.0);
        assert_eq!(
            c.touch(4, TouchPhase::Ended, x + 200.0, y, 2.0),
            Touched::Yes(None)
        );
    }

    #[test]
    fn its_text_reads_on_every_session_colour_of_every_theme() {
        for t in &crate::theme::Themes::shipped().list {
            for i in 0..16u8 {
                let c = t.colors.session(i);
                let ink = on(c);
                let (a, b) = (crate::theme::luminance(c), crate::theme::luminance(ink));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(ratio >= 3.0, "{} session_{}: {ratio:.2}", t.name, i + 1);
            }
        }
    }

    #[test]
    fn none_takes_no_room() {
        let mut c = SessionCards::new();
        assert_eq!(c.height(1000.0, 2.0), 0.0);
        c.place(0.0, 100.0, 1000.0, 2.0);
        assert!(c.placed().is_none());
        c.set(vec![card("w1:p1", SessionCardKind::Finished, 1, "x")]);
        assert!(c.take_dirty());
        c.set(vec![]);
        c.place(0.0, 100.0, 1000.0, 2.0);
        assert!(c.placed().is_none());
    }
}
