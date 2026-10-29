//! One timer for every 2 s press in the bottom bar and the input rows
//! (docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md, decision 7): the
//! centre (✦ Gemini) and right (record) buttons, and the text input's +.
//!
//! As the drawer's remote card: [`HOLD`] is 2 s and [`SLOP`] 10 pt. A
//! release before 2 s is a tap; at 2 s the press fires at once, before the
//! release, and the release is then no tap. A finger that moves more than
//! the slop, or a cancelled touch, ends the press with neither. While it is
//! timed, the screens draw its progress ([`bar`]) along the control's
//! bottom, with no press colour.

use std::time::Duration;

use libcontrols::Rect;

use crate::agent::AgentHit;

/// How long a press is held to fire.
pub const HOLD: Duration = Duration::from_secs(2);
/// How far a finger moves, in points, before a press is no tap.
pub const SLOP: f32 = 10.0; // points

/// A press on a control that a 2 s hold gives a second action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    /// The touch.
    pub id: u64,
    /// The control pressed.
    pub hit: AgentHit,
    start: Duration,
    at: [f32; 2],
    fired: bool,
    moved: bool,
}

impl Hold {
    /// A press of touch `id` on `hit` at `at`, from `now` (any monotonic
    /// clock, the same as later calls are given).
    pub fn start(id: u64, hit: AgentHit, at: [f32; 2], now: Duration) -> Self {
        Hold {
            id,
            hit,
            start: now,
            at,
            fired: false,
            moved: false,
        }
    }

    /// The press's progress in 0..1 while it is timed; None once fired or
    /// moved.
    pub fn progress(&self, now: Duration) -> Option<f32> {
        if self.fired || self.moved {
            return None;
        }
        let t = now.saturating_sub(self.start).as_secs_f32() / HOLD.as_secs_f32();
        Some(t.clamp(0.0, 1.0))
    }

    /// True once, when 2 s have passed unmoved.
    pub fn fire(&mut self, now: Duration) -> bool {
        if self.fired || self.moved || now.saturating_sub(self.start) < HOLD {
            return false;
        }
        self.fired = true;
        true
    }

    /// A move: past SLOP (scaled by `s`) the press is no tap and never
    /// fires.
    pub fn moved_to(&mut self, at: [f32; 2], s: f32) {
        let slop = SLOP * s;
        if (at[0] - self.at[0]).abs() > slop || (at[1] - self.at[1]).abs() > slop {
            self.moved = true;
        }
    }

    /// The release: a tap if it neither fired nor moved.
    pub fn is_tap(&self) -> bool {
        !self.fired && !self.moved
    }

    /// Whether it fired.
    pub fn fired(&self) -> bool {
        self.fired
    }
}

/// The progress bar of a press on `rect` at `progress` (0..1), at scale
/// `s`: 3 pt high, inset 6 pt from the control's sides and bottom, filled
/// left to right (decision 7).
pub fn bar(rect: Rect, progress: f32, s: f32) -> Rect {
    let inset = 6.0 * s;
    let h = 3.0 * s;
    let w = (rect.w - 2.0 * inset).max(0.0) * progress.clamp(0.0, 1.0);
    Rect::new(rect.x + inset, rect.y + rect.h - inset - h, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Duration = Duration::from_secs(10);

    #[test]
    fn a_release_before_2_s_is_a_tap_and_2_s_fires_once() {
        let mut h = Hold::start(1, AgentHit::Message, [100.0, 100.0], T0);
        assert_eq!(h.progress(T0), Some(0.0));
        assert_eq!(h.progress(T0 + Duration::from_secs(1)), Some(0.5));
        assert!(!h.fire(T0 + Duration::from_millis(1999)));
        assert!(h.is_tap());
        assert!(h.fire(T0 + HOLD));
        assert!(!h.fire(T0 + Duration::from_secs(3)), "once");
        assert!(!h.is_tap(), "the release after it fired is no tap");
        assert_eq!(h.progress(T0 + HOLD), None);
    }

    #[test]
    fn a_move_past_the_slop_cancels_both() {
        let s = 2.0;
        let mut h = Hold::start(1, AgentHit::GeneralAgent, [100.0, 100.0], T0);
        // 10 pt is still the press.
        h.moved_to([100.0 + 10.0 * s, 100.0], s);
        assert!(h.is_tap());
        // 11 pt is not.
        h.moved_to([100.0, 100.0 - 11.0 * s], s);
        assert!(!h.is_tap());
        assert!(!h.fire(T0 + HOLD));
        assert_eq!(h.progress(T0 + Duration::from_secs(1)), None);
    }

    #[test]
    fn the_bar_is_inset_along_the_bottom_and_fills_left_to_right() {
        let s = 2.0;
        let r = Rect::new(10.0, 20.0, 104.0, 104.0);
        let half = bar(r, 0.5, s);
        assert_eq!(half.x, 22.0);
        assert_eq!(half.h, 6.0);
        assert_eq!(half.y + half.h, 20.0 + 104.0 - 12.0);
        assert_eq!(half.w, (104.0 - 24.0) * 0.5);
        assert_eq!(bar(r, 1.0, s).w, 80.0);
        assert_eq!(bar(r, 0.0, s).w, 0.0);
    }
}
