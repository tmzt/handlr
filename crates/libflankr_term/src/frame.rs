//! A terminal frame and its bytes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Color, Cursor, Grid, Row, Run};

/// One row of a [`Frame`]: its index and its runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RowDiff {
    pub row: u16,
    pub runs: Row,
}

/// One update of a terminal.
///
/// `seq` counts the terminal's frames from 1. A full frame has
/// `base_seq == 0` and carries every row `0..rows`; any other frame is a
/// diff and carries only the rows that differ from frame `base_seq`, which
/// has the same `cols` and `rows`.
///
/// # Bytes
///
/// This is the payload after the link's flag byte (kind 1, terminal; see
/// `libflankr_proto::link`), and the link frames it like any message, so
/// an encoded frame is at most 1 MiB. All numbers are big-endian:
///
/// ```text
/// u32 terminal_id, u32 seq, u32 base_seq, u16 cols, u16 rows, u16 count
/// count × { u16 row, u16 run_count,
///           run_count × { u32 fg, u32 bg, u8 flags, u16 len, len bytes UTF-8 } }
/// ```
///
/// A run whose text is longer than 65535 bytes is written as several runs
/// of the same style, and a row is cut at 65535 runs. Neither happens for
/// a [`crate::parse_ansi`] grid, whose rows are at most 1000 characters.
///
/// ## The cursor trailer (0.1.1)
///
/// When [`Frame::cursor`] is `Some`, [`Frame::encode`] writes
/// [`CURSOR_TRAILER_LEN`] more bytes after the rows
/// (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md, decision 9):
///
/// ```text
/// u8 1, u16 col, u16 row, u8 bits, u8 shape
/// ```
///
/// `bits`: bit 0 visible, 1 mouse reporting, 2 alternate screen, 3 stale;
/// bits 4-7 are zero. A frame without the trailer is byte for byte the
/// 0.1.0 frame, and [`Frame::decode`] reads it as before, with `cursor:
/// None`. A phone that has not asked for the cursor (its `terminal.open`
/// without `cursor: true`, libflankr_proto 0.4.2) cannot read the trailer
/// (0.1.0 decodes it as `TrailingBytes`), so the daemon writes that
/// phone's frames with [`Frame::encode_with`]`(false)`, which leaves it
/// out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Frame {
    pub terminal_id: u32,
    pub seq: u32,
    pub base_seq: u32,
    pub cols: u16,
    pub rows: u16,
    pub diffs: Vec<RowDiff>,
    /// The terminal's cursor, written as the trailer when `Some`. Every
    /// frame of a terminal whose grids carry a cursor carries it whole, a
    /// diff included. Added in 0.1.1.
    #[serde(default)]
    pub cursor: Option<Cursor>,
}

/// The length of the cursor trailer that follows a frame's rows when it
/// carries a cursor.
pub const CURSOR_TRAILER_LEN: usize = 1 + 2 + 2 + 1 + 1;

const CURSOR_TAG: u8 = 1;
const CURSOR_VISIBLE: u8 = 1 << 0;
const CURSOR_MOUSE_REPORTING: u8 = 1 << 1;
const CURSOR_ALTERNATE_SCREEN: u8 = 1 << 2;
const CURSOR_STALE: u8 = 1 << 3;
const CURSOR_BITS: u8 =
    CURSOR_VISIBLE | CURSOR_MOUSE_REPORTING | CURSOR_ALTERNATE_SCREEN | CURSOR_STALE;

const HEADER_LEN: usize = 4 + 4 + 4 + 2 + 2 + 2;
const ROW_HEADER_LEN: usize = 2 + 2;
const RUN_HEADER_LEN: usize = 4 + 4 + 1 + 2;

/// Why [`Frame::decode`] refused its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The bytes end inside the frame.
    Truncated,
    /// Bytes are left after the frame.
    TrailingBytes(usize),
    /// A run's text is not UTF-8.
    BadUtf8,
    /// A row index is not below `rows`.
    RowOutOfRange { row: u16, rows: u16 },
    /// A row appears twice.
    DuplicateRow(u16),
    /// A full frame does not carry every row.
    MissingRows { count: u16, rows: u16 },
    /// `seq` is 0, or a diff's `base_seq` is not older than its `seq`.
    BadSeq { seq: u32, base_seq: u32 },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "terminal frame truncated"),
            DecodeError::TrailingBytes(n) => {
                write!(f, "{n} trailing bytes after the terminal frame")
            }
            DecodeError::BadUtf8 => write!(f, "terminal frame text is not UTF-8"),
            DecodeError::RowOutOfRange { row, rows } => {
                write!(f, "terminal frame row {row} is not below its {rows} rows")
            }
            DecodeError::DuplicateRow(row) => write!(f, "terminal frame row {row} appears twice"),
            DecodeError::MissingRows { count, rows } => {
                write!(f, "full terminal frame carries {count} of its {rows} rows")
            }
            DecodeError::BadSeq { seq, base_seq } => {
                write!(f, "terminal frame seq {seq} cannot have base {base_seq}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// `run`'s text in pieces of at most `u16::MAX` bytes, cut at `char`
/// boundaries.
fn chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = rest.len().min(u16::MAX as usize);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (head, tail) = rest.split_at(end);
        rest = tail;
        Some(head)
    })
}

/// A row's runs as written: long texts split, at most `u16::MAX` runs.
/// An empty run is written as one empty run.
fn wire_runs(row: &Row) -> impl Iterator<Item = (&Run, &str)> {
    row.iter()
        .flat_map(|run| {
            let pieces: Box<dyn Iterator<Item = &str>> = if run.text.is_empty() {
                Box::new(std::iter::once(""))
            } else {
                Box::new(chunks(&run.text))
            };
            pieces.map(move |piece| (run, piece))
        })
        .take(u16::MAX as usize)
}

impl Frame {
    /// A full frame of `grid`.
    pub fn full(terminal_id: u32, seq: u32, grid: &Grid) -> Frame {
        Frame {
            terminal_id,
            seq,
            base_seq: 0,
            cols: grid.cols,
            rows: grid.rows,
            diffs: grid
                .lines
                .iter()
                .take(grid.rows as usize)
                .enumerate()
                .map(|(row, runs)| RowDiff {
                    row: row as u16,
                    runs: runs.clone(),
                })
                .collect(),
            cursor: grid.cursor,
        }
    }

    /// True when this frame carries every row (`base_seq == 0`).
    pub fn is_full(&self) -> bool {
        self.base_seq == 0
    }

    /// The length of [`Frame::encode`]'s bytes, without encoding: the
    /// cursor trailer included when the frame carries a cursor.
    pub fn encoded_len(&self) -> usize {
        self.encoded_len_with(true)
    }

    /// The length of [`Frame::encode_with`]`(cursor)`'s bytes.
    pub fn encoded_len_with(&self, cursor: bool) -> usize {
        let trailer = if cursor && self.cursor.is_some() {
            CURSOR_TRAILER_LEN
        } else {
            0
        };
        trailer
            + HEADER_LEN
            + self
                .diffs
                .iter()
                .take(u16::MAX as usize)
                .map(|diff| {
                    ROW_HEADER_LEN
                        + wire_runs(&diff.runs)
                            .map(|(_, text)| RUN_HEADER_LEN + text.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }

    /// The frame's bytes, with the cursor trailer when the frame carries a
    /// cursor. At most `u16::MAX` rows are written.
    pub fn encode(&self) -> Vec<u8> {
        self.encode_with(true)
    }

    /// The frame's bytes, with the cursor trailer only when `cursor` is
    /// true and the frame carries one. `encode_with(false)` is the 0.1.0
    /// encoding, for a phone that did not ask for the cursor.
    pub fn encode_with(&self, cursor: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len_with(cursor));
        out.extend_from_slice(&self.terminal_id.to_be_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.base_seq.to_be_bytes());
        out.extend_from_slice(&self.cols.to_be_bytes());
        out.extend_from_slice(&self.rows.to_be_bytes());
        let diffs = &self.diffs[..self.diffs.len().min(u16::MAX as usize)];
        out.extend_from_slice(&(diffs.len() as u16).to_be_bytes());
        for diff in diffs {
            out.extend_from_slice(&diff.row.to_be_bytes());
            let count_at = out.len();
            out.extend_from_slice(&[0, 0]);
            let mut count: u16 = 0;
            for (run, text) in wire_runs(&diff.runs) {
                out.extend_from_slice(&run.fg.0.to_be_bytes());
                out.extend_from_slice(&run.bg.0.to_be_bytes());
                out.push(run.flags);
                out.extend_from_slice(&(text.len() as u16).to_be_bytes());
                out.extend_from_slice(text.as_bytes());
                count += 1;
            }
            out[count_at..count_at + 2].copy_from_slice(&count.to_be_bytes());
        }
        if cursor && let Some(c) = &self.cursor {
            let mut bits = 0;
            for (on, bit) in [
                (c.visible, CURSOR_VISIBLE),
                (c.mouse_reporting, CURSOR_MOUSE_REPORTING),
                (c.alternate_screen, CURSOR_ALTERNATE_SCREEN),
                (c.stale, CURSOR_STALE),
            ] {
                if on {
                    bits |= bit;
                }
            }
            out.push(CURSOR_TAG);
            out.extend_from_slice(&c.col.to_be_bytes());
            out.extend_from_slice(&c.row.to_be_bytes());
            out.push(bits);
            out.push(c.shape);
        }
        out
    }

    /// Read a frame's bytes, with or without the cursor trailer. Every byte
    /// must belong to the frame: after the rows only a whole trailer (tag
    /// 1, bits 4-7 zero) is read, and anything else is
    /// [`DecodeError::TrailingBytes`]. Rows are
    /// below `rows` and appear once; a full frame carries every row; `seq`
    /// is not 0 and a diff's `base_seq` is older than its `seq`.
    pub fn decode(bytes: &[u8]) -> Result<Frame, DecodeError> {
        let mut r = Reader { bytes };
        let terminal_id = r.u32()?;
        let seq = r.u32()?;
        let base_seq = r.u32()?;
        let cols = r.u16()?;
        let rows = r.u16()?;
        let count = r.u16()?;
        if seq == 0 || base_seq >= seq {
            return Err(DecodeError::BadSeq { seq, base_seq });
        }
        let mut seen = vec![false; rows as usize];
        let mut diffs = Vec::with_capacity(count.min(rows) as usize);
        for _ in 0..count {
            let row = r.u16()?;
            let Some(slot) = seen.get_mut(row as usize) else {
                return Err(DecodeError::RowOutOfRange { row, rows });
            };
            if std::mem::replace(slot, true) {
                return Err(DecodeError::DuplicateRow(row));
            }
            let run_count = r.u16()?;
            let mut runs = Vec::with_capacity(run_count as usize);
            for _ in 0..run_count {
                let fg = Color(r.u32()?);
                let bg = Color(r.u32()?);
                let flags = r.u8()?;
                let len = r.u16()? as usize;
                let text = std::str::from_utf8(r.take(len)?).map_err(|_| DecodeError::BadUtf8)?;
                runs.push(Run {
                    text: text.to_owned(),
                    fg,
                    bg,
                    flags,
                });
            }
            diffs.push(RowDiff { row, runs });
        }
        let cursor = match *r.bytes {
            [] => None,
            [CURSOR_TAG, c0, c1, r0, r1, bits, shape] if bits & !CURSOR_BITS == 0 => Some(Cursor {
                col: u16::from_be_bytes([c0, c1]),
                row: u16::from_be_bytes([r0, r1]),
                visible: bits & CURSOR_VISIBLE != 0,
                shape,
                mouse_reporting: bits & CURSOR_MOUSE_REPORTING != 0,
                alternate_screen: bits & CURSOR_ALTERNATE_SCREEN != 0,
                stale: bits & CURSOR_STALE != 0,
            }),
            ref rest => return Err(DecodeError::TrailingBytes(rest.len())),
        };
        if base_seq == 0 && count != rows {
            return Err(DecodeError::MissingRows { count, rows });
        }
        Ok(Frame {
            terminal_id,
            seq,
            base_seq,
            cols,
            rows,
            diffs,
            cursor,
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.bytes.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flags, parse_ansi};

    fn sample() -> Frame {
        let grid = parse_ansi("\x1b[1;31mhé\x1b[0m llo\n\n\x1b[48;2;1;2;3m  x\x1b[0m\n");
        Frame::full(7, 3, &grid)
    }

    #[test]
    fn round_trip() {
        let frame = sample();
        let bytes = frame.encode();
        assert_eq!(bytes.len(), frame.encoded_len());
        assert_eq!(Frame::decode(&bytes).unwrap(), frame);

        let diff = Frame {
            terminal_id: 1,
            seq: 9,
            base_seq: 4,
            cols: 5,
            rows: 10,
            diffs: vec![
                RowDiff {
                    row: 8,
                    runs: vec![],
                },
                RowDiff {
                    row: 2,
                    runs: vec![Run {
                        text: "ab".into(),
                        fg: Color::palette(200),
                        bg: Color::rgb(9, 8, 7),
                        flags: flags::INVERSE,
                    }],
                },
            ],
            cursor: None,
        };
        assert_eq!(Frame::decode(&diff.encode()).unwrap(), diff);
    }

    fn a_cursor() -> Cursor {
        Cursor {
            col: 0x0102,
            row: 3,
            visible: true,
            shape: 2,
            mouse_reporting: false,
            alternate_screen: true,
            stale: true,
        }
    }

    #[test]
    fn a_frame_without_a_trailer_is_the_old_frame() {
        // The 0.1.0 bytes of `the_header_is_big_endian` and of `sample`
        // decode unchanged, with no cursor.
        let old = [1, 2, 3, 4, 0, 0, 0, 5, 0, 0, 0, 0, 0x0a, 0x0b, 0, 0, 0, 0];
        let frame = Frame::decode(&old).unwrap();
        assert_eq!(frame.cursor, None);
        assert_eq!(frame.encode(), old);
        let sample = sample();
        assert_eq!(sample.cursor, None, "parse_ansi grids carry no cursor");
        assert_eq!(Frame::decode(&sample.encode()).unwrap(), sample);
    }

    #[test]
    fn the_cursor_trailer_round_trips() {
        let mut frame = sample();
        let plain = frame.encode();
        frame.cursor = Some(a_cursor());
        let bytes = frame.encode();
        assert_eq!(bytes.len(), frame.encoded_len());
        assert_eq!(bytes.len(), plain.len() + CURSOR_TRAILER_LEN);
        assert_eq!(&bytes[..plain.len()], &plain[..]);
        assert_eq!(&bytes[plain.len()..], [1, 0x01, 0x02, 0, 3, 0b1101, 2]);
        assert_eq!(Frame::decode(&bytes).unwrap(), frame);

        // Each bit alone.
        for (bit, set) in [
            (
                0b0001,
                (|c: &mut Cursor| c.visible = true) as fn(&mut Cursor),
            ),
            (0b0010, |c| c.mouse_reporting = true),
            (0b0100, |c| c.alternate_screen = true),
            (0b1000, |c| c.stale = true),
        ] {
            let mut c = Cursor::default();
            set(&mut c);
            frame.cursor = Some(c);
            let bytes = frame.encode();
            assert_eq!(bytes[bytes.len() - 2], bit);
            assert_eq!(Frame::decode(&bytes).unwrap().cursor, Some(c));
        }

        // Left out for a phone that did not ask.
        frame.cursor = Some(a_cursor());
        assert_eq!(frame.encode_with(false), plain);
        assert_eq!(frame.encoded_len_with(false), plain.len());
    }

    #[test]
    fn other_trailing_bytes_are_refused() {
        let mut frame = sample();
        frame.cursor = Some(a_cursor());
        let bytes = frame.encode();
        let rows_end = bytes.len() - CURSOR_TRAILER_LEN;
        // A cut trailer, a wrong tag, a high bit and one byte too many.
        for n in 1..CURSOR_TRAILER_LEN {
            assert_eq!(
                Frame::decode(&bytes[..rows_end + n]),
                Err(DecodeError::TrailingBytes(n))
            );
        }
        let mut tag = bytes.clone();
        tag[rows_end] = 2;
        assert_eq!(Frame::decode(&tag), Err(DecodeError::TrailingBytes(7)));
        let mut high = bytes.clone();
        high[rows_end + 5] |= 0x10;
        assert_eq!(Frame::decode(&high), Err(DecodeError::TrailingBytes(7)));
        let mut long = bytes;
        long.push(0);
        assert_eq!(Frame::decode(&long), Err(DecodeError::TrailingBytes(8)));
    }

    #[test]
    fn the_header_is_big_endian() {
        let bytes = Frame {
            terminal_id: 0x0102_0304,
            seq: 5,
            base_seq: 0,
            cols: 0x0a0b,
            rows: 0,
            diffs: vec![],
            cursor: None,
        }
        .encode();
        assert_eq!(
            bytes,
            [1, 2, 3, 4, 0, 0, 0, 5, 0, 0, 0, 0, 0x0a, 0x0b, 0, 0, 0, 0]
        );
    }

    #[test]
    fn long_runs_are_split() {
        let text = "é".repeat(40_000);
        let frame = Frame {
            terminal_id: 1,
            seq: 1,
            base_seq: 0,
            cols: 40_000,
            rows: 1,
            diffs: vec![RowDiff {
                row: 0,
                runs: vec![Run {
                    text: text.clone(),
                    ..Run::default()
                }],
            }],
            cursor: None,
        };
        let bytes = frame.encode();
        assert_eq!(bytes.len(), frame.encoded_len());
        let back = Frame::decode(&bytes).unwrap();
        assert_eq!(back.diffs[0].runs.len(), 2);
        let joined: String = back.diffs[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn truncation_and_trailing_bytes() {
        let bytes = sample().encode();
        for n in 0..bytes.len() {
            assert_eq!(
                Frame::decode(&bytes[..n]),
                Err(DecodeError::Truncated),
                "{n}"
            );
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(Frame::decode(&long), Err(DecodeError::TrailingBytes(1)));
    }

    fn one_row(row: u16, rows: u16, text: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&4u16.to_be_bytes());
        b.extend_from_slice(&rows.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&row.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0; 9]);
        b.extend_from_slice(&(text.len() as u16).to_be_bytes());
        b.extend_from_slice(text);
        b
    }

    #[test]
    fn bad_utf8_and_bad_rows() {
        assert!(Frame::decode(&one_row(0, 1, b"ok")).is_ok());
        assert_eq!(
            Frame::decode(&one_row(0, 1, b"\xff")),
            Err(DecodeError::BadUtf8)
        );
        assert_eq!(
            Frame::decode(&one_row(3, 3, b"x")),
            Err(DecodeError::RowOutOfRange { row: 3, rows: 3 })
        );

        let dup = Frame {
            terminal_id: 1,
            seq: 2,
            base_seq: 1,
            cols: 0,
            rows: 3,
            diffs: vec![RowDiff::default(), RowDiff::default()],
            cursor: None,
        };
        assert_eq!(
            Frame::decode(&dup.encode()),
            Err(DecodeError::DuplicateRow(0))
        );
    }

    #[test]
    fn a_full_frame_needs_every_row() {
        let mut frame = sample();
        frame.diffs.pop();
        assert_eq!(
            Frame::decode(&frame.encode()),
            Err(DecodeError::MissingRows { count: 2, rows: 3 })
        );
    }

    #[test]
    fn seq_zero_and_a_base_not_older_are_refused() {
        let mut frame = sample();
        frame.seq = 0;
        assert_eq!(
            Frame::decode(&frame.encode()),
            Err(DecodeError::BadSeq {
                seq: 0,
                base_seq: 0
            })
        );
        let diff = Frame {
            terminal_id: 1,
            seq: 4,
            base_seq: 4,
            cols: 0,
            rows: 1,
            diffs: vec![],
            cursor: None,
        };
        assert_eq!(
            Frame::decode(&diff.encode()),
            Err(DecodeError::BadSeq {
                seq: 4,
                base_seq: 4
            })
        );
    }
}
