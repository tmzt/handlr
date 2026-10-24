//! A settings row that opens a page (decision 2): its label, a dim
//! subtitle under it when it has one (*Exempt*, *Optimized*), and a "›" on
//! the right. A tap opens it. A row with a long explanation has an ⓘ after
//! its label ([`super::info`]).

use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::info::{self, INFO_PART};
use super::screen::{LABEL_PT, SUBTITLE_PT, label_y};
use crate::agent::{DIM, PAD, line_h};
use crate::ui::TEXT;

/// The "›".
const CHEVRON: &str = "\u{203A}";

#[derive(Clone, Debug, PartialEq)]
pub struct LinkRow {
    pub label: String,
    pub subtitle: Option<String>,
    pub rect: Rect,
    pub scale: f32,
    /// It has an ⓘ, after its label, which is `label_w` wide.
    pub info: bool,
    pub label_w: f32,
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
        }
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
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    /// On the ⓘ, its explanation; elsewhere, the page.
    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } => Some(ControlAction::Activated {
                part: if self.on_info(*pos) {
                    INFO_PART
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
