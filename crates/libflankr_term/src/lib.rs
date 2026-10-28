//! A terminal mirrored over flankr's link
//! (docs/design/20261027_07_TERMINAL_MODE_PLAN.md, decisions 1, 3 and 5,
//! step 1, and the amendments of 14:23 and 20:44 EDT).
//!
//! * [`parse_ansi`] turns herdr's `pane.read { source: "visible", format:
//!   "ansi" }` text into a [`Grid`]: rows of styled [`Run`]s, SGR colours
//!   and attributes kept, every other escape dropped.
//! * A [`Frame`] is one update of a terminal: either full (`base_seq == 0`,
//!   every row) or the rows that differ from an earlier frame the phone
//!   acknowledged. [`Frame::encode`] and [`Frame::decode`] are its bytes.
//! * The daemon keeps a [`Mirror`] per open terminal, the phone a
//!   [`Screen`]. The mirror diffs each new grid from the last one the phone
//!   acknowledged, so a lost or reordered frame costs nothing: the screen
//!   applies what it can and ignores what is older than what it shows, and
//!   both converge on the newest frame (the sync model of mosh, without its
//!   emulator or prediction).
//! * Since 0.1.1 a grid and a frame may carry a [`Cursor`]
//!   (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md, decision 9),
//!   written as a trailer after a frame's rows; see [`Frame`].
//!
//! The crate is pure state: no herdr, link or protocol dependency.

mod ansi;
mod frame;
mod mirror;
mod screen;

pub use ansi::{MAX_COLS, MAX_ROWS, parse_ansi};
pub use frame::{CURSOR_TRAILER_LEN, DecodeError, Frame, RowDiff};
pub use mirror::Mirror;
pub use screen::Screen;

use serde::{Deserialize, Serialize};

/// How many sent grids a [`Mirror`] keeps for acknowledgements, and how
/// many shown grids a [`Screen`] keeps as bases for diffs. A mirror never
/// diffs from a frame this many frames or more older than the one it
/// sends, so the screen always holds the base it needs.
pub const HISTORY: usize = 16;

/// A colour: `0` is the terminal's default, `0x0100_00NN` palette index
/// `NN` (0-255), `0x02RR_GGBB` a 24-bit RGB colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Color(pub u32);

impl Color {
    /// The terminal's default foreground or background.
    pub const DEFAULT: Color = Color(0);

    const PALETTE_TAG: u32 = 0x0100_0000;
    const RGB_TAG: u32 = 0x0200_0000;

    /// Palette index `index` (0-15 are the ANSI colours and their bright
    /// forms, 16-255 the 256-colour cube and greys).
    pub const fn palette(index: u8) -> Color {
        Color(Self::PALETTE_TAG | index as u32)
    }

    /// A 24-bit colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(Self::RGB_TAG | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    /// True for the terminal's default.
    pub const fn is_default(self) -> bool {
        self.0 == 0
    }

    /// The palette index, when this is a palette colour.
    pub const fn as_palette(self) -> Option<u8> {
        if self.0 & 0xFF00_0000 == Self::PALETTE_TAG {
            Some((self.0 & 0xFF) as u8)
        } else {
            None
        }
    }

    /// The red, green and blue bytes, when this is a 24-bit colour.
    pub const fn as_rgb(self) -> Option<(u8, u8, u8)> {
        if self.0 & 0xFF00_0000 == Self::RGB_TAG {
            Some(((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8))
        } else {
            None
        }
    }
}

/// The bits of [`Run::flags`]. Bits 6 and 7 are reserved and zero.
pub mod flags {
    pub const BOLD: u8 = 1 << 0;
    pub const DIM: u8 = 1 << 1;
    pub const ITALIC: u8 = 1 << 2;
    pub const UNDERLINE: u8 = 1 << 3;
    pub const INVERSE: u8 = 1 << 4;
    pub const STRIKETHROUGH: u8 = 1 << 5;
}

/// A run of characters in one style.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    /// [`flags`] bits.
    pub flags: u8,
}

impl Run {
    /// True when `other` has the same colours and flags.
    pub fn same_style(&self, other: &Run) -> bool {
        self.fg == other.fg && self.bg == other.bg && self.flags == other.flags
    }

    /// The run's width in columns: one per `char` (see [`parse_ansi`]).
    pub fn width(&self) -> usize {
        self.text.chars().count()
    }
}

/// A row: its runs, left to right. [`parse_ansi`] merges adjacent runs of
/// the same style and cuts blank trailing spaces, so equal rows are equal
/// values.
pub type Row = Vec<Run>;

/// The width of `row` in columns.
pub fn row_width(row: &Row) -> usize {
    row.iter().map(Run::width).sum()
}

/// A terminal's visible frame: `rows` rows, the widest `cols` columns wide.
/// `lines.len() == rows`; a row narrower than `cols` is blank to its right.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Row>,
    /// The terminal's cursor, when the source knows it (herdr's attached
    /// surface does; [`parse_ansi`]'s text does not, so its grids have
    /// `None`). Added in 0.1.1.
    #[serde(default)]
    pub cursor: Option<Cursor>,
}

/// A terminal's cursor and the modes that go with it
/// (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md, decision 9).
/// Added in 0.1.1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cursor {
    /// The cursor's column, from 0 at the left.
    pub col: u16,
    /// The cursor's row, from 0 at the top.
    pub row: u16,
    /// False while the program has hidden it.
    pub visible: bool,
    /// The cursor's shape as the source numbers it (herdr's
    /// `CursorShape`), passed through unread.
    pub shape: u8,
    /// The program asked for mouse reports.
    pub mouse_reporting: bool,
    /// The program is on the alternate screen.
    pub alternate_screen: bool,
    /// The view is not the one flankr asked for: the desktop has the tab
    /// (plan 20261027_08, the amendment of 23:32 EDT).
    pub stale: bool,
}

impl Grid {
    /// A grid of `lines`, `cols` the widest of them (both clamped to
    /// `u16`), with no cursor.
    pub fn from_lines(lines: Vec<Row>) -> Grid {
        let cols = lines.iter().map(row_width).max().unwrap_or(0);
        Grid {
            cols: cols.min(u16::MAX as usize) as u16,
            rows: lines.len().min(u16::MAX as usize) as u16,
            lines,
            cursor: None,
        }
    }

    /// The grid's rows as plain text, one string per row.
    pub fn text_lines(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|row| row.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }
}
