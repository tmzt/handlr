//! The foreground service's notification and the alerts (plan 20261024_02;
//! CHANGES.md, 2026-10-24 12:38 EDT, "Background and Notifications as two
//! sections", and 12:59 EDT, item 3, "the notification shows the
//! session's state").
//!
//! With *Stay connected in the background* on, the app starts libmorph's
//! foreground service while it is on screen and paired ([`Notifier::start`]),
//! so the link survives in the background. Its ongoing notification shows
//! the session ([`Model::content`]):
//!
//! - **title:** the agent that matters most (needs you, then working, then
//!   done), or with several, "handlr · 1 needs you, 2 working";
//! - **text:** that agent's state as the jump line gives it: `question: …`,
//!   `approval: …`, `working: <newest line>`, `done: <summary>`;
//! - **all idle:** "All agents idle · n MB this session", from the client's
//!   count of bytes over the link;
//! - **updates:** at most every [`MIN_GAP`], only on a change ([`Throttle`]):
//!   `update_text` when only the text changed, `update` when the title did.
//!
//! "Done" is an agent whose turn ended, from working, since handlr was last
//! on screen; coming back to the screen clears it.
//!
//! Android runs none of the app's frames in the background, so the
//! notifier follows the client's events on the `net` thread
//! (`Companion::spawn_tapped`, [`Notifier::tap`]) and updates the
//! notification from a thread of its own, `notify`.
//!
//! With *Alert when an agent needs you* on, while the app is not on screen,
//! a question, an approval or a finished turn is an [`Alert`], given to
//! [`Host::alert`]. libmorph 0.4.1 has no notification apart from the
//! service's, so [`MorphHost`] only logs it, and the ongoing notification
//! shows it with the next update.
//!
//! The templates are `notification_*` in `SETTINGS_COPY.md`.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use handlr_companion::Event;
use libhandlr_proto::common::{AgentInfo, Turn};

use crate::overview::agent_card::AgentCard;
use crate::settings::copy;

/// The least time between two updates of the notification.
pub const MIN_GAP: Duration = Duration::from_secs(2);

/// The longest text the notification shows of a line, in characters.
pub const LINE_MAX: usize = 120;

/// The title with no agent to name: the app's own name.
pub const APP_TITLE: &str = "handlr";

/// The `specialUse` reason, as build.rs declares it in the manifest.
pub const SERVICE_REASON: &str = "Keeps the encrypted link to the user's own handlr daemon open \
     while the app is in the background, so that coding agents' questions, approvals and \
     finished turns reach the phone.";

/// What the notification shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub title: String,
    pub text: String,
}

/// What one agent is doing, in the order the title picks from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    Question(String),
    Approval(String),
    Working(String),
    Done(String),
}

impl State {
    fn needs_you(&self) -> bool {
        matches!(self, State::Question(_) | State::Approval(_))
    }

    /// The line, as the jump line's states read.
    fn text(&self) -> String {
        match self {
            State::Question(q) => {
                copy::NOTIFICATION_TEXT_QUESTION.fill(None, &[("question", &one_line(q))])
            }
            State::Approval(a) => {
                copy::NOTIFICATION_TEXT_APPROVAL.fill(None, &[("approval", &one_line(a))])
            }
            State::Working(l) => {
                copy::NOTIFICATION_TEXT_WORKING.fill(None, &[("line", &one_line(l))])
            }
            State::Done(s) => copy::NOTIFICATION_TEXT_DONE.fill(None, &[("summary", &one_line(s))]),
        }
        .trim()
        .to_string()
    }
}

/// What called for an alert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Question,
    Approval,
    Finished,
}

/// A question, an approval or a finished turn, while handlr is not on
/// screen: its agent's pane, the agent's name, and the state's line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub kind: AlertKind,
    pub pane_id: String,
    pub title: String,
    pub text: String,
}

/// `text`'s first line that is not blank, trimmed, cut to [`LINE_MAX`]
/// characters with an ellipsis.
pub fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() <= LINE_MAX {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(LINE_MAX - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// `bytes` in MB, as the idle text gives it: one decimal under 10, and
/// "0" for none.
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if bytes == 0 {
        "0".to_string()
    } else if mb < 10.0 {
        format!("{mb:.1}")
    } else {
        format!("{mb:.0}")
    }
}

/// An agent's name in the notification: its shown name, else its card's
/// heading.
fn agent_name(agent: &AgentInfo) -> String {
    agent
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            AgentCard {
                agent,
                approval: false,
            }
            .heading()
        })
}

/// What the notification knows of the session, from the client's events.
#[derive(Debug, Default)]
pub struct Model {
    agents: Vec<AgentInfo>,
    /// The panes with a handlr confirmation waiting.
    pending: Vec<String>,
    /// The confirmation being asked, and its command.
    confirm: Option<(u64, String)>,
    /// The panes whose turn ended, from working, since handlr was last on
    /// screen; the newest last.
    done: Vec<String>,
    bytes: u64,
    /// A list of agents has come: the next is compared with it.
    listed: bool,
}

impl Model {
    /// Takes in `event`, with the link's bytes so far. Returns the alerts
    /// it calls for: a new question, a new approval, or a turn that ended
    /// from working. The first list of agents calls for none.
    pub fn apply(&mut self, event: &Event, bytes: u64) -> Vec<Alert> {
        self.bytes = bytes;
        let mut alerts = Vec::new();
        match event {
            Event::Agents(agents) => {
                let first = !std::mem::replace(&mut self.listed, true);
                for agent in agents {
                    let before = self.agents.iter().find(|a| a.pane_id == agent.pane_id);
                    let was = before.and_then(|a| a.turn.as_ref());
                    match (&agent.turn, was) {
                        (Some(Turn::Asking { question }), was)
                            if !first
                                && !matches!(was, Some(Turn::Asking { question: q }) if q == question) =>
                        {
                            alerts.push(self.alert(AlertKind::Question, agent, question));
                        }
                        (Some(Turn::Stopped { line }), Some(Turn::Working { .. })) => {
                            self.done.retain(|p| p != &agent.pane_id);
                            self.done.push(agent.pane_id.clone());
                            alerts.push(self.alert(AlertKind::Finished, agent, line));
                        }
                        _ => {}
                    }
                }
                self.agents = agents.clone();
                // A pane gone, or working again, is no longer done.
                let agents = &self.agents;
                self.done.retain(|p| {
                    agents
                        .iter()
                        .any(|a| &a.pane_id == p && matches!(a.turn, Some(Turn::Stopped { .. })))
                });
            }
            Event::PendingPanes(panes) => {
                for pane in panes.iter().filter(|p| !self.pending.contains(p)) {
                    if let Some(agent) = self.agents.iter().find(|a| &a.pane_id == pane) {
                        let what = self.approval_text();
                        alerts.push(self.alert(AlertKind::Approval, agent, &what));
                    }
                }
                self.pending = panes.clone();
            }
            Event::Confirm {
                confirm_id,
                command,
                ..
            } => self.confirm = Some((*confirm_id, command.clone())),
            Event::Answered { confirm_id, .. }
            | Event::ReplyFailed { confirm_id, .. }
            | Event::Resolved { confirm_id, .. } => {
                if self
                    .confirm
                    .as_ref()
                    .is_some_and(|(id, _)| id == confirm_id)
                {
                    self.confirm = None;
                }
            }
            Event::Unpaired { .. } => *self = Model::default(),
            _ => {}
        }
        alerts
    }

    fn alert(&self, kind: AlertKind, agent: &AgentInfo, line: &str) -> Alert {
        let state = match kind {
            AlertKind::Question => State::Question(line.to_string()),
            AlertKind::Approval => State::Approval(line.to_string()),
            AlertKind::Finished => State::Done(line.to_string()),
        };
        Alert {
            kind,
            pane_id: agent.pane_id.clone(),
            title: agent_name(agent),
            text: state.text(),
        }
    }

    /// An approval's line: the command being asked, when it is known.
    fn approval_text(&self) -> String {
        self.confirm
            .as_ref()
            .map(|(_, c)| c.clone())
            .unwrap_or_default()
    }

    /// handlr is on screen: the finished turns have been seen.
    pub fn seen(&mut self) {
        self.done.clear();
    }

    /// Each agent with something to say, and what.
    fn states(&self) -> Vec<(&AgentInfo, State)> {
        self.agents
            .iter()
            .filter_map(|a| {
                let state = if self.pending.contains(&a.pane_id) {
                    State::Approval(self.approval_text())
                } else {
                    match &a.turn {
                        Some(Turn::Asking { question }) => State::Question(question.clone()),
                        Some(Turn::Working { line }) => State::Working(line.clone()),
                        Some(Turn::Stopped { line }) if self.done.contains(&a.pane_id) => {
                            State::Done(line.clone())
                        }
                        _ => return None,
                    }
                };
                Some((a, state))
            })
            .collect()
    }

    /// What the notification shows now.
    pub fn content(&self) -> Content {
        let states = self.states();
        let needs = states.iter().filter(|(_, s)| s.needs_you()).count();
        let working = states
            .iter()
            .filter(|(_, s)| matches!(s, State::Working(_)))
            .count();
        // The most important; of several done, the newest.
        let top = states.iter().min_by(|(a, x), (b, y)| {
            let rank = |s: &State| match s {
                State::Question(_) => 0,
                State::Approval(_) => 1,
                State::Working(_) => 2,
                State::Done(_) => 3,
            };
            rank(x).cmp(&rank(y)).then_with(|| {
                let at = |p: &str| self.done.iter().position(|d| d == p);
                at(&b.pane_id).cmp(&at(&a.pane_id))
            })
        });
        let Some((agent, state)) = top else {
            return Content {
                title: APP_TITLE.to_string(),
                text: copy::NOTIFICATION_TEXT_IDLE
                    .fill(None, &[("megabytes", &megabytes(self.bytes))])
                    .trim()
                    .to_string(),
            };
        };
        let title = if needs + working > 1 {
            copy::NOTIFICATION_TITLE_SEVERAL
                .fill(
                    None,
                    &[
                        ("needs", &needs.to_string()),
                        ("working", &working.to_string()),
                    ],
                )
                .trim()
                .to_string()
        } else {
            agent_name(agent)
        };
        Content {
            title,
            text: state.text(),
        }
    }
}

/// How to bring the notification up to date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// Only the text changed: `update_text`.
    Text(String),
    /// The title changed: `update`.
    All(Content),
}

/// What [`Throttle::next`] says to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The notification shows it already.
    Same,
    /// Send this now.
    Send(Update),
    /// Too soon after the last: ask again after this long.
    Wait(Duration),
}

/// At most one update every `gap`, and only on a change.
#[derive(Clone, Debug)]
pub struct Throttle {
    gap: Duration,
    /// What the notification shows, and when it was sent.
    shown: Option<(Content, Duration)>,
}

impl Throttle {
    pub fn new(gap: Duration) -> Self {
        Throttle { gap, shown: None }
    }

    /// The service started at `now` showing `content`.
    pub fn started(&mut self, content: Content, now: Duration) {
        self.shown = Some((content, now));
    }

    /// The service stopped: nothing is shown.
    pub fn stopped(&mut self) {
        self.shown = None;
    }

    /// What to do to show `content` at `now`.
    pub fn next(&mut self, content: &Content, now: Duration) -> Step {
        let Some((shown, at)) = &self.shown else {
            return Step::Same;
        };
        if shown == content {
            return Step::Same;
        }
        let due = *at + self.gap;
        if now < due {
            return Step::Wait(due - now);
        }
        let update = if shown.title == content.title {
            Update::Text(content.text.clone())
        } else {
            Update::All(content.clone())
        };
        self.shown = Some((content.clone(), now));
        Step::Send(update)
    }
}

/// libmorph's foreground service, or a test's fake. Called from any
/// thread.
pub trait Host: Send + Sync + 'static {
    fn start(&self, content: &Content) -> Result<(), String>;
    fn update(&self, update: &Update);
    fn stop(&self);
    fn alert(&self, alert: &Alert);
}

/// libmorph 0.4.1's service: `specialUse`, ongoing, with Exit and the
/// wake-lock toggle.
pub struct MorphHost;

impl Host for MorphHost {
    fn start(&self, content: &Content) -> Result<(), String> {
        use libmorph::service::{self, Controls, NotificationContent, ServiceOptions, ServiceType};
        let mut options = ServiceOptions::new(
            vec![ServiceType::SpecialUse(SERVICE_REASON)],
            NotificationContent::new(&content.title, &content.text),
        );
        let mut controls = Controls::default();
        controls.ongoing = true;
        controls.exit = true;
        controls.wake_lock_toggle = true;
        options.controls = controls;
        service::start(&options).map_err(|e| e.to_string())
    }

    fn update(&self, update: &Update) {
        use libmorph::service::{self, NotificationContent};
        let done = match update {
            Update::Text(text) => service::update_text(text),
            Update::All(c) => service::update(&NotificationContent::new(&c.title, &c.text)),
        };
        if let Err(e) = done {
            log::warn!("service: updating the notification: {e}");
        }
    }

    fn stop(&self) {
        if let Err(e) = libmorph::service::stop() {
            log::warn!("service: stopping: {e}");
        }
    }

    fn alert(&self, alert: &Alert) {
        log::info!(
            "alert: {:?}; libmorph 0.4.1 has no separate notification, so the ongoing one shows it",
            alert.kind
        );
    }
}

/// What the app and the `net` thread share with the `notify` thread.
struct Shared {
    model: Model,
    throttle: Throttle,
    /// The service is in the foreground (`ServiceEvent::Started`).
    running: bool,
    /// handlr is on screen.
    visible: bool,
    /// *Alert when an agent needs you*, with the background connection on.
    alerts: bool,
    /// Something changed since the `notify` thread last looked.
    dirty: bool,
    quit: bool,
}

/// The notification's keeper: the model, fed by [`Notifier::tap`]; the
/// service's start and stop; and the `notify` thread's updates. Dropping
/// it ends the thread.
pub struct Notifier {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    host: Arc<dyn Host>,
    clock: Instant,
}

/// Locks `shared`, through a panic on another thread.
fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

impl Notifier {
    /// A notifier on `host`, updating at most every `gap`.
    pub fn spawn(host: Arc<dyn Host>, gap: Duration) -> Self {
        let shared = Arc::new((
            Mutex::new(Shared {
                model: Model::default(),
                throttle: Throttle::new(gap),
                running: false,
                visible: true,
                alerts: false,
                dirty: false,
                quit: false,
            }),
            Condvar::new(),
        ));
        let clock = Instant::now();
        let (thread_shared, thread_host) = (Arc::clone(&shared), Arc::clone(&host));
        let spawned = std::thread::Builder::new()
            .name("notify".into())
            .spawn(move || run(&thread_shared, thread_host.as_ref(), clock));
        if let Err(e) = spawned {
            log::error!("notify: starting the thread: {e}");
        }
        Notifier {
            shared,
            host,
            clock,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        let (mutex, wake) = &*self.shared;
        let r = f(&mut lock(mutex));
        wake.notify_all();
        r
    }

    /// The function for `Companion::spawn_tapped`: each event into the
    /// model, and its alerts to the host while handlr is off screen with
    /// alerts on.
    pub fn tap(&self) -> impl Fn(&Event, u64) + Send + 'static {
        let shared = Arc::clone(&self.shared);
        let host = Arc::clone(&self.host);
        move |event, bytes| {
            let (mutex, wake) = &*shared;
            let mut s = lock(mutex);
            let alerts = s.model.apply(event, bytes);
            let alerting = s.alerts && !s.visible;
            s.dirty = true;
            drop(s);
            wake.notify_all();
            if alerting {
                for alert in &alerts {
                    host.alert(alert);
                }
            }
        }
    }

    /// Starts the service, showing the session as it is. Only while handlr
    /// is on screen; Android refuses it otherwise.
    pub fn start(&self) -> Result<(), String> {
        let now = self.clock.elapsed();
        self.with(|s| {
            let content = s.model.content();
            self.host.start(&content)?;
            s.throttle.started(content, now);
            Ok(())
        })
    }

    /// Stops the service and removes its notification.
    pub fn stop(&self) {
        self.host.stop();
        self.running(false);
    }

    /// The service is in the foreground, or no longer.
    pub fn running(&self, running: bool) {
        self.with(|s| {
            s.running = running;
            if !running {
                s.throttle.stopped();
            }
            s.dirty = true;
        });
    }

    /// handlr came on screen (`true`): finished turns are seen; or left it.
    pub fn visible(&self, visible: bool) {
        self.with(|s| {
            s.visible = visible;
            if visible {
                s.model.seen();
            }
            s.dirty = true;
        });
    }

    /// Whether alerts are given: *Alert when an agent needs you* with
    /// *Stay connected in the background*.
    pub fn set_alerts(&self, on: bool) {
        self.with(|s| s.alerts = on);
    }

    /// What the notification would show now.
    pub fn content(&self) -> Content {
        self.with(|s| s.model.content())
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        self.with(|s| s.quit = true);
    }
}

/// The `notify` thread: on each change, the update the throttle allows,
/// while the service runs.
fn run(shared: &(Mutex<Shared>, Condvar), host: &dyn Host, clock: Instant) {
    let (mutex, wake) = shared;
    let mut s = lock(mutex);
    loop {
        while !s.dirty && !s.quit {
            s = wake.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        if s.quit {
            return;
        }
        if !s.running {
            s.dirty = false;
            continue;
        }
        let content = s.model.content();
        match s.throttle.next(&content, clock.elapsed()) {
            Step::Same => s.dirty = false,
            Step::Send(update) => {
                s.dirty = false;
                drop(s);
                host.update(&update);
                s = lock(mutex);
            }
            Step::Wait(d) => {
                s = wake
                    .wait_timeout(s, d)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::agent_card::tests::agent;

    fn working(line: &str) -> Option<Turn> {
        Some(Turn::Working {
            line: line.to_string(),
        })
    }

    fn stopped(line: &str) -> Option<Turn> {
        Some(Turn::Stopped {
            line: line.to_string(),
        })
    }

    fn asking(q: &str) -> Option<Turn> {
        Some(Turn::Asking {
            question: q.to_string(),
        })
    }

    fn named(pane: &str, name: &str, turn: Option<Turn>) -> AgentInfo {
        AgentInfo {
            name: Some(name.to_string()),
            ..agent(pane, "w1", 1, turn)
        }
    }

    #[test]
    fn the_service_reason_is_build_rs_s() {
        let build = include_str!("../build.rs");
        let start = build
            .find("const SERVICE_REASON: &str = ")
            .expect("build.rs declares it");
        let declared = &build[start..];
        let end = declared.find("\";").expect("its end");
        // The same lines, joined as Rust joins them.
        let joined: String = declared[..end]
            .split_once('"')
            .unwrap()
            .1
            .split("\\\n")
            .map(str::trim_start)
            .collect();
        assert_eq!(joined, SERVICE_REASON);
    }

    #[test]
    fn all_idle_is_the_session_s_traffic() {
        let mut m = Model::default();
        assert_eq!(
            m.content(),
            Content {
                title: "handlr".into(),
                text: "All agents idle \u{b7} 0 MB this session".into()
            }
        );
        m.apply(
            &Event::Agents(vec![named("p1", "A", stopped("ok"))]),
            2_621_440,
        );
        assert_eq!(
            m.content().text,
            "All agents idle \u{b7} 2.5 MB this session"
        );
        assert_eq!(megabytes(15 * 1024 * 1024), "15");
    }

    #[test]
    fn one_agent_names_itself_and_says_its_state_as_the_jump_line() {
        let mut m = Model::default();
        m.apply(
            &Event::Agents(vec![named(
                "p1",
                "Claude in handlr",
                working("Reading src/lib.rs"),
            )]),
            0,
        );
        assert_eq!(
            m.content(),
            Content {
                title: "Claude in handlr".into(),
                text: "working: Reading src/lib.rs".into()
            }
        );
        m.apply(
            &Event::Agents(vec![named(
                "p1",
                "Claude in handlr",
                asking("Push it?\nOr wait?"),
            )]),
            0,
        );
        assert_eq!(m.content().text, "question: Push it?");
        // A long line is cut to one.
        let long = "x".repeat(300);
        m.apply(&Event::Agents(vec![named("p1", "C", working(&long))]), 0);
        let text = m.content().text;
        assert!(
            text.ends_with('\u{2026}') && text.chars().count() <= LINE_MAX + 9,
            "{text}"
        );
    }

    #[test]
    fn several_are_counted_needs_you_first() {
        let mut m = Model::default();
        m.apply(
            &Event::Agents(vec![
                named("p1", "A", working("building")),
                named("p2", "B", asking("Which one?")),
                named("p3", "C", working("testing")),
            ]),
            0,
        );
        assert_eq!(
            m.content(),
            Content {
                title: "handlr \u{b7} 1 needs you, 2 working".into(),
                text: "question: Which one?".into()
            }
        );
        // An approval needs you too, and shows its command.
        m.apply(
            &Event::Confirm {
                confirm_id: 7,
                text: String::new(),
                label: None,
                phrase: String::new(),
                command: "cargo publish".into(),
                agent: "A".into(),
                project: None,
            },
            0,
        );
        m.apply(&Event::PendingPanes(vec!["p1".into()]), 0);
        let c = m.content();
        assert_eq!(c.title, "handlr \u{b7} 2 needs you, 1 working");
        assert_eq!(
            c.text, "question: Which one?",
            "a question before an approval"
        );
        m.apply(
            &Event::Agents(vec![named("p1", "A", working("building"))]),
            0,
        );
        assert_eq!(
            m.content(),
            Content {
                title: "A".into(),
                text: "approval: cargo publish".into()
            }
        );
    }

    #[test]
    fn done_is_a_turn_that_ended_while_away_until_handlr_is_seen() {
        let mut m = Model::default();
        m.apply(&Event::Agents(vec![named("p1", "A", working("x"))]), 0);
        let alerts = m.apply(
            &Event::Agents(vec![named("p1", "A", stopped("Pushed main."))]),
            0,
        );
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::Finished);
        assert_eq!(
            m.content(),
            Content {
                title: "A".into(),
                text: "done: Pushed main.".into()
            }
        );
        m.seen();
        assert!(m.content().text.starts_with("All agents idle"));
        // Stopped from the start was never seen to finish.
        let mut m = Model::default();
        m.apply(&Event::Agents(vec![named("p1", "A", stopped("old"))]), 0);
        assert!(m.content().text.starts_with("All agents idle"));
    }

    #[test]
    fn alerts_are_new_questions_approvals_and_finished_turns_only() {
        let mut m = Model::default();
        // The first list is the state as it was: no alert.
        assert!(
            m.apply(&Event::Agents(vec![named("p1", "A", asking("Go?"))]), 0)
                .is_empty()
        );
        // A new agent that asks at once, after an empty list: an alert.
        let mut empty = Model::default();
        empty.apply(&Event::Agents(vec![]), 0);
        let a = empty.apply(&Event::Agents(vec![named("p9", "N", asking("Hi?"))]), 0);
        assert_eq!(a.len(), 1);
        // The same question again: none.
        assert!(
            m.apply(&Event::Agents(vec![named("p1", "A", asking("Go?"))]), 0)
                .is_empty()
        );
        let a = m.apply(
            &Event::Agents(vec![named("p1", "A", asking("And now?"))]),
            0,
        );
        assert_eq!(
            a,
            [Alert {
                kind: AlertKind::Question,
                pane_id: "p1".into(),
                title: "A".into(),
                text: "question: And now?".into()
            }]
        );
        let a = m.apply(&Event::PendingPanes(vec!["p1".into()]), 0);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, AlertKind::Approval);
        assert!(
            m.apply(&Event::PendingPanes(vec!["p1".into()]), 0)
                .is_empty()
        );
        // Working is not an alert.
        assert!(
            m.apply(&Event::Agents(vec![named("p1", "A", working("w"))]), 0)
                .is_empty()
        );
    }

    #[test]
    fn the_throttle_sends_changes_at_most_every_gap() {
        let gap = Duration::from_secs(2);
        let mut t = Throttle::new(gap);
        let c = |title: &str, text: &str| Content {
            title: title.into(),
            text: text.into(),
        };
        // Nothing shown, nothing to update.
        assert_eq!(t.next(&c("a", "1"), Duration::ZERO), Step::Same);
        t.started(c("a", "1"), Duration::ZERO);
        assert_eq!(t.next(&c("a", "1"), Duration::from_secs(9)), Step::Same);
        assert_eq!(
            t.next(&c("a", "2"), Duration::from_millis(500)),
            Step::Wait(Duration::from_millis(1500))
        );
        assert_eq!(
            t.next(&c("a", "2"), Duration::from_secs(2)),
            Step::Send(Update::Text("2".into()))
        );
        assert_eq!(
            t.next(&c("b", "2"), Duration::from_secs(3)),
            Step::Wait(Duration::from_secs(1))
        );
        assert_eq!(
            t.next(&c("b", "2"), Duration::from_secs(4)),
            Step::Send(Update::All(c("b", "2")))
        );
        t.stopped();
        assert_eq!(t.next(&c("c", "3"), Duration::from_secs(9)), Step::Same);
    }

    /// The service as a list of what was asked of it.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
    }

    impl Host for Fake {
        fn start(&self, c: &Content) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("start {}: {}", c.title, c.text));
            Ok(())
        }
        fn update(&self, u: &Update) {
            let call = match u {
                Update::Text(t) => format!("text {t}"),
                Update::All(c) => format!("update {}: {}", c.title, c.text),
            };
            self.calls.lock().unwrap().push(call);
        }
        fn stop(&self) {
            self.calls.lock().unwrap().push("stop".into());
        }
        fn alert(&self, a: &Alert) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("alert {:?}", a.kind));
        }
    }

    fn calls(f: &Fake) -> Vec<String> {
        f.calls.lock().unwrap().clone()
    }

    /// Waits up to 2 s for `f` to have made `n` calls.
    fn wait_for(f: &Fake, n: usize) -> Vec<String> {
        let until = Instant::now() + Duration::from_secs(2);
        while calls(f).len() < n && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        calls(f)
    }

    #[test]
    fn the_notifier_updates_from_the_tap_throttled_and_only_while_running() {
        let fake = Arc::new(Fake::default());
        let n = Notifier::spawn(fake.clone(), Duration::from_millis(100));
        let tap = n.tap();
        tap(&Event::Agents(vec![named("p1", "A", working("one"))]), 0);
        std::thread::sleep(Duration::from_millis(50));
        assert!(calls(&fake).is_empty(), "nothing before the service starts");
        n.start().unwrap();
        n.running(true);
        assert_eq!(calls(&fake), ["start A: working: one"]);
        // Three changes inside the gap: one update, with the last.
        for line in ["two", "three", "four"] {
            tap(&Event::Agents(vec![named("p1", "A", working(line))]), 0);
        }
        let seen = wait_for(&fake, 2);
        assert_eq!(seen[1], "text working: four");
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(calls(&fake).len(), 2, "no change, no update");
        // A new title is a full update.
        tap(&Event::Agents(vec![named("p1", "B", working("four"))]), 0);
        assert_eq!(wait_for(&fake, 3)[2], "update B: working: four");
        n.stop();
        tap(&Event::Agents(vec![named("p1", "B", working("five"))]), 0);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(calls(&fake)[3..], ["stop"], "nothing after stopping");
    }

    #[test]
    fn alerts_go_to_the_host_only_off_screen_with_alerts_on() {
        let fake = Arc::new(Fake::default());
        let n = Notifier::spawn(fake.clone(), Duration::from_millis(100));
        let tap = n.tap();
        tap(&Event::Agents(vec![named("p1", "A", working("w"))]), 0);
        let ask = |q: &str| Event::Agents(vec![named("p1", "A", asking(q))]);
        n.set_alerts(true);
        tap(&ask("On screen?"), 0);
        assert!(calls(&fake).is_empty(), "on screen: no alert");
        n.visible(false);
        n.set_alerts(false);
        tap(&ask("Alerts off?"), 0);
        assert!(calls(&fake).is_empty(), "alerts off: no alert");
        n.set_alerts(true);
        tap(&ask("Away?"), 0);
        assert_eq!(calls(&fake), ["alert Question"]);
    }
}
