//! The agent view's priority cards (plan 20261022_03, decision 9, as
//! amended at 02:34 EDT: "the agent view as terminal scrollback with
//! priority cards", "L3-1: pinned to the bottom, scrolling overrides" and
//! "L3-2: rules for now").
//!
//! A card sits at its place in the agent's transcript and is found by four
//! rules from data the client already has, highest priority first:
//! 1. **Approval needed:** a handlr confirmation pending for this pane;
//! 2. **The agent is asking:** its pending question (decision 8);
//! 3. **Something broke:** an error result, or failing tests, as the last
//!    tool result of the last turn; or the pane `blocked`;
//! 4. **Finished:** the agent is idle after a turn that ended with a reply,
//!    shown with the reply's first paragraph; or the daemon's turn says
//!    stopped and the reply is the newest entry, whatever herdr says (plan
//!    20261023_04, A4: herdr says working while a background agent runs).
//!
//! A card is live until it is resolved: the confirmation resolves, a user
//! entry follows the question, a later tool result succeeds (or the pane
//! is no longer blocked), or the agent works again. [`Cards`] remembers the
//! cards it has seen while the view is open, so a resolved one stays in
//! the flow, faded, instead of vanishing. The pinned card is the highest
//! live one.

use libhandlr_proto::common::{Entry, EntryKind, Link, PendingQuestion, Role, Turn};

/// The kinds of card, highest priority first (the order compares so).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CardKind {
    Approval,
    Question,
    Broke,
    Finished,
}

/// A card as the view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCard {
    pub kind: CardKind,
    /// Live cards stand out; a resolved one fades to scrollback styling.
    pub live: bool,
    /// Where it sits: how many of the view's entries come before it.
    pub at: usize,
    /// "Approval needed", "<name> is asking", "Something broke",
    /// "Finished".
    pub title: String,
    pub text: String,
    /// The confirmation an Approval card answers.
    pub confirm_id: Option<u64>,
    /// A Question card's question is about a plan: it offers approve,
    /// improve and implement in place of Yes and No (plan 20261022_03, the
    /// 21:57 EDT amendment).
    pub about_plan: bool,
    /// The repowiki links for the `docs/…` paths in `text`.
    pub links: Vec<Link>,
}

/// The links of `links` whose text is in `text`.
fn links_in(links: &[Link], text: &str) -> Vec<Link> {
    links
        .iter()
        .filter(|l| text.contains(&l.text))
        .cloned()
        .collect()
}

/// The longest text a Broke or Finished card carries, in characters.
pub const CARD_TEXT_MAX: usize = 300;

/// A handlr confirmation pending for the viewed pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConfirm {
    pub confirm_id: u64,
    /// The question, as spoken.
    pub text: String,
}

/// What the rules read.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    /// The agent's shown name.
    pub name: &'a str,
    /// herdr's `agent_status`.
    pub status: &'a str,
    pub entries: &'a [Entry],
    pub pending_question: Option<&'a PendingQuestion>,
    /// Confirmations pending for this pane, oldest first.
    pub confirms: &'a [PendingConfirm],
    /// The turn `agents.list` last gave for the pane, from its session.
    pub turn: Option<&'a Turn>,
}

/// Where a found card goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// After this entry.
    Entry(Entry),
    /// After the newest entry when the card is first seen: a confirmation,
    /// or the pane being blocked, which no entry marks.
    Newest,
}

/// A live card, as the rules found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub kind: CardKind,
    pub anchor: Anchor,
    pub title: String,
    pub text: String,
    pub confirm_id: Option<u64>,
    pub about_plan: bool,
    pub links: Vec<Link>,
}

/// The live cards the four rules find now, highest priority first.
pub fn live_cards(f: &Facts) -> Vec<Found> {
    let mut found = Vec::new();
    // 1. Approval needed.
    for c in f.confirms {
        found.push(Found {
            kind: CardKind::Approval,
            anchor: Anchor::Newest,
            title: "Approval needed".to_string(),
            text: c.text.clone(),
            confirm_id: Some(c.confirm_id),
            about_plan: false,
            links: Vec::new(),
        });
    }
    // 2. The agent is asking.
    let mut question_at = None;
    if let Some(q) = f.pending_question {
        let reply = f
            .entries
            .iter()
            .rposition(|e| is_reply(e) && e.ts == q.ts)
            .or_else(|| f.entries.iter().rposition(is_reply));
        question_at = reply;
        found.push(Found {
            kind: CardKind::Question,
            anchor: reply.map_or(Anchor::Newest, |i| Anchor::Entry(f.entries[i].clone())),
            title: format!("{} is asking", f.name),
            text: q.text.clone(),
            confirm_id: None,
            about_plan: q.about_plan,
            links: links_in(&q.links, &q.text),
        });
    }
    // 3. Something broke: the pane blocked, or the last turn's last tool
    // result failed.
    if f.status == "blocked" {
        found.push(Found {
            kind: CardKind::Broke,
            anchor: Anchor::Newest,
            title: "Something broke".to_string(),
            text: format!("{} is blocked and needs your input.", f.name),
            confirm_id: None,
            about_plan: false,
            links: Vec::new(),
        });
    }
    let turn = f
        .entries
        .iter()
        .rposition(|e| e.role == Role::User && e.kind == EntryKind::Text)
        .map_or(0, |i| i + 1);
    if let Some(result) = f.entries[turn..]
        .iter()
        .rev()
        .find(|e| e.kind == EntryKind::ToolResult)
        && (is_error(&result.summary) || fails_tests(&result.summary))
    {
        let text = result
            .summary
            .strip_prefix("error: ")
            .unwrap_or(&result.summary);
        let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let what = match &result.tool {
            Some(tool) => format!("{tool}: {}", first.trim()),
            None => first.trim().to_string(),
        };
        let text = cut(&what, CARD_TEXT_MAX);
        found.push(Found {
            kind: CardKind::Broke,
            anchor: Anchor::Entry(result.clone()),
            title: "Something broke".to_string(),
            links: links_in(&result.links, &text),
            text,
            confirm_id: None,
            about_plan: false,
        });
    }
    // 4. Finished: idle after a turn that ended with a reply, or the turn
    // stopped on the newest entry. The question card already stands for a
    // reply that asks.
    let stopped =
        matches!(f.turn, Some(Turn::Stopped { .. })) && f.entries.last().is_some_and(is_reply);
    if matches!(f.status, "idle" | "done") || stopped {
        let last_text = f.entries.iter().rposition(|e| e.kind == EntryKind::Text);
        if let Some(i) = last_text.filter(|&i| is_reply(&f.entries[i]))
            && question_at != Some(i)
        {
            let text = cut(first_paragraph(&f.entries[i].summary), CARD_TEXT_MAX);
            found.push(Found {
                kind: CardKind::Finished,
                anchor: Anchor::Entry(f.entries[i].clone()),
                title: "Finished".to_string(),
                links: links_in(&f.entries[i].links, &text),
                text,
                confirm_id: None,
                about_plan: false,
            });
        }
    }
    found
}

fn is_reply(e: &Entry) -> bool {
    e.role == Role::Assistant && e.kind == EntryKind::Text
}

/// A tool result the reader marked as an error (decision 2).
pub fn is_error(summary: &str) -> bool {
    summary.starts_with("error: ")
}

/// Whether a tool result reports failing tests: cargo's `test result:
/// FAILED`, a count such as `2 failed` (pytest, jest, go), or a line
/// starting `FAIL` (jest, go).
pub fn fails_tests(summary: &str) -> bool {
    if summary.contains("test result: FAILED") {
        return true;
    }
    if summary.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("FAIL ") || l.starts_with("--- FAIL") || l == "FAIL"
    }) {
        return true;
    }
    let words: Vec<&str> = summary.split_whitespace().collect();
    words.windows(2).any(|w| {
        let count = w[0].trim_start_matches(['(', '[', ',']);
        let failed = w[1].trim_end_matches([',', ';', '.', ')', ']']);
        failed.eq_ignore_ascii_case("failed") && count.parse::<u64>().is_ok_and(|n| n > 0)
    })
}

/// The first paragraph of a reply: up to its first blank line.
pub fn first_paragraph(text: &str) -> &str {
    let text = text.trim();
    let mut end = text.len();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            end = offset;
            break;
        }
        offset += line.len();
    }
    text[..end].trim()
}

/// `s` cut to `max` characters, ending in "…" when it was cut.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

/// What identifies a card across refreshes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CardId {
    Confirm(u64),
    /// Blocked, from the newest entry when it was first seen.
    Blocked(Option<Entry>),
    /// A card at an entry.
    At(CardKind, Entry),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Remembered {
    id: CardId,
    kind: CardKind,
    /// The entry it follows; `None` before every entry.
    anchor: Option<Entry>,
    title: String,
    text: String,
    confirm_id: Option<u64>,
    about_plan: bool,
    links: Vec<Link>,
    live: bool,
}

/// Resolved cards kept at most; the oldest go first.
pub const RESOLVED_KEPT: usize = 32;

/// The cards of one open view: live ones by the rules, and resolved ones
/// as they were last seen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cards {
    known: Vec<Remembered>,
}

impl Cards {
    /// Apply the rules to `facts`: new live cards are added, known ones
    /// that are no longer found are resolved.
    pub fn update(&mut self, facts: &Facts) {
        let newest = facts.entries.last().cloned();
        let was_live: Vec<bool> = self.known.iter().map(|r| r.live).collect();
        for r in &mut self.known {
            r.live = false;
        }
        for f in live_cards(facts) {
            let id = match (&f.anchor, f.confirm_id) {
                (_, Some(id)) => CardId::Confirm(id),
                (Anchor::Entry(e), None) => CardId::At(f.kind, e.clone()),
                (Anchor::Newest, None) => {
                    // The same blocked spell continues while it is live.
                    let ongoing = self.known.iter().zip(&was_live).find_map(|(r, &live)| {
                        (live && matches!(r.id, CardId::Blocked(_)) && r.kind == f.kind)
                            .then(|| r.id.clone())
                    });
                    ongoing.unwrap_or_else(|| CardId::Blocked(newest.clone()))
                }
            };
            match self.known.iter_mut().find(|r| r.id == id) {
                Some(r) => {
                    r.live = true;
                    r.title = f.title;
                    r.text = f.text;
                    r.about_plan = f.about_plan;
                    r.links = f.links;
                }
                None => {
                    let anchor = match f.anchor {
                        Anchor::Entry(e) => Some(e),
                        Anchor::Newest => newest.clone(),
                    };
                    self.known.push(Remembered {
                        id,
                        kind: f.kind,
                        anchor,
                        title: f.title,
                        text: f.text,
                        confirm_id: f.confirm_id,
                        about_plan: f.about_plan,
                        links: f.links,
                        live: true,
                    });
                }
            }
        }
        // Resolved cards whose entry has left the window go, and at most
        // `RESOLVED_KEPT` stay.
        self.known.retain(|r| {
            r.live
                || match &r.anchor {
                    None => true,
                    Some(a) => facts.entries.contains(a),
                }
        });
        let resolved = self.known.iter().filter(|r| !r.live).count();
        let mut drop = resolved.saturating_sub(RESOLVED_KEPT);
        self.known.retain(|r| {
            if !r.live && drop > 0 {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }

    /// The cards placed in `entries`, in flow order (by place, then
    /// priority), and the index of the pinned one: the highest live card,
    /// the newest of its kind.
    pub fn placed(&self, entries: &[Entry]) -> (Vec<AgentCard>, Option<usize>) {
        let mut cards: Vec<AgentCard> = self
            .known
            .iter()
            .filter_map(|r| {
                let at = match &r.anchor {
                    None => 0,
                    Some(a) => match entries.iter().rposition(|e| e == a) {
                        Some(i) => i + 1,
                        // A live card whose entry left the window: first.
                        None if r.live => 0,
                        None => return None,
                    },
                };
                Some(AgentCard {
                    kind: r.kind,
                    live: r.live,
                    at,
                    title: r.title.clone(),
                    text: r.text.clone(),
                    confirm_id: r.confirm_id,
                    about_plan: r.about_plan,
                    links: r.links.clone(),
                })
            })
            .collect();
        cards.sort_by_key(|c| (c.at, c.kind));
        let pinned = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| c.live)
            .min_by_key(|(_, c)| (c.kind, std::cmp::Reverse(c.at)))
            .map(|(i, _)| i);
        (cards, pinned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: &str, role: Role, kind: EntryKind, summary: &str) -> Entry {
        Entry {
            ts: ts.to_string(),
            role,
            kind,
            tool: (kind != EntryKind::Text).then(|| "Bash".to_string()),
            summary: summary.to_string(),
            links: Vec::new(),
        }
    }

    fn user(ts: &str, text: &str) -> Entry {
        entry(ts, Role::User, EntryKind::Text, text)
    }

    fn reply(ts: &str, text: &str) -> Entry {
        entry(ts, Role::Assistant, EntryKind::Text, text)
    }

    fn call(ts: &str) -> Entry {
        entry(
            ts,
            Role::Assistant,
            EntryKind::ToolUse,
            "Bash {\"command\":\"cargo test\"}",
        )
    }

    fn result(ts: &str, text: &str) -> Entry {
        entry(ts, Role::Tool, EntryKind::ToolResult, text)
    }

    fn facts<'a>(
        status: &'a str,
        entries: &'a [Entry],
        pending_question: Option<&'a PendingQuestion>,
        confirms: &'a [PendingConfirm],
    ) -> Facts<'a> {
        Facts {
            name: "Claude",
            status,
            entries,
            pending_question,
            confirms,
            turn: None,
        }
    }

    fn kinds(found: &[Found]) -> Vec<CardKind> {
        found.iter().map(|f| f.kind).collect()
    }

    fn question(ts: &str) -> PendingQuestion {
        PendingQuestion {
            text: "Shall I go on?".to_string(),
            ts: ts.to_string(),
            about_plan: false,
            plan: None,
            links: Vec::new(),
        }
    }

    fn confirm(id: u64) -> PendingConfirm {
        PendingConfirm {
            confirm_id: id,
            text: "Approve sending \"continue\" to Claude?".to_string(),
        }
    }

    #[test]
    fn rule_1_a_pending_confirmation_needs_approval() {
        let entries = [user("1", "go"), reply("2", "Working on it.")];
        let confirms = [confirm(7)];
        let found = live_cards(&facts("working", &entries, None, &confirms));
        assert_eq!(kinds(&found), [CardKind::Approval]);
        assert_eq!(found[0].confirm_id, Some(7));
        assert_eq!(found[0].anchor, Anchor::Newest);
        assert_eq!(found[0].text, "Approve sending \"continue\" to Claude?");
    }

    #[test]
    fn rule_2_the_pending_question_sits_at_its_reply() {
        let entries = [
            user("1", "go"),
            reply("2", "Done.\n\nShall I go on?"),
            call("3"),
        ];
        let q = question("2");
        let found = live_cards(&facts("idle", &entries, Some(&q), &[]));
        // The question stands for the reply: no Finished card for it.
        assert_eq!(kinds(&found), [CardKind::Question]);
        assert_eq!(found[0].anchor, Anchor::Entry(entries[1].clone()));
        assert_eq!(found[0].title, "Claude is asking");
        assert_eq!(found[0].text, "Shall I go on?");
    }

    #[test]
    fn rule_3_an_error_or_failing_tests_in_the_last_turn_or_blocked() {
        let error = [
            user("1", "test it"),
            call("2"),
            result("3", "error: exit status 101\nmore"),
        ];
        let found = live_cards(&facts("working", &error, None, &[]));
        assert_eq!(kinds(&found), [CardKind::Broke]);
        assert_eq!(found[0].text, "Bash: exit status 101");
        assert_eq!(found[0].anchor, Anchor::Entry(error[2].clone()));

        let failing = [
            user("1", "test it"),
            call("2"),
            result(
                "3",
                "running 4 tests\ntest result: FAILED. 3 passed; 1 failed",
            ),
        ];
        assert_eq!(
            kinds(&live_cards(&facts("working", &failing, None, &[]))),
            [CardKind::Broke]
        );

        // An error before the last user entry is not this turn's.
        let earlier = [call("1"), result("2", "error: boom"), user("3", "again")];
        assert!(live_cards(&facts("working", &earlier, None, &[])).is_empty());

        // Blocked: a card at the newest entry.
        let quiet = [user("1", "go")];
        let found = live_cards(&facts("blocked", &quiet, None, &[]));
        assert_eq!(kinds(&found), [CardKind::Broke]);
        assert_eq!(found[0].anchor, Anchor::Newest);
        assert_eq!(found[0].text, "Claude is blocked and needs your input.");
    }

    #[test]
    fn failing_tests_are_recognised() {
        assert!(fails_tests("test result: FAILED. 1 passed; 2 failed"));
        assert!(fails_tests("=== 2 failed, 10 passed in 0.3s ==="));
        assert!(fails_tests("Tests: 1 failed, 4 passed, 5 total"));
        assert!(fails_tests("FAIL src/app.test.js"));
        assert!(fails_tests("--- FAIL: TestThing (0.00s)"));
        assert!(!fails_tests("test result: ok. 12 passed; 0 failed"));
        assert!(!fails_tests("nothing failed"));
        assert!(!fails_tests("src/notify.rs"));
    }

    #[test]
    fn rule_4_idle_after_a_reply_is_finished_with_its_first_paragraph() {
        let entries = [
            user("1", "go"),
            call("2"),
            result("3", "ok"),
            reply(
                "4",
                "All 12 tests pass.\nThe build is clean.\n\nDetails follow.",
            ),
        ];
        let found = live_cards(&facts("idle", &entries, None, &[]));
        assert_eq!(kinds(&found), [CardKind::Finished]);
        assert_eq!(found[0].text, "All 12 tests pass.\nThe build is clean.");
        assert_eq!(found[0].anchor, Anchor::Entry(entries[3].clone()));
        // Working, or a turn that has not replied, is not finished.
        assert!(live_cards(&facts("working", &entries, None, &[])).is_empty());
        let unanswered = [reply("1", "Hi."), user("2", "go")];
        assert!(live_cards(&facts("idle", &unanswered, None, &[])).is_empty());
    }

    #[test]
    fn rule_4_a_stopped_turn_is_finished_while_herdr_says_working() {
        let entries = [user("1", "go"), reply("2", "All 12 tests pass.")];
        let stopped = Turn::Stopped {
            line: "All 12 tests pass.".to_string(),
        };
        let with = |turn, entries| Facts {
            turn: Some(turn),
            ..facts("working", entries, None, &[])
        };
        assert_eq!(
            kinds(&live_cards(&with(&stopped, &entries))),
            [CardKind::Finished]
        );
        // Working, by the session: no card.
        let working = Turn::Working {
            line: "All 12 tests pass.".to_string(),
        };
        assert!(live_cards(&with(&working, &entries)).is_empty());
        // A stopped turn the view has seen go on (a tool call after the
        // reply): the list is older than the session.
        let went_on = [entries[0].clone(), entries[1].clone(), call("3")];
        assert!(live_cards(&with(&stopped, &went_on)).is_empty());
    }

    #[test]
    fn the_rules_are_ordered_by_priority() {
        let entries = [
            user("1", "go"),
            call("2"),
            result("3", "error: boom"),
            reply("4", "It failed.\n\nShall I go on?"),
        ];
        let q = question("4");
        let confirms = [confirm(3)];
        let found = live_cards(&facts("blocked", &entries, Some(&q), &confirms));
        assert_eq!(
            kinds(&found),
            [
                CardKind::Approval,
                CardKind::Question,
                CardKind::Broke,
                CardKind::Broke
            ]
        );
    }

    #[test]
    fn a_resolved_card_stays_faded_in_the_flow() {
        let mut cards = Cards::default();
        let asked = vec![user("1", "go"), reply("2", "Shall I go on?")];
        let q = question("2");
        cards.update(&facts("idle", &asked, Some(&q), &[]));
        let (placed, pinned) = cards.placed(&asked);
        assert_eq!(placed.len(), 1);
        assert!(placed[0].live);
        assert_eq!(placed[0].at, 2);
        assert_eq!(pinned, Some(0));

        // The user answers: the question is resolved, and stays in place.
        let answered = vec![
            user("1", "go"),
            reply("2", "Shall I go on?"),
            user("3", "yes"),
        ];
        cards.update(&facts("working", &answered, None, &[]));
        let (placed, pinned) = cards.placed(&answered);
        assert_eq!(placed.len(), 1);
        assert!(!placed[0].live);
        assert_eq!((placed[0].kind, placed[0].at), (CardKind::Question, 2));
        assert_eq!(pinned, None);
    }

    #[test]
    fn each_card_resolves_its_own_way() {
        let mut cards = Cards::default();
        let entries = vec![user("1", "go"), call("2"), result("3", "error: boom")];
        let confirms = [confirm(9)];
        cards.update(&facts("blocked", &entries, None, &confirms));
        let live = |cards: &Cards, entries: &[Entry]| -> Vec<CardKind> {
            cards
                .placed(entries)
                .0
                .into_iter()
                .filter(|c| c.live)
                .map(|c| c.kind)
                .collect()
        };
        assert_eq!(
            live(&cards, &entries),
            [CardKind::Approval, CardKind::Broke, CardKind::Broke]
        );

        // The confirmation resolves; the pane is no longer blocked.
        cards.update(&facts("working", &entries, None, &[]));
        assert_eq!(live(&cards, &entries), [CardKind::Broke]);

        // A later tool result succeeds.
        let mut later = entries.clone();
        later.extend([call("4"), result("5", "ok")]);
        cards.update(&facts("working", &later, None, &[]));
        assert!(live(&cards, &later).is_empty());
        assert_eq!(cards.placed(&later).0.len(), 3, "all three stay, faded");

        // Finished, then the agent works again.
        later.push(reply("6", "Fixed."));
        cards.update(&facts("idle", &later, None, &[]));
        assert_eq!(live(&cards, &later), [CardKind::Finished]);
        cards.update(&facts("working", &later, None, &[]));
        assert!(live(&cards, &later).is_empty());
    }

    #[test]
    fn a_confirmation_keeps_its_place_as_entries_arrive() {
        let mut cards = Cards::default();
        let mut entries = vec![user("1", "go")];
        let confirms = [confirm(4)];
        cards.update(&facts("working", &entries, None, &confirms));
        entries.push(call("2"));
        cards.update(&facts("working", &entries, None, &confirms));
        let (placed, _) = cards.placed(&entries);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].at, 1, "after the entry it was asked at");
    }

    #[test]
    fn the_pin_is_the_highest_live_card() {
        let mut cards = Cards::default();
        let entries = vec![
            user("1", "go"),
            call("2"),
            result("3", "error: boom"),
            reply("4", "Shall I go on?"),
        ];
        let q = question("4");
        cards.update(&facts("idle", &entries, Some(&q), &[]));
        let (placed, pinned) = cards.placed(&entries);
        // In flow order: the error at 3, the question at 4.
        assert_eq!(
            placed.iter().map(|c| c.kind).collect::<Vec<_>>(),
            [CardKind::Broke, CardKind::Question]
        );
        assert_eq!(placed[pinned.unwrap()].kind, CardKind::Question);

        // A confirmation outranks it.
        let confirms = [confirm(2)];
        cards.update(&facts("idle", &entries, Some(&q), &confirms));
        let (placed, pinned) = cards.placed(&entries);
        assert_eq!(placed[pinned.unwrap()].kind, CardKind::Approval);

        // Resolved, the pin goes to the next live card.
        cards.update(&facts("idle", &entries, Some(&q), &[]));
        let (placed, pinned) = cards.placed(&entries);
        assert_eq!(placed[pinned.unwrap()].kind, CardKind::Question);

        // Nothing live: nothing pinned.
        cards.update(&facts("working", &entries[..1], None, &[]));
        assert_eq!(cards.placed(&entries[..1]).1, None);
    }

    #[test]
    fn the_newest_of_two_live_cards_of_a_kind_is_pinned() {
        let mut cards = Cards::default();
        let entries = vec![user("1", "go")];
        cards.update(&facts("working", &entries, None, &[confirm(1)]));
        let more = vec![user("1", "go"), call("2")];
        cards.update(&facts("working", &more, None, &[confirm(1), confirm(2)]));
        let (placed, pinned) = cards.placed(&more);
        assert_eq!(placed[pinned.unwrap()].confirm_id, Some(2));
    }

    #[test]
    fn first_paragraphs() {
        assert_eq!(first_paragraph("One.\nTwo.\n\nThree."), "One.\nTwo.");
        assert_eq!(first_paragraph("\n\nOnly.\n"), "Only.");
        assert_eq!(first_paragraph(""), "");
    }
}
