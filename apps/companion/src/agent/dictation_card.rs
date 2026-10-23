//! The message being taken, pinned above the controls: on its own card, or
//! in the card it replies to with the controls inside it (owner,
//! 2026-10-23: "the entered text should be on a pinned card with the
//! scrolling pane pushed up and out of the way. the same applied to an
//! inline message, it expands the card being responded to with the same
//! buttons in place, hiding the global ones").

use libcontrols::Rect;

use super::control_row::ControlRow;
use super::{
    AgentCard, BUTTON_H, CARD_BG, CARD_PT, DICTATION_BG, DICTATION_LINES, EDGE, FG, Fill, Flow,
    PAD, PIN_LINES, Row, SHADOW, TITLE_PT, accent, columns, line_h, wrap_hard,
};
use crate::ui::Line;

/// The message being taken, as a block whose bottom is at `bottom`: its
/// lines on their own card, or, `replying`, under that card's title and
/// text with the control row inside it.
pub struct DictationCard;

impl DictationCard {
    pub fn lay_out(
        lines: &[Line],
        replying: Option<&AgentCard>,
        x: f32,
        w: f32,
        bottom: f32,
        s: f32,
    ) -> Flow {
        let pad = PAD * s;
        let edge = if replying.is_some() { EDGE * s } else { 0.0 };
        let inner_x = x + edge + pad;
        let inner_w = w - edge - 2.0 * pad;
        let mut rows: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
        if let Some(card) = replying {
            rows.push((card.title.clone(), TITLE_PT * s, accent(card.kind), 0.0));
            let px = CARD_PT * s;
            let mut text = wrap_hard(card.text.trim(), columns(inner_w, px));
            if text.len() > PIN_LINES {
                text.truncate(PIN_LINES);
                if let Some(last) = text.last_mut() {
                    last.push('\u{2026}');
                }
            }
            for (i, t) in text.into_iter().enumerate() {
                rows.push((t, px, FG, if i == 0 { 4.0 * s } else { 0.0 }));
            }
        }
        let head = rows.len();
        let mut said: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
        for line in lines {
            let px = line.size * s;
            for (i, t) in wrap_hard(&line.text, columns(inner_w, px))
                .into_iter()
                .enumerate()
            {
                let gap = if i == 0 { line.gap.min(8.0) * s } else { 0.0 };
                said.push((t, px, line.color, gap));
            }
        }
        let skip = said.len().saturating_sub(DICTATION_LINES);
        if head > 0 && skip < said.len() {
            said[skip].3 = said[skip].3.max(10.0 * s);
        }
        rows.extend(said.into_iter().skip(skip));
        let controls = if replying.is_some() {
            BUTTON_H * s + pad
        } else {
            0.0
        };
        let h: f32 = rows
            .iter()
            .map(|(_, px, _, gap)| line_h(*px) + gap)
            .sum::<f32>()
            + 2.0 * pad
            + controls;
        let top = bottom - h;
        let mut flow = Flow::default();
        if let Some(card) = replying {
            flow.fills.push(Fill {
                rect: Rect::new(x + 3.0 * s, top + 4.0 * s, w, h),
                color: SHADOW,
                radius: 10.0 * s,
            });
            flow.fills.push(Fill {
                rect: Rect::new(x, top, w, h),
                color: CARD_BG,
                radius: 10.0 * s,
            });
            flow.fills.push(Fill {
                rect: Rect::new(x, top, EDGE * s, h),
                color: accent(card.kind),
                radius: 3.0 * s,
            });
            flow.buttons = ControlRow::lay_out(inner_x, x + w - pad, bottom - pad, s, true);
        } else {
            flow.fills.push(Fill {
                rect: Rect::new(x, top, w, h),
                color: DICTATION_BG,
                radius: 10.0 * s,
            });
        }
        let mut y = top + pad;
        for (text, px, color, gap) in rows {
            y += gap;
            flow.rows.push(Row {
                text,
                x: inner_x,
                y,
                px,
                color,
                mono: false,
            });
            y += line_h(px);
        }
        flow.height = h;
        flow
    }
}
