//! A terminal frame and its bytes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Color, Grid, Row, Run};

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Frame {
    pub terminal_id: u32,
    pub seq: u32,
    pub base_seq: u32,
    pub cols: u16,
    pub rows: u16,
    pub diffs: Vec<RowDiff>,
}

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
        }
    }

    /// True when this frame carries every row (`base_seq == 0`).
    pub fn is_full(&self) -> bool {
        self.base_seq == 0
    }

    /// The length of [`Frame::encode`]'s bytes, without encoding.
    pub fn encoded_len(&self) -> usize {
        HEADER_LEN
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

    /// The frame's bytes. At most `u16::MAX` rows are written.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
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
        out
    }

    /// Read a frame's bytes. Every byte must belong to the frame; rows are
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
        if !r.bytes.is_empty() {
            return Err(DecodeError::TrailingBytes(r.bytes.len()));
        }
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
        };
        assert_eq!(Frame::decode(&diff.encode()).unwrap(), diff);
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
