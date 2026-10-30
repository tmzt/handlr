//! The *While you spoke* card (docs/design/20261023_02_BULK_INPUT_PLAN.md,
//! the 2026-10-30 15:10 EDT amendment): the catch-up the client gives as
//! [`flankr_companion::Event::CatchUp`] after a voice input, pinned with
//! the prompt cards.
//!
//! It is drawn as a prompt card is: the card's colour and corner, an edge
//! and a small header in the spoken colour, and the text under it, cut to
//! [`TEXT_LINES`]. An ✕ at its top right, drawn as two strokes (the faces
//! may lack the glyph), dismisses it ([`AgentHit::CatchUpDismiss`]);
//! nothing else on it does anything. It stays until dismissed or replaced
//! by the next catch-up, is not kept across restarts, and shows the same
//! muted or not.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{AgentHit, Fill, PAD, Row, columns, fill, line_h, wrap_hard};
use crate::settings::copy;
use crate::theme::theme;

/// The accent edge at the card's left, as the prompt cards have it.
const EDGE: f32 = 6.0;
/// The header's and the text's type sizes, in points.
const HEADER_PT: f32 = 13.0;
const TEXT_PT: f32 = 15.0;
/// Lines of the catch-up shown at most: about 40 words.
pub const TEXT_LINES: usize = 5;
/// The ✕'s square at the card's top right, and its arms' half length, in
/// points.
const CLOSE: f32 = 36.0;
const CROSS: f32 = 6.0;

/// The card: its text, and where it is as last laid out.
#[derive(Debug, Default)]
pub struct CatchUpCard {
    text: Option<String>,
    rect: Rect,
    close: Rect,
    fills: Vec<Fill>,
    rows: Vec<Row>,
    scale: f32,
}

impl CatchUpCard {
    pub fn new() -> Self {
        Self::default()
    }

    /// The catch-up shown, or `None` once dismissed. Returns whether it
    /// changed.
    pub fn set_text(&mut self, text: Option<String>) -> bool {
        let text = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        if self.text == text {
            return false;
        }
        self.text = text;
        if self.text.is_none() {
            self.clear_layout();
        }
        true
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn clear_layout(&mut self) {
        self.rect = Rect::default();
        self.close = Rect::default();
        self.fills.clear();
        self.rows.clear();
    }

    /// Lays the card out, `x` to `x + w`, its bottom at `bottom`. Returns
    /// its height: none without a catch-up.
    pub fn lay_out(&mut self, x: f32, w: f32, bottom: f32, s: f32) -> f32 {
        self.clear_layout();
        self.scale = s;
        let Some(text) = &self.text else {
            return 0.0;
        };
        let pad = PAD * s;
        let inner_x = x + EDGE * s + pad;
        let inner_w = w - EDGE * s - 2.0 * pad;
        let close = CLOSE * s;
        let head_px = HEADER_PT * s;
        let text_px = TEXT_PT * s;
        let mut lines = wrap_hard(text, columns(inner_w, text_px));
        if lines.len() > TEXT_LINES {
            lines.truncate(TEXT_LINES);
            if let Some(last) = lines.last_mut() {
                last.push('\u{2026}');
            }
        }
        let gap = 4.0 * s;
        let h = 2.0 * pad + line_h(head_px) + gap + lines.len() as f32 * line_h(text_px);
        let top = bottom - h;
        self.rect = Rect::new(x, top, w, h);
        self.fills.push(Fill {
            rect: self.rect,
            color: theme().card,
            radius: 10.0 * s,
        });
        self.fills.push(Fill {
            rect: Rect::new(x, top, EDGE * s, h),
            color: theme().spoken,
            radius: 3.0 * s,
        });
        let mut y = top + pad;
        self.rows.push(Row {
            text: copy::PROMPT_CATCH_UP.text.to_string(),
            x: inner_x,
            y,
            px: head_px,
            color: theme().spoken,
            mono: false,
        });
        y += line_h(head_px) + gap;
        for line in lines {
            self.rows.push(Row {
                text: line,
                x: inner_x,
                y,
                px: text_px,
                color: theme().fg,
                mono: false,
            });
            y += line_h(text_px);
        }
        self.close = Rect::new(x + w - close - 2.0 * s, top + 2.0 * s, close, close);
        h
    }

    /// The card as placed; empty while it is not shown.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Its ✕, as placed.
    pub fn close_rect(&self) -> Option<Rect> {
        self.shown().then_some(self.close)
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    fn shown(&self) -> bool {
        self.rect.h > 0.0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.shown() && self.rect.contains([x, y])
    }

    /// The ✕, if `(x, y)` is on it.
    pub fn hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        (self.shown() && self.close.contains([x, y])).then_some(AgentHit::CatchUpDismiss)
    }

    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        if !self.shown() {
            return;
        }
        let s = self.scale;
        for f in &self.fills {
            fill(list, f.rect, f.color, f.radius);
        }
        for r in &self.rows {
            ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
        }
        let [cx, cy] = self.close.center();
        let (h, t) = (CROSS * s, 2.0 * s);
        list.push_line([cx - h, cy - h], [cx + h, cy + h], t, theme().dim);
        list.push_line([cx - h, cy + h], [cx + h, cy - h], t, theme().dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laid(card: &mut CatchUpCard) -> f32 {
        card.lay_out(24.0, 1032.0, 2000.0, 2.0)
    }

    #[test]
    fn it_is_headed_while_you_spoke_with_the_text_and_an_x() {
        let mut c = CatchUpCard::new();
        assert_eq!(laid(&mut c), 0.0, "nothing to show");
        assert!(c.close_rect().is_none());
        assert!(c.set_text(Some(" Codex finished the tests. ".into())));
        assert!(!c.set_text(Some("Codex finished the tests.".into())));
        let h = laid(&mut c);
        assert!(h > 0.0);
        let texts: Vec<&str> = c.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["While you spoke", "Codex finished the tests."]);
        assert_eq!(c.rows()[0].color, theme().spoken);
        assert!((c.rect().y + c.rect().h - 2000.0).abs() < 0.01);
        // The ✕ at its top right, and only it does anything.
        let x = c.close_rect().unwrap();
        assert!(x.x + x.w <= c.rect().x + c.rect().w);
        assert!(x.x > c.rect().x + c.rect().w * 0.8);
        assert_eq!(
            c.hit(x.x + x.w * 0.5, x.y + x.h * 0.5),
            Some(AgentHit::CatchUpDismiss)
        );
        assert_eq!(
            c.hit(c.rect().x + 40.0, c.rect().y + c.rect().h - 10.0),
            None
        );
        let mut list = DrawList::new();
        c.draw(&mut list, &DrawContext::default());
        assert!(!list.instances.is_empty());
        // Dismissed: nothing shows.
        assert!(c.set_text(None));
        assert_eq!(c.rect().h, 0.0);
        assert!(!c.contains(x.x + 1.0, x.y + 1.0));
    }

    #[test]
    fn a_long_catch_up_is_cut_to_its_lines() {
        let mut c = CatchUpCard::new();
        c.set_text(Some("word ".repeat(200)));
        laid(&mut c);
        assert_eq!(c.rows().len(), 1 + TEXT_LINES);
        assert!(c.rows().last().unwrap().text.ends_with('\u{2026}'));
    }
}
