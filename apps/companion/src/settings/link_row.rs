//! A settings row that opens a page (decision 2): its label, a dim
//! subtitle under it when it has one (*Exempt*, *Optimized*), and a "›" on
//! the right. A tap opens it.

use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

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
}

impl LinkRow {
    pub fn new(label: &str, scale: f32) -> Self {
        LinkRow {
            label: label.to_string(),
            subtitle: None,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            scale,
        }
    }
}

impl DrawControl for LinkRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = LABEL_PT * s;
        let x = self.rect.x + PAD * s;
        match &self.subtitle {
            Some(subtitle) => {
                let sub_px = SUBTITLE_PT * s;
                let h = line_h(px) + line_h(sub_px);
                let y = self.rect.y + (self.rect.h - h) * 0.5;
                ctx.draw_text(list, &self.label, [x, y], px, TEXT);
                ctx.draw_text(list, subtitle, [x, y + line_h(px)], sub_px, DIM);
            }
            None => {
                ctx.draw_text(list, &self.label, [x, label_y(self.rect, px)], px, TEXT);
            }
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
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                button: MouseButton::Left,
                ..
            } => Some(ControlAction::Activated {
                part: ControlPart::Body,
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
