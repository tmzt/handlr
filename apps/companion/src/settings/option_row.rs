//! A settings row choosing one of a few values (the 11:32 EDT amendment's
//! *Scrollbar*): its label on the left, and on the right libcontrols'
//! `ArrowLabel`, "‹ Hidden ›", whose arrows step through the values. The
//! row says which arrow was tapped (`StepLeft`, `StepRight`); the screen
//! picks the value, wrapping at both ends, and sets the text.
//!
//! Inactive, as *Voice* is before Phonon has loaded and while *All sound*
//! is off, it is drawn dim and its arrows take no taps. A row with a long
//! explanation has an ⓘ after its label ([`super::info`]), which takes
//! taps even while the row is dim, as a toggle row's does.

use libcontrols::{
    ArrowLabel, ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::info::{self, INFO_PART};
use super::screen::{LABEL_PT, label_y};
use crate::agent::{DIM, PAD};
use crate::settings::dim_since;
use crate::ui::TEXT;

#[derive(Clone, Debug)]
pub struct OptionRow {
    pub label: String,
    pub rect: Rect,
    pub arrows: ArrowLabel,
    pub scale: f32,
    /// The arrows step; otherwise it is drawn dim.
    pub enabled: bool,
    /// It has an ⓘ, after its label, which is `label_w` wide.
    pub info: bool,
    pub label_w: f32,
}

impl OptionRow {
    /// The arrows' width and each arrow's, in points.
    pub const ARROWS_W: f32 = 170.0;
    pub const ARROW_W: f32 = 44.0;

    pub fn new(label: &str, value: &str, scale: f32) -> Self {
        let arrows = ArrowLabel::new(value, Rect::new(0.0, 0.0, 1.0, 1.0))
            .with_arrow_width(Self::ARROW_W * scale)
            .with_font_size(LABEL_PT * scale)
            .with_text_color(TEXT)
            .with_arrow_color(DIM);
        OptionRow {
            label: label.to_string(),
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            arrows,
            scale,
            enabled: true,
            info: false,
            label_w: 0.0,
        }
    }

    /// The ⓘ's target, after the label.
    pub fn info_rect(&self) -> Rect {
        let s = self.scale;
        info::target(
            self.rect.x + PAD * s + self.label_w,
            self.rect.y + self.rect.h * 0.5,
            s,
        )
    }

    fn on_info(&self, pos: [f32; 2]) -> bool {
        self.info && self.info_rect().contains(pos)
    }

    /// Moves the row to `rect`, the arrows at its right.
    pub fn set_rect(&mut self, rect: Rect) {
        let s = self.scale;
        self.rect = rect;
        let w = Self::ARROWS_W * s;
        self.arrows.rect = Rect::new(rect.x + rect.w - PAD * s - w, rect.y, w, rect.h);
        self.arrows.chevron_thickness = 1.8 * s;
    }

    /// The value shown.
    pub fn value(&self) -> String {
        self.arrows.text()
    }

    pub fn set_value(&self, value: &str) {
        self.arrows.set_text(value);
    }
}

impl DrawControl for OptionRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let start = list.instances.len();
        let px = LABEL_PT * self.scale;
        ctx.draw_text(
            list,
            &self.label,
            [self.rect.x + PAD * self.scale, label_y(self.rect, px)],
            px,
            TEXT,
        );
        self.arrows.draw(list, ctx);
        if !self.enabled {
            dim_since(list, start);
        }
        if self.info {
            info::draw(list, ctx, self.info_rect(), self.scale);
        }
    }
}

impl HostedControl for OptionRow {
    fn bounds(&self) -> Rect {
        self.rect
    }

    /// The arrows' parts on the arrows, the body elsewhere.
    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        if self.on_info(pos) {
            return Some((INFO_PART, self.rect.normalized_x(pos[0])));
        }
        if !self.rect.contains(pos) {
            return None;
        }
        self.arrows
            .hit_test(pos)
            .or_else(|| Some((ControlPart::Body, self.rect.normalized_x(pos[0]))))
    }

    /// A tap on an arrow steps; the press, which may become a scroll, does
    /// not (the arrow label steps on both). A tap on the ⓘ is its
    /// explanation; inactive, the arrows do nothing.
    fn handle_mouse(&self, event: &MouseEvent, ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } if self.on_info(*pos) => Some(ControlAction::Activated { part: INFO_PART }),
            MouseEvent::Click { .. } if !self.enabled => None,
            MouseEvent::Click {
                button: MouseButton::Left,
                ..
            } => match ctx.part {
                ControlPart::LeftArrow | ControlPart::RightArrow => {
                    self.arrows.handle_mouse(event, ctx)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn handle_keyboard(&self, event: &KeyboardEvent, ctx: &EventContext) -> Option<ControlAction> {
        if !self.enabled {
            return None;
        }
        self.arrows.handle_keyboard(event, ctx)
    }
}
