//! Whether a session is processing (plan 20261028_03, section 3.6): the
//! rule the drawer's rows are judged by, here so it is tested once.
//!
//! The inputs come from `agents.list`: herdr's status, the session's turn,
//! and the daemon's `idle_ms` and `subagents_running`. `idle_ms` is an age
//! on the daemon's clock as the list was made; [`Signals::received`] turns
//! it into a time on the caller's own clock, so the phone's clock and the
//! daemon's need not agree. The caller judges again with each spinner
//! frame ([`activity`] with a later `now`), so a row stops spinning
//! [`RECENT`] after its last write even between lists.

use std::time::Duration;

use libflankr_proto::common::{AgentInfo, Turn};

use crate::cross::is_terminal_only;

/// A write this recent means something is happening.
pub const RECENT: Duration = Duration::from_secs(10);
/// The longest a `Working` turn whose herdr status is `working` counts as
/// processing with no write: a long build or test run writes nothing
/// until its result.
pub const TOOL_WAIT: Duration = Duration::from_secs(10 * 60);
/// A subagent counts as running while its file was written this recently
/// and its meta does not say finished. The daemon applies it when it
/// counts `subagents_running`; it is here for reference.
pub const SUBAGENT_LIVE: Duration = Duration::from_secs(60);

/// What a session needs from the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedsInput {
    /// A flankr confirmation is pending for the pane.
    Approval,
    /// The agent asks a question (`Turn::Asking`).
    Question,
}

/// A session's activity, as a drawer row shows it (section 3.5, item 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Working: the Braille spinner.
    Processing,
    /// Waiting for the user: a dot in the colour of what it waits for.
    NeedsInput(NeedsInput),
    /// herdr reports the pane `broke` (not a shell): a red dot.
    Broken,
    /// Nothing happening; `since` is the time since the session's last
    /// event, when known.
    Idle { since: Option<Duration> },
}

/// The times a session's activity is judged by, on the caller's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signals {
    /// When the session file (or any of its subagents' files) was last
    /// written; `None` when the daemon does not say.
    pub last_event: Option<Duration>,
    /// The session's running subagents, when the daemon reads them.
    pub subagents_running: Option<u32>,
}

impl Signals {
    /// The signals of a list received at `received_at` on the caller's
    /// clock: `last_event = received_at − idle_ms` (section 3.12, item 46).
    pub fn received(
        idle_ms: Option<u64>,
        subagents_running: Option<u32>,
        received_at: Duration,
    ) -> Signals {
        Signals {
            last_event: idle_ms.map(|ms| received_at.saturating_sub(Duration::from_millis(ms))),
            subagents_running,
        }
    }

    /// The signals of `agent` from a list received at `received_at`.
    pub fn of(agent: &AgentInfo, received_at: Duration) -> Signals {
        Signals::received(agent.idle_ms, agent.subagents_running, received_at)
    }
}

/// The session's activity at `now` (section 3.6, item 22), first match
/// winning: a shell is idle; then needs input (an approval pending for its
/// pane, `approval`, before a question); herdr's `broke`; a running
/// subagent; a write within [`RECENT`]; a `Working` turn that herdr
/// reports `working` within [`TOOL_WAIT`] of the last write; else idle.
/// Without a last write (item 23), the last two are one: a `Working` turn
/// that herdr reports `working`, and the idle age is not known.
pub fn activity(agent: &AgentInfo, approval: bool, signals: Signals, now: Duration) -> Activity {
    let age = signals.last_event.map(|at| now.saturating_sub(at));
    if is_terminal_only(agent) {
        return Activity::Idle { since: age };
    }
    if approval {
        return Activity::NeedsInput(NeedsInput::Approval);
    }
    if matches!(agent.turn, Some(Turn::Asking { .. })) {
        return Activity::NeedsInput(NeedsInput::Question);
    }
    if agent.status == "broke" {
        return Activity::Broken;
    }
    if signals.subagents_running.is_some_and(|n| n > 0) {
        return Activity::Processing;
    }
    let working = matches!(agent.turn, Some(Turn::Working { .. })) && agent.status == "working";
    let processing = match age {
        Some(age) => age <= RECENT || (working && age <= TOOL_WAIT),
        None => working,
    };
    if processing {
        Activity::Processing
    } else {
        Activity::Idle { since: age }
    }
}

/// An idle row's age as it is shown (section 3.5, item 18): `1m` … `59m`,
/// `1h` … `23h`, `1d` …; `None` under a minute, where the app shows its
/// copy for *now*.
pub fn short_age(age: Duration) -> Option<String> {
    let secs = age.as_secs();
    match secs {
        0..60 => None,
        60..3600 => Some(format!("{}m", secs / 60)),
        3600..86_400 => Some(format!("{}h", secs / 3600)),
        _ => Some(format!("{}d", secs / 86_400)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(status: &str, turn: Option<Turn>) -> AgentInfo {
        AgentInfo {
            pane_id: "w1:p1".into(),
            name: None,
            spoken: None,
            agent: "claude".into(),
            project: None,
            cwd: None,
            workspace_id: None,
            tab_id: None,
            tab_label: None,
            tab_number: None,
            status: status.into(),
            focused: false,
            session: None,
            turn,
            idle_ms: None,
            subagents_running: None,
            summary: None,
            children: Vec::new(),
        }
    }

    fn working() -> Option<Turn> {
        Some(Turn::Working { line: "…".into() })
    }

    fn stopped() -> Option<Turn> {
        Some(Turn::Stopped {
            line: "done".into(),
        })
    }

    const NOW: Duration = Duration::from_secs(10_000);

    /// Signals whose last write was `ago` before [`NOW`].
    fn ago(ago: Duration) -> Signals {
        Signals {
            last_event: Some(NOW - ago),
            subagents_running: None,
        }
    }

    fn idle(since: Duration) -> Activity {
        Activity::Idle { since: Some(since) }
    }

    #[test]
    fn a_shell_is_idle_whatever_herdr_says() {
        let mut shell = agent("working", working());
        shell.agent = "shell".into();
        let s = ago(Duration::from_secs(1));
        assert_eq!(activity(&shell, true, s, NOW), idle(Duration::from_secs(1)));
        assert_eq!(
            activity(&shell, false, Signals::default(), NOW),
            Activity::Idle { since: None }
        );
    }

    #[test]
    fn needs_input_comes_before_broken_and_processing() {
        let asking = agent(
            "broke",
            Some(Turn::Asking {
                question: "?".into(),
            }),
        );
        let s = Signals {
            subagents_running: Some(2),
            ..ago(Duration::ZERO)
        };
        assert_eq!(
            activity(&asking, true, s, NOW),
            Activity::NeedsInput(NeedsInput::Approval)
        );
        assert_eq!(
            activity(&asking, false, s, NOW),
            Activity::NeedsInput(NeedsInput::Question)
        );
        let broke = agent("broke", working());
        assert_eq!(activity(&broke, false, s, NOW), Activity::Broken);
    }

    #[test]
    fn a_running_subagent_spins_a_stopped_turn() {
        let a = agent("working", stopped());
        let old = ago(Duration::from_secs(3600));
        assert_eq!(
            activity(&a, false, old, NOW),
            idle(Duration::from_secs(3600))
        );
        let s = Signals {
            subagents_running: Some(1),
            ..old
        };
        assert_eq!(activity(&a, false, s, NOW), Activity::Processing);
        let none = Signals {
            subagents_running: Some(0),
            ..old
        };
        assert_eq!(
            activity(&a, false, none, NOW),
            idle(Duration::from_secs(3600))
        );
    }

    #[test]
    fn a_recent_write_spins_whatever_herdr_says() {
        let a = agent("idle", stopped());
        assert_eq!(activity(&a, false, ago(RECENT), NOW), Activity::Processing);
        let past = RECENT + Duration::from_millis(1);
        assert_eq!(activity(&a, false, ago(past), NOW), idle(past));
        // A write after now (clocks apart) counts as now.
        let ahead = Signals {
            last_event: Some(NOW + Duration::from_secs(5)),
            subagents_running: None,
        };
        assert_eq!(activity(&a, false, ahead, NOW), Activity::Processing);
    }

    #[test]
    fn a_working_turn_spins_for_tool_wait_after_its_last_write() {
        let a = agent("working", working());
        assert_eq!(
            activity(&a, false, ago(TOOL_WAIT), NOW),
            Activity::Processing
        );
        let past = TOOL_WAIT + Duration::from_millis(1);
        assert_eq!(activity(&a, false, ago(past), NOW), idle(past));
        // Either alone is not enough past RECENT.
        let quiet = ago(Duration::from_secs(60));
        let herdr_only = agent("working", stopped());
        assert_eq!(
            activity(&herdr_only, false, quiet, NOW),
            idle(Duration::from_secs(60))
        );
        let turn_only = agent("idle", working());
        assert_eq!(
            activity(&turn_only, false, quiet, NOW),
            idle(Duration::from_secs(60))
        );
    }

    #[test]
    fn without_times_a_working_turn_and_herdr_working_spin() {
        let none = Signals::default();
        assert_eq!(
            activity(&agent("working", working()), false, none, NOW),
            Activity::Processing
        );
        assert_eq!(
            activity(&agent("working", stopped()), false, none, NOW),
            Activity::Idle { since: None }
        );
        assert_eq!(
            activity(&agent("idle", working()), false, none, NOW),
            Activity::Idle { since: None }
        );
        assert_eq!(
            activity(&agent("working", None), false, none, NOW),
            Activity::Idle { since: None }
        );
    }

    #[test]
    fn the_last_event_is_the_received_time_less_the_age() {
        let at = Duration::from_secs(100);
        assert_eq!(
            Signals::received(Some(4_500), Some(1), at),
            Signals {
                last_event: Some(Duration::from_millis(95_500)),
                subagents_running: Some(1),
            }
        );
        assert_eq!(
            Signals::received(Some(200_000), None, at).last_event,
            Some(Duration::ZERO)
        );
        assert_eq!(Signals::received(None, None, at), Signals::default());
        let mut a = agent("working", working());
        a.idle_ms = Some(1_000);
        a.subagents_running = Some(2);
        assert_eq!(
            Signals::of(&a, at),
            Signals {
                last_event: Some(Duration::from_secs(99)),
                subagents_running: Some(2),
            }
        );
    }

    #[test]
    fn ages_are_shown_in_their_largest_unit() {
        let s = Duration::from_secs;
        assert_eq!(short_age(s(59)), None);
        assert_eq!(short_age(s(60)).as_deref(), Some("1m"));
        assert_eq!(short_age(s(3599)).as_deref(), Some("59m"));
        assert_eq!(short_age(s(3600)).as_deref(), Some("1h"));
        assert_eq!(short_age(s(86_399)).as_deref(), Some("23h"));
        assert_eq!(short_age(s(86_400)).as_deref(), Some("1d"));
        assert_eq!(short_age(s(3 * 86_400 + 5)).as_deref(), Some("3d"));
    }
}
