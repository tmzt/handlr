//! The daemon's half: what was sent, what the phone acknowledged.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::{Frame, Grid, HISTORY, RowDiff};

/// One open terminal on the daemon's side (decision 5).
///
/// Each grid read from herdr goes through [`Mirror::update`], which numbers
/// a new frame and diffs it from the newest grid the phone acknowledged
/// with [`Mirror::ack`]. A full frame is sent instead when nothing is
/// acknowledged, the size changed, the acknowledged frame is [`HISTORY`]
/// or more frames old, or the diff would not be smaller.
///
/// A frame that is lost is not resent as such: the next frame is diffed
/// from what the phone holds, so it carries the lost changes too. When the
/// grid stops changing before the phone has acknowledged the newest frame,
/// [`Mirror::retransmit`] sends it again under a new `seq`; the caller calls
/// it on its idle tick.
///
/// A new `Mirror` numbers from 1 and pairs with a new [`crate::Screen`]: a
/// screen ignores frames not newer than the one it shows.
#[derive(Debug, Clone)]
pub struct Mirror {
    terminal_id: u32,
    /// The newest `seq` sent; 0 before the first frame.
    seq: u32,
    /// The last [`HISTORY`] sent grids, oldest first.
    sent: VecDeque<(u32, Arc<Grid>)>,
    /// The newest acknowledged frame.
    acked: Option<(u32, Arc<Grid>)>,
    /// Send the next update even when the grid is unchanged.
    force: bool,
}

impl Mirror {
    /// A mirror for `terminal_id` that has sent nothing.
    pub fn new(terminal_id: u32) -> Mirror {
        Mirror {
            terminal_id,
            seq: 0,
            sent: VecDeque::with_capacity(HISTORY),
            acked: None,
            force: false,
        }
    }

    /// The terminal's id, as frames carry it.
    pub fn terminal_id(&self) -> u32 {
        self.terminal_id
    }

    /// The newest `seq` sent; 0 before the first frame.
    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// The newest acknowledged `seq`; 0 when none is.
    pub fn acked_seq(&self) -> u32 {
        self.acked.as_ref().map_or(0, |(seq, _)| *seq)
    }

    /// The frame to send for `grid`, or `None` when it equals the last grid
    /// sent (and [`Mirror::reset`] has not been called since).
    pub fn update(&mut self, grid: Grid) -> Option<Frame> {
        if !self.force
            && let Some((_, last)) = self.sent.back()
            && **last == grid
        {
            return None;
        }
        Some(self.send(Arc::new(grid)))
    }

    /// The last grid sent, under a new `seq`, when the phone has not
    /// acknowledged the newest frame; `None` when it has, or nothing was
    /// sent.
    pub fn retransmit(&mut self) -> Option<Frame> {
        let (seq, last) = self.sent.back()?;
        if self.acked_seq() == *seq {
            return None;
        }
        let last = Arc::clone(last);
        Some(self.send(last))
    }

    /// The phone holds frame `seq`. Later frames are diffed from its grid.
    /// An ack for a frame not among the last [`HISTORY`] sent, or not newer
    /// than the acknowledged one, is ignored.
    pub fn ack(&mut self, seq: u32) {
        if seq <= self.acked_seq() {
            return;
        }
        if let Some((_, grid)) = self.sent.iter().find(|(sent, _)| *sent == seq) {
            self.acked = Some((seq, Arc::clone(grid)));
        }
    }

    /// Forget what the phone holds, so the next [`Mirror::update`] sends a
    /// full frame even if the grid is unchanged (a reconnect or a reopen).
    /// Acks for frames sent before the reset are ignored; `seq` carries on.
    pub fn reset(&mut self) {
        self.acked = None;
        self.sent.clear();
        self.force = true;
    }

    fn send(&mut self, grid: Arc<Grid>) -> Frame {
        self.force = false;
        self.seq += 1;
        let full = Frame::full(self.terminal_id, self.seq, &grid);
        let frame = match &self.acked {
            Some((base_seq, base))
                if base.cols == grid.cols
                    && base.rows == grid.rows
                    && ((self.seq - base_seq) as usize) < HISTORY =>
            {
                let diff = Frame {
                    base_seq: *base_seq,
                    diffs: grid
                        .lines
                        .iter()
                        .zip(&base.lines)
                        .enumerate()
                        .filter(|(_, (new, old))| new != old)
                        .map(|(row, (new, _))| RowDiff {
                            row: row as u16,
                            runs: new.clone(),
                        })
                        .collect(),
                    ..full.clone()
                };
                if diff.encoded_len() < full.encoded_len() {
                    diff
                } else {
                    full
                }
            }
            _ => full,
        };
        if self.sent.len() == HISTORY {
            self.sent.pop_front();
        }
        self.sent.push_back((self.seq, grid));
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Screen, parse_ansi};

    #[test]
    fn unchanged_grids_send_nothing_until_reset() {
        let mut m = Mirror::new(3);
        let f = m.update(parse_ansi("a\nb")).unwrap();
        assert!(f.is_full());
        assert_eq!((f.terminal_id, f.seq), (3, 1));
        assert!(m.update(parse_ansi("a\nb")).is_none());
        m.reset();
        let f = m.update(parse_ansi("a\nb")).unwrap();
        assert!(f.is_full());
        assert_eq!(f.seq, 2);
    }

    #[test]
    fn a_diff_from_the_acknowledged_frame_yields_the_new_screen() {
        let rows: Vec<String> = (0..20)
            .map(|i| format!("row {i} of a long screen"))
            .collect();
        let old = parse_ansi(&rows.join("\n"));
        let mut changed = rows.clone();
        changed[4] = "row 4 changed".into();
        changed[11] = "\x1b[1mrow 11 bold\x1b[0m".into();
        let new = parse_ansi(&changed.join("\n"));

        let mut m = Mirror::new(1);
        let mut s = Screen::default();
        assert!(s.apply(&m.update(old.clone()).unwrap()));
        m.ack(s.seq());
        let diff = m.update(new.clone()).unwrap();
        assert_eq!(diff.base_seq, 1);
        assert_eq!(
            diff.diffs.iter().map(|d| d.row).collect::<Vec<_>>(),
            [4, 11]
        );
        let diff = Frame::decode(&diff.encode()).unwrap();
        assert!(s.apply(&diff));
        assert_eq!(s.grid(), &new);
        assert_eq!(s.seq(), 2);
    }

    #[test]
    fn diffs_build_on_the_ack_not_the_last_send() {
        let mut m = Mirror::new(1);
        let mut s = Screen::default();
        let rows = |x: &str| parse_ansi(&format!("{x}\nfixed row one\nfixed row two\nfixed row 3"));
        s.apply(&m.update(rows("a")).unwrap());
        m.ack(1);
        // Frame 2 is lost; frame 3 is still diffed from frame 1.
        let _lost = m.update(rows("b")).unwrap();
        let f3 = m.update(rows("c")).unwrap();
        assert_eq!(f3.base_seq, 1);
        assert!(s.apply(&f3));
        assert_eq!(s.grid(), &rows("c"));
    }

    #[test]
    fn a_diff_bigger_than_full_is_sent_full() {
        let mut m = Mirror::new(1);
        m.update(parse_ansi("a\nb\nc")).unwrap();
        m.ack(1);
        let f = m.update(parse_ansi("x\ny\nz")).unwrap();
        assert!(f.is_full(), "every row changed: the diff is no smaller");
        // A size change is always full.
        m.ack(2);
        let f = m.update(parse_ansi("x\ny\nz\nw")).unwrap();
        assert!(f.is_full());
    }

    #[test]
    fn acks_never_go_backwards_and_unknown_acks_are_ignored() {
        let mut m = Mirror::new(1);
        for i in 0..3 {
            m.update(parse_ansi(&format!("{i}"))).unwrap();
        }
        m.ack(2);
        m.ack(1);
        assert_eq!(m.acked_seq(), 2);
        m.ack(99);
        assert_eq!(m.acked_seq(), 2);
        m.reset();
        m.ack(3);
        assert_eq!(m.acked_seq(), 0, "an ack from before the reset");
    }

    #[test]
    fn an_old_ack_falls_back_to_full() {
        let mut m = Mirror::new(1);
        let grid = |i: usize| parse_ansi(&format!("{i}\nsome fixed text\nmore fixed text"));
        m.update(grid(0)).unwrap();
        m.ack(1);
        for i in 1..HISTORY {
            let f = m.update(grid(i)).unwrap();
            assert!(!f.is_full(), "{i}");
        }
        assert!(m.update(grid(HISTORY)).unwrap().is_full());
    }

    #[test]
    fn retransmit_until_acknowledged() {
        let mut m = Mirror::new(1);
        assert!(m.retransmit().is_none());
        m.update(parse_ansi("a")).unwrap();
        let again = m.retransmit().unwrap();
        assert_eq!((again.seq, again.base_seq), (2, 0));
        m.ack(2);
        assert!(m.retransmit().is_none());
    }
}
