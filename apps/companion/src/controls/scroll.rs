//! A finger's scrolling of one window, with the agent view's drag, fling
//! and slop (plan 20261023_04, A1: "each section scrolls separately").
//! libcontrols has no scroll view, so a screen keeps one `Scroll` per
//! window and gives it the touches that started there.
//!
//! A drag moves the content; a fling carries on with momentum that decays.
//! The fling starts at the finger's release speed, measured as the agent
//! view measures it ([`release_speed`]): moves that arrive together (the
//! S23 sends them microseconds apart) no longer throw the content to an
//! end (owner, 2026-10-30: "dragging on the overview screen snaps back to
//! the beginning or end").
//! A sticky scroll follows the end while it is there, as the agent view's
//! flow follows the newest entry, and holds still once scrolled back.

use std::time::Duration;

use libmorph::TouchPhase;

use crate::agent::{
    DECAY, FLING_DT_MAX, MAX_FLING_OUT, MIN_FLING, SAMPLES, SLOP, fling_curve, release_speed,
};

/// A finger on the window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    id: u64,
    start: [f32; 2],
    last_y: f32,
    last_t: Duration,
    /// Past the slop: a scroll, not a tap.
    moved: bool,
    /// The latest moves' times and positions, oldest first, for the
    /// release speed.
    samples: [(Duration, f32); SAMPLES],
    sampled: usize,
}

impl Drag {
    fn sample(&mut self, t: Duration, y: f32) {
        if self.sampled == SAMPLES {
            self.samples.copy_within(1.., 0);
            self.sampled -= 1;
        }
        self.samples[self.sampled] = (t, y);
        self.sampled += 1;
    }
}

/// What a touch did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touched {
    /// Nothing yet, or a touch of another finger.
    Nothing,
    /// The content moved, or a drag ended.
    Scrolled,
    /// A tap ended, within the slop: the caller finds what it hit.
    Tap,
}

/// One window's scroll state, in pixels from the top of its content.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scroll {
    offset: f32,
    /// Where the user left it: kept when the window or content shrinks for
    /// a while, so the offset comes back to it (owner, 2026-10-23: "The
    /// scroll should remember where it is on the overview page").
    wanted: f32,
    max: f32,
    /// Follows the end while it is there.
    sticky: bool,
    following: bool,
    drag: Option<Drag>,
    /// A fling's speed, in pixels per second (positive: the finger moved
    /// down, scrolling back).
    fling: f32,
}

impl Scroll {
    /// At the top; `sticky`, at the end and following it.
    pub fn new(sticky: bool) -> Self {
        Scroll {
            sticky,
            following: sticky,
            ..Default::default()
        }
    }

    /// The content's height and the window's: the offset is kept within
    /// them, and a following scroll goes to the end.
    pub fn set_extent(&mut self, content: f32, window: f32) {
        self.max = (content - window).max(0.0);
        if self.following {
            self.wanted = self.max;
        }
        self.offset = self.wanted.clamp(0.0, self.max);
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// The largest offset.
    pub fn max(&self) -> f32 {
        self.max
    }

    /// Whether it follows the end.
    pub fn following(&self) -> bool {
        self.following
    }

    /// Whether the finger `id` is on this window.
    pub fn holds(&self, id: u64) -> bool {
        self.drag.is_some_and(|d| d.id == id)
    }

    fn scroll_to(&mut self, offset: f32) {
        self.offset = offset.clamp(0.0, self.max);
        self.wanted = self.offset;
        if self.sticky {
            self.following = self.offset >= self.max - 1.0;
        }
    }

    /// A touch at `now` (any monotonic clock), at scale `s`. A movement
    /// within the slop is a tap.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
        s: f32,
    ) -> Touched {
        match phase {
            TouchPhase::Started => {
                self.fling = 0.0;
                self.drag = Some(Drag {
                    id,
                    start: [x, y],
                    last_y: y,
                    last_t: now,
                    moved: false,
                    samples: [(now, y); SAMPLES],
                    sampled: 1,
                });
                Touched::Nothing
            }
            TouchPhase::Moved => {
                let Some(mut d) = self.drag.filter(|d| d.id == id) else {
                    return Touched::Nothing;
                };
                if !d.moved
                    && ((y - d.start[1]).abs() > SLOP * s || (x - d.start[0]).abs() > SLOP * s)
                {
                    d.moved = true;
                }
                let dy = y - d.last_y;
                d.sample(now, y);
                d.last_y = y;
                d.last_t = now;
                self.drag = Some(d);
                if d.moved {
                    self.scroll_to(self.offset - dy);
                    Touched::Scrolled
                } else {
                    Touched::Nothing
                }
            }
            TouchPhase::Ended => {
                let Some(d) = self.drag.take().filter(|d| d.id == id) else {
                    return Touched::Nothing;
                };
                if !d.moved {
                    return Touched::Tap;
                }
                // A finger that stopped before lifting does not fling.
                let still = now.saturating_sub(d.last_t) > Duration::from_millis(100);
                let velocity = fling_curve(release_speed(&d.samples[..d.sampled], s), s)
                    .clamp(-MAX_FLING_OUT * s, MAX_FLING_OUT * s);
                if !still && velocity.abs() > MIN_FLING * s {
                    self.fling = velocity;
                }
                Touched::Scrolled
            }
            TouchPhase::Cancelled => {
                self.drag = None;
                Touched::Nothing
            }
        }
    }

    /// Carries a fling on by `dt`. Returns whether the content moved.
    pub fn tick(&mut self, dt: Duration, s: f32) -> bool {
        if self.fling == 0.0 || self.drag.is_some() {
            return false;
        }
        // A slow frame cannot throw the content, as in the agent view.
        let dt = dt.as_secs_f32().min(FLING_DT_MAX);
        let before = self.offset;
        self.scroll_to(self.offset - self.fling * dt);
        self.fling *= (-DECAY * dt).exp();
        let stopped = self.offset == before
            || self.fling.abs() < MIN_FLING * s
            || self.offset <= 0.0
            || (self.sticky && self.following);
        if stopped {
            self.fling = 0.0;
        }
        self.offset != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(16);

    /// Drags from `from` to `to` in `steps`, 16 ms apart, starting at `t`.
    fn drag(s: &mut Scroll, from: f32, to: f32, steps: u32, t: Duration) -> Duration {
        s.touch(1, TouchPhase::Started, 100.0, from, t, 2.0);
        let mut now = t;
        for i in 1..=steps {
            now += MS;
            let y = from + (to - from) * i as f32 / steps as f32;
            s.touch(1, TouchPhase::Moved, 100.0, y, now, 2.0);
        }
        now
    }

    #[test]
    fn a_short_movement_is_a_tap_and_a_longer_one_a_scroll() {
        let mut s = Scroll::new(false);
        s.set_extent(3000.0, 1000.0);
        s.touch(1, TouchPhase::Started, 100.0, 500.0, Duration::ZERO, 2.0);
        s.touch(1, TouchPhase::Moved, 100.0, 510.0, MS, 2.0);
        assert_eq!(
            s.touch(1, TouchPhase::Ended, 100.0, 510.0, MS * 2, 2.0),
            Touched::Tap
        );
        assert_eq!(s.offset(), 0.0);
        // Up by 400 px: the content moves up, and the end is a scroll.
        let t = drag(&mut s, 900.0, 500.0, 10, Duration::ZERO);
        assert!((s.offset() - 400.0).abs() < 1.0, "{}", s.offset());
        assert_eq!(
            s.touch(1, TouchPhase::Ended, 100.0, 500.0, t + MS * 20, 2.0),
            Touched::Scrolled
        );
        // Another finger does nothing.
        assert_eq!(
            s.touch(2, TouchPhase::Moved, 100.0, 0.0, t, 2.0),
            Touched::Nothing
        );
    }

    #[test]
    fn a_fling_carries_on_and_decays() {
        let mut s = Scroll::new(false);
        s.set_extent(20_000.0, 1000.0);
        let t = drag(&mut s, 1500.0, 900.0, 6, Duration::ZERO);
        s.touch(1, TouchPhase::Ended, 100.0, 900.0, t, 2.0);
        let after_drag = s.offset();
        assert!(s.tick(MS, 2.0));
        assert!(s.offset() > after_drag);
        let mut steps = 0;
        while s.tick(MS, 2.0) {
            steps += 1;
            assert!(steps < 10_000, "the fling never stopped");
        }
        assert!(s.offset() <= s.max());
    }

    #[test]
    fn moves_arriving_together_do_not_throw_the_content() {
        // The S23's bursts: two moves 0.1 ms apart every 20 ms, a finger
        // at 200 px/s. Speeds of each pair alone are 20 000 px/s.
        let mut s = Scroll::new(false);
        s.set_extent(20_000.0, 1000.0);
        s.touch(1, TouchPhase::Started, 100.0, 1500.0, Duration::ZERO, 2.0);
        let (mut now, mut y) = (Duration::ZERO, 1500.0);
        for _ in 0..20 {
            for gap in [Duration::from_millis(20), Duration::from_micros(100)] {
                now += gap;
                y -= 2.0;
                s.touch(1, TouchPhase::Moved, 100.0, y, now, 2.0);
            }
        }
        s.touch(1, TouchPhase::Ended, 100.0, y, now, 2.0);
        let dragged = s.offset();
        while s.tick(MS, 2.0) {}
        // 200 px/s decaying at 2.0/s goes at most 100 px further.
        assert!(
            s.offset() - dragged <= 100.0,
            "{} after {dragged}",
            s.offset()
        );
    }

    #[test]
    fn a_sticky_scroll_follows_the_end_until_scrolled_back() {
        let mut s = Scroll::new(true);
        s.set_extent(500.0, 1000.0);
        assert_eq!(s.offset(), 0.0);
        s.set_extent(3000.0, 1000.0);
        assert!(s.following());
        assert_eq!(s.offset(), 2000.0);
        // Back by 300 px: it holds there as the content grows.
        let t = drag(&mut s, 500.0, 800.0, 10, Duration::ZERO);
        s.touch(1, TouchPhase::Ended, 100.0, 800.0, t + MS * 20, 2.0);
        assert!(!s.following());
        let back = s.offset();
        s.set_extent(4000.0, 1000.0);
        assert_eq!(s.offset(), back);
        // To the end again: following.
        let t = drag(&mut s, 2000.0, 0.0, 10, t + MS * 40);
        s.touch(1, TouchPhase::Ended, 100.0, 0.0, t + MS * 20, 2.0);
        assert!(s.following());
        s.set_extent(5000.0, 1000.0);
        assert_eq!(s.offset(), 4000.0);
    }

    #[test]
    fn the_place_comes_back_when_the_window_grows_again() {
        let mut sc = Scroll::new(false);
        sc.set_extent(1000.0, 400.0);
        sc.scroll_to(300.0);
        // A card appears below and the window shrinks... or the content
        // is briefly shorter: the offset is held within it.
        sc.set_extent(500.0, 400.0);
        assert_eq!(sc.offset(), 100.0);
        // Room again: back where the user left it.
        sc.set_extent(1000.0, 400.0);
        assert_eq!(sc.offset(), 300.0);
    }
}
