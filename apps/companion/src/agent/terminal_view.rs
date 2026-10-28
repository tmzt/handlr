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

use std::time::Duration;

use flankr_companion::{Screen, TerminalClosedReason, TerminalStatus};
use libcontrols::{DrawContext, Rect};
use libflankr_term::{Color, Run, flags};
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
/// The armed frame's width and its label's type size, in points.
const ARMED_EDGE: f32 = 3.0;
const ARMED_PT: f32 = 13.0;

/// The terminal as the app last read it from the client.
#[derive(Clone, Debug, Default)]
pub struct TermState {
    /// The daemon offers the terminal mode on this connection.
    pub supported: bool,
    /// The daemon lets keys be sent (its `[terminal] input`).
    pub input_allowed: bool,
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

    /// The armed input's seconds left, rounded up.
    pub fn armed_secs(&self) -> Option<u64> {
        self.status
            .as_ref()
            .filter(|s| s.open)
            .and_then(|s| s.armed_for)
            .map(|d| d.as_millis().div_ceil(1000) as u64)
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    /// The view moved (a pan or a zoom): draw again.
    pub moved: bool,
    /// A tap, with input allowed: arm, and raise the soft keyboard.
    pub tap: bool,
    /// A vertical drag began, with input allowed: arm (no keyboard).
    pub drag: bool,
    /// Lines to scroll the pane by, positive back into the scrollback.
    pub lines: i32,
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
    /// for `terminal.open`.
    pub fn hints(rect: Rect, s: f32) -> (Option<u16>, Option<u16>) {
        let px = START_PT * s;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return (None, None);
        }
        let cols = (rect.w / (CELL_EM * px)).floor() as u16;
        let rows = (rect.h / term_h(px)).floor() as u16;
        (Some(cols.max(1)), Some(rows.max(1)))
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

    /// A new screen or size: the first frame is placed at the fit-width
    /// size and its bottom rows; afterwards the pan is kept, within the
    /// frame. Returns whether anything moved.
    pub fn follow(&mut self, screen: Option<&Screen>, rect: Rect, s: f32) -> bool {
        let before = (self.pt, self.pan);
        if !self.placed
            && let Some(sc) = screen.filter(|sc| sc.cols() > 0)
        {
            self.placed = true;
            self.before_fit = self.pt;
            self.pt = Self::fit_pt(sc.cols(), rect.w, s);
            self.fit = true;
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
                    let axis = *self.axis.get_or_insert_with(|| {
                        if !input {
                            Axis::Free
                        } else if dy.abs() > dx.abs() {
                            Axis::Wheel
                        } else {
                            Axis::Across
                        }
                    });
                    // The wheel's first move: its arm.
                    out.drag = first && axis == Axis::Wheel;
                    match axis {
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
                let tap = phase == TouchPhase::Ended && self.fingers.is_empty() && !self.moved;
                if !tap {
                    return out;
                }
                if !self.multi {
                    out.tap = input;
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
        agent: &str,
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
        // Armed: a coloured frame, and what the keys go into.
        if let Some(secs) = state.armed_secs() {
            let c = theme().warn;
            let e = ARMED_EDGE * s;
            for r in [
                Rect::new(rect.x, rect.y, rect.w, e),
                Rect::new(rect.x, rect.y + rect.h - e, rect.w, e),
                Rect::new(rect.x, rect.y, e, rect.h),
                Rect::new(rect.x + rect.w - e, rect.y, e, rect.h),
            ] {
                fill(list, r, c, 0.0);
            }
            let px = ARMED_PT * s;
            let label = format!("Typing into {agent} \u{00B7} {secs} s");
            let w = label.chars().count() as f32 * px * 0.5 + 16.0 * s;
            let h = line_h(px) + 6.0 * s;
            let pill = Rect::new(rect.x + rect.w - w - 8.0 * s, rect.y + 8.0 * s, w, h);
            fill(list, pill, with_alpha(c, 0.92), h * 0.5);
            ctx.draw_text(
                list,
                &label,
                [pill.x + 8.0 * s, pill.y + 3.0 * s],
                px,
                theme().bg,
            );
        }
        // Scrolled back: how far, at the top left.
        if let Some(label) = scroll_label(state) {
            let px = ARMED_PT * s;
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
    fn armed_seconds_round_up_and_need_an_open_terminal() {
        let mut st = TermState::default();
        let mut s = status(true);
        s.armed_for = Some(Duration::from_millis(41_200));
        st.status = Some(s.clone());
        assert!(st.armed());
        assert_eq!(st.armed_secs(), Some(42));
        s.open = false;
        st.status = Some(s);
        assert!(!st.armed());
        assert_eq!(st.armed_secs(), None);
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
        assert!(v.follow(Some(&sc), rect, s));
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
        v.follow(Some(&sc), rect, 2.0);
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
        v.follow(Some(&sc), rect, s);
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
        v.follow(Some(&sc), rect, s);
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
        v.follow(Some(&sc), rect, 2.0);
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
}
