//! The confirming modal of *Unpair this device* and *Delete data*
//! (decision 3): its question, wrapped, and *Cancel* and the action (in
//! red) under it. It is libcontrols' `ModalDialog`, centred, over a dimmed
//! screen; both buttons are its close controls, so the modal stack says
//! which one closed it, and a tap outside it or Back is *Cancel*.

use libcontrols::{Button, Label, ModalDialog, ModalOptions, NodeId, PanelFrame, Rect, TextAlign};

use crate::agent::{BACK_BG, CARD_BG, DENY_BG, FG, PAD, line_h};
use crate::ui::{TEXT, wrap};

/// The question's size, in points.
const TEXT_PT: f32 = 17.0;
/// The buttons' height and the space between them, in points.
const BUTTON_H: f32 = 48.0;
const GAP: f32 = 12.0;

/// The modal, and its two buttons in its content router.
pub struct Confirm {
    pub dialog: ModalDialog,
    pub cancel: NodeId,
    pub action: NodeId,
}

impl Confirm {
    /// A modal asking `question` over `viewport`, at scale `s`, `action`
    /// on its red button.
    pub fn new(viewport: Rect, question: &[String], action: &str, s: f32) -> Self {
        let pad = 2.0 * PAD * s;
        let w = (viewport.w - 2.0 * PAD * s).min(560.0 * s);
        let inner_w = w - 2.0 * pad;
        let px = TEXT_PT * s;
        // Roboto at half an em a character.
        let columns = ((inner_w / (px * 0.5)) as usize).max(8);
        let mut lines: Vec<(String, f32)> = Vec::new();
        for (i, paragraph) in question.iter().enumerate() {
            for (j, line) in wrap(paragraph, columns).into_iter().enumerate() {
                let gap = if i > 0 && j == 0 { 10.0 * s } else { 0.0 };
                lines.push((line, gap));
            }
        }
        let text_h: f32 = lines.iter().map(|(_, gap)| gap + line_h(px)).sum();
        let h = pad + text_h + GAP * 2.0 * s + BUTTON_H * s + pad;
        let options = ModalOptions::default();
        let mut dialog = ModalDialog::centered(viewport, [w, h], pad)
            .with_frame(PanelFrame {
                bg_color: Some(CARD_BG),
                border_color: None,
                border_thickness: 0.0,
                corner_radius: Some(14.0 * s),
            })
            .with_options(options);
        let content = dialog.content_mut();
        let frame_x = viewport.center()[0] - w * 0.5;
        let frame_y = viewport.center()[1] - h * 0.5;
        let x = frame_x + pad;
        let mut y = frame_y + pad;
        for (line, gap) in lines {
            y += gap;
            content.add(
                None,
                Label::new(line, Rect::new(x, y, inner_w, line_h(px)))
                    .with_font_size(px)
                    .with_color(TEXT)
                    .with_align(TextAlign::Left),
            );
            y += line_h(px);
        }
        let by = frame_y + h - pad - BUTTON_H * s;
        let bw = (inner_w - GAP * s) / 2.0;
        let button = |label: &str, bx: f32, bg| {
            Button::new(label, Rect::new(bx, by, bw, BUTTON_H * s))
                .with_bg(bg)
                .with_text_color(FG)
                .with_font_size(px)
                .with_corner_radius(10.0 * s)
        };
        let cancel = content.add(None, button("Cancel", x, BACK_BG));
        let action = content.add(None, button(action, x + bw + GAP * s, DENY_BG));
        let dialog = dialog.with_close_control(cancel).with_close_control(action);
        Confirm {
            dialog,
            cancel,
            action,
        }
    }
}
