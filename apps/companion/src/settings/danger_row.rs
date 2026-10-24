//! A destructive settings row (decision 3, *Device*): its label in red,
//! nothing on the right. A tap asks first, in a confirming modal.

use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::screen::{LABEL_PT, label_y};
use crate::agent::{PAD, RED};

#[derive(Clone, Debug, PartialEq)]
pub struct DangerRow {
    pub label: String,
    pub rect: Rect,
    pub scale: f32,
}

impl DangerRow {
    pub fn new(label: &str, scale: f32) -> Self {
        DangerRow {
            label: label.to_string(),
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            scale,
        }
    }
}

impl DrawControl for DangerRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let px = LABEL_PT * self.scale;
        ctx.draw_text(
            list,
            &self.label,
            [self.rect.x + PAD * self.scale, label_y(self.rect, px)],
            px,
            RED,
        );
    }
}

impl HostedControl for DangerRow {
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
