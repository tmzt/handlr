//! A plugin's row in Settings → Plugins
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.9): two lines and
//! a toggle at the right, as a remote's row has two lines and its X.
//!
//! - Line 1, at the label's size: the plugin's name, and an ⓘ after it
//!   that opens its description ([`super::info`]).
//! - Line 2, at the subtitle's size and dim: its version as the daemon
//!   gives it (`0.1.0`), or why it can't be used here (*Needs a newer
//!   app*, *Needs a newer flankr*).
//! - The toggle, as a [`super::toggle_row::ToggleRow`]'s: a tap anywhere on
//!   the row (but the ⓘ) flips it. Disabled for a plugin the app can't
//!   show; while the daemon writes the change, a spinner in the switch's
//!   place until `server.capabilities` settles it.
//!
//! With no plugins listed (an older daemon), the section is one standard
//! row, *This flankr has no plugins.* ([`PluginRow::placeholder`]), which
//! takes no taps.

use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::info::{self, INFO_PART};
use super::screen::{LABEL_PT, SUBTITLE_PT};
use super::toggle_row::ToggleRow;
use crate::agent::{PAD, fill, line_h};
use crate::settings::dim_since;
use crate::theme::theme;

/// The spinner's dots.
const DOTS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct PluginRow {
    pub rect: Rect,
    pub name: String,
    /// The second line: its version, or why it can't be used.
    pub second: String,
    pub on: bool,
    /// The toggle takes taps.
    pub enabled: bool,
    /// A change is being written: the spinner, turned to `phase` (0 to 1).
    pub pending: bool,
    pub phase: f32,
    pub scale: f32,
    /// The name's width, for the ⓘ after it.
    pub label_w: f32,
    /// With no plugin, a standard row's label that takes no taps.
    pub placeholder: Option<&'static str>,
}

impl PluginRow {
    pub fn new(scale: f32) -> Self {
        PluginRow {
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            name: String::new(),
            second: String::new(),
            on: false,
            enabled: true,
            pending: false,
            phase: 0.0,
            scale,
            label_w: 0.0,
            placeholder: None,
        }
    }

    fn lines_y(&self) -> f32 {
        let s = self.scale;
        self.rect.y + (self.rect.h - line_h(LABEL_PT * s) - line_h(SUBTITLE_PT * s)) * 0.5
    }

    /// The ⓘ's target, after the name.
    pub fn info_rect(&self) -> Rect {
        let s = self.scale;
        info::target(
            self.rect.x + PAD * s + self.label_w,
            self.lines_y() + line_h(LABEL_PT * s) * 0.5,
            s,
        )
    }

    fn on_info(&self, pos: [f32; 2]) -> bool {
        self.placeholder.is_none() && self.info_rect().contains(pos)
    }

    /// The switch, at the row's right, as a toggle row's.
    pub fn switch_rect(&self) -> Rect {
        let s = self.scale;
        let (w, h) = (ToggleRow::SWITCH_W * s, ToggleRow::SWITCH_H * s);
        Rect::new(
            self.rect.x + self.rect.w - PAD * s - w,
            self.rect.y + (self.rect.h - h) * 0.5,
            w,
            h,
        )
    }

    fn draw_switch(&self, list: &mut DrawList) {
        let s = self.scale;
        let track = self.switch_rect();
        fill(
            list,
            track,
            if self.on {
                theme().toggle
            } else {
                theme().band
            },
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
            if self.on { theme().fg } else { theme().bg },
            d * 0.5,
        );
    }

    /// A ring of dots, the one at `phase` brightest.
    fn draw_spinner(&self, list: &mut DrawList) {
        let s = self.scale;
        let [cx, cy] = self.switch_rect().center();
        let r = 10.0 * s;
        let d = 4.0 * s;
        let lead = (self.phase.rem_euclid(1.0) * DOTS as f32) as usize % DOTS;
        for i in 0..DOTS {
            let a = i as f32 * std::f32::consts::TAU / DOTS as f32;
            let (x, y) = (cx + a.cos() * r, cy + a.sin() * r);
            let mut color = theme().dim;
            let behind = (lead + DOTS - i) % DOTS;
            color[3] *= 1.0 - behind as f32 / DOTS as f32 * 0.8;
            fill(
                list,
                Rect::new(x - d * 0.5, y - d * 0.5, d, d),
                color,
                d * 0.5,
            );
        }
    }
}

impl DrawControl for PluginRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = LABEL_PT * s;
        let x = self.rect.x + PAD * s;
        if let Some(label) = self.placeholder {
            let y = self.rect.y + (self.rect.h - line_h(px)) * 0.5;
            ctx.draw_text(list, label, [x, y], px, theme().dim);
            return;
        }
        let start = list.instances.len();
        let sub = SUBTITLE_PT * s;
        let y = self.lines_y();
        ctx.draw_text(list, &self.name, [x, y], px, theme().text);
        ctx.draw_text(list, &self.second, [x, y + line_h(px)], sub, theme().dim);
        if self.pending {
            self.draw_spinner(list);
        } else {
            self.draw_switch(list);
        }
        if !self.enabled {
            dim_since(list, start);
        }
        info::draw(list, ctx, self.info_rect(), s);
    }
}

impl HostedControl for PluginRow {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        if self.placeholder.is_some() {
            return None;
        }
        if self.on_info(pos) {
            return Some((INFO_PART, self.rect.normalized_x(pos[0])));
        }
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    /// On the ⓘ, the description; elsewhere, the toggle, unless it is
    /// disabled or a change is being written.
    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        if self.placeholder.is_some() {
            return None;
        }
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } if self.on_info(*pos) => Some(ControlAction::Activated { part: INFO_PART }),
            MouseEvent::Click {
                button: MouseButton::Left,
                ..
            } if self.enabled && !self.pending => Some(ControlAction::Activated {
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

    fn click(row: &PluginRow, pos: [f32; 2]) -> Option<ControlAction> {
        row.handle_mouse(
            &MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            },
            &EventContext::default(),
        )
    }

    fn row() -> PluginRow {
        let mut r = PluginRow::new(2.0);
        r.rect = Rect::new(0.0, 100.0, 1080.0, 104.0);
        r.name = "Planning".into();
        r.second = "0.1.0".into();
        r.label_w = 140.0;
        r
    }

    #[test]
    fn a_tap_toggles_unless_disabled_or_pending_and_the_info_always_opens() {
        let mut r = row();
        let body = [900.0, 150.0];
        let i = r.info_rect().center();
        assert_eq!(
            click(&r, body),
            Some(ControlAction::Activated {
                part: ControlPart::Body
            })
        );
        assert_eq!(
            click(&r, i),
            Some(ControlAction::Activated { part: INFO_PART })
        );
        r.enabled = false;
        assert_eq!(click(&r, body), None);
        assert!(click(&r, i).is_some());
        r.enabled = true;
        r.pending = true;
        assert_eq!(click(&r, body), None);
        // The placeholder takes nothing.
        let mut p = PluginRow::new(2.0);
        p.rect = r.rect;
        p.placeholder = Some("This flankr has no plugins.");
        assert_eq!(click(&p, body), None);
        assert!(p.hit_test(body).is_none());
    }

    #[test]
    fn pending_draws_the_spinner_in_the_switchs_place() {
        let mut r = row();
        let mut list = DrawList::new();
        r.draw(&mut list, &DrawContext::default());
        let plain = list.instances.len();
        r.pending = true;
        let mut list = DrawList::new();
        r.draw(&mut list, &DrawContext::default());
        // Two fills of the switch become eight dots.
        assert_eq!(list.instances.len(), plain - 2 + DOTS);
        let sw = r.switch_rect();
        assert!(sw.x > 900.0 && sw.x + sw.w <= 1080.0);
    }
}
