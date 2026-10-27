//! Gemini's two bars, while its card is hidden (plan 20261027_03,
//! decision 2, as the amendment of 2026-10-27 01:14 EDT answers open
//! question 1): pinned above the controls, each one line high and drawn
//! dim with a small "✦" at its left:
//!
//! - the input bar: what is being heard or typed, or "Listening…", else
//!   the user's last words;
//! - the output bar: Gemini's latest line.
//!
//! A bar with nothing to say is left out. A tap on either opens the card
//! again ([`super::Prompts::open_from_bars`]).

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{GEMINI_GLYPH, columns, fill, line_h, one_line};
use crate::theme::theme;

/// The bars as laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bars {
    /// Each bar shown, top to bottom: its rectangle and its line, uncut.
    bars: Vec<(Rect, String)>,
    scale: f32,
}

impl Bars {
    /// A bar's height, and its type size, in points.
    pub const HEIGHT: f32 = 26.0;
    pub const PT: f32 = 13.0;
    /// The gap between the two.
    const GAP: f32 = 4.0;

    /// Lays out `input` above `output` (each shown only with something to
    /// say), the lower's bottom at `bottom`. Returns the height taken.
    pub fn lay_out(
        &mut self,
        input: Option<String>,
        output: Option<String>,
        x: f32,
        w: f32,
        bottom: f32,
        s: f32,
    ) -> f32 {
        self.scale = s;
        self.bars.clear();
        let lines: Vec<String> = [input, output].into_iter().flatten().collect();
        let h = Self::HEIGHT * s;
        let gap = Self::GAP * s;
        let n = lines.len() as f32;
        if lines.is_empty() {
            return 0.0;
        }
        let total = n * h + (n - 1.0) * gap;
        let mut y = bottom - total;
        for line in lines {
            self.bars.push((Rect::new(x, y, w, h), line));
            y += h + gap;
        }
        total
    }

    /// Leaves nothing shown.
    pub fn clear(&mut self) {
        self.bars.clear();
    }

    /// The lines shown, top to bottom, uncut.
    pub fn lines(&self) -> Vec<&str> {
        self.bars.iter().map(|(_, l)| l.as_str()).collect()
    }

    /// Both bars together, while one shows.
    pub fn rect(&self) -> Option<Rect> {
        let first = self.bars.first()?.0;
        let last = self.bars.last()?.0;
        Some(Rect::new(
            first.x,
            first.y,
            first.w,
            last.y + last.h - first.y,
        ))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.bars.iter().any(|(r, _)| r.contains([x, y]))
    }

    /// Each bar: a dim card, "✦" at its left, and its line, dim.
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = Self::PT * s;
        for (r, line) in &self.bars {
            fill(list, *r, theme().card, 8.0 * s);
            let y = r.y + (r.h - line_h(px)) * 0.5;
            ctx.draw_text(list, GEMINI_GLYPH, [r.x + 10.0 * s, y], px, theme().dim);
            let text_x = r.x + 30.0 * s;
            let text = one_line(line, columns(r.x + r.w - 10.0 * s - text_x, px));
            ctx.draw_text(list, &text, [text_x, y], px, theme().dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_one_line_bars_or_only_what_has_something_to_say() {
        let mut b = Bars::default();
        let h = b.lay_out(
            Some("Listening\u{2026}".into()),
            Some("< Done.".into()),
            24.0,
            1032.0,
            2000.0,
            2.0,
        );
        assert_eq!(b.lines(), ["Listening\u{2026}", "< Done."]);
        let r = b.rect().unwrap();
        assert!((r.y + r.h - 2000.0).abs() < 0.01);
        assert!((h - r.h).abs() < 0.01);
        assert!(b.contains(100.0, r.y + 5.0));
        assert!(!b.contains(100.0, r.y - 5.0));
        let mut list = DrawList::new();
        b.draw(&mut list, &DrawContext::default());
        // Dim: nothing in the text colour.
        assert!(list.instances.iter().all(|i| i.color != theme().text));
        assert_eq!(
            b.lay_out(None, Some("< Done.".into()), 24.0, 1032.0, 2000.0, 2.0),
            Bars::HEIGHT * 2.0
        );
        assert_eq!(b.lay_out(None, None, 24.0, 1032.0, 2000.0, 2.0), 0.0);
        assert!(b.rect().is_none());
    }
}
