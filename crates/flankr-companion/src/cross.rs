//! Other sessions' cards at the bottom of the app (plan 20261022_03, the
//! L3 amendments of 2026-10-24 15:56, 16:00 and 16:01 EDT).
//!
//! Each session has a colour, from a hash of its session id
//! ([`session_colour`]). The client keeps each pane's last interaction (its
//! view opened, a message sent to it, one of its cards answered); a
//! session used within [`INTERACTED_WITHIN`] that asks a question, needs an
//! approval, or finishes a turn of at least [`LONG_TURN`] while the user is
//! anywhere but its own view gets a card. [`Cross`] works out which, and
//! [`crate::Event::SessionCards`] gives them to the app, newest first.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use libflankr_proto::common::{self, AgentInfo, Entry, EntryKind, Role, Turn};

/// How long after the last interaction a session's cards still show.
pub const INTERACTED_WITHIN: Duration = Duration::from_secs(60 * 60);
/// How long a finished card shows before it hides itself.
pub const FINISHED_SHOWN: Duration = Duration::from_secs(10);
/// The shortest turn, from the user's prompt to the Finished card in the
/// agent's own times, that gives a finished card (the 16:01 EDT
/// amendment).
pub const LONG_TURN: Duration = Duration::from_secs(3 * 60);
/// The session colours: the theme roles `session_1` to `session_16`.
pub const SESSION_COLOURS: u8 = 16;

/// What a session card is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionCardKind {
    /// The agent asks a question: "<agent> is asking".
    Question,
    /// A flankr confirmation waits for its pane: "<agent> needs approval".
    Approval,
    /// A long turn finished: "<agent> finished: <line>", one line.
    Finished,
}

/// Another session's card, as the app draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCard {
    /// The pane, which a tap opens.
    pub pane_id: String,
    /// The session's id, or the pane's when it has none.
    pub session: String,
    pub kind: SessionCardKind,
    /// The agent's shown name.
    pub agent: String,
    /// Its colour, 0 to 15: the theme role `session_<colour + 1>`.
    pub colour: u8,
    /// The question, the command to approve, or the last reply's first
    /// line.
    pub text: String,
    /// When the card came, on the client's clock.
    pub at: Duration,
}

/// A session's colour, 0 to 15, from the 32-bit FNV-1a hash of its id:
/// the same for the life of the session, on every phone and in every theme.
pub fn session_colour(session: &str) -> u8 {
    let mut h: u32 = 0x811c_9dc5;
    for b in session.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    (h % u32::from(SESSION_COLOURS)) as u8
}

/// The agent's session id, or its pane's when it has none.
pub fn session_key(a: &AgentInfo) -> String {
    session_id(a).unwrap_or(&a.pane_id).to_string()
}

fn session_id(a: &AgentInfo) -> Option<&str> {
    a.session.as_ref().and_then(|s| s.session_id.as_deref())
}

/// The agent's colour: [`session_colour`] of [`session_key`].
pub fn colour_of(a: &AgentInfo) -> u8 {
    session_colour(&session_key(a))
}

/// The agent name the daemon gives a pane that runs no known agent.
pub const SHELL: &str = "shell";

/// Whether `a`'s session has only the terminal view (plan 20261022_03,
/// the 2026-10-28 00:47 EDT amendment): a shell, which has no transcript,
/// takes no messages and has no summaries or suggestions.
pub fn is_terminal_only(a: &AgentInfo) -> bool {
    a.agent == SHELL
}

/// The agent's shown name: its name, else its agent with a capital.
pub fn shown_name(a: &AgentInfo) -> String {
    if let Some(name) = a.name.as_deref().filter(|n| !n.is_empty()) {
        return name.to_string();
    }
    let mut chars = a.agent.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// An RFC 3339 time (`2026-10-24T15:56:00.123Z`, `…+02:00`) in seconds
/// since 1970.
pub fn rfc3339_secs(ts: &str) -> Option<f64> {
    let ts = ts.trim();
    let b = ts.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| ts.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if b[13] != b':' || b[16] != b':' || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut rest = &ts[19..];
    let mut frac = 0.0;
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        frac = format!("0.{}", &r[..digits]).parse().ok()?;
        rest = &r[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 && matches!(&r[..1], "+" | "-") && &r[3..4] == ":" => {
            let oh: i64 = r[1..3].parse().ok()?;
            let om: i64 = r[4..6].parse().ok()?;
            let o = oh * 3600 + om * 60;
            if r.starts_with('-') { -o } else { o }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + s - offset;
    Some(secs as f64 + frac)
}

/// How long the turn that ended the session ran, in the agent's own
/// times: from the user's last prompt (a text the user typed, not context)
/// to the last entry. With no prompt among `entries`, from the first of
/// them: the turn ran at least that long.
pub fn turn_span(entries: &[Entry]) -> Option<Duration> {
    let end = rfc3339_secs(&entries.last()?.ts)?;
    let prompt = entries
        .iter()
        .rev()
        .find(|e| e.role == Role::User && e.kind == EntryKind::Text && e.origin.is_none())
        .unwrap_or(&entries[0]);
    let start = rfc3339_secs(&prompt.ts)?;
    Some(Duration::from_secs_f64((end - start).max(0.0)))
}

/// A pane's last interaction, and its session then, where known.
#[derive(Debug, Clone)]
struct Interaction {
    at: Duration,
    session: Option<String>,
}

/// The sessions' cards (see the module).
#[derive(Debug, Default)]
pub struct Cross {
    interacted: HashMap<String, Interaction>,
    /// The cards showing, oldest first.
    cards: Vec<SessionCard>,
    /// Questions and approvals swiped away, by pane, kind and text, until
    /// they are no longer asked.
    dismissed: HashSet<(String, SessionCardKind, String)>,
    /// The cards as last given.
    shown: Vec<SessionCard>,
}

impl Cross {
    /// The user interacted with the session in `pane_id` at `now`.
    pub fn interact(&mut self, pane_id: &str, agent: Option<&AgentInfo>, now: Duration) {
        let session = agent.and_then(session_id).map(str::to_string);
        self.interacted
            .insert(pane_id.to_string(), Interaction { at: now, session });
    }

    /// When the pane was last interacted with.
    pub fn last_interaction(&self, pane_id: &str) -> Option<Duration> {
        self.interacted.get(pane_id).map(|i| i.at)
    }

    /// The session of `a` was used within [`INTERACTED_WITHIN`] of `now`:
    /// its pane was, and by the same session where both are known.
    pub fn recent(&self, a: &AgentInfo, now: Duration) -> bool {
        self.interacted.get(&a.pane_id).is_some_and(|i| {
            now.saturating_sub(i.at) <= INTERACTED_WITHIN
                && match (&i.session, session_id(a)) {
                    (Some(was), Some(is)) => was == is,
                    _ => true,
                }
        })
    }

    /// A pane other than `viewed` was used within the hour: its questions
    /// and turns are to be watched.
    pub fn watching(&self, viewed: Option<&str>, now: Duration) -> bool {
        self.interacted.iter().any(|(pane, i)| {
            Some(pane.as_str()) != viewed && now.saturating_sub(i.at) <= INTERACTED_WITHIN
        })
    }

    /// The questions and approvals as they stand: `agents` as listed,
    /// `approvals` the panes with a flankr confirmation waiting and its
    /// command, `viewed` the open view's pane. Returns whether a question
    /// or an approval came that was not showing. A question or approval
    /// takes the place of every finished card.
    pub fn update(
        &mut self,
        agents: &[AgentInfo],
        approvals: &[(String, String)],
        viewed: Option<&str>,
        now: Duration,
    ) -> bool {
        // A session's interaction learns its session once listed.
        for a in agents {
            if let (Some(i), Some(id)) = (self.interacted.get_mut(&a.pane_id), session_id(a))
                && i.session.is_none()
            {
                i.session = Some(id.to_string());
            }
        }
        let mut wanted: Vec<(&AgentInfo, SessionCardKind, String)> = Vec::new();
        for a in agents {
            if Some(a.pane_id.as_str()) == viewed || !self.recent(a, now) {
                continue;
            }
            if let Some((_, command)) = approvals.iter().find(|(p, _)| *p == a.pane_id) {
                wanted.push((a, SessionCardKind::Approval, command.clone()));
            } else if let Some(Turn::Asking { question }) = &a.turn {
                wanted.push((a, SessionCardKind::Question, question.clone()));
            }
        }
        let keys: HashSet<(String, SessionCardKind, String)> = wanted
            .iter()
            .map(|(a, k, t)| (a.pane_id.clone(), *k, t.clone()))
            .collect();
        // A swipe holds only while the same thing is asked.
        self.dismissed.retain(|k| keys.contains(k));
        self.cards.retain(|c| {
            c.kind == SessionCardKind::Finished
                || keys.contains(&(c.pane_id.clone(), c.kind, c.text.clone()))
        });
        let mut came = false;
        for (a, kind, text) in wanted {
            let key = (a.pane_id.clone(), kind, text.clone());
            let held = self
                .cards
                .iter()
                .any(|c| c.pane_id == a.pane_id && c.kind == kind && c.text == text);
            if held || self.dismissed.contains(&key) {
                continue;
            }
            self.cards.push(card(a, kind, text, now));
            came = true;
        }
        if self
            .cards
            .iter()
            .any(|c| c.kind != SessionCardKind::Finished)
        {
            self.cards.retain(|c| c.kind != SessionCardKind::Finished);
        }
        // The viewed pane's finished card goes too.
        self.cards.retain(|c| Some(c.pane_id.as_str()) != viewed);
        came
    }

    /// The turn of `a` finished after `span`, its last reply's first line
    /// `line`: a finished card when the turn was long, its session used
    /// within the hour, it is not the one viewed, and no question or
    /// approval shows. Returns whether one came.
    pub fn finished(
        &mut self,
        a: &AgentInfo,
        line: String,
        span: Duration,
        viewed: Option<&str>,
        now: Duration,
    ) -> bool {
        if span < LONG_TURN
            || Some(a.pane_id.as_str()) == viewed
            || !self.recent(a, now)
            || self
                .cards
                .iter()
                .any(|c| c.kind != SessionCardKind::Finished)
        {
            return false;
        }
        self.cards.retain(|c| c.pane_id != a.pane_id);
        self.cards
            .push(card(a, SessionCardKind::Finished, line, now));
        true
    }

    /// The pane's newest card was swiped away (or its view opened, which
    /// takes all of them).
    pub fn dismiss(&mut self, pane_id: &str, all: bool) {
        while let Some(i) = self.cards.iter().rposition(|c| c.pane_id == pane_id) {
            let c = self.cards.remove(i);
            if c.kind != SessionCardKind::Finished {
                self.dismissed.insert((c.pane_id, c.kind, c.text));
            }
            if !all {
                break;
            }
        }
    }

    /// Finished cards older than [`FINISHED_SHOWN`] hide themselves.
    pub fn expire(&mut self, now: Duration) {
        self.cards.retain(|c| {
            c.kind != SessionCardKind::Finished || now.saturating_sub(c.at) < FINISHED_SHOWN
        });
    }

    /// The cards, newest first.
    pub fn cards(&self) -> Vec<SessionCard> {
        let mut cards = self.cards.clone();
        cards.reverse();
        cards
    }

    /// The cards, if they changed since last taken.
    pub fn take_changed(&mut self) -> Option<Vec<SessionCard>> {
        let cards = self.cards();
        if cards == self.shown {
            return None;
        }
        self.shown = cards.clone();
        Some(cards)
    }
}

fn card(a: &AgentInfo, kind: SessionCardKind, text: String, now: Duration) -> SessionCard {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(common::TURN_TEXT_MAX)
        .collect::<String>();
    SessionCard {
        pane_id: a.pane_id.clone(),
        session: session_key(a),
        kind,
        agent: shown_name(a),
        colour: colour_of(a),
        // A question keeps its lines, for the card's few; the rest is one.
        text: if kind == SessionCardKind::Question {
            text
        } else {
            first
        },
        at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::common::{AgentSessionInfo, SessionMatched};

    fn agent(pane: &str, session: Option<&str>, turn: Option<Turn>) -> AgentInfo {
        AgentInfo {
            pane_id: pane.to_string(),
            name: Some(format!("Claude in {pane}")),
            spoken: None,
            agent: "claude".to_string(),
            project: None,
            cwd: None,
            workspace_id: None,
            tab_id: None,
            tab_label: None,
            tab_number: None,
            status: "idle".to_string(),
            focused: false,
            session: session.map(|s| AgentSessionInfo {
                session_id: Some(s.to_string()),
                matched: SessionMatched::Id,
                pending_question: None,
            }),
            turn,
        }
    }

    fn asking(q: &str) -> Option<Turn> {
        Some(Turn::Asking {
            question: q.to_string(),
        })
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn a_sessions_colour_is_fnv_1a_of_its_id_and_never_changes() {
        // FNV-1a 32 of "" is 0x811c9dc5, of "a" 0xe40c292c.
        assert_eq!(session_colour(""), (0x811c_9dc5u32 % 16) as u8);
        assert_eq!(session_colour("a"), (0xe40c_292cu32 % 16) as u8);
        let a = agent("w1:p1", Some("0b8e-session"), None);
        assert_eq!(colour_of(&a), colour_of(&a.clone()));
        assert_eq!(colour_of(&a), session_colour("0b8e-session"));
        // Without a session, the pane's.
        assert_eq!(
            colour_of(&agent("w1:p1", None, None)),
            session_colour("w1:p1")
        );
        // The colours spread over all 16.
        let used: HashSet<u8> = (0..200).map(|i| session_colour(&format!("s{i}"))).collect();
        assert_eq!(used.len(), 16);
    }

    #[test]
    fn rfc3339_times_are_read_with_their_offsets() {
        assert_eq!(rfc3339_secs("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(rfc3339_secs("2000-03-01T00:00:00Z"), Some(951_868_800.0));
        assert_eq!(
            rfc3339_secs("2026-10-24T16:01:00-04:00"),
            rfc3339_secs("2026-10-24T20:01:00Z")
        );
        assert_eq!(
            rfc3339_secs("2026-10-24T20:01:00.250Z").map(|s| s.fract()),
            Some(0.25)
        );
        assert_eq!(rfc3339_secs("3"), None);
        assert_eq!(rfc3339_secs("2026-10-24T20:01:00"), None);
    }

    fn entry(ts: &str, role: Role, origin: Option<&str>) -> Entry {
        serde_json::from_value(serde_json::json!({
            "ts": ts, "role": role, "kind": "text", "summary": "x", "origin": origin,
        }))
        .unwrap()
    }

    #[test]
    fn a_turn_runs_from_the_users_prompt_to_its_last_entry() {
        let e = vec![
            entry("2026-10-24T10:00:00Z", Role::User, None),
            entry("2026-10-24T10:01:00Z", Role::Assistant, None),
            entry("2026-10-24T10:02:00Z", Role::User, None),
            entry("2026-10-24T10:03:00Z", Role::User, Some("task")),
            entry("2026-10-24T10:05:30Z", Role::Assistant, None),
        ];
        assert_eq!(turn_span(&e), Some(secs(210)));
        // No prompt held: at least from the first entry.
        assert_eq!(turn_span(&e[3..]), Some(secs(150)));
        assert_eq!(turn_span(&[]), None);
    }

    #[test]
    fn only_a_session_used_within_the_hour_and_not_viewed_gets_a_card() {
        let mut c = Cross::default();
        let a = agent("w1:p1", Some("s1"), asking("Ship it?"));
        let b = agent("w1:p2", Some("s2"), asking("Merge?"));
        c.interact("w1:p1", Some(&a), secs(0));
        assert!(c.update(&[a.clone(), b.clone()], &[], None, secs(10)));
        let cards = c.cards();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].pane_id, "w1:p1");
        assert_eq!(cards[0].kind, SessionCardKind::Question);
        assert_eq!(cards[0].colour, session_colour("s1"));
        assert_eq!(cards[0].agent, "Claude in w1:p1");
        // In its own view: none.
        c.update(std::slice::from_ref(&a), &[], Some("w1:p1"), secs(11));
        assert!(c.cards().is_empty());
        // Over an hour on: none.
        let mut c = Cross::default();
        c.interact("w1:p1", Some(&a), secs(0));
        assert!(!c.update(std::slice::from_ref(&a), &[], None, secs(3601)));
        assert!(c.cards().is_empty());
        // Another session in the same pane is not the one used.
        let mut c = Cross::default();
        c.interact("w1:p1", Some(&a), secs(0));
        let other = agent("w1:p1", Some("s9"), asking("Ship it?"));
        assert!(!c.update(&[other], &[], None, secs(5)));
    }

    #[test]
    fn an_approval_comes_before_the_question_and_a_swipe_holds_while_asked() {
        let mut c = Cross::default();
        let a = agent("w1:p1", Some("s1"), asking("Ship it?"));
        c.interact("w1:p1", None, secs(0));
        let approvals = vec![("w1:p1".to_string(), "cargo publish".to_string())];
        assert!(c.update(std::slice::from_ref(&a), &approvals, None, secs(1)));
        assert_eq!(c.cards()[0].kind, SessionCardKind::Approval);
        assert_eq!(c.cards()[0].text, "cargo publish");
        c.dismiss("w1:p1", false);
        assert!(!c.update(std::slice::from_ref(&a), &approvals, None, secs(2)));
        assert!(c.cards().is_empty());
        // The approval resolved: the question shows.
        assert!(c.update(std::slice::from_ref(&a), &[], None, secs(3)));
        assert_eq!(c.cards()[0].kind, SessionCardKind::Question);
        // The same question again is the same card: nothing new.
        assert!(!c.update(std::slice::from_ref(&a), &[], None, secs(4)));
        // Answered: it goes.
        let done = agent("w1:p1", Some("s1"), None);
        c.update(&[done], &[], None, secs(5));
        assert!(c.cards().is_empty());
    }

    #[test]
    fn finished_cards_need_a_long_turn_hide_after_10_s_and_give_way_to_a_question() {
        let mut c = Cross::default();
        let a = agent("w1:p1", Some("s1"), None);
        let b = agent("w1:p2", Some("s2"), asking("Merge?"));
        c.interact("w1:p1", None, secs(0));
        c.interact("w1:p2", None, secs(0));
        // Just short of 3 minutes: none.
        assert!(!c.finished(&a, "Done.".into(), LONG_TURN - secs(1), None, secs(5)));
        assert!(c.cards().is_empty());
        assert!(c.finished(&a, "Done.\nmore".into(), LONG_TURN, None, secs(5)));
        assert_eq!(c.cards()[0].kind, SessionCardKind::Finished);
        assert_eq!(c.cards()[0].text, "Done.");
        c.expire(secs(14));
        assert_eq!(c.cards().len(), 1);
        c.expire(secs(15));
        assert!(c.cards().is_empty());
        // A question takes its place.
        assert!(c.finished(&a, "Done.".into(), LONG_TURN, None, secs(20)));
        assert!(c.update(&[a.clone(), b.clone()], &[], None, secs(21)));
        let cards = c.cards();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, SessionCardKind::Question);
        // And while it shows, no finished card comes.
        assert!(!c.finished(&a, "Again.".into(), LONG_TURN, None, secs(22)));
    }

    #[test]
    fn several_come_newest_first() {
        let mut c = Cross::default();
        let a = agent("w1:p1", Some("s1"), asking("One?"));
        let b = agent("w1:p2", Some("s2"), asking("Two?"));
        c.interact("w1:p1", None, secs(0));
        c.interact("w1:p2", None, secs(0));
        c.update(std::slice::from_ref(&a), &[], None, secs(1));
        c.update(&[a.clone(), b.clone()], &[], None, secs(2));
        let cards = c.cards();
        assert_eq!(
            cards.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["Two?", "One?"]
        );
        assert!(c.take_changed().is_some());
        assert!(c.take_changed().is_none());
        assert!(c.watching(Some("w1:p1"), secs(3)));
        assert!(!c.watching(Some("w1:p1"), secs(3700)));
    }
}
