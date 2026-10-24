//! The confirming modal of *Unpair this device* and *Delete data*
//! (decision 3): its question, in Markdown from `SETTINGS_COPY.md`, and
//! *Cancel* and the action (in red) under it. It is libcontrols' `ModalDialog`, centred, over a dimmed
//! screen; both buttons are its close controls, so the modal stack says
//! which one closed it, and a tap outside it or Back is *Cancel*.

use std::sync::Arc;

use libcontrols::{Button, ModalDialog, ModalOptions, NodeId, PanelFrame, Rect};

use super::copy;
use crate::agent::PAD;
use crate::markdown_view::{Faces, MarkdownView};
use crate::theme::theme;

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
    /// The question.
    pub text: NodeId,
}

impl Confirm {
    /// A modal asking `question` (Markdown) over `viewport`, at scale `s`,
    /// `action` on its red button.
    pub fn new(
        viewport: Rect,
        question: &str,
        action: &str,
        s: f32,
        faces: Option<Arc<Faces>>,
    ) -> Self {
        let pad = 2.0 * PAD * s;
        let w = (viewport.w - 2.0 * PAD * s).min(560.0 * s);
        let inner_w = w - 2.0 * pad;
        let px = TEXT_PT * s;
        let mut body = MarkdownView::new(question, 0.0, 0.0, inner_w, px, theme().text, faces);
        let text_h = body.height();
        let h = pad + text_h + GAP * 2.0 * s + BUTTON_H * s + pad;
        let options = ModalOptions::default();
        let mut dialog = ModalDialog::centered(viewport, [w, h], pad)
            .with_frame(PanelFrame {
                bg_color: Some(theme().card),
                border_color: None,
                border_thickness: 0.0,
                corner_radius: Some(14.0 * s),
            })
            .with_options(options);
        let content = dialog.content_mut();
        let frame_x = viewport.center()[0] - w * 0.5;
        let frame_y = viewport.center()[1] - h * 0.5;
        let x = frame_x + pad;
        body.move_to(x, frame_y + pad);
        let text = content.add(None, body);
        let by = frame_y + h - pad - BUTTON_H * s;
        let bw = (inner_w - GAP * s) / 2.0;
        let button = |label: &str, bx: f32, bg| {
            Button::new(label, Rect::new(bx, by, bw, BUTTON_H * s))
                .with_bg(bg)
                .with_text_color(theme().fg)
                .with_font_size(px)
                .with_corner_radius(10.0 * s)
        };
        let cancel = content.add(None, button(copy::CANCEL.text, x, theme().back));
        let action = content.add(None, button(action, x + bw + GAP * s, theme().deny));
        let dialog = dialog.with_close_control(cancel).with_close_control(action);
        Confirm {
            dialog,
            cancel,
            action,
            text,
        }
    }
}
