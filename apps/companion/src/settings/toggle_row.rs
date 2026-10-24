//! A settings row with a toggle (decision 2): its label on the left and a
//! switch on the right. A tap anywhere on the row flips it. Inactive, as
//! *Ding when an agent is done* is while *All sound* is off, it is drawn
//! dim and takes no taps. A row with a long explanation has an ⓘ after its
//! label ([`super::info`]), which takes taps even while the row is dim.

use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::info::{self, INFO_PART};
use super::screen::{LABEL_PT, label_y};
use crate::agent::{BAND, BG, FG, PAD, fill};
use crate::settings::dim_since;
use crate::ui::TEXT;

/// The switch's track when on.
const TRACK_ON: [f32; 4] = [0.22, 0.62, 0.36, 1.0];

#[derive(Clone, Debug, PartialEq)]
pub struct ToggleRow {
    pub label: String,
    pub rect: Rect,
    pub on: bool,
    /// Taps flip it; otherwise it is drawn dim.
    pub enabled: bool,
    pub scale: f32,
    /// It has an ⓘ, after its label, which is `label_w` wide.
    pub info: bool,
    pub label_w: f32,
}

impl ToggleRow {
    /// The switch's size, in points.
    pub const SWITCH_W: f32 = 50.0;
    pub const SWITCH_H: f32 = 30.0;

    pub fn new(label: &str, scale: f32) -> Self {
        ToggleRow {
            label: label.to_string(),
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            on: true,
            enabled: true,
            scale,
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

    /// Where the switch is, at the row's right.
    pub fn switch_rect(&self) -> Rect {
        let s = self.scale;
        let (w, h) = (Self::SWITCH_W * s, Self::SWITCH_H * s);
        Rect::new(
            self.rect.x + self.rect.w - PAD * s - w,
            self.rect.y + (self.rect.h - h) * 0.5,
            w,
            h,
        )
    }
}

impl DrawControl for ToggleRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let start = list.instances.len();
        let s = self.scale;
        let px = LABEL_PT * s;
        ctx.draw_text(
            list,
            &self.label,
            [self.rect.x + PAD * s, label_y(self.rect, px)],
            px,
            TEXT,
        );
        let track = self.switch_rect();
        fill(
            list,
            track,
            if self.on { TRACK_ON } else { BAND },
            track.h * 0.5,
        );
        let inset = 3.0 * s;
        let d = track.h - 2.0 * inset;
        let knob_x = if self.on {
            track.x + track.w - inset - d
        } else {
            track.x + inset
        };
        fill(
            list,
            Rect::new(knob_x, track.y + inset, d, d),
            if self.on { FG } else { BG },
            d * 0.5,
        );
        if !self.enabled {
            dim_since(list, start);
        }
        if self.info {
            info::draw(list, ctx, self.info_rect(), s);
        }
    }
}

impl HostedControl for ToggleRow {
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

    /// A tap, not the press that may become a scroll: on the ⓘ, its
    /// explanation; elsewhere, the toggle.
    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } if self.on_info(*pos) => Some(ControlAction::Activated { part: INFO_PART }),
            MouseEvent::Click {
                button: MouseButton::Left,
                ..
            } if self.enabled => Some(ControlAction::Activated {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn click(row: &ToggleRow) -> Option<ControlAction> {
        let pos = [row.rect.x + 5.0, row.rect.y + 5.0];
        let ctx = EventContext::default();
        row.handle_mouse(
            &MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            },
            &ctx,
        )
    }

    #[test]
    fn a_tap_activates_it_unless_it_is_inactive_which_draws_it_dim() {
        let mut row = ToggleRow::new("All sound", 2.0);
        row.rect = Rect::new(0.0, 100.0, 1080.0, 104.0);
        assert!(click(&row).is_some());
        let mut list = DrawList::new();
        row.draw(&mut list, &DrawContext::default());
        let live: Vec<f32> = list.instances.iter().map(|i| i.color[3]).collect();
        row.enabled = false;
        assert!(click(&row).is_none());
        let mut list = DrawList::new();
        row.draw(&mut list, &DrawContext::default());
        let dim: Vec<f32> = list.instances.iter().map(|i| i.color[3]).collect();
        assert_eq!(live.len(), dim.len());
        assert!(
            live.iter()
                .zip(&dim)
                .all(|(l, d)| (d - l * crate::settings::DISABLED_ALPHA).abs() < 1e-6)
        );
        // The switch sits at the right.
        let sw = row.switch_rect();
        assert!(sw.x + sw.w <= 1080.0 && sw.x > 900.0);
    }
}
