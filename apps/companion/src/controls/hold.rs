//! One timer for every held press in the bottom bar and the input rows
//! (docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md, decision 7): the
//! centre (✦ Gemini) and right (record) buttons, volume, and the text
//! input's +.
//!
//! A press fires after its hold ([`hold_for`]): [`CARD_HOLD`], 500 ms, on
//! the centre and right (the 2026-10-29 14:43 EDT amendment), [`HOLD`],
//! 2 s, on the others; [`SLOP`] is 10 pt. A release before then is a tap;
//! at the hold the press fires at once, before the release, and the
//! release is then no tap. A finger that moves more than
//! the slop, or a cancelled touch, ends the press with neither. While it is
//! timed, the screens draw its progress ([`bar`]) along the control's
//! bottom, with no press colour.
//!
//! A press may also be given an earlier step ([`Hold::with_early`]), as
//! the volume button's 1 s stop while something is being said (the
//! 2026-10-29 00:43 EDT amendment, item 1): it fires once at that time
//! ([`Hold::fire_early`]) and the press goes on to its hold; a release
//! after it is no tap. The bar fills over each step in turn: to the early
//! time, then from it to the hold.

use std::time::Duration;

use libcontrols::Rect;

use crate::agent::AgentHit;

/// How long a press is held to fire.
pub const HOLD: Duration = Duration::from_secs(2);
/// How long a press on the centre or right button is held to open its card
/// typed (the 2026-10-29 14:43 EDT amendment).
pub const CARD_HOLD: Duration = Duration::from_millis(500);
/// The volume button's earlier step while something is being said: a 1 s
/// hold stops it (the 2026-10-29 00:43 EDT amendment, item 1).
pub const STOP_HOLD: Duration = Duration::from_secs(1);
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
    /// How long after the start it fires.
    hold: Duration,
    at: [f32; 2],
    fired: bool,
    moved: bool,
    /// The earlier step, after the start, if the press has one.
    early: Option<Duration>,
    early_fired: bool,
}

impl Hold {
    /// A press of touch `id` on `hit` at `at`, from `now` (any monotonic
    /// clock, the same as later calls are given).
    pub fn start(id: u64, hit: AgentHit, at: [f32; 2], now: Duration) -> Self {
        Hold {
            id,
            hit,
            start: now,
            hold: hold_for(hit),
            at,
            fired: false,
            moved: false,
            early: None,
            early_fired: false,
        }
    }

    /// The press with an earlier step at `after` from its start (less
    /// than its hold): [`Hold::fire_early`] fires once then, and the press
    /// goes on to the hold.
    pub fn with_early(self, after: Duration) -> Self {
        debug_assert!(after < self.hold, "the early step comes before the hold");
        Hold {
            early: Some(after),
            ..self
        }
    }

    /// Whether the press has an earlier step.
    pub fn has_early(&self) -> bool {
        self.early.is_some()
    }

    /// True once, when the earlier step's time has passed unmoved; never
    /// for a press without one.
    pub fn fire_early(&mut self, now: Duration) -> bool {
        let Some(after) = self.early else {
            return false;
        };
        if self.early_fired || self.fired || self.moved || now.saturating_sub(self.start) < after {
            return false;
        }
        self.early_fired = true;
        true
    }

    /// The press's progress in 0..1 through the step it is in (the early
    /// one, then the hold) while it is timed; None once fired or moved.
    pub fn progress(&self, now: Duration) -> Option<f32> {
        if self.fired || self.moved {
            return None;
        }
        let held = now.saturating_sub(self.start);
        let (from, to) = match self.early {
            Some(after) if !self.early_fired => (Duration::ZERO, after),
            Some(after) => (after, self.hold),
            None => (Duration::ZERO, self.hold),
        };
        let t = held.saturating_sub(from).as_secs_f32() / (to - from).as_secs_f32();
        Some(t.clamp(0.0, 1.0))
    }

    /// True once, when its hold has passed unmoved.
    pub fn fire(&mut self, now: Duration) -> bool {
        if self.fired || self.moved || now.saturating_sub(self.start) < self.hold {
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

    /// The release: a tap if it neither fired (either step) nor moved.
    pub fn is_tap(&self) -> bool {
        !self.fired && !self.early_fired && !self.moved
    }

    /// Whether it fired.
    pub fn fired(&self) -> bool {
        self.fired
    }
}

/// How long a press on `hit` is held to fire: [`CARD_HOLD`] for the
/// centre and right buttons, [`HOLD`] for the others.
pub fn hold_for(hit: AgentHit) -> Duration {
    match hit {
        AgentHit::GeneralAgent | AgentHit::Message => CARD_HOLD,
        _ => HOLD,
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
        let mut h = Hold::start(1, AgentHit::Volume, [100.0, 100.0], T0);
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
    fn the_centre_and_right_fire_at_500_ms() {
        let ms = Duration::from_millis;
        for hit in [AgentHit::GeneralAgent, AgentHit::Message] {
            let mut h = Hold::start(1, hit, [100.0, 100.0], T0);
            assert_eq!(h.progress(T0 + ms(250)), Some(0.5));
            assert!(!h.fire(T0 + ms(499)));
            assert!(h.is_tap(), "released before 500 ms: a tap");
            assert!(h.fire(T0 + CARD_HOLD));
            assert!(!h.is_tap());
        }
        assert_eq!(hold_for(AgentHit::Volume), HOLD);
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
    fn an_early_step_fires_once_and_the_press_goes_on_to_the_hold() {
        let ms = Duration::from_millis;
        let mut h = Hold::start(1, AgentHit::Volume, [100.0, 100.0], T0).with_early(STOP_HOLD);
        assert!(h.has_early());
        // The bar fills over the first second.
        assert_eq!(h.progress(T0 + ms(500)), Some(0.5));
        assert!(!h.fire_early(T0 + ms(999)));
        assert!(h.is_tap(), "released before 1 s: a tap");
        assert!(!h.fire(T0 + ms(999)));
        assert!(h.fire_early(T0 + STOP_HOLD));
        assert!(!h.fire_early(T0 + ms(1500)), "once");
        assert!(!h.is_tap(), "released after 1 s: no tap");
        // Then it fills again, from 1 s to 2 s, and fires the hold.
        assert_eq!(h.progress(T0 + STOP_HOLD), Some(0.0));
        assert_eq!(h.progress(T0 + ms(1500)), Some(0.5));
        assert!(!h.fire(T0 + ms(1999)));
        assert!(h.fire(T0 + HOLD));
        assert_eq!(h.progress(T0 + HOLD), None);
        // Both due at once (a late frame): each fires.
        let mut late = Hold::start(2, AgentHit::Volume, [0.0, 0.0], T0).with_early(STOP_HOLD);
        assert!(late.fire_early(T0 + ms(2500)));
        assert!(late.fire(T0 + ms(2500)));
    }

    #[test]
    fn a_press_without_an_early_step_never_fires_one() {
        let mut h = Hold::start(1, AgentHit::Volume, [0.0, 0.0], T0);
        assert!(!h.has_early());
        assert!(!h.fire_early(T0 + HOLD));
        assert!(h.is_tap());
        // A move cancels the early step too.
        let mut m = Hold::start(1, AgentHit::Volume, [0.0, 0.0], T0).with_early(STOP_HOLD);
        m.moved_to([0.0, 50.0], 1.0);
        assert!(!m.fire_early(T0 + STOP_HOLD));
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
