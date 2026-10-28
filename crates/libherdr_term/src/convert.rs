//! Conversions between herdr's types and flankr's, `From`/`Into`
//! (decision 8). They live here, beside the generated herdr types, so
//! `libflankr_term` and `libflankr_proto` never see herdr.

use libflankr_proto::common::{
    MouseButton, TerminalMouseKind, TerminalMouseParams, TerminalScrollParams,
};
use libflankr_term::{Color, Cursor, Grid, Row, Run, flags};

use crate::herdr::{self, ClientKeyCode, ClientKeyKind, ClientMousePosition, ClientPaneInputEvent};
use crate::surface::PaneView;

/// A `CellData.fg`/`bg` value: herdr's packed colour (findings 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color32(pub u32);

/// A `CellData.modifier` value: ratatui's `Modifier` bits, the underline
/// style in bits 12-15 (findings 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modifier16(pub u16);

impl Modifier16 {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 2;
    pub const ITALIC: u16 = 4;
    pub const UNDERLINED: u16 = 8;
    pub const SLOW_BLINK: u16 = 16;
    pub const RAPID_BLINK: u16 = 32;
    pub const REVERSED: u16 = 64;
    pub const HIDDEN: u16 = 128;
    pub const CROSSED_OUT: u16 = 256;
    /// herdr's underline style (`UNDERLINE_STYLE_MASK`).
    pub const UNDERLINE_STYLE_MASK: u16 = 0xF000;

    /// HIDDEN: the run's text is drawn as spaces.
    pub fn hidden(self) -> bool {
        self.0 & Self::HIDDEN != 0
    }
}

impl From<Color32> for Color {
    /// `0` is the default; `0x00_0000NN`, `NN` in 1-16, the named colours
    /// in ANSI order (palette `NN - 1`); `0x01_0000NN` palette `NN`;
    /// `0x02RRGGBB` RGB; anything else the default.
    fn from(c: Color32) -> Color {
        let v = c.0;
        let low = (v & 0xFF) as u8;
        match v >> 24 {
            0x00 if v & 0x00FF_FF00 == 0 && (1..=16).contains(&low) => Color::palette(low - 1),
            0x01 if v & 0x00FF_FF00 == 0 => Color::palette(low),
            0x02 => Color::rgb((v >> 16) as u8, (v >> 8) as u8, low),
            _ => Color::DEFAULT,
        }
    }
}

impl From<Modifier16> for u8 {
    /// BOLD, DIM, ITALIC, UNDERLINED (or any underline style), REVERSED and
    /// CROSSED_OUT to [`libflankr_term::flags`]; blink and HIDDEN dropped
    /// (HIDDEN blanks the text instead, in the `Grid` conversion).
    fn from(m: Modifier16) -> u8 {
        let m = m.0;
        let mut out = 0;
        if m & Modifier16::BOLD != 0 {
            out |= flags::BOLD;
        }
        if m & Modifier16::DIM != 0 {
            out |= flags::DIM;
        }
        if m & Modifier16::ITALIC != 0 {
            out |= flags::ITALIC;
        }
        if m & Modifier16::UNDERLINED != 0 || m & Modifier16::UNDERLINE_STYLE_MASK != 0 {
            out |= flags::UNDERLINE;
        }
        if m & Modifier16::REVERSED != 0 {
            out |= flags::INVERSE;
        }
        if m & Modifier16::CROSSED_OUT != 0 {
            out |= flags::STRIKETHROUGH;
        }
        out
    }
}

/// Spaces with the default background and no underline, inverse or
/// strikethrough draw nothing, as `parse_ansi` cuts them.
fn blank_style(run: &Run) -> bool {
    run.bg.is_default()
        && run.flags & (flags::UNDERLINE | flags::INVERSE | flags::STRIKETHROUGH) == 0
}

/// One row of herdr cells as `parse_ansi` would make it: runs of one style,
/// `skip` cells (a wide glyph's second half) emit nothing, adjacent equal
/// styles merge, trailing blank spaces are cut.
pub fn row_from_cells(cells: &[herdr::CellData]) -> Row {
    let mut row: Row = Vec::new();
    for cell in cells {
        if cell.skip {
            continue;
        }
        let modifier = Modifier16(cell.modifier);
        let symbol = if cell.symbol.is_empty() {
            " "
        } else {
            cell.symbol.as_str()
        };
        let text = if modifier.hidden() {
            " ".repeat(symbol.chars().count())
        } else {
            symbol.to_owned()
        };
        let fg = Color::from(Color32(cell.fg));
        let bg = Color::from(Color32(cell.bg));
        let fl = u8::from(modifier);
        match row.last_mut() {
            Some(last) if last.fg == fg && last.bg == bg && last.flags == fl => {
                last.text.push_str(&text)
            }
            _ => row.push(Run {
                text,
                fg,
                bg,
                flags: fl,
            }),
        }
    }
    while let Some(last) = row.last_mut() {
        if !blank_style(last) {
            break;
        }
        let kept = last.text.trim_end_matches(' ').len();
        last.text.truncate(kept);
        if !last.text.is_empty() {
            break;
        }
        row.pop();
    }
    row
}

impl From<PaneView<'_>> for Grid {
    /// The pane's rows, `cols` and `rows` its `inner_rect` size, and the
    /// pane-local cursor with the pane's mouse and alternate-screen modes
    /// (decision 9; `stale` is the daemon's to set). A pane whose cursor is
    /// outside it has `cursor: None`.
    fn from(view: PaneView<'_>) -> Grid {
        let (cols, rows) = view.size();
        let cursor = view.cursor.clone().map(|c| Cursor {
            mouse_reporting: view.pane.mouse_reporting,
            alternate_screen: view.pane.alternate_screen_active,
            ..Cursor::from(c)
        });
        Grid {
            cols,
            rows,
            lines: view
                .cells
                .iter()
                .map(|cells| row_from_cells(cells))
                .collect(),
            cursor,
        }
    }
}

impl From<&PaneView<'_>> for Grid {
    fn from(view: &PaneView<'_>) -> Grid {
        Grid::from(view.clone())
    }
}

/// One key token as the daemon's `parse_keys` makes them
/// (`src/server/terminal.rs`): a named key, a `ctrl+`/`alt+` combo, or
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyToken {
    /// Text (`space` is `" "`), sent as one key per character: `Char(c)`
    /// with no modifiers, pressed and released, so herdr encodes it for the
    /// pane's keyboard protocol as it does a typed key (the owner's
    /// "ime keyboard input should be translated to keys", departing from
    /// decision 8's `TextCommit`, which herdr writes to the PTY as raw
    /// bytes). An uppercase letter is its own `Char`: herdr's key builder
    /// (`src/pane/input.rs`) takes it as Shift and the lowercase key.
    Text(String),
    /// `enter`, `escape`/`esc`, `tab`, `backspace` and the arrows, sent as
    /// a press and a release.
    Named(ClientKeyCode),
    /// `ctrl+<c>`, sent as `Char(c)` with Ctrl (modifiers 2), pressed and
    /// released.
    Ctrl(char),
    /// Any other combo `parse_keys` makes (the L3 amendment of 2026-10-28
    /// 10:42 EDT of plan 20261022_03): `alt+<printable>` (its case kept),
    /// `ctrl+alt+<a-z or \>`, and `ctrl+`, `alt+` or both before a
    /// [`KeyToken::Named`] key, with crossterm's modifier bits (Ctrl 2,
    /// Alt 4), pressed and released.
    Modified { code: ClientKeyCode, modifiers: u8 },
}

/// crossterm's Ctrl bit, as herdr's `modifiers` carry it.
const CTRL: u8 = 2;
/// crossterm's Alt bit, as herdr's `modifiers` carry it.
const ALT: u8 = 4;

fn named_key(lower: &str) -> Option<ClientKeyCode> {
    Some(match lower {
        "enter" => ClientKeyCode::Enter,
        "escape" | "esc" => ClientKeyCode::Esc,
        "tab" => ClientKeyCode::Tab,
        "backspace" => ClientKeyCode::Backspace,
        "up" => ClientKeyCode::Up,
        "down" => ClientKeyCode::Down,
        "left" => ClientKeyCode::Left,
        "right" => ClientKeyCode::Right,
        _ => return None,
    })
}

impl KeyToken {
    /// The token `token`, or `None` for an empty one, a control character
    /// or a combo `parse_keys` does not make.
    pub fn parse(token: &str) -> Option<KeyToken> {
        let lower = token.to_lowercase();
        if lower == "space" {
            return Some(KeyToken::Text(" ".into()));
        }
        if let Some(code) = named_key(&lower) {
            return Some(KeyToken::Named(code));
        }
        let mut modifiers = 0;
        let mut rest = token;
        loop {
            let l = rest.to_ascii_lowercase();
            let bit = if l.starts_with("ctrl+") {
                CTRL
            } else if l.starts_with("alt+") {
                ALT
            } else {
                break;
            };
            if modifiers & bit != 0 {
                return None;
            }
            modifiers |= bit;
            rest = &rest[l.find('+')? + 1..];
        }
        if modifiers == 0 {
            if token.is_empty() || token.chars().any(char::is_control) {
                return None;
            }
            return Some(KeyToken::Text(token.to_owned()));
        }
        if let Some(code) = named_key(&rest.to_lowercase()) {
            return Some(KeyToken::Modified { code, modifiers });
        }
        let mut chars = rest.chars();
        let c = match (chars.next(), chars.next()) {
            (Some(c), None) => c,
            _ => return None,
        };
        if modifiers & CTRL != 0 {
            let c = c.to_ascii_lowercase();
            if !(c.is_ascii_lowercase() || c == '\\') {
                return None;
            }
            return Some(if modifiers == CTRL {
                KeyToken::Ctrl(c)
            } else {
                KeyToken::Modified {
                    code: ClientKeyCode::Char(c),
                    modifiers,
                }
            });
        }
        if c.is_control() || c.is_whitespace() {
            return None;
        }
        Some(KeyToken::Modified {
            code: ClientKeyCode::Char(c),
            modifiers,
        })
    }
}

fn key(code: ClientKeyCode, modifiers: u8, kind: ClientKeyKind) -> ClientPaneInputEvent {
    ClientPaneInputEvent::Key {
        code,
        modifiers,
        kind,
        repeat_count: 1,
        shifted_codepoint: None,
        generated_text: None,
        tracks_release: true,
        physical_key_id: None,
        windows_record: None,
    }
}

impl From<KeyToken> for Vec<ClientPaneInputEvent> {
    /// Each key is a `Press` and then a `Release`, so herdr's held-key
    /// tracking ends clean: a named key or combo is one key, a text
    /// token one key per character. No `TextCommit` is made (it stays in
    /// herdr's types for a paste).
    fn from(token: KeyToken) -> Vec<ClientPaneInputEvent> {
        let press_release = |code: ClientKeyCode, modifiers: u8| {
            [
                key(code.clone(), modifiers, ClientKeyKind::Press),
                key(code, modifiers, ClientKeyKind::Release),
            ]
        };
        match token {
            KeyToken::Text(text) => text
                .chars()
                .flat_map(|c| press_release(ClientKeyCode::Char(c), 0))
                .collect(),
            KeyToken::Named(code) => press_release(code, 0).to_vec(),
            KeyToken::Ctrl(c) => press_release(ClientKeyCode::Char(c), CTRL).to_vec(),
            KeyToken::Modified { code, modifiers } => press_release(code, modifiers).to_vec(),
        }
    }
}

/// `parse_keys`' tokens as herdr input events, in order, every key a press
/// and a release. A token [`KeyToken::parse`] refuses is the error.
pub fn key_events<S: AsRef<str>>(tokens: &[S]) -> Result<Vec<ClientPaneInputEvent>, String> {
    let mut out: Vec<ClientPaneInputEvent> = Vec::new();
    for token in tokens {
        let token = token.as_ref();
        let parsed = KeyToken::parse(token).ok_or_else(|| token.to_owned())?;
        out.extend(Vec::<ClientPaneInputEvent>::from(parsed));
    }
    Ok(out)
}

/// The most wheel lines in one event.
const MAX_WHEEL_LINES: u32 = 1000;

impl From<TerminalScrollParams> for ClientPaneInputEvent {
    /// `lines > 0` (back into the scrollback) is `ScrollUp`, `lines <= 0`
    /// `ScrollDown`, with `lines.unsigned_abs()` clamped to 1-1000, at the
    /// pane's top-left cell. herdr routes it by the pane's modes: a mouse
    /// report, arrows in an alternate screen, or its own scrollback
    /// (findings 2.4).
    fn from(p: TerminalScrollParams) -> ClientPaneInputEvent {
        let kind = if p.lines > 0 {
            herdr::ClientMouseKind::ScrollUp
        } else {
            herdr::ClientMouseKind::ScrollDown
        };
        ClientPaneInputEvent::Mouse {
            kind,
            position: ClientMousePosition::Cell { column: 0, row: 0 },
            geometry: None,
            modifiers: 0,
            lines: p.lines.unsigned_abs().clamp(1, MAX_WHEEL_LINES) as u16,
        }
    }
}

impl From<herdr::CursorState> for Cursor {
    /// Position, visibility and DECSCUSR shape; the mode flags and `stale`
    /// are false (the `Grid` conversion sets the modes from the pane).
    fn from(c: herdr::CursorState) -> Cursor {
        Cursor {
            col: c.x,
            row: c.y,
            visible: c.visible,
            shape: c.shape,
            ..Cursor::default()
        }
    }
}

fn button(b: MouseButton) -> herdr::ClientMouseButton {
    match b {
        MouseButton::Left => herdr::ClientMouseButton::Left,
        MouseButton::Right => herdr::ClientMouseButton::Right,
        MouseButton::Middle => herdr::ClientMouseButton::Middle,
    }
}

/// A `terminal.mouse` kind herdr has no event for
/// (`TerminalMouseKind::Unknown`, from a newer phone): the daemon ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownMouseKind;

impl std::fmt::Display for UnknownMouseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a mouse event kind herdr has no event for")
    }
}

impl std::error::Error for UnknownMouseKind {}

impl TryFrom<&TerminalMouseParams> for ClientPaneInputEvent {
    type Error = UnknownMouseKind;

    /// `Down`/`Up`/`Drag` with their button and `Moved`, at the pane-local
    /// cell `col`, `row` with the phone's crossterm modifier bits (decision
    /// 10). A `TryFrom`, not decision 8's `From`: the open
    /// `TerminalMouseKind` has `Unknown`, which maps to no event.
    fn try_from(p: &TerminalMouseParams) -> Result<ClientPaneInputEvent, UnknownMouseKind> {
        let kind = match p.kind {
            TerminalMouseKind::Down { button: b } => herdr::ClientMouseKind::Down(button(b)),
            TerminalMouseKind::Up { button: b } => herdr::ClientMouseKind::Up(button(b)),
            TerminalMouseKind::Drag { button: b } => herdr::ClientMouseKind::Drag(button(b)),
            TerminalMouseKind::Moved => herdr::ClientMouseKind::Moved,
            _ => return Err(UnknownMouseKind),
        };
        Ok(ClientPaneInputEvent::Mouse {
            kind,
            position: ClientMousePosition::Cell {
                column: p.col,
                row: p.row,
            },
            geometry: None,
            modifiers: p.modifiers,
            lines: 1,
        })
    }
}
