//! The phone's half: the newest frame it can build.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::{Cursor, Frame, Grid, HISTORY, Row};

static EMPTY: Grid = Grid {
    cols: 0,
    rows: 0,
    lines: Vec::new(),
    cursor: None,
};

/// One open terminal on the phone's side (decision 5).
///
/// Frames arrive in any order, some never. [`Screen::apply`] shows a frame
/// when it is newer than the one shown and it can be built: a full frame
/// always, a diff when the screen still holds its base (the last
/// [`HISTORY`] frames shown are kept). Anything older is ignored. After each
/// frame shown, the caller acknowledges [`Screen::seq`] to the daemon.
#[derive(Debug, Clone, Default)]
pub struct Screen {
    /// The last [`HISTORY`] grids shown, oldest first; the shown one last.
    shown: VecDeque<(u32, Arc<Grid>)>,
}

impl Screen {
    /// Show `frame` if it is newer than the frame shown and can be built.
    /// True when it was shown (the caller acks [`Screen::seq`]); false when
    /// it was older, its base is not held, or it is malformed (a row out of
    /// range, or a full frame missing a row).
    pub fn apply(&mut self, frame: &Frame) -> bool {
        if frame.seq <= self.seq() {
            return false;
        }
        let lines = if frame.is_full() {
            let mut lines: Vec<Option<Row>> = vec![None; frame.rows as usize];
            for diff in &frame.diffs {
                match lines.get_mut(diff.row as usize) {
                    Some(slot) => *slot = Some(diff.runs.clone()),
                    None => return false,
                }
            }
            match lines.into_iter().collect::<Option<Vec<Row>>>() {
                Some(lines) => lines,
                None => return false,
            }
        } else {
            let Some((_, base)) = self.shown.iter().find(|(seq, _)| *seq == frame.base_seq) else {
                return false;
            };
            if base.cols != frame.cols || base.rows != frame.rows {
                return false;
            }
            let mut lines = base.lines.clone();
            for diff in &frame.diffs {
                match lines.get_mut(diff.row as usize) {
                    Some(slot) => *slot = diff.runs.clone(),
                    None => return false,
                }
            }
            lines
        };
        if self.shown.len() == HISTORY {
            self.shown.pop_front();
        }
        let grid = Grid {
            cols: frame.cols,
            rows: frame.rows,
            lines,
            cursor: frame.cursor,
        };
        self.shown.push_back((frame.seq, Arc::new(grid)));
        true
    }

    /// The `seq` of the frame shown, to acknowledge; 0 before the first.
    pub fn seq(&self) -> u32 {
        self.shown.back().map_or(0, |(seq, _)| *seq)
    }

    /// The grid shown; empty before the first frame.
    pub fn grid(&self) -> &Grid {
        self.shown.back().map_or(&EMPTY, |(_, grid)| grid)
    }

    /// The cursor of the frame shown: every frame carries the cursor whole,
    /// so this is the newest frame's, `None` when it carried none.
    pub fn cursor(&self) -> Option<Cursor> {
        self.grid().cursor
    }

    /// The width of the grid shown.
    pub fn cols(&self) -> u16 {
        self.grid().cols
    }

    /// The rows of the grid shown.
    pub fn rows(&self) -> u16 {
        self.grid().rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirror, parse_ansi};

    fn grid(top: &str) -> Grid {
        parse_ansi(&format!(
            "{top}\nthe same second row\nand the same third row"
        ))
    }

    #[test]
    fn a_reordered_pair_ends_at_the_newer() {
        let mut m = Mirror::new(1);
        let mut s = Screen::default();
        s.apply(&m.update(grid("one")).unwrap());
        m.ack(s.seq());
        let f2 = m.update(grid("two")).unwrap();
        let f3 = m.update(grid("three")).unwrap();
        assert_eq!((f2.base_seq, f3.base_seq), (1, 1));
        assert!(s.apply(&f3));
        assert!(!s.apply(&f2), "older than the frame shown");
        assert_eq!(s.grid(), &grid("three"));
        assert_eq!(s.seq(), 3);
    }

    #[test]
    fn a_diff_without_its_base_is_ignored_and_a_full_frame_recovers() {
        let mut m = Mirror::new(1);
        let mut s = Screen::default();
        let _lost = m.update(grid("one")).unwrap();
        m.ack(1); // as if the phone had it
        let diff = m.update(grid("two")).unwrap();
        assert!(!diff.is_full());
        assert!(!s.apply(&diff));
        assert_eq!(s.seq(), 0);
        assert_eq!(s.grid(), &Grid::default());
        m.reset();
        assert!(s.apply(&m.update(grid("two")).unwrap()));
        assert_eq!(s.grid(), &grid("two"));
        assert_eq!((s.cols(), s.rows()), (22, 3));
    }

    #[test]
    fn malformed_frames_are_ignored() {
        let mut full = Frame::full(1, 1, &grid("x"));
        full.diffs.pop();
        assert!(!Screen::default().apply(&full));
        let mut out_of_range = Frame::full(1, 1, &grid("x"));
        out_of_range.diffs[0].row = 9;
        assert!(!Screen::default().apply(&out_of_range));
    }
}
