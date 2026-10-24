//! The response just sent from the view, shown as typed input in amber in
//! an input box above the controls until the session shows it (owner,
//! 2026-10-23: "let's convert the responses to typed amber text with the
//! input box").

use libcontrols::Rect;

use super::{Fill, Flow, PAD, Row, TEXT_PT, term_columns, term_h, wrap_hard};

use crate::theme::theme;

/// The most lines of the typed text shown.
const LINES: usize = 3;

pub struct InputBox;

impl InputBox {
    /// `text` in a bordered box `w` wide from `x`, its bottom at `bottom`,
    /// on the terminal grid after a `›` prompt.
    pub fn lay_out(text: &str, x: f32, w: f32, bottom: f32, s: f32) -> Flow {
        let pad = PAD * s;
        let px = TEXT_PT * s;
        let border = 1.5 * s;
        let mut lines = wrap_hard(text, term_columns(w - 2.0 * pad, px).saturating_sub(2));
        if lines.len() > LINES {
            lines.truncate(LINES);
            if let Some(last) = lines.last_mut() {
                last.push('\u{2026}');
            }
        }
        let h = lines.len() as f32 * term_h(px) + 2.0 * pad * 0.75;
        let top = bottom - h;
        let mut flow = Flow::default();
        flow.fills.push(Fill {
            rect: Rect::new(x, top, w, h),
            color: theme().band,
            radius: 8.0 * s,
        });
        flow.fills.push(Fill {
            rect: Rect::new(x + border, top + border, w - 2.0 * border, h - 2.0 * border),
            color: theme().card,
            radius: 8.0 * s - border,
        });
        let mut y = top + pad * 0.75;
        for (i, line) in lines.into_iter().enumerate() {
            let prompt = if i == 0 { "\u{203A} " } else { "  " };
            flow.rows.push(Row {
                text: format!("{prompt}{line}"),
                x: x + pad,
                y,
                px,
                color: theme().accent,
                mono: true,
            });
            y += term_h(px);
        }
        flow.height = h;
        flow
    }
}
