//! The terminal mode's view (plan 20261027_07, decision 7 and step 5; step
//! 6's app half): the session's pane as herdr shows it, a cell grid of
//! attributed runs, in place of the transcript below the header.
//!
//! - **Drawing.** One terminal line per run at a fixed advance (Noto Sans
//!   Mono and the symbol page, as the transcript), its background a filled
//!   rectangle, underline and strikethrough as lines. Each row's shaped
//!   runs are cached until the row changes.
//! - **Colours.** The theme's text and background for the defaults; palette
//!   0-15 from an ANSI palette for dark or light backgrounds; 16-231 the
//!   xterm 6x6x6 cube and 232-255 its greys; 24-bit colours as they are.
//!   Bold is the bold cut and, on palette 0-7, the bright colour; dim is
//!   mixed toward the background; inverse swaps.
//! - **Gestures** (plan 20261022_03, the L3 amendment of 2026-10-27
//!   21:58 EDT, in place of plan 20261027_07 decision 6's hold). When the
//!   daemon allows input, a tap arms it and raises the soft keyboard, and a
//!   drag is locked to its first direction: vertical scrolls the pane as a
//!   mouse wheel does (drag down goes back into the scrollback, one line
//!   per row pitch, and arms first), horizontal pans the frame across.
//!   Without input, one finger pans in both axes and a tap does nothing.
//!   Pans are clamped to the frame; two fingers pinch the type between 7
//!   and 16 pt about their midpoint, and their mean movement pans; a
//!   two-finger double tap toggles fit-to-width (a one-finger tap now
//!   acts at once, so it cannot wait to tell a double tap). The first
//!   frame lands at the fit-width size (clamped to 7 pt), scrolled to its
//!   bottom rows (plan risk 3).
//!
//! Plan 20261027_08, step 9:
//!
//! - **Fit.** The columns and rows that fit the terminal's rect at the
//!   current type size ([`TermView::fit_at`]) go in `terminal.open` and, as
//!   a [`TermAsk::Resize`], whenever they change: the soft keyboard shown
//!   or hidden (the app's layout already takes libmorph's
//!   `keyboard_inset` off the bottom, so the rect is above the keyboard),
//!   rotation, the cards over the terminal changing height, and a pinch,
//!   once it ends (never while two fingers are down). On an attached
//!   terminal the type size is not fitted to the frame's width: the daemon
//!   sizes the pane to the fit, so the frame fits and does not pan; a
//!   frame larger than the view (the poll, or the desktop's size while it
//!   has the tab) still pans.
//! - **Cursor.** Drawn from [`Screen::cursor`] by herdr's DECSCUSR shape
//!   (`herdr::CursorState.shape`, wire.rs's `CursorShapeParam`, passed
//!   through unchanged by libherdr_term's conversion): 0 (the terminal's
//!   default), 1 and 2 a block, reverse video of the cell under it; 3 and
//!   4 an underline; 5 and 6 a bar at the cell's left edge; anything else
//!   a block. Odd (blinking) and even (steady) are drawn alike, steady.
//!   Hidden while the program hides it (`visible` false) or while the
//!   frame is `stale`.
//! - **Stale.** While the desktop has the tab (`Cursor::stale`, the 23:32
//!   EDT amendment) a pill says so over the frames, which keep coming; the
//!   next touch that sends input takes it back (the daemon re-claims on
//!   input).
//! - **Mouse.** When the daemon serves the mouse and the frame's cursor
//!   says the program asked for mouse reports, a tap is a left click
//!   (`terminal.mouse` down and up at the tapped cell) instead of the
//!   keyboard, and a drag that starts across is a left-button drag (down,
//!   a drag for each new cell, up); a drag that starts up or down stays the
//!   wheel (`terminal.scroll`, which an attached terminal delivers as a
//!   wheel event the program scrolls itself by). The cell is pane-local,
//!   from the touch point at the current size and pan, clamped to the
//!   frame. Both still need the arm: a click or drag on an unarmed terminal
//!   arms first and is held until the arm lands, then sent; it is dropped
//!   if the arm never comes. Without mouse reports a tap arms and raises the
//!   keyboard, as before.

use std::time::Duration;

use flankr_companion::{
    Cursor, MouseButton, Screen, TerminalClosedReason, TerminalMouseKind, TerminalSource,
    TerminalStatus,
};
use libcontrols::{DrawContext, Rect};
use libflankr_term::{Color, Row, Run, flags};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;
use libmsdf::font::{GlyphStyle, TerminalLine};

use super::{CELL_EM, SLOP, Terminal, fill, line_h, term_h};
use crate::theme::{luminance, theme, with_alpha};

/// The type size the view starts at, and the pinch's limits, in points.
pub const START_PT: f32 = 11.0;
pub const MIN_PT: f32 = 7.0;
pub const MAX_PT: f32 = 16.0;
/// Two two-finger taps this close in time are a double tap.
const DOUBLE_TAP: Duration = Duration::from_millis(400);
/// The status line's type size, in points.
const STATUS_PT: f32 = 14.0;
/// The pills' type size (the scroll offset, the stale tab), in points.
const PILL_PT: f32 = 13.0;
/// The stale pill's words (plan 20261027_08, the 23:32 EDT amendment).
pub const STALE_LABEL: &str = "Desktop has the tab \u{2014} touch to take it back";
/// A fit's cells are counted with this much slack, so a size that fills
/// the rect exactly is not one short from rounding.
const FIT_SLACK: f32 = 1e-3;

/// The terminal as the app last read it from the client.
#[derive(Clone, Debug, Default)]
pub struct TermState {
    /// The daemon offers the terminal mode on this connection.
    pub supported: bool,
    /// The daemon lets keys be sent (its `[terminal] input`).
    pub input_allowed: bool,
    /// The daemon serves `terminal.mouse` (`Terminals::mouse_allowed`).
    pub mouse_allowed: bool,
    /// The session's screen while its terminal is wanted.
    pub screen: Option<Screen>,
    /// The session's terminal, as the app can show it.
    pub status: Option<TerminalStatus>,
}

impl TermState {
    /// Keys may be sent now: the terminal is open and armed.
    pub fn armed(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|s| s.open && s.armed_for.is_some())
    }

    /// The cursor of the frame shown, when the frames carry one.
    pub fn cursor(&self) -> Option<Cursor> {
        self.screen.as_ref().and_then(Screen::cursor)
    }

    /// The desktop has the tab: the frames are not the phone's size.
    pub fn stale(&self) -> bool {
        self.cursor().is_some_and(|c| c.stale)
    }

    /// The daemon reads the terminal from its attachment to herdr, which
    /// it sizes to the phone's fit.
    pub fn attached(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|s| s.open && s.source == TerminalSource::Attached)
    }

    /// Touches are the program's mouse: input and the mouse are allowed,
    /// and the program asked for mouse reports.
    pub fn mouse_mode(&self) -> bool {
        self.input_allowed && self.mouse_allowed && self.cursor().is_some_and(|c| c.mouse_reporting)
    }
}

/// What the view asks of the client, taken each frame by the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermAsk {
    /// `terminal_open`, with what fits at the starting size.
    Open {
        cols: Option<u16>,
        rows: Option<u16>,
    },
    /// `terminal_close`.
    Close,
    /// `terminal_arm_input`: a tap or a vertical drag on the terminal.
    Arm,
    /// Raise the soft keyboard into the terminal once it is armed: a tap.
    Keyboard,
    /// `terminal_scroll`: lines from vertical drags, positive back into the
    /// scrollback.
    Scroll(i32),
    /// `terminal_resize`: what fits now, when it changed.
    Resize { cols: u16, rows: u16 },
    /// `terminal_mouse`: a click or drag at a pane-local cell.
    Mouse(MouseAt),
}

/// A mouse event at a pane-local cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseAt {
    pub kind: TerminalMouseKind,
    pub col: u16,
    pub row: u16,
}

impl MouseAt {
    fn left(kind: fn(MouseButton) -> TerminalMouseKind, (col, row): (u16, u16)) -> MouseAt {
        MouseAt {
            kind: kind(MouseButton::Left),
            col,
            row,
        }
    }
}

fn down(button: MouseButton) -> TerminalMouseKind {
    TerminalMouseKind::Down { button }
}
fn up(button: MouseButton) -> TerminalMouseKind {
    TerminalMouseKind::Up { button }
}
fn dragged(button: MouseButton) -> TerminalMouseKind {
    TerminalMouseKind::Drag { button }
}

/// How the cursor is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
}

/// herdr's DECSCUSR shape (`CursorState.shape`) as drawn: 3 and 4 an
/// underline, 5 and 6 a bar, the rest (0 the default, 1 and 2) a block.
pub fn cursor_shape(shape: u8) -> CursorShape {
    match shape {
        3 | 4 => CursorShape::Underline,
        5 | 6 => CursorShape::Bar,
        _ => CursorShape::Block,
    }
}

/// Where the cursor is drawn for a grid whose top left cell is at
/// `origin`, at `cell` by `lh` pixels a cell: its shape and rectangle, or
/// `None` while it is hidden (`!visible`) or the frame is stale.
pub fn cursor_mark(
    cursor: Option<Cursor>,
    origin: [f32; 2],
    cell: f32,
    lh: f32,
) -> Option<(CursorShape, Rect)> {
    let c = cursor.filter(|c| c.visible && !c.stale)?;
    let x = origin[0] + c.col as f32 * cell;
    let y = origin[1] + c.row as f32 * lh;
    let shape = cursor_shape(c.shape);
    let rect = match shape {
        CursorShape::Block => Rect::new(x, y, cell, lh),
        CursorShape::Underline => {
            let h = (lh * 0.1).max(2.0);
            Rect::new(x, y + lh - h, cell, h)
        }
        CursorShape::Bar => Rect::new(x, y, (cell * 0.15).max(2.0), lh),
    };
    Some((shape, rect))
}

/// The character at column `col` of `row` and its run, if the row reaches
/// it (one column per character, as libflankr_term counts).
pub fn char_at(row: &Row, col: usize) -> Option<(char, &Run)> {
    let mut at = 0usize;
    for run in row {
        let w = run.width();
        if col < at + w {
            return run.text.chars().nth(col - at).map(|c| (c, run));
        }
        at += w;
    }
    None
}

/// The stale pill's words, while the desktop has the tab.
pub fn stale_label(state: &TermState) -> Option<&'static str> {
    state.stale().then_some(STALE_LABEL)
}

/// The status line for `state`, when there is one to show instead of (or
/// with) the screen: opening, refused, ended or offline.
pub fn status_line(state: &TermState) -> Option<String> {
    let Some(st) = &state.status else {
        return Some(if state.supported {
            "Opening terminal\u{2026}".to_string()
        } else {
            "No terminal: the daemon is offline, or does not offer terminal mode \
             (no `terminal` in its hello)."
                .to_string()
        });
    };
    if let Some(reason) = st.closed {
        return Some(closed_words(reason).to_string());
    }
    if let Some(e) = &st.error {
        return Some(format!("The daemon refused the terminal: {e}"));
    }
    let empty = state.screen.as_ref().is_none_or(|s| s.seq() == 0);
    if st.opening && !state.supported {
        return Some(
            "Waiting for the link: the terminal opens again once the daemon answers hello."
                .to_string(),
        );
    }
    if empty {
        return Some("Opening terminal\u{2026}".to_string());
    }
    None
}

/// Why the terminal ended, in plain words.
pub fn closed_words(reason: TerminalClosedReason) -> &'static str {
    match reason {
        TerminalClosedReason::PaneClosed => {
            "The pane closed in herdr; the terminal ended. Tap the list glyph to go back."
        }
        TerminalClosedReason::AgentChanged => {
            "The pane's agent changed; the terminal ended. Switch to the transcript and back to open it again."
        }
        TerminalClosedReason::InputRevoked => {
            "Keyboard input was revoked by the daemon; the terminal ended."
        }
        TerminalClosedReason::Closed => "The daemon closed the terminal (it may be shutting down).",
        _ => "The daemon ended the terminal, for a reason this app does not know.",
    }
}

/// `rgb` bytes as a colour.
fn rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// The ANSI colours on a dark background (Tango's, which keep black and
/// blue legible on dark grey and navy).
const DARK_PALETTE: [u32; 16] = [
    0x55_5753, 0xEF_2929, 0x8A_E234, 0xFC_E94F, 0x72_9FCF, 0xAD_7FA8, 0x34_E2E2, 0xD3_D7CF,
    0x88_8A85, 0xFF_5F5F, 0xA6_F25A, 0xFF_F37A, 0x9C_C4F2, 0xD0_A6CC, 0x6F_F0F0, 0xFF_FFFF,
];
/// The ANSI colours on a light background: darker forms, so yellow and
/// white text stay legible on white.
const LIGHT_PALETTE: [u32; 16] = [
    0x00_0000, 0xC4_1A16, 0x1A_7F1E, 0x8A_6A00, 0x1F_4FBF, 0x9A_2A9A, 0x0F_7A80, 0x6E_6E6E,
    0x55_5555, 0xE0_2B26, 0x2A_9A2E, 0xA6_8000, 0x3B_6FE0, 0xB8_3EB8, 0x16_96A0, 0x3A_3A3A,
];

/// Palette index `i` (0-255) on a dark or a light background.
pub fn palette_colour(i: u8, dark: bool) -> [f32; 4] {
    let hex = |v: u32| rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    match i {
        0..=15 => hex(if dark { DARK_PALETTE } else { LIGHT_PALETTE }[i as usize]),
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        232..=255 => {
            let g = 8 + 10 * (i - 232);
            rgb(g, g, g)
        }
    }
}

/// Whether the theme in force has a dark background.
pub fn dark_theme() -> bool {
    luminance(theme().bg) < 0.5
}

/// A run's text and background colours (`None`: the default background,
/// not filled), in the theme in force.
pub fn run_colours(run: &Run, dark: bool) -> ([f32; 4], Option<[f32; 4]>) {
    let bold = run.flags & flags::BOLD != 0;
    let resolve = |c: Color, fg: bool| -> Option<[f32; 4]> {
        if c.is_default() {
            return None;
        }
        if let Some(i) = c.as_palette() {
            // Bold brightens the eight base colours, as xterm does.
            let i = if fg && bold && i < 8 { i + 8 } else { i };
            return Some(palette_colour(i, dark));
        }
        c.as_rgb().map(|(r, g, b)| rgb(r, g, b))
    };
    let mut fg = resolve(run.fg, true);
    let mut bg = resolve(run.bg, false);
    if run.flags & flags::INVERSE != 0 {
        let f = fg.unwrap_or(theme().text);
        let b = bg.unwrap_or(theme().bg);
        fg = Some(b);
        bg = Some(f);
    }
    let mut fg = fg.unwrap_or(theme().text);
    if run.flags & flags::DIM != 0 {
        let under = bg.unwrap_or(theme().bg);
        for (f, u) in fg.iter_mut().zip(under).take(3) {
            *f = *f * 0.55 + u * 0.45;
        }
    }
    (fg, bg)
}

/// What a touch on the terminal did, beyond panning and zooming the view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    /// The view moved (a pan or a zoom): draw again.
    pub moved: bool,
    /// A tap, with input allowed: arm, and raise the soft keyboard.
    pub tap: bool,
    /// A vertical drag, or in mouse mode a drag across, began with input
    /// allowed: arm (no keyboard).
    pub drag: bool,
    /// In mouse mode, a tap: arm (no keyboard); [`mouse`](Self::mouse)
    /// has its click.
    pub click: bool,
    /// Lines to scroll the pane by, positive back into the scrollback.
    pub lines: i32,
    /// Mouse events, in order, in mouse mode.
    pub mouse: Vec<MouseAt>,
}

/// The axis a one-finger drag is locked to, from its first direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Without input: the frame pans both ways.
    Free,
    /// Across: the frame pans horizontally.
    Across,
    /// Up and down: the pane scrolls.
    Wheel,
    /// In mouse mode, across first: the left button drags.
    Mouse,
}

/// One finger on the terminal.
#[derive(Clone, Copy, Debug)]
struct Finger {
    id: u64,
    start: [f32; 2],
    at: [f32; 2],
}

/// A pinch: the fingers' distance and the type size when it began.
#[derive(Clone, Copy, Debug)]
struct Pinch {
    dist: f32,
    pt: f32,
}

/// The whole lines in a wheel drag's `distance` (pixels, down positive) at
/// row `pitch`, taken out of it; the rest stays for the next move. Down,
/// the content follows the finger back into the scrollback: positive, as
/// a mouse wheel turned up.
pub fn wheel_lines(distance: &mut f32, pitch: f32) -> i32 {
    if pitch <= 0.0 {
        return 0;
    }
    let lines = (*distance / pitch).trunc();
    *distance -= lines * pitch;
    lines as i32
}

/// One row's shaped runs, kept until the row changes.
struct RowCache {
    row: Vec<Run>,
    lines: Vec<Option<TerminalLine>>,
}

/// The terminal view's own state: its size, where it is panned to, the
/// fingers on it, and the shaped rows.
pub struct TermView {
    /// The type size, in points.
    pt: f32,
    /// At the fit-width size, and the size before it, for the double tap.
    fit: bool,
    before_fit: f32,
    /// How far the grid is panned, in pixels from its top left.
    pan: [f32; 2],
    /// The first frame was placed.
    placed: bool,
    fingers: Vec<Finger>,
    moved: bool,
    /// Two fingers were down during this touch: not a one-finger tap.
    multi: bool,
    pinch: Option<Pinch>,
    /// The one-finger drag's axis, once it moved.
    axis: Option<Axis>,
    /// A wheel drag's distance not yet a whole line, in pixels.
    wheel: f32,
    /// When the last two-finger tap ended, for the double tap.
    last_tap: Option<Duration>,
    cache: Vec<Option<RowCache>>,
    /// Touches are the program's mouse ([`TermState::mouse_mode`]), as last
    /// set; a gesture keeps what it was when its first finger came down.
    mouse_mode: bool,
    gesture_mouse: bool,
    /// The left button is held at this cell (a mouse drag under way).
    mouse_held: Option<(u16, u16)>,
}

impl Default for TermView {
    fn default() -> Self {
        TermView {
            pt: START_PT,
            fit: false,
            before_fit: START_PT,
            pan: [0.0, 0.0],
            placed: false,
            fingers: Vec::new(),
            moved: false,
            multi: false,
            pinch: None,
            axis: None,
            wheel: 0.0,
            last_tap: None,
            cache: Vec::new(),
            mouse_mode: false,
            gesture_mouse: false,
            mouse_held: None,
        }
    }
}

impl TermView {
    pub fn pt(&self) -> f32 {
        self.pt
    }

    pub fn pan(&self) -> [f32; 2] {
        self.pan
    }

    pub fn fit(&self) -> bool {
        self.fit
    }

    /// The columns and rows that fit `rect` at the starting size: the hints
    /// for `terminal.open` of a new view.
    pub fn hints(rect: Rect, s: f32) -> (Option<u16>, Option<u16>) {
        Self::fit_at(rect, START_PT, s).unzip()
    }

    /// The columns and rows of `pt`-point type at scale `s` that fit
    /// `rect` (the terminal's area, above the soft keyboard when it shows):
    /// whole cells of the renderer's cell and line pitch, at least one
    /// each; `None` for an empty rect.
    pub fn fit_at(rect: Rect, pt: f32, s: f32) -> Option<(u16, u16)> {
        let px = pt * s;
        if rect.w <= 0.0 || rect.h <= 0.0 || px <= 0.0 {
            return None;
        }
        let count =
            |len: f32, pitch: f32| (len / pitch + FIT_SLACK).floor().clamp(1.0, 65535.0) as u16;
        Some((count(rect.w, CELL_EM * px), count(rect.h, term_h(px))))
    }

    /// What fits `rect` at the view's own size now.
    pub fn fit_size(&self, rect: Rect, s: f32) -> Option<(u16, u16)> {
        Self::fit_at(rect, self.pt, s)
    }

    /// Two fingers are pinching: the size is not settled.
    pub fn pinching(&self) -> bool {
        self.pinch.is_some()
    }

    /// Whether touches are the program's mouse
    /// ([`TermState::mouse_mode`]); a gesture under way keeps its mode.
    pub fn set_mouse_mode(&mut self, on: bool) {
        self.mouse_mode = on;
    }

    /// The pane-local cell under screen point `x`, `y`, at the current size
    /// and pan, clamped to the frame; `None` without a frame.
    pub fn cell_at(
        &self,
        x: f32,
        y: f32,
        screen: Option<&Screen>,
        rect: Rect,
        s: f32,
    ) -> Option<(u16, u16)> {
        let sc = screen.filter(|sc| sc.cols() > 0 && sc.rows() > 0)?;
        let px = self.pt * s;
        let col = ((x - rect.x + self.pan[0]) / (CELL_EM * px)).floor();
        let row = ((y - rect.y + self.pan[1]) / term_h(px)).floor();
        let clamp = |v: f32, n: u16| v.clamp(0.0, (n - 1) as f32) as u16;
        Some((clamp(col, sc.cols()), clamp(row, sc.rows())))
    }

    /// The type size at which `cols` columns fill `w`, within the limits.
    pub fn fit_pt(cols: u16, w: f32, s: f32) -> f32 {
        if cols == 0 {
            return START_PT;
        }
        (w / (cols as f32 * CELL_EM * s)).clamp(MIN_PT, MAX_PT)
    }

    /// The grid's size in pixels at the current size.
    fn grid_size(&self, screen: Option<&Screen>, s: f32) -> [f32; 2] {
        let px = self.pt * s;
        let (cols, rows) = screen.map_or((0, 0), |sc| (sc.cols(), sc.rows()));
        [cols as f32 * CELL_EM * px, rows as f32 * term_h(px)]
    }

    fn max_pan(&self, screen: Option<&Screen>, rect: Rect, s: f32) -> [f32; 2] {
        let g = self.grid_size(screen, s);
        [(g[0] - rect.w).max(0.0), (g[1] - rect.h).max(0.0)]
    }

    fn clamp(&mut self, screen: Option<&Screen>, rect: Rect, s: f32) {
        let m = self.max_pan(screen, rect, s);
        self.pan = [self.pan[0].clamp(0.0, m[0]), self.pan[1].clamp(0.0, m[1])];
    }

    /// A new screen or size: the first frame is placed at its bottom rows,
    /// at the fit-width size unless `attached` (the daemon sizes an
    /// attached pane to the view's fit, so its frame fits at the size the
    /// fit was worked out at); afterwards the pan is kept, within the
    /// frame. Returns whether anything moved.
    pub fn follow(&mut self, screen: Option<&Screen>, rect: Rect, s: f32, attached: bool) -> bool {
        let before = (self.pt, self.pan);
        if !self.placed
            && let Some(sc) = screen.filter(|sc| sc.cols() > 0)
        {
            self.placed = true;
            if !attached {
                self.before_fit = self.pt;
                self.pt = Self::fit_pt(sc.cols(), rect.w, s);
                self.fit = true;
            }
            self.pan = [0.0, f32::MAX];
        }
        self.clamp(screen, rect, s);
        before != (self.pt, self.pan)
    }

    /// Sets the type size, keeping the grid's point under `anchor` (screen
    /// coordinates) where it is.
    fn zoom_to(&mut self, pt: f32, anchor: [f32; 2], rect: Rect) {
        let pt = pt.clamp(MIN_PT, MAX_PT);
        let k = pt / self.pt;
        let ax = anchor[0] - rect.x;
        let ay = anchor[1] - rect.y;
        self.pan = [(self.pan[0] + ax) * k - ax, (self.pan[1] + ay) * k - ay];
        self.pt = pt;
    }

    /// The two-finger double tap: to the fit-width size, or back to the
    /// size before.
    pub fn toggle_fit(&mut self, screen: Option<&Screen>, rect: Rect, s: f32, at: [f32; 2]) {
        let cols = screen.map_or(0, Screen::cols);
        if self.fit {
            self.fit = false;
            let pt = self.before_fit;
            self.zoom_to(pt, at, rect);
        } else {
            self.before_fit = self.pt;
            let pt = Self::fit_pt(cols, rect.w, s);
            self.zoom_to(pt, [rect.x, at[1]], rect);
            self.pan[0] = 0.0;
            self.fit = true;
        }
        self.clamp(screen, rect, s);
    }

    /// A finger on the terminal at `now` (the touch's clock); `input`: the
    /// daemon allows typing, so a tap arms and a vertical drag scrolls the
    /// pane. Returns what it did.
    #[allow(clippy::too_many_arguments)]
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
        screen: Option<&Screen>,
        rect: Rect,
        s: f32,
        input: bool,
    ) -> Touched {
        let mut out = Touched::default();
        match phase {
            TouchPhase::Started => {
                if self.fingers.iter().any(|f| f.id == id) {
                    // A lift that was lost: start over.
                    self.fingers.clear();
                }
                if self.fingers.is_empty() {
                    self.moved = false;
                    self.multi = false;
                    self.axis = None;
                    self.wheel = 0.0;
                    self.gesture_mouse = self.mouse_mode;
                    self.mouse_held = None;
                }
                self.fingers.push(Finger {
                    id,
                    start: [x, y],
                    at: [x, y],
                });
                if self.fingers.len() >= 2 {
                    self.multi = true;
                    self.pinch = Some(Pinch {
                        dist: self.spread().max(1.0),
                        pt: self.pt,
                    });
                    // A second finger ends a mouse drag: the button is let go.
                    if let Some(cell) = self.mouse_held.take() {
                        out.mouse.push(MouseAt::left(up, cell));
                    }
                }
            }
            TouchPhase::Moved => {
                let Some(i) = self.fingers.iter().position(|f| f.id == id) else {
                    return out;
                };
                let was = self.fingers[i].at;
                self.fingers[i].at = [x, y];
                let start = self.fingers[i].start;
                let (dx, dy) = (x - start[0], y - start[1]);
                if !self.moved && (dx.abs() > SLOP * s || dy.abs() > SLOP * s) {
                    self.moved = true;
                }
                if !self.moved {
                    return out;
                }
                let before = (self.pt, self.pan);
                if let (Some(p), true) = (self.pinch, self.fingers.len() >= 2) {
                    let pt = p.pt * self.spread() / p.dist;
                    let mid = self.mid();
                    // The pair's mean movement pans too.
                    let n = self.fingers.len() as f32;
                    self.pan[0] -= (x - was[0]) / n;
                    self.pan[1] -= (y - was[1]) / n;
                    self.zoom_to(pt, mid, rect);
                    self.fit = false;
                } else if self.fingers.len() == 1 && !self.multi {
                    let first = self.axis.is_none();
                    let mouse = self.gesture_mouse;
                    let axis = *self.axis.get_or_insert_with(|| {
                        if !input {
                            Axis::Free
                        } else if dy.abs() > dx.abs() {
                            Axis::Wheel
                        } else if mouse {
                            Axis::Mouse
                        } else {
                            Axis::Across
                        }
                    });
                    // The wheel's or the mouse drag's first move: its arm.
                    out.drag = first && matches!(axis, Axis::Wheel | Axis::Mouse);
                    match axis {
                        Axis::Mouse => {
                            if first
                                && let Some(cell) =
                                    self.cell_at(start[0], start[1], screen, rect, s)
                            {
                                out.mouse.push(MouseAt::left(down, cell));
                                self.mouse_held = Some(cell);
                            }
                            if let (Some(held), Some(cell)) =
                                (self.mouse_held, self.cell_at(x, y, screen, rect, s))
                                && held != cell
                            {
                                out.mouse.push(MouseAt::left(dragged, cell));
                                self.mouse_held = Some(cell);
                            }
                        }
                        Axis::Free => {
                            self.pan[0] -= x - was[0];
                            self.pan[1] -= y - was[1];
                        }
                        Axis::Across => self.pan[0] -= x - was[0],
                        Axis::Wheel => {
                            self.wheel += y - was[1];
                            out.lines = wheel_lines(&mut self.wheel, term_h(self.pt * s));
                        }
                    }
                }
                self.clamp(screen, rect, s);
                out.moved = before != (self.pt, self.pan);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(i) = self.fingers.iter().position(|f| f.id == id) else {
                    return out;
                };
                self.fingers.remove(i);
                if self.fingers.len() < 2 {
                    self.pinch = None;
                }
                if self.fingers.is_empty()
                    && let Some(cell) = self.mouse_held.take()
                {
                    out.mouse.push(MouseAt::left(up, cell));
                }
                let tap = phase == TouchPhase::Ended && self.fingers.is_empty() && !self.moved;
                if !tap {
                    return out;
                }
                if !self.multi {
                    if input && self.gesture_mouse {
                        // A left click at the tapped cell.
                        if let Some(cell) = self.cell_at(x, y, screen, rect, s) {
                            out.click = true;
                            out.mouse.push(MouseAt::left(down, cell));
                            out.mouse.push(MouseAt::left(up, cell));
                        }
                    } else {
                        out.tap = input;
                    }
                    return out;
                }
                // Two fingers, lifted without moving: twice is the double
                // tap.
                if self
                    .last_tap
                    .is_some_and(|t| now.saturating_sub(t) <= DOUBLE_TAP)
                {
                    self.last_tap = None;
                    self.toggle_fit(screen, rect, s, [x, y]);
                    out.moved = true;
                } else {
                    self.last_tap = Some(now);
                }
            }
        }
        out
    }

    fn spread(&self) -> f32 {
        match self.fingers.as_slice() {
            [a, b, ..] => ((a.at[0] - b.at[0]).powi(2) + (a.at[1] - b.at[1]).powi(2)).sqrt(),
            _ => 0.0,
        }
    }

    fn mid(&self) -> [f32; 2] {
        match self.fingers.as_slice() {
            [a, b, ..] => [(a.at[0] + b.at[0]) * 0.5, (a.at[1] + b.at[1]) * 0.5],
            [a] => a.at,
            [] => [0.0, 0.0],
        }
    }

    /// A finger is on the terminal.
    pub fn touching(&self) -> bool {
        !self.fingers.is_empty()
    }

    /// The axis the drag under way is locked to, once it moved.
    pub fn axis(&self) -> Option<Axis> {
        self.axis
    }

    /// Draws the grid in `rect`, clipped to it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        state: &TermState,
        rect: Rect,
        s: f32,
    ) {
        list.push_clip(rect.pos(), rect.size());
        let px = self.pt * s;
        let cell = CELL_EM * px;
        let lh = term_h(px);
        let dark = dark_theme();
        if let Some(screen) = &state.screen {
            let grid = screen.grid();
            if self.cache.len() != grid.lines.len() {
                self.cache.clear();
                self.cache.resize_with(grid.lines.len(), || None);
            }
            let first = (self.pan[1] / lh).floor().max(0.0) as usize;
            let last = (((self.pan[1] + rect.h) / lh).ceil() as usize).min(grid.lines.len());
            for r in first..last {
                let row = &grid.lines[r];
                let y = rect.y - self.pan[1] + r as f32 * lh;
                let cache = &mut self.cache[r];
                if cache.as_ref().is_none_or(|c| c.row != *row) {
                    *cache = Some(RowCache {
                        row: row.clone(),
                        lines: vec![None; row.len()],
                    });
                }
                let cache = cache.as_mut().expect("just filled");
                let mut col = 0usize;
                for (k, run) in row.iter().enumerate() {
                    let w = run.width();
                    let x = rect.x - self.pan[0] + col as f32 * cell;
                    col += w;
                    let wpx = w as f32 * cell;
                    if x + wpx < rect.x || x > rect.x + rect.w {
                        continue;
                    }
                    let (fg, bg) = run_colours(run, dark);
                    if let Some(bg) = bg {
                        fill(list, Rect::new(x, y, wpx, lh), bg, 0.0);
                    }
                    if !run.text.trim().is_empty() {
                        let style = if run.flags & flags::BOLD != 0 {
                            GlyphStyle::Bold
                        } else {
                            GlyphStyle::Regular
                        };
                        match term {
                            Some(t) => {
                                let line = cache.lines[k]
                                    .get_or_insert_with(|| t.shaper.line(&run.text, style));
                                list.push_terminal_line(
                                    line,
                                    t.text,
                                    t.symbols,
                                    [x, y],
                                    px,
                                    t.px_range,
                                    fg,
                                );
                            }
                            None => {
                                ctx.draw_text(list, &run.text, [x, y], px, fg);
                            }
                        }
                    }
                    let t = (px * 0.07).max(1.0);
                    if run.flags & flags::UNDERLINE != 0 {
                        let uy = y + lh * 0.86;
                        list.push_line([x, uy], [x + wpx, uy], t, fg);
                    }
                    if run.flags & flags::STRIKETHROUGH != 0 {
                        let sy = y + lh * 0.52;
                        list.push_line([x, sy], [x + wpx, sy], t, fg);
                    }
                }
            }
            // The cursor, over its cell.
            let origin = [rect.x - self.pan[0], rect.y - self.pan[1]];
            if let Some((shape, r)) = cursor_mark(screen.cursor(), origin, cell, lh) {
                let under = screen.cursor().and_then(|c| {
                    grid.lines
                        .get(c.row as usize)
                        .and_then(|row| char_at(row, c.col as usize))
                });
                let (fg, bg) =
                    under.map_or((theme().text, None), |(_, run)| run_colours(run, dark));
                fill(list, r, fg, 0.0);
                // A block shows its cell in reverse video.
                if shape == CursorShape::Block
                    && let Some((ch, run)) = under.filter(|(ch, _)| !ch.is_whitespace())
                {
                    let text = ch.to_string();
                    let colour = bg.unwrap_or(theme().bg);
                    let style = if run.flags & flags::BOLD != 0 {
                        GlyphStyle::Bold
                    } else {
                        GlyphStyle::Regular
                    };
                    match term {
                        Some(t) => {
                            let line = t.shaper.line(&text, style);
                            list.push_terminal_line(
                                &line,
                                t.text,
                                t.symbols,
                                [r.x, r.y],
                                px,
                                t.px_range,
                                colour,
                            );
                        }
                        None => {
                            ctx.draw_text(list, &text, [r.x, r.y], px, colour);
                        }
                    }
                }
            }
        }
        list.push_clip_end();
        // What is happening, over the grid's top.
        if let Some(text) = status_line(state) {
            let px = STATUS_PT * s;
            let pad = 12.0 * s;
            let cols = super::columns(rect.w - 2.0 * pad, px);
            for (i, line) in super::wrap_hard(&text, cols).iter().enumerate() {
                ctx.draw_text(
                    list,
                    line,
                    [rect.x + pad, rect.y + pad + i as f32 * line_h(px)],
                    px,
                    theme().dim,
                );
            }
        }
        // Armed: nothing is drawn for it, no frame and no pill (plan
        // 20261022_03, the L3 amendment of 2026-10-28 10:42 EDT, item 2).
        // Scrolled back: how far, at the top left.
        if let Some(label) = scroll_label(state) {
            let px = PILL_PT * s;
            let w = label.chars().count() as f32 * px * 0.5 + 16.0 * s;
            let h = line_h(px) + 6.0 * s;
            let pill = Rect::new(rect.x + 8.0 * s, rect.y + 8.0 * s, w, h);
            fill(list, pill, with_alpha(theme().card, 0.92), h * 0.5);
            ctx.draw_text(
                list,
                &label,
                [pill.x + 8.0 * s, pill.y + 3.0 * s],
                px,
                theme().fg,
            );
        }
        // The desktop has the tab: the frames keep coming, at its size.
        if let Some(label) = stale_label(state) {
            let px = PILL_PT * s;
            let w = (label.chars().count() as f32 * px * 0.5 + 16.0 * s).min(rect.w - 16.0 * s);
            let h = line_h(px) + 6.0 * s;
            let pill = Rect::new(
                rect.x + (rect.w - w) * 0.5,
                rect.y + rect.h - h - 8.0 * s,
                w,
                h,
            );
            fill(list, pill, with_alpha(theme().warn, 0.92), h * 0.5);
            ctx.draw_text(
                list,
                label,
                [pill.x + 8.0 * s, pill.y + 3.0 * s],
                px,
                theme().bg,
            );
        }
    }
}

/// The pill's words while the pane is scrolled back, as the last
/// `terminal.scroll` answer put it.
pub fn scroll_label(state: &TermState) -> Option<String> {
    let offset = state
        .status
        .as_ref()
        .filter(|s| s.open)
        .and_then(|s| s.scroll_offset)
        .filter(|&o| o > 0)?;
    Some(if offset == 1 {
        "1 line back".to_string()
    } else {
        format!("{offset} lines back")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_term::{Frame, parse_ansi};

    fn screen_of(ansi: &str) -> Screen {
        let mut sc = Screen::default();
        assert!(sc.apply(&Frame::full(1, 1, &parse_ansi(ansi))));
        sc
    }

    fn status(open: bool) -> TerminalStatus {
        TerminalStatus {
            open,
            opening: !open,
            cols: 0,
            rows: 0,
            closed: None,
            error: None,
            armed_for: None,
            keys_pending: 0,
            scroll_offset: None,
            scroll_max: None,
            cursor: false,
            source: TerminalSource::Poll,
        }
    }

    #[test]
    fn the_palette_is_ansi_then_the_cube_then_greys() {
        assert_eq!(palette_colour(16, true), rgb(0, 0, 0));
        assert_eq!(palette_colour(231, true), rgb(255, 255, 255));
        assert_eq!(palette_colour(196, true), rgb(255, 0, 0));
        assert_eq!(palette_colour(21, false), rgb(0, 0, 255));
        assert_eq!(palette_colour(232, true), rgb(8, 8, 8));
        assert_eq!(palette_colour(255, true), rgb(238, 238, 238));
        // The base sixteen differ with the background: white text stays
        // legible on white.
        assert_ne!(palette_colour(7, true), palette_colour(7, false));
        assert!(luminance(palette_colour(15, false)) < 0.2);
        assert!(luminance(palette_colour(0, true)) > 0.05);
    }

    #[test]
    fn a_runs_colours_follow_its_attributes() {
        let run = |fg: Color, bg: Color, f: u8| Run {
            text: "x".into(),
            fg,
            bg,
            flags: f,
        };
        // Defaults: the theme's text, no fill.
        let (fg, bg) = run_colours(&run(Color::DEFAULT, Color::DEFAULT, 0), true);
        assert_eq!((fg, bg), (theme().text, None));
        // Palette and RGB.
        let (fg, bg) = run_colours(&run(Color::palette(1), Color::rgb(1, 2, 3), 0), true);
        assert_eq!(fg, palette_colour(1, true));
        assert_eq!(bg, Some(rgb(1, 2, 3)));
        // Bold brightens the base eight.
        let (fg, _) = run_colours(&run(Color::palette(2), Color::DEFAULT, flags::BOLD), true);
        assert_eq!(fg, palette_colour(10, true));
        let (fg, _) = run_colours(&run(Color::palette(100), Color::DEFAULT, flags::BOLD), true);
        assert_eq!(fg, palette_colour(100, true));
        // Inverse swaps, defaults included.
        let (fg, bg) = run_colours(&run(Color::DEFAULT, Color::DEFAULT, flags::INVERSE), true);
        assert_eq!((fg, bg), (theme().bg, Some(theme().text)));
        let (fg, bg) = run_colours(
            &run(Color::palette(4), Color::palette(3), flags::INVERSE),
            true,
        );
        assert_eq!(
            (fg, bg),
            (palette_colour(3, true), Some(palette_colour(4, true)))
        );
        // Dim goes toward the background.
        let (fg, _) = run_colours(
            &run(Color::rgb(255, 255, 255), Color::DEFAULT, flags::DIM),
            true,
        );
        assert!(fg[0] < 1.0 && fg[0] > theme().bg[0]);
    }

    #[test]
    fn the_status_line_says_what_is_happening() {
        let mut st = TermState {
            supported: true,
            ..Default::default()
        };
        assert_eq!(
            status_line(&st).as_deref(),
            Some("Opening terminal\u{2026}")
        );
        st.status = Some(status(true));
        assert_eq!(
            status_line(&st).as_deref(),
            Some("Opening terminal\u{2026}")
        );
        st.screen = Some(screen_of("hello"));
        assert_eq!(status_line(&st), None);
        let mut closed = status(false);
        closed.closed = Some(TerminalClosedReason::PaneClosed);
        st.status = Some(closed);
        assert!(status_line(&st).unwrap().contains("pane closed"));
        let mut refused = status(false);
        refused.error = Some("not permitted".into());
        st.status = Some(refused);
        assert!(status_line(&st).unwrap().contains("not permitted"));
        st.supported = false;
        st.status = None;
        assert!(status_line(&st).unwrap().starts_with("No terminal"));
    }

    #[test]
    fn armed_needs_an_open_terminal() {
        let mut st = TermState::default();
        let mut s = status(true);
        s.armed_for = Some(Duration::MAX);
        st.status = Some(s.clone());
        assert!(st.armed());
        s.open = false;
        st.status = Some(s);
        assert!(!st.armed());
    }

    /// One finger down at `from`, moved to `to` in `steps`, and lifted.
    #[allow(clippy::too_many_arguments)]
    fn drag(
        v: &mut TermView,
        id: u64,
        from: [f32; 2],
        to: [f32; 2],
        steps: usize,
        screen: Option<&Screen>,
        rect: Rect,
        s: f32,
        input: bool,
    ) -> Vec<Touched> {
        let t = Duration::ZERO;
        let mut out = vec![v.touch(
            id,
            TouchPhase::Started,
            from[0],
            from[1],
            t,
            screen,
            rect,
            s,
            input,
        )];
        for i in 1..=steps {
            let k = i as f32 / steps as f32;
            let at = [
                from[0] + (to[0] - from[0]) * k,
                from[1] + (to[1] - from[1]) * k,
            ];
            out.push(v.touch(
                id,
                TouchPhase::Moved,
                at[0],
                at[1],
                t,
                screen,
                rect,
                s,
                input,
            ));
        }
        out.push(v.touch(
            id,
            TouchPhase::Ended,
            to[0],
            to[1],
            t,
            screen,
            rect,
            s,
            input,
        ));
        out
    }

    /// Two fingers down and up at once, without moving, at `at`.
    fn two_finger_tap(v: &mut TermView, at: Duration, sc: &Screen, rect: Rect, s: f32) -> bool {
        let mut moved = false;
        for (id, x) in [(10, 300.0), (11, 400.0)] {
            v.touch(
                id,
                TouchPhase::Started,
                x,
                300.0,
                at,
                Some(sc),
                rect,
                s,
                true,
            );
        }
        for (id, x) in [(10, 300.0), (11, 400.0)] {
            let t = v.touch(id, TouchPhase::Ended, x, 300.0, at, Some(sc), rect, s, true);
            assert!(!t.tap, "two fingers are not a tap");
            moved |= t.moved;
        }
        moved
    }

    fn tall() -> Screen {
        let rows: String = (0..60)
            .map(|i| format!("{i:>3} {}\n", "x".repeat(96)))
            .collect();
        screen_of(&rows)
    }

    #[test]
    fn the_first_frame_lands_at_the_fit_width_bottom_and_pans_clamp() {
        let rect = Rect::new(0.0, 100.0, 800.0, 600.0);
        let s = 2.0;
        let sc = tall();
        assert_eq!(sc.cols(), 100);
        let mut v = TermView::default();
        assert_eq!(v.pt(), START_PT);
        assert!(v.follow(Some(&sc), rect, s, false));
        // 100 columns into 800 px at scale 2: 800 / (100 * 0.6 * 2).
        assert!((v.pt() - 800.0 / 120.0).abs() < 1e-3 || v.pt() == MIN_PT);
        assert!(v.fit());
        let m = v.max_pan(Some(&sc), rect, s);
        assert_eq!(v.pan(), [v.pan()[0], m[1]], "at the bottom rows");
        // Without input, a drag right past the left edge and down pans
        // both ways, stopping at the edge.
        let out = drag(
            &mut v,
            1,
            [400.0, 400.0],
            [600.0, 900.0],
            1,
            Some(&sc),
            rect,
            s,
            false,
        );
        assert_eq!(v.pan(), [0.0, (m[1] - 500.0).max(0.0)]);
        assert_eq!(v.axis(), Some(Axis::Free));
        assert!(out.iter().all(|t| !t.tap && !t.drag && t.lines == 0));
        // The two-finger double tap goes back to the size before, and
        // again to fit.
        let t = Duration::from_secs(1);
        assert!(!two_finger_tap(&mut v, t, &sc, rect, s));
        assert!(two_finger_tap(
            &mut v,
            t + Duration::from_millis(150),
            &sc,
            rect,
            s
        ));
        assert!(!v.fit());
        assert_eq!(v.pt(), START_PT);
        let t = t + Duration::from_secs(1);
        two_finger_tap(&mut v, t, &sc, rect, s);
        // Too slow: not a double tap.
        assert!(!two_finger_tap(
            &mut v,
            t + Duration::from_millis(900),
            &sc,
            rect,
            s
        ));
        assert!(two_finger_tap(
            &mut v,
            t + Duration::from_millis(1000),
            &sc,
            rect,
            s
        ));
        assert!(v.fit());
        assert_eq!(v.pan()[0], 0.0);
    }

    #[test]
    fn a_tap_arms_and_raises_the_keyboard_only_with_input() {
        let rect = Rect::new(0.0, 0.0, 800.0, 1200.0);
        let sc = tall();
        let mut v = TermView::default();
        v.follow(Some(&sc), rect, 2.0, false);
        let tap = |v: &mut TermView, id: u64, input: bool, at: Duration| {
            v.touch(
                id,
                TouchPhase::Started,
                300.0,
                300.0,
                at,
                Some(&sc),
                rect,
                2.0,
                input,
            );
            v.touch(
                id,
                TouchPhase::Ended,
                302.0,
                301.0,
                at,
                Some(&sc),
                rect,
                2.0,
                input,
            )
        };
        assert!(tap(&mut v, 1, true, Duration::ZERO).tap);
        // At once, again: each tap acts, no double tap waits.
        assert!(tap(&mut v, 2, true, Duration::from_millis(100)).tap);
        assert!(v.fit(), "a one-finger double tap no longer toggles fit");
        assert_eq!(
            tap(&mut v, 3, false, Duration::from_secs(1)),
            Touched::default()
        );
        // A cancelled touch is no tap.
        v.touch(
            4,
            TouchPhase::Started,
            300.0,
            300.0,
            Duration::ZERO,
            Some(&sc),
            rect,
            2.0,
            true,
        );
        let t = v.touch(
            4,
            TouchPhase::Cancelled,
            300.0,
            300.0,
            Duration::ZERO,
            Some(&sc),
            rect,
            2.0,
            true,
        );
        assert!(!t.tap);
    }

    #[test]
    fn a_vertical_drag_is_the_wheel_in_lines_of_the_row_pitch() {
        let rect = Rect::new(0.0, 0.0, 800.0, 600.0);
        let s = 2.0;
        let sc = tall();
        let mut v = TermView::default();
        v.follow(Some(&sc), rect, s, false);
        let pan = v.pan();
        let pitch = term_h(v.pt() * s);
        // Down 10.5 rows (past the slop first): content follows the finger
        // back into the scrollback, positive.
        let from = [400.0, 100.0];
        let slop = SLOP * s + 1.0;
        let out = drag(
            &mut v,
            1,
            from,
            [from[0] + 3.0, from[1] + slop + 10.5 * pitch],
            40,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(v.axis(), Some(Axis::Wheel));
        assert_eq!(out.iter().filter(|t| t.drag).count(), 1, "arms once");
        let lines: i32 = out.iter().map(|t| t.lines).sum();
        // The slop's own distance is not scrolled: 10 whole rows.
        assert_eq!(lines, 10);
        assert_eq!(v.pan(), pan, "the frame does not move");
        assert!(out.iter().all(|t| !t.tap));
        // Up: negative, towards the bottom.
        let out = drag(
            &mut v,
            2,
            [400.0, 500.0],
            [400.0, 500.0 - slop - 4.0 * pitch],
            8,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(out.iter().map(|t| t.lines).sum::<i32>(), -4);
        // A remainder is not carried into the next drag.
        v.touch(
            3,
            TouchPhase::Started,
            400.0,
            100.0,
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(v.wheel, 0.0);
        assert_eq!(v.axis(), None);
    }

    #[test]
    fn the_axis_locks_to_the_first_direction() {
        let rect = Rect::new(0.0, 0.0, 400.0, 600.0);
        let s = 2.0;
        let sc = tall();
        let mut v = TermView::default();
        v.follow(Some(&sc), rect, s, false);
        // Zoomed in, so the frame is wider than the view.
        v.pt = MAX_PT;
        v.pan = [100.0, 100.0];
        let pitch = term_h(v.pt() * s);
        // Across first, then down: only across, and no lines.
        let z = Duration::ZERO;
        let mut out = vec![v.touch(
            1,
            TouchPhase::Started,
            300.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        )];
        out.push(v.touch(
            1,
            TouchPhase::Moved,
            270.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        ));
        out.push(v.touch(
            1,
            TouchPhase::Moved,
            200.0,
            300.0 + 6.0 * pitch,
            z,
            Some(&sc),
            rect,
            s,
            true,
        ));
        out.push(v.touch(
            1,
            TouchPhase::Ended,
            200.0,
            300.0 + 6.0 * pitch,
            z,
            Some(&sc),
            rect,
            s,
            true,
        ));
        assert_eq!(v.axis(), Some(Axis::Across));
        assert!(out.iter().all(|t| t.lines == 0 && !t.drag && !t.tap));
        assert_eq!(v.pan(), [200.0, 100.0]);
        // Down first, then across: lines, and the frame stays.
        let pan = v.pan();
        let out = drag(
            &mut v,
            2,
            [300.0, 300.0],
            [300.0, 300.0 + 30.0],
            1,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(v.axis(), Some(Axis::Wheel));
        assert!(out.iter().any(|t| t.drag));
        let t = v.touch(
            2,
            TouchPhase::Started,
            300.0,
            300.0,
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(t, Touched::default());
        let t = v.touch(
            2,
            TouchPhase::Moved,
            300.0,
            330.0,
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert!(t.drag);
        let t = v.touch(
            2,
            TouchPhase::Moved,
            100.0,
            330.0 + 2.0 * pitch,
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(t.lines, 2);
        assert_eq!(v.pan(), pan, "still the wheel");
    }

    #[test]
    fn wheel_lines_keep_the_remainder_and_the_sign() {
        let mut d = 25.0;
        assert_eq!(wheel_lines(&mut d, 10.0), 2);
        assert!((d - 5.0).abs() < 1e-4);
        d -= 31.0;
        assert_eq!(wheel_lines(&mut d, 10.0), -2);
        assert!((d + 6.0).abs() < 1e-4);
        assert_eq!(wheel_lines(&mut d, 0.0), 0);
    }

    #[test]
    fn a_pinch_zooms_within_the_limits() {
        let rect = Rect::new(0.0, 0.0, 800.0, 1200.0);
        let sc = screen_of(&"y".repeat(200));
        let mut v = TermView::default();
        v.follow(Some(&sc), rect, 2.0, false);
        let t = Duration::ZERO;
        let touch = |v: &mut TermView, id: u64, phase: TouchPhase, x: f32| {
            v.touch(id, phase, x, 500.0, t, Some(&sc), rect, 2.0, true)
        };
        touch(&mut v, 1, TouchPhase::Started, 300.0);
        touch(&mut v, 2, TouchPhase::Started, 500.0);
        // Spread to five times: capped at 16 pt.
        touch(&mut v, 2, TouchPhase::Moved, 1300.0);
        assert_eq!(v.pt(), MAX_PT);
        assert!(!v.fit());
        // Pinched in: down to 7 pt.
        let out = touch(&mut v, 2, TouchPhase::Moved, 301.0);
        assert_eq!(v.pt(), MIN_PT);
        assert_eq!(out.lines, 0, "a pinch never scrolls the pane");
        // Not a tap, nor a two-finger tap.
        assert_eq!(
            touch(&mut v, 2, TouchPhase::Ended, 301.0),
            Touched::default()
        );
        assert_eq!(
            touch(&mut v, 1, TouchPhase::Ended, 300.0),
            Touched::default()
        );
        assert_eq!(v.last_tap, None);
    }

    #[test]
    fn the_scroll_pill_shows_only_while_scrolled_back() {
        let mut st = TermState::default();
        assert_eq!(scroll_label(&st), None);
        let mut s = status(true);
        s.scroll_offset = Some(0);
        st.status = Some(s.clone());
        assert_eq!(scroll_label(&st), None);
        s.scroll_offset = Some(12);
        st.status = Some(s.clone());
        assert_eq!(scroll_label(&st).as_deref(), Some("12 lines back"));
        s.scroll_offset = Some(1);
        st.status = Some(s.clone());
        assert_eq!(scroll_label(&st).as_deref(), Some("1 line back"));
        s.open = false;
        st.status = Some(s);
        assert_eq!(scroll_label(&st), None);
    }

    #[test]
    fn hints_are_what_fits_at_the_starting_size() {
        let (c, r) = TermView::hints(Rect::new(0.0, 0.0, 1080.0, 1500.0), 2.625);
        // 1080 / (0.6 * 11 * 2.625) = 62.3; 1500 / (1.3 * 11 * 2.625) = 39.9.
        assert_eq!((c, r), (Some(62), Some(39)));
        assert_eq!(TermView::hints(Rect::default(), 2.0), (None, None));
    }

    // Plan 20261027_08, step 9.

    fn cursor(col: u16, row: u16, shape: u8) -> Cursor {
        Cursor {
            col,
            row,
            visible: true,
            shape,
            mouse_reporting: false,
            alternate_screen: false,
            stale: false,
        }
    }

    /// `ansi` as a frame carrying `c`.
    fn screen_with(ansi: &str, c: Option<Cursor>) -> Screen {
        let mut grid = parse_ansi(ansi);
        grid.cursor = c;
        let mut sc = Screen::default();
        assert!(sc.apply(&Frame::full(1, 1, &grid)));
        sc
    }

    #[test]
    fn the_fit_is_whole_cells_of_the_rect_at_the_size() {
        let s = 2.625;
        let rect = Rect::new(0.0, 0.0, 1080.0, 1500.0);
        assert_eq!(TermView::fit_at(rect, START_PT, s), Some((62, 39)));
        assert_eq!(
            TermView::hints(rect, s),
            (Some(62), Some(39)),
            "the hints are the fit at the starting size"
        );
        // Bigger type, fewer cells.
        let (c, r) = TermView::fit_at(rect, 14.0, s).unwrap();
        assert!(c < 62 && r < 39);
        // A width that is exactly 80 cells is 80, not 79 from rounding.
        let px = START_PT * s;
        let exact = Rect::new(0.0, 0.0, 80.0 * CELL_EM * px, 24.0 * term_h(px));
        assert_eq!(TermView::fit_at(exact, START_PT, s), Some((80, 24)));
        assert_eq!(TermView::fit_at(Rect::default(), START_PT, s), None);
        // A tiny rect is still one cell.
        assert_eq!(
            TermView::fit_at(Rect::new(0.0, 0.0, 1.0, 1.0), START_PT, s),
            Some((1, 1))
        );
        let v = TermView::default();
        assert_eq!(v.fit_size(rect, s), Some((62, 39)));
    }

    #[test]
    fn herdrs_cursor_shapes_draw_as_block_underline_and_bar() {
        use CursorShape::*;
        let want = [Block, Block, Block, Underline, Underline, Bar, Bar, Block];
        for (shape, w) in want.iter().enumerate() {
            assert_eq!(cursor_shape(shape as u8), *w, "shape {shape}");
        }
        assert_eq!(cursor_shape(200), Block);
        let (cell, lh) = (12.0, 26.0);
        let origin = [100.0, 50.0];
        let (shape, r) = cursor_mark(Some(cursor(3, 2, 2)), origin, cell, lh).unwrap();
        assert_eq!(shape, Block);
        assert_eq!(r, Rect::new(136.0, 102.0, 12.0, 26.0));
        let (shape, r) = cursor_mark(Some(cursor(3, 2, 4)), origin, cell, lh).unwrap();
        assert_eq!(shape, Underline);
        assert_eq!((r.x, r.w), (136.0, 12.0));
        assert!(
            r.h < 4.0 && (r.y + r.h - 128.0).abs() < 1e-3,
            "at the cell's foot"
        );
        let (shape, r) = cursor_mark(Some(cursor(3, 2, 6)), origin, cell, lh).unwrap();
        assert_eq!(shape, Bar);
        assert_eq!((r.x, r.y, r.h), (136.0, 102.0, 26.0));
        assert!(r.w < 3.0, "a thin bar at the left edge");
        // Hidden: by the program, or while the desktop has the tab.
        let hidden = Cursor {
            visible: false,
            ..cursor(3, 2, 2)
        };
        assert_eq!(cursor_mark(Some(hidden), origin, cell, lh), None);
        let stale = Cursor {
            stale: true,
            ..cursor(3, 2, 2)
        };
        assert_eq!(cursor_mark(Some(stale), origin, cell, lh), None);
        assert_eq!(cursor_mark(None, origin, cell, lh), None);
    }

    #[test]
    fn the_character_under_the_cursor_is_found_across_runs() {
        let grid = parse_ansi("ab\x1b[31mcd\x1b[0mef");
        let row = &grid.lines[0];
        assert_eq!(char_at(row, 0).map(|(c, _)| c), Some('a'));
        let (c, run) = char_at(row, 3).unwrap();
        assert_eq!(c, 'd');
        assert_eq!(run.fg, Color::palette(1));
        assert_eq!(char_at(row, 5).map(|(c, _)| c), Some('f'));
        assert_eq!(char_at(row, 6), None);
    }

    /// The `SdfKind::Box` fills in `list` exactly at `r`.
    fn fills_at(list: &DrawList, r: Rect) -> Vec<[f32; 4]> {
        list.instances
            .iter()
            .filter(|i| matches!(i.kind, libmsdf::drawlist::SdfKind::Box))
            .filter(|i| i.position == r.pos() && i.size == r.size())
            .map(|i| i.color)
            .collect()
    }

    #[test]
    fn the_cursor_is_drawn_by_its_shape_and_not_when_hidden_or_stale() {
        let rect = Rect::new(0.0, 0.0, 800.0, 600.0);
        let s = 2.0;
        let draw = |c: Option<Cursor>| {
            let mut v = TermView::default();
            let state = TermState {
                supported: true,
                screen: Some(screen_with("hello\n\x1b[31mred\x1b[0m", c)),
                status: Some(status(true)),
                ..Default::default()
            };
            v.follow(state.screen.as_ref(), rect, s, true);
            let mut list = DrawList::new();
            v.draw(&mut list, &DrawContext::default(), None, &state, rect, s);
            let px = v.pt() * s;
            let mark = cursor_mark(
                c,
                [rect.x - v.pan()[0], rect.y - v.pan()[1]],
                CELL_EM * px,
                term_h(px),
            );
            (list, mark)
        };
        // A block over the red 'e' of row 1, col 1: filled in the cell's
        // own colour (reverse video).
        let (list, mark) = draw(Some(cursor(1, 1, 2)));
        let (shape, r) = mark.unwrap();
        assert_eq!(shape, CursorShape::Block);
        assert_eq!(fills_at(&list, r), vec![palette_colour(1, dark_theme())]);
        // The default shape, over a blank: the theme's text colour.
        let (list, mark) = draw(Some(cursor(9, 0, 0)));
        assert_eq!(fills_at(&list, mark.unwrap().1), vec![theme().text]);
        // A bar and an underline.
        for shape in [4, 6] {
            let (list, mark) = draw(Some(cursor(0, 0, shape)));
            let (_, r) = mark.unwrap();
            assert_eq!(fills_at(&list, r).len(), 1, "shape {shape}");
        }
        // Hidden or stale: no mark is drawn at the cell.
        let px = START_PT * s;
        let cell = Rect::new(CELL_EM * px, term_h(px), CELL_EM * px, term_h(px));
        for c in [
            Cursor {
                visible: false,
                ..cursor(1, 1, 2)
            },
            Cursor {
                stale: true,
                ..cursor(1, 1, 2)
            },
        ] {
            let (list, mark) = draw(Some(c));
            assert_eq!(mark, None);
            assert!(fills_at(&list, cell).is_empty());
        }
    }

    #[test]
    fn the_stale_pill_shows_while_the_desktop_has_the_tab() {
        let mut st = TermState {
            supported: true,
            status: Some(status(true)),
            screen: Some(screen_with("hello", Some(cursor(0, 0, 0)))),
            ..Default::default()
        };
        assert_eq!(stale_label(&st), None);
        assert!(!st.stale());
        let stale = Cursor {
            stale: true,
            ..cursor(0, 0, 0)
        };
        st.screen = Some(screen_with("hello", Some(stale)));
        assert!(st.stale());
        assert_eq!(
            stale_label(&st),
            Some("Desktop has the tab \u{2014} touch to take it back")
        );
        // Frames keep showing: no status line hides them.
        assert_eq!(status_line(&st), None);
        // Drawn in the warning colour.
        let rect = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut v = TermView::default();
        v.follow(st.screen.as_ref(), rect, 2.0, true);
        let mut list = DrawList::new();
        v.draw(&mut list, &DrawContext::default(), None, &st, rect, 2.0);
        let warn = with_alpha(theme().warn, 0.92);
        assert!(list.instances.iter().any(|i| i.color == warn));
        // Without a cursor (the poll): never stale.
        st.screen = Some(screen_of("hello"));
        assert_eq!(stale_label(&st), None);
    }

    #[test]
    fn an_attached_frame_keeps_the_size_and_does_not_pan_when_it_fits() {
        let rect = Rect::new(0.0, 100.0, 800.0, 600.0);
        let s = 2.0;
        let mut v = TermView::default();
        let (cols, rows) = v.fit_size(rect, s).unwrap();
        let text: String = (0..rows)
            .map(|i| format!("{}\n", "x".repeat(cols as usize - 1 - (i as usize % 3))))
            .collect();
        let sc = screen_with(&text, Some(cursor(0, 0, 0)));
        assert!(sc.cols() <= cols && sc.rows() <= rows);
        v.follow(Some(&sc), rect, s, true);
        assert_eq!(v.pt(), START_PT, "not fitted to the frame's width");
        assert_eq!(v.pan(), [0.0, 0.0]);
        assert_eq!(v.max_pan(Some(&sc), rect, s), [0.0, 0.0]);
        // A larger frame (the desktop's size) still pans.
        let wide = tall();
        let mut v = TermView::default();
        v.follow(Some(&wide), rect, s, true);
        assert_eq!(v.pt(), START_PT);
        let m = v.max_pan(Some(&wide), rect, s);
        assert!(m[0] > 0.0 && m[1] > 0.0);
        assert_eq!(v.pan(), [0.0, m[1]], "at the bottom rows");
    }

    #[test]
    fn a_touch_maps_to_its_cell_at_the_zoom_and_pan() {
        let rect = Rect::new(10.0, 100.0, 400.0, 600.0);
        let s = 2.0;
        let sc = tall();
        let mut v = TermView {
            pt: 10.0,
            pan: [30.0, 45.0],
            ..Default::default()
        };
        let px = 10.0 * s;
        let (cw, lh) = (CELL_EM * px, term_h(px));
        // The grid point (30 + 5.5 cells, 45 + 3.5 rows) from the rect.
        let x = rect.x - 30.0 + 7.5 * cw;
        let y = rect.y - 45.0 + 4.5 * lh;
        assert_eq!(v.cell_at(x, y, Some(&sc), rect, s), Some((7, 4)));
        // Zoomed in: the same point is an earlier cell.
        v.pt = 16.0;
        let (c, r) = v.cell_at(x, y, Some(&sc), rect, s).unwrap();
        assert!(c < 7 && r < 4);
        // Clamped to the frame.
        assert_eq!(v.cell_at(-500.0, -500.0, Some(&sc), rect, s), Some((0, 0)));
        assert_eq!(
            v.cell_at(1e6, 1e6, Some(&sc), rect, s),
            Some((sc.cols() - 1, sc.rows() - 1))
        );
        assert_eq!(v.cell_at(x, y, None, rect, s), None);
    }

    fn left(kind: fn(MouseButton) -> TerminalMouseKind, col: u16, row: u16) -> MouseAt {
        MouseAt::left(kind, (col, row))
    }

    #[test]
    fn a_tap_clicks_when_the_program_reports_the_mouse_and_types_when_not() {
        let rect = Rect::new(0.0, 0.0, 800.0, 1200.0);
        let s = 2.0;
        let sc = tall();
        let mut v = TermView::default();
        v.follow(Some(&sc), rect, s, true);
        let tap = |v: &mut TermView, id: u64, at: [f32; 2]| {
            v.touch(
                id,
                TouchPhase::Started,
                at[0],
                at[1],
                Duration::ZERO,
                Some(&sc),
                rect,
                s,
                true,
            );
            v.touch(
                id,
                TouchPhase::Ended,
                at[0],
                at[1],
                Duration::ZERO,
                Some(&sc),
                rect,
                s,
                true,
            )
        };
        let at = [200.0, 300.0];
        let cell = v.cell_at(at[0], at[1], Some(&sc), rect, s).unwrap();
        // Without mouse reports: the keyboard tap of before.
        let t = tap(&mut v, 1, at);
        assert!(t.tap && !t.click && t.mouse.is_empty());
        // With them: a left click at the cell, no keyboard.
        v.set_mouse_mode(true);
        let t = tap(&mut v, 2, at);
        assert!(!t.tap && t.click);
        assert_eq!(
            t.mouse,
            vec![left(down, cell.0, cell.1), left(up, cell.0, cell.1)]
        );
        // Without input: nothing, mouse or not.
        v.touch(
            3,
            TouchPhase::Started,
            at[0],
            at[1],
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            false,
        );
        let t = v.touch(
            3,
            TouchPhase::Ended,
            at[0],
            at[1],
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            false,
        );
        assert_eq!(t, Touched::default());
        // The mode a gesture started with holds to its end.
        v.touch(
            4,
            TouchPhase::Started,
            at[0],
            at[1],
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            true,
        );
        v.set_mouse_mode(false);
        let t = v.touch(
            4,
            TouchPhase::Ended,
            at[0],
            at[1],
            Duration::ZERO,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert!(t.click);
    }

    #[test]
    fn in_mouse_mode_a_drag_across_drags_the_button_and_a_vertical_one_is_the_wheel() {
        let rect = Rect::new(0.0, 0.0, 800.0, 1200.0);
        let s = 2.0;
        let sc = tall();
        let mut v = TermView::default();
        v.follow(Some(&sc), rect, s, true);
        v.set_mouse_mode(true);
        let pan = v.pan();
        let px = v.pt() * s;
        let cw = CELL_EM * px;
        let from = [100.0, 300.0];
        let to = [100.0 + 6.0 * cw + SLOP * s, 300.0];
        let start = v.cell_at(from[0], from[1], Some(&sc), rect, s).unwrap();
        let end = v.cell_at(to[0], to[1], Some(&sc), rect, s).unwrap();
        let out = drag(&mut v, 1, from, to, 12, Some(&sc), rect, s, true);
        assert_eq!(v.axis(), Some(Axis::Mouse));
        assert_eq!(out.iter().filter(|t| t.drag).count(), 1, "arms once");
        let events: Vec<MouseAt> = out.iter().flat_map(|t| t.mouse.clone()).collect();
        assert_eq!(events.first(), Some(&left(down, start.0, start.1)));
        assert_eq!(events.last(), Some(&left(up, end.0, end.1)));
        let drags: Vec<&MouseAt> = events[1..events.len() - 1].iter().collect();
        assert!(!drags.is_empty());
        assert!(
            drags
                .iter()
                .all(|m| matches!(m.kind, TerminalMouseKind::Drag { .. }))
        );
        // One per new cell, in order.
        assert!(drags.windows(2).all(|w| w[0].col < w[1].col));
        assert_eq!(drags.last().map(|m| (m.col, m.row)), Some(end));
        assert!(out.iter().all(|t| !t.tap && !t.click && t.lines == 0));
        assert_eq!(v.pan(), pan, "the frame does not pan");
        // Down first: the wheel, as without mouse reports.
        let pitch = term_h(px);
        let out = drag(
            &mut v,
            2,
            [400.0, 100.0],
            [400.0, 100.0 + SLOP * s + 1.0 + 4.0 * pitch],
            10,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert_eq!(v.axis(), Some(Axis::Wheel));
        assert_eq!(out.iter().map(|t| t.lines).sum::<i32>(), 4);
        assert!(out.iter().all(|t| t.mouse.is_empty()));
        // A second finger during a mouse drag lets the button go.
        let z = Duration::ZERO;
        v.touch(
            5,
            TouchPhase::Started,
            100.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        );
        v.touch(
            5,
            TouchPhase::Moved,
            160.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        );
        let t = v.touch(
            6,
            TouchPhase::Started,
            400.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert!(matches!(
            t.mouse.as_slice(),
            [MouseAt {
                kind: TerminalMouseKind::Up { .. },
                ..
            }]
        ));
        v.touch(
            6,
            TouchPhase::Ended,
            400.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        );
        let t = v.touch(
            5,
            TouchPhase::Ended,
            160.0,
            300.0,
            z,
            Some(&sc),
            rect,
            s,
            true,
        );
        assert!(t.mouse.is_empty(), "let go once");
    }

    #[test]
    fn mouse_mode_needs_the_daemons_mouse_and_the_programs_reports() {
        let reporting = Cursor {
            mouse_reporting: true,
            ..cursor(0, 0, 0)
        };
        let mut st = TermState {
            supported: true,
            input_allowed: true,
            mouse_allowed: true,
            status: Some(status(true)),
            screen: Some(screen_with("htop", Some(reporting))),
        };
        assert!(st.mouse_mode());
        st.mouse_allowed = false;
        assert!(!st.mouse_mode());
        st.mouse_allowed = true;
        st.input_allowed = false;
        assert!(!st.mouse_mode());
        st.input_allowed = true;
        st.screen = Some(screen_with("bash", Some(cursor(0, 0, 0))));
        assert!(!st.mouse_mode());
        st.screen = Some(screen_of("polled"));
        assert!(!st.mouse_mode());
    }
}
