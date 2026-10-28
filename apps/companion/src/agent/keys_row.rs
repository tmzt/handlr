//! The terminal's keys row (plan 20261022_03, the L3 amendment of
//! 2026-10-28 10:42 EDT, items 3 and 4): one row the full width of the
//! view, just above the soft keyboard while it is raised into the
//! terminal, of **ESC**, **TAB**, **CTRL**, **ALT** and a collapsed d-pad
//! laid out as a 101-key keyboard's arrow block, **↑** above **← ↓ →**,
//! the two halves sharing the row's height.
//!
//! - **Size.** [`KEYS_H`], 44 pt: a little less than the bottom row's
//!   52 pt buttons, so each d-pad half is 22 pt less its gaps. The row is
//!   seven equal columns: ESC, TAB, CTRL and ALT one each, the d-pad the
//!   last three (↑ over the middle one).
//! - **Keys.** ESC, TAB and the arrows send at once (`escape`, `tab`,
//!   `up`, `down`, `left`, `right`), with any latched modifiers
//!   (`ctrl+up`, `ctrl+alt+left`), and the latch clears.
//! - **Modifiers.** CTRL and ALT latch ([`Latch`]), drawn highlighted; a
//!   tap on a latched one clears it. The next key from the soft keyboard is
//!   sent with them ([`Latch::apply`]), and the latch clears.
//! - **Drawing.** The labels in the UI font; the arrows as strokes (a shaft
//!   and a head), since the faces' arrow glyphs are not relied on.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use super::{fill, line_h};
use crate::theme::theme;

/// The row's height, in points.
pub const KEYS_H: f32 = 44.0;
/// The row's columns: four keys and the d-pad's three.
const COLUMNS: usize = 7;
/// Space around and between the keys, in points.
const GAP: f32 = 4.0;
/// Space between the d-pad's halves, in points.
const HALF_GAP: f32 = 2.0;
/// The labels' type size, in points.
const LABEL_PT: f32 = 14.0;

/// A key of the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TermKey {
    Esc,
    Tab,
    Ctrl,
    Alt,
    Up,
    Left,
    Down,
    Right,
}

impl TermKey {
    /// Every key, in the row's order (the d-pad's top half first).
    pub const ALL: [TermKey; 8] = [
        TermKey::Esc,
        TermKey::Tab,
        TermKey::Ctrl,
        TermKey::Alt,
        TermKey::Up,
        TermKey::Left,
        TermKey::Down,
        TermKey::Right,
    ];

    /// The `terminal.input` token it sends, unmodified; `None` for the
    /// modifiers, which latch.
    pub fn token(self) -> Option<&'static str> {
        Some(match self {
            TermKey::Esc => "escape",
            TermKey::Tab => "tab",
            TermKey::Up => "up",
            TermKey::Down => "down",
            TermKey::Left => "left",
            TermKey::Right => "right",
            TermKey::Ctrl | TermKey::Alt => return None,
        })
    }

    /// The label drawn on it; `None` for the arrows, drawn as strokes.
    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            TermKey::Esc => "ESC",
            TermKey::Tab => "TAB",
            TermKey::Ctrl => "CTRL",
            TermKey::Alt => "ALT",
            _ => return None,
        })
    }

    /// The arrow's direction, `[dx, dy]` with y down.
    fn arrow(self) -> Option<[f32; 2]> {
        Some(match self {
            TermKey::Up => [0.0, -1.0],
            TermKey::Down => [0.0, 1.0],
            TermKey::Left => [-1.0, 0.0],
            TermKey::Right => [1.0, 0.0],
            _ => return None,
        })
    }
}

/// The modifiers latched for the next key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Latch {
    pub ctrl: bool,
    pub alt: bool,
}

/// The named keys the daemon takes with `ctrl+`/`alt+` (its
/// `parse_keys`).
const MODIFIABLE: [&str; 9] = [
    "enter",
    "escape",
    "esc",
    "tab",
    "backspace",
    "up",
    "down",
    "left",
    "right",
];

impl Latch {
    pub fn is_empty(self) -> bool {
        !self.ctrl && !self.alt
    }

    /// Whether `key` is latched (only CTRL and ALT can be).
    pub fn has(self, key: TermKey) -> bool {
        match key {
            TermKey::Ctrl => self.ctrl,
            TermKey::Alt => self.alt,
            _ => false,
        }
    }

    /// A tap on CTRL or ALT: latched, or cleared when it was.
    pub fn toggle(&mut self, key: TermKey) {
        match key {
            TermKey::Ctrl => self.ctrl = !self.ctrl,
            TermKey::Alt => self.alt = !self.alt,
            _ => {}
        }
    }

    fn prefix(self) -> &'static str {
        match (self.ctrl, self.alt) {
            (true, true) => "ctrl+alt+",
            (true, false) => "ctrl+",
            (false, true) => "alt+",
            (false, false) => "",
        }
    }

    /// `token` (one of `terminal_keys`' tokens) with the latched
    /// modifiers, as the daemon takes them: a named key it modifies
    /// (`ctrl+backspace`, `alt+enter`, `ctrl+left`); with CTRL a letter,
    /// lowercased (`ctrl+c`, `ctrl+alt+x`); with ALT alone any printable
    /// character (`alt+.`). A token the daemon takes no modifier on
    /// (`space`, `ctrl+1`, a `ctrl+` token already) is sent as it is.
    pub fn apply(self, token: &str) -> String {
        if self.is_empty() {
            return token.to_string();
        }
        if MODIFIABLE.contains(&token) {
            return format!("{}{token}", self.prefix());
        }
        let mut chars = token.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return token.to_string();
        };
        if self.ctrl {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() {
                return format!("{}{c}", self.prefix());
            }
            return token.to_string();
        }
        if c.is_control() || c.is_whitespace() {
            return token.to_string();
        }
        format!("{}{c}", self.prefix())
    }

    /// `keys` (whitespace-separated tokens) with the latched modifiers on
    /// the first, the next key typed; the rest as they are.
    pub fn apply_first(self, keys: &str) -> String {
        let mut tokens = keys.split_whitespace();
        let Some(first) = tokens.next() else {
            return keys.to_string();
        };
        std::iter::once(self.apply(first))
            .chain(tokens.map(str::to_string))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The keys' rects in `row`, at scale `s`, in [`TermKey::ALL`]'s order.
/// Their tap areas are [`tap_area`]s; a tap elsewhere in the row (the
/// d-pad's empty top corners) does nothing.
pub fn lay_out(row: Rect, s: f32) -> Vec<(TermKey, Rect)> {
    let gap = GAP * s;
    let cw = (row.w - gap * (COLUMNS as f32 + 1.0)) / COLUMNS as f32;
    let col_x = |i: usize| row.x + gap + i as f32 * (cw + gap);
    let top = row.y + gap;
    let h = (row.h - 2.0 * gap).max(0.0);
    let half = ((h - HALF_GAP * s) / 2.0).max(0.0);
    let low = top + half + HALF_GAP * s;
    let whole = |i| Rect::new(col_x(i), top, cw, h);
    let upper = |i| Rect::new(col_x(i), top, cw, half);
    let lower = |i| Rect::new(col_x(i), low, cw, half);
    vec![
        (TermKey::Esc, whole(0)),
        (TermKey::Tab, whole(1)),
        (TermKey::Ctrl, whole(2)),
        (TermKey::Alt, whole(3)),
        (TermKey::Up, upper(5)),
        (TermKey::Left, lower(4)),
        (TermKey::Down, lower(5)),
        (TermKey::Right, lower(6)),
    ]
}

/// The key drawn in `rect` grown by half the gaps around it, for its tap
/// area.
pub fn tap_area(rect: Rect, s: f32) -> Rect {
    let g = GAP * s * 0.5;
    let v = HALF_GAP * s * 0.5;
    Rect::new(rect.x - g, rect.y - v, rect.w + 2.0 * g, rect.h + 2.0 * v)
}

/// The row in `row`: its band, the keys, CTRL and ALT highlighted while
/// latched.
pub fn draw(
    list: &mut DrawList,
    ctx: &DrawContext,
    row: Rect,
    keys: &[(TermKey, Rect)],
    latch: Latch,
    s: f32,
) {
    fill(list, row, theme().header, 0.0);
    let px = LABEL_PT * s;
    for &(key, r) in keys {
        let on = latch.has(key);
        let (bg, fg) = if on {
            (theme().accent, theme().bg)
        } else {
            (theme().card, theme().fg)
        };
        fill(list, r, bg, 6.0 * s);
        if let Some(label) = key.label() {
            // Roboto's capitals are about 0.62 em wide.
            let w = label.chars().count() as f32 * px * 0.62;
            let pos = [r.x + (r.w - w) * 0.5, r.y + (r.h - line_h(px)) * 0.5];
            ctx.draw_text(list, label, pos, px, fg);
        } else if let Some(d) = key.arrow() {
            arrow(list, r, d, s, fg);
        }
    }
}

/// An arrow centred in `r` pointing along `d`: a shaft and a two-stroke
/// head.
fn arrow(list: &mut DrawList, r: Rect, d: [f32; 2], s: f32, color: [f32; 4]) {
    let c = [r.x + r.w * 0.5, r.y + r.h * 0.5];
    let len = (r.h.min(r.w) * 0.32).max(2.0 * s);
    let head = len * 0.6;
    let p = [-d[1], d[0]];
    let tip = [c[0] + d[0] * len, c[1] + d[1] * len];
    let tail = [c[0] - d[0] * len, c[1] - d[1] * len];
    let t = 1.8 * s;
    list.push_line(tail, tip, t, color);
    for side in [1.0, -1.0] {
        let end = [
            tip[0] - d[0] * head + p[0] * head * side,
            tip[1] - d[1] * head + p[1] * head * side,
        ];
        list.push_line(tip, end, t, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_sends_its_token_and_the_modifiers_latch() {
        let tokens: Vec<_> = TermKey::ALL.iter().map(|k| k.token()).collect();
        assert_eq!(
            tokens,
            [
                Some("escape"),
                Some("tab"),
                None,
                None,
                Some("up"),
                Some("left"),
                Some("down"),
                Some("right")
            ]
        );
        let mut l = Latch::default();
        assert!(l.is_empty());
        l.toggle(TermKey::Ctrl);
        assert!(l.has(TermKey::Ctrl) && !l.has(TermKey::Alt));
        l.toggle(TermKey::Alt);
        assert_eq!(
            l,
            Latch {
                ctrl: true,
                alt: true
            }
        );
        // Tapped again: cleared.
        l.toggle(TermKey::Ctrl);
        assert_eq!(
            l,
            Latch {
                ctrl: false,
                alt: true
            }
        );
        l.toggle(TermKey::Alt);
        assert!(l.is_empty());
        // The other keys do not latch.
        l.toggle(TermKey::Esc);
        assert!(l.is_empty());
    }

    #[test]
    fn a_latch_modifies_the_next_token_as_the_daemon_takes_it() {
        let ctrl = Latch {
            ctrl: true,
            alt: false,
        };
        let alt = Latch {
            ctrl: false,
            alt: true,
        };
        let both = Latch {
            ctrl: true,
            alt: true,
        };
        assert_eq!(Latch::default().apply("a"), "a");
        assert_eq!(ctrl.apply("c"), "ctrl+c");
        assert_eq!(ctrl.apply("R"), "ctrl+r");
        assert_eq!(alt.apply("x"), "alt+x");
        assert_eq!(alt.apply("X"), "alt+X");
        assert_eq!(alt.apply("."), "alt+.");
        assert_eq!(alt.apply("+"), "alt++");
        assert_eq!(both.apply("B"), "ctrl+alt+b");
        for (latch, want) in [(ctrl, "ctrl+"), (alt, "alt+"), (both, "ctrl+alt+")] {
            for named in [
                "up",
                "down",
                "left",
                "right",
                "escape",
                "tab",
                "enter",
                "backspace",
            ] {
                assert_eq!(latch.apply(named), format!("{want}{named}"));
            }
        }
        // What the daemon takes no modifier on goes as it is.
        assert_eq!(ctrl.apply("1"), "1");
        assert_eq!(both.apply("."), ".");
        assert_eq!(alt.apply("space"), "space");
        assert_eq!(ctrl.apply("space"), "space");
        assert_eq!(alt.apply("ctrl+a"), "ctrl+a");
        // Only the first of several.
        assert_eq!(ctrl.apply_first("a b"), "ctrl+a b");
        assert_eq!(ctrl.apply_first(""), "");
    }

    #[test]
    fn the_row_is_seven_columns_with_the_dpad_in_two_halves() {
        let s = 2.0;
        let row = Rect::new(0.0, 1000.0, 1080.0, KEYS_H * s);
        let keys = lay_out(row, s);
        assert_eq!(
            keys.iter().map(|k| k.0).collect::<Vec<_>>(),
            TermKey::ALL.to_vec()
        );
        let rect = |k| keys.iter().find(|(key, _)| *key == k).unwrap().1;
        // Left to right, inside the row.
        for (k, r) in &keys {
            assert!(r.x >= row.x && r.x + r.w <= row.x + row.w + 0.01, "{k:?}");
            assert!(r.y >= row.y && r.y + r.h <= row.y + row.h + 0.01, "{k:?}");
        }
        let (esc, tab, ctrl, alt) = (
            rect(TermKey::Esc),
            rect(TermKey::Tab),
            rect(TermKey::Ctrl),
            rect(TermKey::Alt),
        );
        assert!(esc.x < tab.x && tab.x < ctrl.x && ctrl.x < alt.x);
        let (up, left, down, right) = (
            rect(TermKey::Up),
            rect(TermKey::Left),
            rect(TermKey::Down),
            rect(TermKey::Right),
        );
        assert!(alt.x + alt.w < left.x);
        // ↑ over ↓, ← ↓ → in one line below it, each half the height.
        assert_eq!(up.x, down.x);
        assert!(up.y + up.h <= down.y);
        assert_eq!(left.y, down.y);
        assert_eq!(down.y, right.y);
        assert!(left.x < down.x && down.x < right.x);
        assert!(up.h < esc.h / 2.0 && up.h > esc.h / 2.0 - 4.0 * s);
        // Every key the same width.
        for (_, r) in &keys {
            assert!((r.w - esc.w).abs() < 0.01);
        }
    }

    #[test]
    fn the_row_draws_a_latched_modifier_highlighted() {
        let s = 2.0;
        let row = Rect::new(0.0, 0.0, 1080.0, KEYS_H * s);
        let keys = lay_out(row, s);
        let ctx = DrawContext::default();
        let colours = |latch| {
            let mut list = DrawList::new();
            draw(&mut list, &ctx, row, &keys, latch, s);
            list.instances.iter().map(|i| i.color).collect::<Vec<_>>()
        };
        assert!(!colours(Latch::default()).contains(&theme().accent));
        let latched = colours(Latch {
            ctrl: true,
            alt: false,
        });
        assert_eq!(
            latched.iter().filter(|c| **c == theme().accent).count(),
            1,
            "CTRL alone"
        );
    }
}
