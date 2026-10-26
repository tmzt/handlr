//! The message being taken, pinned above the controls: on its own card, or
//! in the card it replies to with the controls inside it (owner,
//! 2026-10-23: "the entered text should be on a pinned card with the
//! scrolling pane pushed up and out of the way. the same applied to an
//! inline message, it expands the card being responded to with the same
//! buttons in place, hiding the global ones").

use flankr_companion::PromptMode;
use libcontrols::{Rect, TextArea};

use super::{
    AgentCard, BUTTON_H, CARD_PT, DICTATION_LINES, EDGE, Fill, Flow, PAD, PIN_LINES, Row, TITLE_PT,
    accent, columns, line_h, wrap_hard,
};
use crate::controls::ControlRow;
use crate::theme::theme;
use crate::ui::Line;

/// Drops font size as the text wraps onto more lines so that the multi-line
/// message fits comfortably in the input card.
fn fit_font_size(text: &str, base_pt: f32, box_w: f32, s: f32) -> (f32, Vec<String>) {
    let base = base_pt.min(12.0);
    let sizes = [base, 11.0, 10.0];
    for &pt in &sizes {
        if pt > base {
            continue;
        }
        let px = pt * s;
        let mut rows = wrap_hard(text, columns(box_w, px));
        if rows.is_empty() {
            rows.push(String::new());
        }
        let max_lines = match pt as usize {
            12.. => 3,
            11.. => 4,
            _ => usize::MAX,
        };
        if rows.len() <= max_lines || pt <= 10.0 {
            return (pt, rows);
        }
    }
    let pt = 10.0;
    let mut rows = wrap_hard(text, columns(box_w, pt * s));
    if rows.is_empty() {
        rows.push(String::new());
    }
    (pt, rows)
}

/// The message being taken, as a block whose bottom is at `bottom`: its
/// lines on their own card, or, `replying`, under that card's title and
/// text with the control row inside it.
pub struct DictationCard;

impl DictationCard {
    pub fn lay_out(
        lines: &[Line],
        mode: PromptMode,
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
                rows.push((t, px, theme().fg, if i == 0 { 4.0 * s } else { 0.0 }));
            }
        }
        let head = rows.len();

        let controls = if replying.is_some() {
            BUTTON_H * s + pad
        } else {
            0.0
        };

        if mode == PromptMode::Keyboard {
            let (input_line, head_lines) = match lines.split_last() {
                Some((last, rest)) => (Some(last), rest),
                None => (None, lines),
            };

            for line in head_lines {
                let px = line.size * s;
                for (i, t) in wrap_hard(&line.text, columns(inner_w, px))
                    .into_iter()
                    .enumerate()
                {
                    let gap = if i == 0 { line.gap.min(8.0) * s } else { 0.0 };
                    rows.push((t, px, line.color, gap));
                }
            }

            let box_pad_x = 10.0 * s;
            let box_pad_y = 8.0 * s;
            let box_inner_w = inner_w - 2.0 * box_pad_x;
            let input_text = input_line.map_or("", |l| l.text.as_str());
            let input_color = input_line.map_or(theme().muted, |l| l.color);
            let base_pt = input_line.map_or(12.0, |l| l.size.min(12.0));
            let (fit_pt, input_rows) = fit_font_size(input_text, base_pt, box_inner_w, s);
            let px = fit_pt * s;
            let line_count = input_rows.len().max(3);
            let input_text_h = line_count as f32 * line_h(px);
            let box_h = input_text_h + 2.0 * box_pad_y;
            let gap_before_box = if !rows.is_empty() { 8.0 * s } else { 0.0 };

            let rows_h: f32 = rows.iter().map(|(_, px, _, gap)| line_h(*px) + gap).sum();
            let h = rows_h + gap_before_box + box_h + 2.0 * pad + controls;
            let top = bottom - h;
            let mut flow = Flow::default();

            if let Some(card) = replying {
                flow.fills.push(Fill {
                    rect: Rect::new(x + 3.0 * s, top + 4.0 * s, w, h),
                    color: theme().shadow,
                    radius: 10.0 * s,
                });
                flow.fills.push(Fill {
                    rect: Rect::new(x, top, w, h),
                    color: theme().card,
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
                    color: theme().card,
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

            let box_y = y + gap_before_box;
            let box_rect = Rect::new(inner_x, box_y, inner_w, box_h);
            let is_placeholder = input_text == "Type your message";
            let actual_text = if is_placeholder { "" } else { input_text };

            let text_area = TextArea::new(actual_text, box_rect)
                .with_padding([box_pad_x, box_pad_y])
                .with_font_size(px)
                .with_text_color(theme().text)
                .with_bg(theme().input)
                .with_corner_radius(8.0 * s)
                .with_placeholder("Type your message")
                .with_active(true);
            text_area.set_cursor(actual_text.len());
            flow.text_area = Some(text_area);

            flow.fills.push(Fill {
                rect: box_rect,
                color: theme().input,
                radius: 8.0 * s,
            });

            for (i, text) in input_rows.into_iter().enumerate() {
                flow.rows.push(Row {
                    text,
                    x: inner_x + box_pad_x,
                    y: box_y + box_pad_y + i as f32 * line_h(px),
                    px,
                    color: input_color,
                    mono: false,
                });
            }

            flow.height = h;
            return flow;
        }

        let mut said: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            let is_last = n == lines.len() - 1;
            let (fit_pt, line_rows) = if is_last && line.size >= 18.0 {
                fit_font_size(&line.text, line.size, inner_w, s)
            } else {
                let px = line.size * s;
                (line.size, wrap_hard(&line.text, columns(inner_w, px)))
            };
            let px = fit_pt * s;
            for (i, t) in line_rows.into_iter().enumerate() {
                let gap = if i == 0 { line.gap.min(8.0) * s } else { 0.0 };
                said.push((t, px, line.color, gap));
            }
        }
        let skip = said.len().saturating_sub(DICTATION_LINES);
        if head > 0 && skip < said.len() {
            said[skip].3 = said[skip].3.max(10.0 * s);
        }
        rows.extend(said.into_iter().skip(skip));
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
                color: theme().shadow,
                radius: 10.0 * s,
            });
            flow.fills.push(Fill {
                rect: Rect::new(x, top, w, h),
                color: theme().card,
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
                color: theme().input,
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
