//! A sideways drag on a pinned card (plan 20261022_03, the L3 amendment of
//! 2026-10-30 15:20 EDT, item 2): the card follows the finger left or
//! right; past [`REMOVES`], or flung, it goes; let go short, it springs
//! back. A finger that moves up or down first is the flow's scroll, never
//! a drag of the card.

use std::collections::VecDeque;
use std::time::Duration;

/// How far the card is dragged before letting go removes it, in points.
pub const REMOVES: f32 = 40.0;
/// A release at least this fast sideways (points per second), the way
/// the card was dragged, removes it however far it went.
pub const FLING: f32 = 600.0;
/// The span the release speed is measured over, back from the last move;
/// a finger still for longer than this before lifting does not fling.
const SPEED_WINDOW: Duration = Duration::from_millis(100);
/// How fast the card springs back: its offset falls by e every 1/SPRING
/// seconds.
const SPRING: f32 = 18.0;

/// Which way a finger on the card went, once past the slop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// Not past the slop yet: a tap, so far.
    Undecided,
    /// Sideways: the card follows it.
    Sideways,
    /// Up or down first: the flow's, not the card's.
    Vertical,
}

/// What letting go of a sideways drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Removes,
    SpringsBack,
}

/// One finger on a pinned card.
#[derive(Clone, Debug)]
pub struct Swipe {
    id: u64,
    start: [f32; 2],
    way: Way,
    dx: f32,
    /// Recent moves: the time and the offset.
    samples: VecDeque<(Duration, f32)>,
}

impl Swipe {
    pub fn new(id: u64, x: f32, y: f32, now: Duration) -> Self {
        let mut samples = VecDeque::new();
        samples.push_back((now, 0.0));
        Swipe {
            id,
            start: [x, y],
            way: Way::Undecided,
            dx: 0.0,
            samples,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn way(&self) -> Way {
        self.way
    }

    /// The card's offset, in pixels: the finger's sideways travel while it
    /// drags the card.
    pub fn dx(&self) -> f32 {
        if self.way == Way::Sideways {
            self.dx
        } else {
            0.0
        }
    }

    /// The finger moved to `(x, y)`. Past `slop` pixels the way is
    /// decided, once: sideways when it went further across than up or
    /// down.
    pub fn moved(&mut self, x: f32, y: f32, now: Duration, slop: f32) -> Way {
        let (ax, ay) = ((x - self.start[0]).abs(), (y - self.start[1]).abs());
        if self.way == Way::Undecided && (ax > slop || ay > slop) {
            self.way = if ax > ay {
                Way::Sideways
            } else {
                Way::Vertical
            };
        }
        self.dx = x - self.start[0];
        self.samples.push_back((now, self.dx));
        while self
            .samples
            .front()
            .is_some_and(|&(t, _)| now.saturating_sub(t) > SPEED_WINDOW)
            && self.samples.len() > 2
        {
            self.samples.pop_front();
        }
        self.way
    }

    /// The finger lifted at `now`, at scale `s`: past [`REMOVES`], or
    /// flung the way it was dragged, the card goes.
    pub fn release(&self, now: Duration, s: f32) -> Release {
        if self.dx.abs() >= REMOVES * s {
            return Release::Removes;
        }
        let speed = match (self.samples.front(), self.samples.back()) {
            (Some(&(t0, x0)), Some(&(t1, x1)))
                if t1 > t0 && now.saturating_sub(t1) <= SPEED_WINDOW =>
            {
                (x1 - x0) / (t1 - t0).as_secs_f32()
            }
            _ => 0.0,
        };
        if speed.abs() >= FLING * s && speed * self.dx > 0.0 {
            Release::Removes
        } else {
            Release::SpringsBack
        }
    }
}

/// The offset `dx` after `dt` of springing back; 0 once within half a
/// point.
pub fn spring(dx: f32, dt: Duration, s: f32) -> f32 {
    let next = dx * (-SPRING * dt.as_secs_f32().min(0.1)).exp();
    if next.abs() < 0.5 * s { 0.0 } else { next }
}

/// The card's opacity at offset `dx` on a screen `w` wide: it fades as it
/// goes, to 40%.
pub fn fade(dx: f32, w: f32) -> f32 {
    if w <= 0.0 {
        return 1.0;
    }
    1.0 - (dx.abs() / w).min(0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_way_is_decided_once_past_the_slop() {
        let mut w = Swipe::new(1, 100.0, 100.0, Duration::ZERO);
        assert_eq!(w.moved(105.0, 103.0, MS, 20.0), Way::Undecided);
        assert_eq!(w.dx(), 0.0, "no offset before it is sideways");
        assert_eq!(w.moved(130.0, 110.0, 2 * MS, 20.0), Way::Sideways);
        assert_eq!(w.dx(), 30.0);
        // Decided: going up after does not make it a scroll.
        assert_eq!(w.moved(130.0, 300.0, 3 * MS, 20.0), Way::Sideways);
        let mut v = Swipe::new(1, 100.0, 100.0, Duration::ZERO);
        assert_eq!(v.moved(110.0, 130.0, MS, 20.0), Way::Vertical);
        assert_eq!(v.dx(), 0.0);
    }

    #[test]
    fn far_enough_or_flung_removes_short_springs_back() {
        let s = 2.0;
        // Slowly, 40 pt: removed.
        let mut far = Swipe::new(1, 500.0, 900.0, Duration::ZERO);
        far.moved(500.0 - 81.0, 900.0, 800 * MS, 20.0);
        assert_eq!(far.release(1000 * MS, s), Release::Removes);
        // Slowly, 30 pt: springs back.
        let mut short = Swipe::new(1, 500.0, 900.0, Duration::ZERO);
        short.moved(560.0, 900.0, 800 * MS, 20.0);
        assert_eq!(short.release(810 * MS, s), Release::SpringsBack);
        // 30 pt in 30 ms: a fling, removed.
        let mut flung = Swipe::new(1, 500.0, 900.0, Duration::ZERO);
        flung.moved(530.0, 900.0, 10 * MS, 20.0);
        flung.moved(560.0, 900.0, 30 * MS, 20.0);
        assert_eq!(flung.release(35 * MS, s), Release::Removes);
        // The same, but held still before lifting: springs back.
        assert_eq!(flung.release(300 * MS, s), Release::SpringsBack);
        // Flung back the other way: not the way it was dragged.
        let mut back = Swipe::new(1, 500.0, 900.0, Duration::ZERO);
        back.moved(560.0, 900.0, 500 * MS, 20.0);
        back.moved(530.0, 900.0, 510 * MS, 20.0);
        assert_eq!(back.release(512 * MS, s), Release::SpringsBack);
    }

    #[test]
    fn it_springs_back_to_rest_and_fades_as_it_goes() {
        let mut dx = 60.0;
        let mut frames = 0;
        while dx != 0.0 {
            let next = spring(dx, 16 * MS, 2.0);
            assert!(next.abs() < dx.abs());
            dx = next;
            frames += 1;
            assert!(frames < 60, "at rest within a second");
        }
        assert_eq!(fade(0.0, 1080.0), 1.0);
        assert!(fade(540.0, 1080.0) < fade(100.0, 1080.0));
        assert!((fade(-2000.0, 1080.0) - 0.4).abs() < 1e-6);
    }
}
