//! A remote's card at the top of the drawer, or the **+** after the last
//! (plan 20261028_03, 3.2 and 3.3; the 18:51 EDT amendment, item 4):
//!
//! - line 1, 13 pt, the foreground colour: the remote's shown name
//!   ([`RemoteInfo::display_name`]), cut with "…" to the card's inner
//!   width (8 pt in from each side): a name at its right, an IP address at
//!   its left, so its distinguishing end shows (`…1.20`);
//! - line 2, 11 pt, dim: *in use* for the remote in use while online,
//!   *connecting…* while not, and nothing for the others;
//! - the one in use outlined 2 pt in the accent colour; the others the
//!   card colour, no outline;
//! - a filled dot at its top right while a background remote has a
//!   question or an approval waiting;
//! - while a finger is held on it, the press colour and, filling along
//!   its bottom over [`super::HOLD`], the hold's progress: at its end the
//!   remote's details open, and the press is no tap.
//!
//! The **+**: the same size and colour, a 22 pt **+** in the accent
//! colour, no text. The drawer finds the card under a finger itself
//! ([`super::content::DrawerContent::card_at`]), to time the press, so
//! the card takes no events of its own.

use flankr_companion::RemoteInfo;
use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::{cut_left, cut_right};
use crate::agent::{fill, line_h};
use crate::settings::copy;
use crate::theme::theme;

/// The card's height, inner padding, the lines' sizes, the outline, the
/// **+**'s size and the dot's diameter, in points.
pub const CARD_H: f32 = 52.0;
const INNER: f32 = 8.0;
const NAME_PT: f32 = 13.0;
const STATE_PT: f32 = 11.0;
const OUTLINE: f32 = 2.0;
const PLUS_PT: f32 = 22.0;
const DOT: f32 = 7.0;
const RADIUS: f32 = 10.0;

/// What a card is of. At most `MAX_REMOTES` and one **+** are held, so
/// the size between the variants does not matter.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Slot {
    Remote(RemoteInfo),
    /// The **+** after the last card.
    Add,
}

/// Whether `name` is an IP address (v4, or v6 with or without brackets):
/// cut from the left on a card.
pub fn is_address(name: &str) -> bool {
    let bare = name.trim_start_matches('[');
    let bare = bare.split(']').next().unwrap_or(bare);
    bare.parse::<std::net::IpAddr>().is_ok()
}

/// The second line: *in use*, *connecting…*, or none.
pub fn state_line(info: &RemoteInfo) -> Option<&'static str> {
    match (info.active, info.online) {
        (true, true) => Some(copy::REMOTE_CARD_IN_USE.text),
        (true, false) => Some(copy::REMOTE_CARD_CONNECTING.text),
        (false, _) => None,
    }
}

#[derive(Clone, Debug)]
pub struct RemoteCard {
    pub rect: Rect,
    pub slot: Slot,
    /// A background remote has a question or an approval waiting.
    pub waiting: bool,
    /// A finger is held on it, and how far the hold has come (0 to 1).
    pub pressed: Option<f32>,
    pub scale: f32,
}

impl RemoteCard {
    pub fn new(rect: Rect, slot: Slot, waiting: bool, s: f32) -> Self {
        RemoteCard {
            rect,
            slot,
            waiting,
            pressed: None,
            scale: s,
        }
    }

    fn active(&self) -> bool {
        matches!(&self.slot, Slot::Remote(r) if r.active)
    }

    /// The name as it fits: cut at the right, an address at the left.
    pub fn shown_name(&self, ctx: &DrawContext) -> String {
        let Slot::Remote(info) = &self.slot else {
            return String::new();
        };
        let s = self.scale;
        let room = self.rect.w - 2.0 * INNER * s;
        let px = NAME_PT * s;
        if is_address(&info.display_name) {
            cut_left(ctx, &info.display_name, px, room)
        } else {
            cut_right(ctx, &info.display_name, px, room)
        }
    }
}

impl DrawControl for RemoteCard {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let r = self.rect;
        let radius = RADIUS * s;
        let bg = if self.pressed.is_some() {
            theme().band
        } else {
            theme().card
        };
        if self.active() {
            fill(list, r, theme().accent, radius);
            let o = OUTLINE * s;
            fill(
                list,
                Rect::new(r.x + o, r.y + o, r.w - 2.0 * o, r.h - 2.0 * o),
                bg,
                radius - o,
            );
        } else {
            fill(list, r, bg, radius);
        }
        if let Some(t) = self.pressed.filter(|t| *t > 0.0) {
            let h = 3.0 * s;
            let inset = 6.0 * s;
            let w = (r.w - 2.0 * inset) * t.min(1.0);
            fill(
                list,
                Rect::new(r.x + inset, r.y + r.h - inset - h, w, h),
                theme().accent,
                h * 0.5,
            );
        }
        match &self.slot {
            Slot::Add => {
                let px = PLUS_PT * s;
                let w = ctx.text_advance("+", px);
                let [cx, cy] = r.center();
                ctx.draw_text(
                    list,
                    "+",
                    [cx - w * 0.5, cy - line_h(px) * 0.5],
                    px,
                    theme().accent,
                );
            }
            Slot::Remote(info) => {
                let name = self.shown_name(ctx);
                let px = NAME_PT * s;
                let state = state_line(info);
                let spx = STATE_PT * s;
                let h = line_h(px) + state.map_or(0.0, |_| line_h(spx));
                let mut y = r.y + (r.h - h) * 0.5;
                let w = ctx.text_advance(&name, px);
                ctx.draw_text(list, &name, [r.center()[0] - w * 0.5, y], px, theme().fg);
                y += line_h(px);
                if let Some(state) = state {
                    let w = ctx.text_advance(state, spx);
                    ctx.draw_text(list, state, [r.center()[0] - w * 0.5, y], spx, theme().dim);
                }
                if self.waiting {
                    let d = DOT * s;
                    let x = r.x + r.w - INNER * s * 0.75 - d;
                    let y = r.y + INNER * s * 0.75;
                    fill(list, Rect::new(x, y, d, d), theme().accent, d * 0.5);
                }
            }
        }
    }
}

impl HostedControl for RemoteCard {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    /// The drawer times the press itself.
    fn handle_mouse(&self, _event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        None
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
pub(crate) mod tests {
    use super::*;

    #[test]
    fn an_address_is_cut_from_the_left_a_name_from_the_right() {
        assert!(is_address("192.168.1.20"));
        assert!(is_address("fd7a:115c::1"));
        assert!(is_address("[fd7a:115c::1]"));
        assert!(!is_address("m4"));
        assert!(!is_address("m4:7347"));
        let ctx = crate::theme::testing::draw_context();
        let room = ctx.text_advance("…1.20", 26.0) + 0.5;
        let mut card = RemoteCard::new(
            Rect::new(0.0, 0.0, room + 2.0 * INNER * 2.0, CARD_H * 2.0),
            Slot::Remote(info("192.168.1.20")),
            false,
            2.0,
        );
        assert_eq!(card.shown_name(&ctx), "\u{2026}1.20");
        card.slot = Slot::Remote(info("studio-mac-mini"));
        let shown = card.shown_name(&ctx);
        assert!(
            shown.starts_with("st") && shown.ends_with('\u{2026}'),
            "{shown}"
        );
        card.slot = Slot::Remote(info("m4"));
        assert_eq!(card.shown_name(&ctx), "m4");
    }

    #[test]
    fn the_second_line_says_in_use_or_connecting_for_the_one_in_use() {
        let mut r = info("m4");
        assert_eq!(state_line(&r), None);
        r.active = true;
        assert_eq!(state_line(&r), Some("connecting…"));
        r.online = true;
        assert_eq!(state_line(&r), Some("in use"));
    }

    pub(crate) fn info(name: &str) -> RemoteInfo {
        RemoteInfo {
            id: flankr_companion::RemoteId([1; 8]),
            name: name.to_string(),
            host: format!("{name}:7347"),
            fingerprint: "0101 0101".into(),
            paired_at: None,
            active: false,
            online: false,
            fingerprint_hex: String::new(),
            fingerprint_words: String::new(),
            nickname: None,
            display_name: name.to_string(),
            last_session: None,
        }
    }
}
