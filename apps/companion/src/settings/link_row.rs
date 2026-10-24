//! A settings row that opens a page (decision 2): its label, a dim
//! subtitle under it when it has one (*Exempt*, *Optimized*), and a "›" on
//! the right. A tap opens it. A row with a long explanation has an ⓘ after
//! its label ([`super::info`]). A row whose setting needs the system's
//! approval has a small *Enable* button on the right in place of the "›"
//! (owner, 2026-10-24), its text at 0.8 of the label's size.

use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::info::{self, INFO_PART};
use super::screen::{LABEL_PT, SUBTITLE_PT, label_y};
use crate::agent::{DIM, PAD, fill, line_h};
use crate::ui::TEXT;

/// The action button's fill: a quiet raised grey, as the settings buttons.
const BUTTON_BG: [f32; 4] = [0.24, 0.25, 0.31, 1.0];

/// The "›".
const CHEVRON: &str = "\u{203A}";

/// The action button's part: the system's approval, not the page.
/// (`Custom` parts are a Markdown note's links.)
pub const ACTION_PART: ControlPart = ControlPart::Value;

/// The action button's text, as a share of the label's size.
const ACTION_TEXT: f32 = 0.8;

#[derive(Clone, Debug, PartialEq)]
pub struct LinkRow {
    pub label: String,
    pub subtitle: Option<String>,
    pub rect: Rect,
    pub scale: f32,
    /// It has an ⓘ, after its label, which is `label_w` wide.
    pub info: bool,
    pub label_w: f32,
    /// A small button on the right, in place of the "›", for the system's
    /// approval (*Enable*).
    pub action: Option<String>,
}

impl LinkRow {
    pub fn new(label: &str, scale: f32) -> Self {
        LinkRow {
            label: label.to_string(),
            subtitle: None,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            scale,
            info: false,
            label_w: 0.0,
            action: None,
        }
    }

    /// The action button, when the row has one: its text at 0.8 of the
    /// label's size, padded into a pill at the right.
    pub fn action_rect(&self, ctx: &DrawContext) -> Option<Rect> {
        let text = self.action.as_deref()?;
        let s = self.scale;
        let px = ACTION_TEXT * LABEL_PT * s;
        let w = ctx.text_advance(text, px) + 24.0 * s;
        let h = line_h(px) + 12.0 * s;
        Some(Rect::new(
            self.rect.x + self.rect.w - PAD * s - w,
            self.rect.y + (self.rect.h - h) * 0.5,
            w,
            h,
        ))
    }

    /// The action button's target, without a context: its text measured
    /// at half an em a character, as the app's other estimates are.
    pub(crate) fn action_target(&self) -> Option<Rect> {
        let text = self.action.as_deref()?;
        let s = self.scale;
        let px = ACTION_TEXT * LABEL_PT * s;
        let w = text.chars().count() as f32 * 0.55 * px + 24.0 * s;
        let h = (line_h(px) + 12.0 * s).max(44.0 * s).min(self.rect.h);
        Some(Rect::new(
            self.rect.x + self.rect.w - PAD * s - w - 8.0 * s,
            self.rect.y + (self.rect.h - h) * 0.5,
            w + 16.0 * s,
            h,
        ))
    }

    fn on_action(&self, pos: [f32; 2]) -> bool {
        self.action_target().is_some_and(|r| r.contains(pos))
    }

    /// The label's top, with the subtitle under it when there is one.
    fn label_top(&self) -> f32 {
        let px = LABEL_PT * self.scale;
        match &self.subtitle {
            Some(_) => {
                let h = line_h(px) + line_h(SUBTITLE_PT * self.scale);
                self.rect.y + (self.rect.h - h) * 0.5
            }
            None => label_y(self.rect, px),
        }
    }

    /// The ⓘ's target, after the label.
    pub fn info_rect(&self) -> Rect {
        let s = self.scale;
        info::target(
            self.rect.x + PAD * s + self.label_w,
            self.label_top() + line_h(LABEL_PT * s) * 0.5,
            s,
        )
    }

    fn on_info(&self, pos: [f32; 2]) -> bool {
        self.info && self.info_rect().contains(pos)
    }
}

impl DrawControl for LinkRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = LABEL_PT * s;
        let x = self.rect.x + PAD * s;
        let y = self.label_top();
        ctx.draw_text(list, &self.label, [x, y], px, TEXT);
        if let Some(subtitle) = &self.subtitle {
            ctx.draw_text(list, subtitle, [x, y + line_h(px)], SUBTITLE_PT * s, DIM);
        }
        if self.info {
            info::draw(list, ctx, self.info_rect(), s);
        }
        if let (Some(text), Some(r)) = (self.action.as_deref(), self.action_rect(ctx)) {
            let apx = ACTION_TEXT * px;
            fill(list, r, BUTTON_BG, r.h * 0.5);
            let tw = ctx.text_advance(text, apx);
            ctx.draw_text(
                list,
                text,
                [r.x + (r.w - tw) * 0.5, r.y + (r.h - line_h(apx)) * 0.5],
                apx,
                TEXT,
            );
            return;
        }
        let chevron_px = 1.4 * px;
        let w = ctx.text_advance(CHEVRON, chevron_px);
        ctx.draw_text(
            list,
            CHEVRON,
            [
                self.rect.x + self.rect.w - PAD * s - w,
                label_y(self.rect, chevron_px),
            ],
            chevron_px,
            DIM,
        );
    }
}

impl HostedControl for LinkRow {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        if self.on_info(pos) {
            return Some((INFO_PART, self.rect.normalized_x(pos[0])));
        }
        if self.on_action(pos) {
            return Some((ACTION_PART, self.rect.normalized_x(pos[0])));
        }
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    /// On the ⓘ, its explanation; on the action button, the system's
    /// approval; elsewhere, the page.
    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } => Some(ControlAction::Activated {
                part: if self.on_info(*pos) {
                    INFO_PART
                } else if self.on_action(*pos) {
                    ACTION_PART
                } else {
                    ControlPart::Body
                },
            }),
            _ => None,
        }
    }

    fn handle_keyboard(
        &self,
        _event: &KeyboardEvent,
        _ctx: &EventContext,
    ) -> Option<ControlAction> {
        None
    }
}
