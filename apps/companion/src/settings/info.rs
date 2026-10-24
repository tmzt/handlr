//! The ⓘ of a row with a long explanation, and its modal (plan 20261024_02,
//! CHANGES.md 2026-10-24 12:38 EDT, "ⓘ modals").
//!
//! - **The button:** a small ⓘ after the row's label, in the dim colour,
//!   with a 44 pt target. A row reports a tap on it as
//!   `ControlPart::Label` ([`INFO_PART`]).
//! - **The modal:** libcontrols' `ModalDialog`, centred, with the row's name
//!   as its title, the explanation in Markdown, and *OK*. It closes at
//!   once, as the confirm modals do (the screen's modal stack has no close
//!   transition); a tap outside it, or Back, also closes it. A tap on a
//!   link in it is the link's index ([`crate::markdown_view`]).

use std::sync::Arc;

use libcontrols::{Button, ControlPart, DrawContext, ModalDialog, NodeId, PanelFrame, Rect};
use libmsdf::drawlist::DrawList;

use super::copy;
use crate::agent::{PAD, fill, line_h};
use crate::markdown_view::{Faces, MarkdownView};
use crate::theme::theme;

/// The part a row reports for a tap on its ⓘ.
pub const INFO_PART: ControlPart = ControlPart::Label;
/// The ⓘ's target and its drawn circle, in points.
pub const TARGET: f32 = 44.0;
const CIRCLE: f32 = 18.0;
/// The space between the label's end and the circle, in points.
const GAP: f32 = 6.0;

/// The ⓘ's target for a label ending at `label_end`, its line centred at
/// `cy`, at scale `s`.
pub fn target(label_end: f32, cy: f32, s: f32) -> Rect {
    let cx = label_end + (GAP + CIRCLE * 0.5) * s;
    Rect::new(
        cx - TARGET * 0.5 * s,
        cy - TARGET * 0.5 * s,
        TARGET * s,
        TARGET * s,
    )
}

/// Draws the ⓘ centred in `target`: a dim ring with an "i".
pub fn draw(list: &mut DrawList, ctx: &DrawContext, target: Rect, s: f32) {
    let d = CIRCLE * s;
    let [cx, cy] = target.center();
    let ring = Rect::new(cx - d * 0.5, cy - d * 0.5, d, d);
    fill(list, ring, theme().dim, d * 0.5);
    let t = 1.6 * s;
    let inner = Rect::new(ring.x + t, ring.y + t, d - 2.0 * t, d - 2.0 * t);
    fill(list, inner, theme().bg, inner.w * 0.5);
    let px = 13.0 * s;
    let w = ctx.text_advance("i", px);
    ctx.draw_text(
        list,
        "i",
        [cx - w * 0.5, cy - line_h(px) * 0.5],
        px,
        theme().dim,
    );
}

/// The title's and the text's sizes, in points.
const TITLE_PT: f32 = 18.0;
const TEXT_PT: f32 = 15.0;
const BUTTON_H: f32 = 48.0;
const SPACE: f32 = 12.0;

/// An explanation's modal, and its OK button and links.
pub struct InfoModal {
    pub dialog: ModalDialog,
    pub ok: NodeId,
    pub text: NodeId,
    pub links: Vec<String>,
}

impl InfoModal {
    /// `title` and the Markdown `text` over `viewport`, at scale `s`.
    pub fn new(viewport: Rect, title: &str, text: &str, s: f32, faces: Option<Arc<Faces>>) -> Self {
        let pad = 2.0 * PAD * s;
        let w = (viewport.w - 2.0 * PAD * s).min(560.0 * s);
        let inner_w = w - 2.0 * pad;
        let heading = MarkdownView::new(
            &format!("**{title}**"),
            0.0,
            0.0,
            inner_w,
            TITLE_PT * s,
            theme().fg,
            faces.clone(),
        );
        let body = MarkdownView::new(text, 0.0, 0.0, inner_w, TEXT_PT * s, theme().text, faces);
        let h = pad
            + heading.height()
            + SPACE * s
            + body.height()
            + 2.0 * SPACE * s
            + BUTTON_H * s
            + pad;
        let h = h.min(viewport.h - 2.0 * PAD * s);
        let mut dialog = ModalDialog::centered(viewport, [w, h], pad).with_frame(PanelFrame {
            bg_color: Some(theme().card),
            border_color: None,
            border_thickness: 0.0,
            corner_radius: Some(14.0 * s),
        });
        let x = viewport.center()[0] - w * 0.5 + pad;
        let top = viewport.center()[1] - h * 0.5 + pad;
        let mut heading = heading;
        heading.move_to(x, top);
        let mut body = body;
        body.move_to(x, top + heading.height() + SPACE * s);
        let links = body.links().to_vec();
        let content = dialog.content_mut();
        content.add(None, heading);
        let text = content.add(None, body);
        let by = top + h - 2.0 * pad - BUTTON_H * s;
        let ok = content.add(
            None,
            Button::new(copy::OK.text, Rect::new(x, by, inner_w, BUTTON_H * s))
                .with_bg(theme().back)
                .with_text_color(theme().fg)
                .with_font_size(17.0 * s)
                .with_corner_radius(10.0 * s),
        );
        InfoModal {
            dialog: dialog.with_close_control(ok),
            ok,
            text,
            links,
        }
    }
}
