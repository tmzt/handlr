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
use crate::agent::{PAD, fill, line_h};
use crate::theme::theme;

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
    /// The "›" on the right, when there is no action button. Off for a row
    /// whose setting is already granted, which shows nothing there.
    pub chevron: bool,
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
            chevron: true,
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

    /// The subtitle's lines: broken at each "\n", and wrapped at spaces to
    /// the room left of the "›" or the action button (owner, 2026-10-24:
    /// "wide labels should wrap"). Widths are estimated at 0.5 em a
    /// character, as layout runs without a font.
    pub fn subtitle_lines(&self) -> Vec<String> {
        let Some(subtitle) = &self.subtitle else {
            return Vec::new();
        };
        let s = self.scale;
        let px = SUBTITLE_PT * s;
        let right = match self.action_target() {
            Some(a) => self.rect.x + self.rect.w - a.x + 8.0 * s,
            None => PAD * s + 2.0 * LABEL_PT * s,
        };
        let room = (self.rect.w - PAD * s - right).max(px * 6.0);
        let cols = ((room / (0.5 * px)) as usize).max(8);
        let mut lines = Vec::new();
        for part in subtitle.split('\n') {
            let mut line = String::new();
            for word in part.split_whitespace() {
                if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > cols {
                    lines.push(std::mem::take(&mut line));
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            lines.push(line);
        }
        lines
    }

    /// The row's height at its width: one row, and a line more for each
    /// subtitle line past the first.
    pub fn height(&self, row_h: f32) -> f32 {
        let extra = self.subtitle_lines().len().saturating_sub(1) as f32;
        row_h + extra * line_h(SUBTITLE_PT * self.scale)
    }

    /// The label's top, with the subtitle's lines under it.
    fn label_top(&self) -> f32 {
        let px = LABEL_PT * self.scale;
        let lines = self.subtitle_lines().len();
        if lines == 0 {
            return label_y(self.rect, px);
        }
        let h = line_h(px) + lines as f32 * line_h(SUBTITLE_PT * self.scale);
        self.rect.y + (self.rect.h - h) * 0.5
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
        ctx.draw_text(list, &self.label, [x, y], px, theme().text);
        let sub_px = SUBTITLE_PT * s;
        for (i, line) in self.subtitle_lines().iter().enumerate() {
            let ly = y + line_h(px) + i as f32 * line_h(sub_px);
            ctx.draw_text(list, line, [x, ly], sub_px, theme().dim);
        }
        if self.info {
            info::draw(list, ctx, self.info_rect(), s);
        }
        if let (Some(text), Some(r)) = (self.action.as_deref(), self.action_rect(ctx)) {
            let apx = ACTION_TEXT * px;
            fill(list, r, theme().button, r.h * 0.5);
            let tw = ctx.text_advance(text, apx);
            ctx.draw_text(
                list,
                text,
                [r.x + (r.w - tw) * 0.5, r.y + (r.h - line_h(apx)) * 0.5],
                apx,
                theme().text,
            );
            return;
        }
        if !self.chevron {
            return;
        }
        // Drawn as two strokes: the face has no "›", which showed as an
        // empty box.
        let h = 0.36 * px;
        let x1 = self.rect.x + self.rect.w - PAD * s;
        let cy = self.rect.y + self.rect.h * 0.5;
        let t = 1.6 * s;
        list.push_line([x1 - h * 0.6, cy - h], [x1, cy], t, theme().dim);
        list.push_line([x1, cy], [x1 - h * 0.6, cy + h], t, theme().dim);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_status_wraps_short_of_the_button_and_the_row_grows() {
        let mut row = LinkRow::new("Battery optimization", 2.625);
        row.rect = Rect::new(0.0, 0.0, 1080.0, 52.0 * 2.625);
        row.subtitle = Some("Optimized: Android may hold back flankr's network when idle".into());
        row.action = Some("Enable".into());
        let lines = row.subtitle_lines();
        assert!(lines.len() >= 2, "{lines:?}");
        let s = row.scale;
        let button = row.action_target().unwrap();
        for line in &lines {
            let end = PAD * s + line.chars().count() as f32 * 0.5 * SUBTITLE_PT * s;
            assert!(end < button.x, "{line:?} reaches the button");
        }
        assert!(row.height(row.rect.h) > row.rect.h);
        // An explicit break is kept; without a button a short one stays whole.
        row.action = None;
        row.subtitle = Some("one\ntwo".into());
        assert_eq!(row.subtitle_lines(), ["one", "two"]);
    }
}
