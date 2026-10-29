//! The row under a typed card's text box
//! (docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md, decision 8 and the
//! 23:39 EDT amendment): a **+** at its left and the soft keyboard's
//! show/hide toggle at its right, with a microphone left of the toggle (the
//! 2026-10-29 15:09 EDT amendment), each a 44 × 44 pt target, 4 pt under
//! the box. Pinned and in the overlay alike, on both cards.
//!
//! - The **+** is drawn as two 2 pt strokes 18 pt long in the accent, no
//!   background. Gemini's card has it for Paste text (the 23:39 EDT
//!   amendment), on the desktop too (the owner, 2026-10-29: "Show
//!   Gemini's + on desktop"); the message card only on a phone while the
//!   daemon offers `images` (the binary files plan), which it does not
//!   yet (11.2).
//! - The **toggle** is the keyboard glyph with a down chevron under it
//!   while the soft keyboard is up (hide), the glyph alone while it is down
//!   (show); dim and taking no taps while a physical keyboard types. Not on
//!   the desktop (11.3), where a physical keyboard always types.
//! - The **microphone** goes on by voice, the typed text kept; with the
//!   toggle (a phone only), and dim and taking no taps with the
//!   microphone setting off.
//! - The row is not shown when it would be empty.

use libcontrols::Rect;
use libmsdf::drawlist::DrawList;

use super::PromptKind;
use crate::agent::AgentHit;
use crate::controls::square_button::{ime_glyph, mic_glyph, plus_glyph};
use crate::settings;
use crate::theme::theme;

/// The row's height and its targets' side, in points.
pub const ROW_H: f32 = 44.0;
/// The gap between the text box and the row, in points.
pub const ROW_GAP: f32 = 4.0;

/// What the app tells the screens about typing's tools, each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputTools {
    /// A phone: the keyboard toggle is drawn, and the message card's +
    /// with `images` (decisions 11.2 and 11.3). Gemini's + is drawn
    /// everywhere.
    pub mobile: bool,
    /// The daemon offers the `images` namespace: the message card's + and
    /// its Attach image… (the binary files plan; not yet).
    pub images: bool,
    /// The soft keyboard is up for the typed card: the toggle hides it.
    pub keyboard_up: bool,
    /// A physical keyboard types: the toggle is dim and takes no taps.
    pub physical: bool,
    /// The microphone setting is on: the microphone takes taps.
    pub microphone: bool,
}

impl InputTools {
    /// Whether `kind`'s card has the +.
    pub fn attach(&self, kind: PromptKind) -> bool {
        match kind {
            PromptKind::GeneralAgent => true,
            PromptKind::Message => self.mobile && self.images,
        }
    }

    /// Whether the cards have the keyboard toggle.
    pub fn ime(&self) -> bool {
        self.mobile
    }

    /// Whether `kind`'s typed card has a row at all.
    pub fn row(&self, kind: PromptKind) -> bool {
        self.attach(kind) || self.ime()
    }

    /// The height a typed card grows by for its row: the row and its gap,
    /// or nothing.
    pub fn grows(&self, kind: PromptKind, s: f32) -> f32 {
        if self.row(kind) {
            (ROW_H + ROW_GAP) * s
        } else {
            0.0
        }
    }
}

/// A typed card's row, as laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputRow {
    kind: PromptKind,
    pub attach: Option<Rect>,
    pub ime: Option<Rect>,
    pub mic: Option<Rect>,
    keyboard_up: bool,
    ime_live: bool,
    mic_live: bool,
    scale: f32,
}

impl InputRow {
    /// `kind`'s row with its top at `y`, `x` to `x + w`, at scale `s`;
    /// `None` when it would be empty.
    pub fn lay_out(
        kind: PromptKind,
        x: f32,
        y: f32,
        w: f32,
        s: f32,
        tools: &InputTools,
    ) -> Option<InputRow> {
        if !tools.row(kind) {
            return None;
        }
        let side = ROW_H * s;
        Some(InputRow {
            kind,
            attach: tools.attach(kind).then(|| Rect::new(x, y, side, side)),
            ime: tools.ime().then(|| Rect::new(x + w - side, y, side, side)),
            mic: tools
                .ime()
                .then(|| Rect::new(x + w - 2.0 * side, y, side, side)),
            keyboard_up: tools.keyboard_up,
            ime_live: !tools.physical,
            mic_live: tools.microphone,
            scale: s,
        })
    }

    fn moved(r: Rect, dy: f32) -> Rect {
        Rect::new(r.x, r.y + dy, r.w, r.h)
    }

    /// Where `hit` is, moved down by `dy`.
    pub fn rect_of(&self, hit: AgentHit, dy: f32) -> Option<Rect> {
        match hit {
            AgentHit::PromptAttach(k) if k == self.kind => self.attach,
            AgentHit::PromptIme(k) if k == self.kind => self.ime,
            AgentHit::PromptMic(k) if k == self.kind => self.mic,
            _ => None,
        }
        .map(|r| Self::moved(r, dy))
    }

    /// The control at `(x, y)`, the row moved down by `dy`: the +, or the
    /// toggle or the microphone unless it is dim.
    pub fn hit(&self, x: f32, y: f32, dy: f32) -> Option<AgentHit> {
        if self
            .attach
            .is_some_and(|r| Self::moved(r, dy).contains([x, y]))
        {
            return Some(AgentHit::PromptAttach(self.kind));
        }
        if self.ime_live
            && self
                .ime
                .is_some_and(|r| Self::moved(r, dy).contains([x, y]))
        {
            return Some(AgentHit::PromptIme(self.kind));
        }
        if self.mic_live
            && self
                .mic
                .is_some_and(|r| Self::moved(r, dy).contains([x, y]))
        {
            return Some(AgentHit::PromptMic(self.kind));
        }
        None
    }

    /// Draws it moved down by `dy`.
    pub fn draw(&self, list: &mut DrawList, dy: f32) {
        let s = self.scale;
        if let Some(r) = self.attach {
            plus_glyph(list, Self::moved(r, dy), s, theme().accent);
        }
        if let Some(r) = self.ime {
            let start = list.instances.len();
            ime_glyph(list, Self::moved(r, dy), s, theme().fg, self.keyboard_up);
            if !self.ime_live {
                settings::dim_since(list, start);
            }
        }
        if let Some(r) = self.mic {
            let start = list.instances.len();
            mic_glyph(list, Self::moved(r, dy), s, theme().fg);
            if !self.mic_live {
                settings::dim_since(list, start);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: InputTools = InputTools {
        mobile: true,
        images: false,
        keyboard_up: true,
        physical: false,
        microphone: true,
    };

    #[test]
    fn geminis_row_has_the_plus_and_the_toggle_the_messages_the_toggle() {
        let s = 2.0;
        let g =
            InputRow::lay_out(PromptKind::GeneralAgent, 100.0, 500.0, 800.0, s, &PHONE).unwrap();
        let plus = g.attach.unwrap();
        let ime = g.ime.unwrap();
        assert_eq!(plus, Rect::new(100.0, 500.0, 88.0, 88.0));
        assert_eq!(ime, Rect::new(900.0 - 88.0, 500.0, 88.0, 88.0));
        assert_eq!(
            g.hit(110.0, 510.0, 0.0),
            Some(AgentHit::PromptAttach(PromptKind::GeneralAgent))
        );
        assert_eq!(
            g.hit(890.0, 510.0, 0.0),
            Some(AgentHit::PromptIme(PromptKind::GeneralAgent))
        );
        // Moved with the overlay's slide.
        assert_eq!(g.hit(110.0, 510.0, 100.0), None);
        assert_eq!(
            g.rect_of(AgentHit::PromptAttach(PromptKind::GeneralAgent), 100.0)
                .map(|r| r.y),
            Some(600.0)
        );
        // The message card: no + until the daemon offers images.
        let m = InputRow::lay_out(PromptKind::Message, 100.0, 500.0, 800.0, s, &PHONE).unwrap();
        assert!(m.attach.is_none() && m.ime.is_some());
        let images = InputTools {
            images: true,
            ..PHONE
        };
        let m = InputRow::lay_out(PromptKind::Message, 100.0, 500.0, 800.0, s, &images).unwrap();
        assert!(m.attach.is_some());
        assert_eq!(PHONE.grows(PromptKind::Message, s), 96.0, "48 pt");
    }

    #[test]
    fn the_microphone_is_left_of_the_toggle_and_dim_with_the_microphone_off() {
        let s = 2.0;
        for kind in [PromptKind::GeneralAgent, PromptKind::Message] {
            let r = InputRow::lay_out(kind, 100.0, 500.0, 800.0, s, &PHONE).unwrap();
            let mic = r.mic.unwrap();
            assert_eq!(mic, Rect::new(900.0 - 2.0 * 88.0, 500.0, 88.0, 88.0));
            assert_eq!(
                r.hit(mic.x + 5.0, mic.y + 5.0, 0.0),
                Some(AgentHit::PromptMic(kind))
            );
            assert_eq!(r.rect_of(AgentHit::PromptMic(kind), 0.0), Some(mic));
        }
        let off = InputTools {
            microphone: false,
            ..PHONE
        };
        let r = InputRow::lay_out(PromptKind::Message, 0.0, 0.0, 800.0, s, &off).unwrap();
        let mic = r.mic.unwrap();
        assert_eq!(r.hit(mic.x + 5.0, mic.y + 5.0, 0.0), None);
        let mut on = DrawList::new();
        InputRow::lay_out(PromptKind::Message, 0.0, 0.0, 800.0, s, &PHONE)
            .unwrap()
            .draw(&mut on, 0.0);
        let mut dim = DrawList::new();
        r.draw(&mut dim, 0.0);
        assert_eq!(on.instances.len(), dim.instances.len());
        let dimmed = dim
            .instances
            .iter()
            .filter(|i| (i.color[3] - theme().fg[3] * settings::DISABLED_ALPHA).abs() < 1e-6)
            .count();
        assert!(dimmed > 0 && dimmed < dim.instances.len(), "only the mic");
        // The desktop: no microphone, as no toggle.
        let desktop = InputTools::default();
        let g = InputRow::lay_out(PromptKind::GeneralAgent, 0.0, 0.0, 800.0, s, &desktop).unwrap();
        assert!(g.mic.is_none());
    }

    #[test]
    fn a_physical_keyboard_dims_the_toggle_and_the_desktop_has_geminis_plus_alone() {
        let s = 2.0;
        let physical = InputTools {
            physical: true,
            // The microphone off too, so the whole row is dim.
            microphone: false,
            ..PHONE
        };
        let r = InputRow::lay_out(PromptKind::Message, 0.0, 0.0, 800.0, s, &physical).unwrap();
        let ime = r.ime.unwrap();
        assert_eq!(r.hit(ime.x + 5.0, ime.y + 5.0, 0.0), None);
        let mut list = DrawList::new();
        r.draw(&mut list, 0.0);
        assert!(
            list.instances
                .iter()
                .all(|i| (i.color[3] - theme().fg[3] * settings::DISABLED_ALPHA).abs() < 1e-6)
        );
        // Up: the chevron's two strokes more than down.
        let count = |up| {
            let tools = InputTools {
                keyboard_up: up,
                ..PHONE
            };
            let r = InputRow::lay_out(PromptKind::Message, 0.0, 0.0, 800.0, s, &tools).unwrap();
            let mut list = DrawList::new();
            r.draw(&mut list, 0.0);
            list.instances.len()
        };
        assert_eq!(count(true), count(false) + 2);
        // The desktop: Gemini's row has the + alone; the message card has
        // no row, even with images.
        let desktop = InputTools {
            images: true,
            ..InputTools::default()
        };
        let g = InputRow::lay_out(PromptKind::GeneralAgent, 0.0, 0.0, 800.0, s, &desktop)
            .expect("Gemini's + on the desktop");
        assert!(g.attach.is_some() && g.ime.is_none());
        assert!(InputRow::lay_out(PromptKind::Message, 0.0, 0.0, 800.0, s, &desktop).is_none());
        assert_eq!(desktop.grows(PromptKind::Message, s), 0.0);
    }
}
