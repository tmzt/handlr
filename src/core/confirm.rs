//! The confirmation broker (plan 20261022_01, decision 8, as amended; the
//! handle, token and nonce are in CHANGES.md, 2026-10-22).
//!
//! A command that needs confirmation becomes a pending `Confirmation`. The
//! broker, not the requester, sends it once it is approved, so what runs is
//! exactly what was announced, at most once. Two strings travel with it:
//!
//! * the **handle**, written by the requester, describing the action so it
//!   can be found again after the requester's own context has moved on;
//! * the **token**, three BIP-39 words generated here, shown only to
//!   principals that can approve. It never crosses the wire: an approver
//!   that must prove it knows the token sends `nonce()` of it instead.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use blake2::digest::{KeyInit, Mac};
use blake2::Blake2sMac256;
use libherdr::client::HerdrClient;
use tokio::sync::broadcast;
use tokio::time::Instant;

use crate::core::principal::Principal;

/// How long a confirmation asked for by a companion's own voice turn waits.
pub const VOICE_TURN_TTL: Duration = Duration::from_secs(60);
/// How long any other confirmation waits, unless `[server] confirm_ttl` says otherwise.
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);
/// When a long-lived confirmation is announced to approvers a second time.
pub const REMINDER_AFTER: Duration = Duration::from_secs(600);
/// How long a resolved confirmation can still be looked up.
pub const RESOLVED_KEPT: Duration = Duration::from_secs(24 * 3600);
/// Wrong nonces after which a confirmation is denied.
pub const MAX_BAD_NONCES: u32 = 5;

const NONCE_CONTEXT: &[u8] = b"handlr.confirm/1";

/// The BIP-39 English list: 2048 words, SHA-256
/// `2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda`.
const WORDS: &str = include_str!("words.txt");

/// What ends a confirmation if nobody answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    /// Asked for by a companion's own voice turn: it checks what was heard,
    /// and ends at that session's next turn or after `VOICE_TURN_TTL`.
    VoiceTurn,
    /// Anything else: the broker's TTL, with one reminder.
    Standing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    /// Approved, and being sent.
    Approved,
    Sent,
    Failed,
    Denied,
    Expired,
    Cancelled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Pending => "pending",
            Outcome::Approved => "approved",
            Outcome::Sent => "sent",
            Outcome::Failed => "failed",
            Outcome::Denied => "denied",
            Outcome::Expired => "expired",
            Outcome::Cancelled => "cancelled",
        }
    }

    fn is_open(self) -> bool {
        matches!(self, Outcome::Pending)
    }
}

/// Where an approved command goes.
#[async_trait]
pub trait CommandSink: Send + Sync {
    async fn send(&self, pane_id: &str, command: &str) -> Result<(), String>;
}

#[async_trait]
impl CommandSink for HerdrClient {
    async fn send(&self, pane_id: &str, command: &str) -> Result<(), String> {
        self.run_command(pane_id, command)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// A confirmation token. Its `Debug` is redacted, and it is revealed only
/// to principals that can approve.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    fn generate() -> Self {
        let words: Vec<&str> = WORDS.lines().collect();
        debug_assert_eq!(words.len(), 2048);
        let mut bytes = [0u8; 6];
        getrandom::getrandom(&mut bytes).expect("the OS random source is unavailable");
        let picked: Vec<&str> = bytes
            .chunks(2)
            .map(|pair| words[(u16::from_be_bytes([pair[0], pair[1]]) & 0x7ff) as usize])
            .collect();
        Token(picked.join("-"))
    }

    /// The token, if `principal` may see it.
    pub fn reveal_to(&self, principal: &Principal) -> Option<&str> {
        principal.can_approve.then_some(self.0.as_str())
    }

    /// The token, for the daemon's own terminal (`handlr serve`) only.
    pub fn expose_for_terminal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// The nonce that proves knowledge of `token` for one confirmation:
/// hex BLAKE2s-256-MAC keyed by the token over the context, the id (8 bytes,
/// big-endian) and the handle (empty when there is none).
pub fn nonce(token: &str, confirm_id: u64, handle: Option<&str>) -> String {
    let mac = nonce_mac(token, confirm_id, handle);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn nonce_mac(token: &str, confirm_id: u64, handle: Option<&str>) -> Blake2sMac256 {
    // Tokens are at most 26 bytes; BLAKE2s takes keys of up to 32.
    let mut mac = <Blake2sMac256 as KeyInit>::new_from_slice(token.as_bytes())
        .expect("token fits a BLAKE2s key");
    mac.update(NONCE_CONTEXT);
    mac.update(&confirm_id.to_be_bytes());
    mac.update(handle.unwrap_or("").as_bytes());
    mac
}

fn nonce_matches(token: &str, confirm_id: u64, handle: Option<&str>, nonce: &str) -> bool {
    let Some(bytes) = decode_hex(nonce) else {
        return false;
    };
    // verify_slice compares in constant time.
    nonce_mac(token, confirm_id, handle)
        .verify_slice(&bytes)
        .is_ok()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// A handle is 3–64 characters of `a-z`, `0-9` and `-`.
pub fn validate_handle(handle: &str) -> Result<(), ConfirmError> {
    let ok = (3..=64).contains(&handle.len())
        && handle
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(ConfirmError::InvalidHandle(handle.to_string()))
    }
}

// ── Handles written by the model (plan 20261022_08, amendments) ─────────

/// Words dropped when a handle is built from what the user said.
const ARTICLES: [&str; 6] = ["a", "an", "the", "to", "in", "on"];
/// The fewest words a handle may have.
pub const MIN_HANDLE_WORDS: usize = 3;
const MAX_HANDLE_LEN: usize = 64;

/// A handle as the broker stores it: lowercased, every run of anything
/// other than `a-z` and `0-9` folded to one `-`, leading and trailing `-`
/// trimmed, cut to 64 characters. `Usage Claude!` -> `usage-claude`.
pub fn normalise_handle(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(MAX_HANDLE_LEN);
    out.trim_matches('-').to_string()
}

/// `words` joined by `-`, as many whole words as fit in 64 characters.
fn join_handle<S: AsRef<str>>(words: &[S]) -> String {
    let mut out = String::new();
    for w in words {
        let w = w.as_ref();
        let extra = if out.is_empty() { w.len() } else { w.len() + 1 };
        if out.len() + extra > MAX_HANDLE_LEN {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(w);
    }
    out
}

/// The words of a normalised handle.
fn handle_words(handle: &str) -> Vec<&str> {
    handle.split('-').filter(|w| !w.is_empty()).collect()
}

const DIGIT_WORDS: [&str; 10] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];

/// The words loose matching compares: lowercased, split on `-`, spaces,
/// punctuation and between letters and digits, every digit written as a
/// word. `kp-2 High bay` -> `kp two high bay`.
pub fn match_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() {
            word.push(c);
        } else {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if let Some(d) = c.to_digit(10) {
                out.push(DIGIT_WORDS[d as usize].to_string());
            }
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The word rule: an edit distance of at most 1, or 2 when both words have
/// 6 letters or more.
pub fn words_match(a: &str, b: &str) -> bool {
    let allowed = if a.len().min(b.len()) >= 6 { 2 } else { 1 };
    a == b || edit_distance(a, b) <= allowed
}

/// How many of `stored`'s words `returned` matches, in any order: a stored
/// word matches a returned word, or two adjacent returned words joined
/// ("high bay" for `highbay`); two adjacent stored words both match a
/// returned word that matches them joined.
fn matched_words(returned: &[String], stored: &[String]) -> usize {
    let mut matched = vec![false; stored.len()];
    for (i, s) in stored.iter().enumerate() {
        matched[i] = returned.iter().any(|r| words_match(r, s))
            || returned
                .windows(2)
                .any(|pair| words_match(&format!("{}{}", pair[0], pair[1]), s));
    }
    for i in 0..stored.len().saturating_sub(1) {
        let joined = format!("{}{}", stored[i], stored[i + 1]);
        if returned.iter().any(|r| words_match(r, &joined)) {
            matched[i] = true;
            matched[i + 1] = true;
        }
    }
    matched.iter().filter(|m| **m).count()
}

/// `(matched, total)`: how many of `stored`'s words `returned` matches,
/// and how many words `stored` has. The score is their ratio.
pub fn handle_score(returned: &str, stored: &str) -> (usize, usize) {
    let stored = match_words(stored);
    (matched_words(&match_words(returned), &stored), stored.len())
}

fn scores_two_thirds((matched, total): (usize, usize)) -> bool {
    total > 0 && 3 * matched >= 2 * total
}

/// Which of `stored` a `returned` handle loosely means, by index: the best
/// score, if it is at least 2/3 and matches at least one more word than the
/// second best. A returned handle equal to a stored one, once normalised,
/// is that one.
pub fn best_handle_match<S: AsRef<str>>(returned: &str, stored: &[S]) -> Option<usize> {
    let exact = normalise_handle(returned);
    if let Some(i) = stored.iter().position(|s| s.as_ref() == exact) {
        return Some(i);
    }
    loose_match(returned, stored)
}

fn loose_match<S: AsRef<str>>(returned: &str, stored: &[S]) -> Option<usize> {
    let mut scored: Vec<(usize, (usize, usize))> = stored
        .iter()
        .enumerate()
        .map(|(i, s)| (i, handle_score(returned, s.as_ref())))
        .collect();
    // Best score first (compared as fractions), then most words matched.
    scored.sort_by(|(_, (am, at)), (_, (bm, bt))| (bm * at).cmp(&(am * bt)).then(bm.cmp(am)));
    let (best, score) = *scored.first()?;
    let second = scored.get(1).map_or(0, |(_, (m, _))| *m);
    (scores_two_thirds(score) && score.0 > second).then_some(best)
}

/// Whether a new `handle` stays apart from the `pending` ones: returned as
/// written, it would be the one meant by loose matching alone.
fn stands_apart(handle: &str, pending: &[String]) -> bool {
    let mut all: Vec<&str> = pending.iter().map(String::as_str).collect();
    all.push(handle);
    loose_match(handle, &all) == Some(all.len() - 1)
}

/// A random word of the BIP-39 list.
pub fn random_word() -> String {
    let words: Vec<&str> = WORDS.lines().collect();
    let mut bytes = [0u8; 2];
    getrandom::getrandom(&mut bytes).expect("the OS random source is unavailable");
    words[(u16::from_be_bytes(bytes) & 0x7ff) as usize].to_string()
}

/// What the daemon knows when the model asks for a confirmation, to settle
/// its handle (plan 20261022_08, 23:40 and 23:44 EDT amendments).
#[derive(Debug, Clone, Default)]
pub struct HandleRequest<'a> {
    /// The handle the model gave, if any.
    pub given: Option<&'a str>,
    /// What the user said in this turn.
    pub utterance: Option<&'a str>,
    /// The command and the agent's spoken name, used when neither the
    /// handle nor the utterance gives three words.
    pub command: &'a str,
    pub agent: &'a str,
    /// The agent's tab code spelled out ("em bee"), the first distinguishing
    /// word to try.
    pub tab_code: Option<&'a str>,
}

/// The handle to store for `req`, given the requester's `pending` handles:
/// * the model's handle, normalised, if at least 2/3 of its words match
///   what the user said (always, when the utterance is unknown);
/// * otherwise the utterance's words without articles; otherwise the
///   command and the agent;
/// * at least [`MIN_HANDLE_WORDS`] words;
/// * `-2`, `-3`, ... when it equals a pending handle;
/// * a distinguishing word (the tab code, then words from `extra_word`)
///   when it would be confused with a pending handle.
pub fn settle_handle(
    req: &HandleRequest<'_>,
    pending: &[String],
    mut extra_word: impl FnMut() -> String,
) -> String {
    let without_articles = |text: &str| -> Vec<String> {
        handle_words(&normalise_handle(text))
            .into_iter()
            .filter(|w| !ARTICLES.contains(w))
            .map(str::to_string)
            .collect()
    };
    let from_user = |handle: &str| match req.utterance {
        Some(u) => scores_two_thirds(handle_score(u, handle)),
        None => true,
    };
    let long_enough = |handle: &str| handle_words(handle).len() >= MIN_HANDLE_WORDS;

    let given = req.given.map(normalise_handle).unwrap_or_default();
    let mut handle = if !given.is_empty() && from_user(&given) && long_enough(&given) {
        given
    } else {
        let spoken = req.utterance.map(without_articles).unwrap_or_default();
        if spoken.len() >= MIN_HANDLE_WORDS {
            join_handle(&spoken)
        } else {
            let mut words = without_articles(&format!("{} {}", req.command, req.agent));
            words.dedup();
            join_handle(&words)
        }
    };
    let mut tab_code = req.tab_code.map(normalise_handle).filter(|t| !t.is_empty());
    let mut add_word = |handle: &str| -> String {
        let word = normalise_handle(&tab_code.take().unwrap_or_else(&mut extra_word));
        // Keep the new word even when 64 characters would cut it off.
        let mut words: Vec<&str> = handle_words(handle);
        while !words.is_empty() && words.join("-").len() + 1 + word.len() > MAX_HANDLE_LEN {
            words.pop();
        }
        words.push(&word);
        words.join("-")
    };
    while !long_enough(&handle) {
        handle = add_word(&handle);
    }
    for _ in 0..16 {
        if pending.contains(&handle) {
            let base = handle.clone();
            handle = (2..)
                .map(|n| {
                    let suffix = format!("-{n}");
                    let mut stem = base.clone();
                    stem.truncate(MAX_HANDLE_LEN - suffix.len());
                    format!("{}{suffix}", stem.trim_end_matches('-'))
                })
                .find(|h| !pending.contains(h))
                .expect("some suffix is free");
        }
        if stands_apart(&handle, pending) {
            break;
        }
        handle = add_word(&handle);
    }
    handle
}

/// What everyone may know about a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmView {
    pub confirm_id: u64,
    pub pane_id: String,
    pub agent: String,
    pub command: String,
    pub handle: Option<String>,
    pub requested_by: String,
    /// A companion's own voice turn, or standing.
    pub lifetime: Lifetime,
}

#[derive(Debug, Clone)]
pub enum ConfirmEvent {
    /// A new confirmation, or (`reminder`) one still waiting after
    /// `REMINDER_AFTER`. Show `token` only through `Token::reveal_to`.
    Requested {
        view: ConfirmView,
        token: Token,
        reminder: bool,
    },
    Resolved {
        view: ConfirmView,
        outcome: Outcome,
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmError {
    /// No such confirmation, or it is no longer pending (`-32006`).
    NotFound,
    /// This principal may not approve it (`-32007`).
    MayNotApprove,
    /// The nonce does not match (`-32008`).
    BadNonce { remaining: u32 },
    /// The handle is not 3–64 characters of `a-z`, `0-9` and `-` (`-32602`).
    InvalidHandle(String),
    /// The requester already has a pending confirmation with this handle (`-32602`).
    DuplicateHandle(String),
}

impl ConfirmError {
    pub fn code(&self) -> i64 {
        match self {
            ConfirmError::NotFound => -32006,
            ConfirmError::MayNotApprove => -32007,
            ConfirmError::BadNonce { .. } => -32008,
            ConfirmError::InvalidHandle(_) | ConfirmError::DuplicateHandle(_) => -32602,
        }
    }
}

impl fmt::Display for ConfirmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfirmError::NotFound => f.write_str("confirmation not found or already resolved"),
            ConfirmError::MayNotApprove => f.write_str("this principal may not approve it"),
            ConfirmError::BadNonce { remaining } => {
                write!(f, "wrong nonce; {remaining} attempt(s) left")
            }
            ConfirmError::InvalidHandle(h) => write!(
                f,
                "invalid handle '{h}': use 3-64 characters of a-z, 0-9 and '-'"
            ),
            ConfirmError::DuplicateHandle(h) => {
                write!(f, "a pending confirmation already has the handle '{h}'")
            }
        }
    }
}

/// How `Broker::status` finds a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusKey {
    Id(u64),
    Handle(String),
}

struct Entry {
    view: ConfirmView,
    token: Token,
    lifetime: Lifetime,
    outcome: Outcome,
    resolved_at: Option<Instant>,
    bad_nonces: u32,
}

#[derive(Default)]
struct State {
    next_id: u64,
    entries: HashMap<u64, Entry>,
}

pub struct Broker {
    state: Mutex<State>,
    events: broadcast::Sender<ConfirmEvent>,
    sink: Arc<dyn CommandSink>,
    ttl: Duration,
    me: Weak<Broker>,
}

impl Broker {
    /// A broker that sends approved commands to `sink`; standing
    /// confirmations live for `ttl`.
    pub fn new(sink: Arc<dyn CommandSink>, ttl: Duration) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new_cyclic(|me| Broker {
            state: Mutex::new(State::default()),
            events,
            sink,
            ttl,
            me: me.clone(),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConfirmEvent> {
        self.events.subscribe()
    }

    /// Open a confirmation for `command` on `pane_id`, already permitted by
    /// `core::policy::check`. Returns its id; the token goes only to
    /// approvers, through the `Requested` event.
    pub fn request(
        &self,
        requester: &Principal,
        pane_id: &str,
        agent: &str,
        command: &str,
        handle: Option<&str>,
        lifetime: Lifetime,
    ) -> Result<u64, ConfirmError> {
        if let Some(h) = handle {
            validate_handle(h)?;
        }
        let (view, token) = {
            let mut state = self.state.lock().unwrap();
            state.prune();
            if let Some(h) = handle {
                let taken = state.entries.values().any(|e| {
                    e.outcome.is_open()
                        && e.view.requested_by == requester.id
                        && e.view.handle.as_deref() == Some(h)
                });
                if taken {
                    return Err(ConfirmError::DuplicateHandle(h.to_string()));
                }
            }
            state.next_id += 1;
            let view = ConfirmView {
                confirm_id: state.next_id,
                pane_id: pane_id.to_string(),
                agent: agent.to_string(),
                command: command.to_string(),
                handle: handle.map(str::to_string),
                requested_by: requester.id.clone(),
                lifetime,
            };
            let token = Token::generate();
            state.entries.insert(
                view.confirm_id,
                Entry {
                    view: view.clone(),
                    token: token.clone(),
                    lifetime,
                    outcome: Outcome::Pending,
                    resolved_at: None,
                    bad_nonces: 0,
                },
            );
            (view, token)
        };
        let id = view.confirm_id;
        log::info!(
            "Confirmation {} for '{}' on pane '{}' requested by {}",
            id,
            view.command,
            view.pane_id,
            view.requested_by
        );
        let _ = self.events.send(ConfirmEvent::Requested {
            view,
            token,
            reminder: false,
        });
        self.start_timers(id, lifetime);
        Ok(id)
    }

    fn start_timers(&self, id: u64, lifetime: Lifetime) {
        let me = self.me.clone();
        let ttl = match lifetime {
            Lifetime::VoiceTurn => VOICE_TURN_TTL,
            Lifetime::Standing => self.ttl,
        };
        let reminder =
            (lifetime == Lifetime::Standing && REMINDER_AFTER < ttl).then_some(REMINDER_AFTER);
        tokio::spawn(async move {
            let start = Instant::now();
            if let Some(after) = reminder {
                tokio::time::sleep_until(start + after).await;
                let Some(broker) = me.upgrade() else { return };
                broker.remind(id);
            }
            tokio::time::sleep_until(start + ttl).await;
            if let Some(broker) = me.upgrade() {
                broker.resolve_if_pending(id, Outcome::Expired, None);
            }
        });
    }

    fn remind(&self, id: u64) {
        let event = {
            let state = self.state.lock().unwrap();
            state
                .entries
                .get(&id)
                .filter(|e| e.outcome.is_open())
                .map(|e| ConfirmEvent::Requested {
                    view: e.view.clone(),
                    token: e.token.clone(),
                    reminder: true,
                })
        };
        if let Some(event) = event {
            let _ = self.events.send(event);
        }
    }

    /// Answer a confirmation as `principal`. An approval sends the command
    /// and returns `Sent` or `Failed`; a denial returns `Denied`.
    pub async fn reply(
        &self,
        principal: &Principal,
        confirm_id: u64,
        approve: bool,
        nonce: Option<&str>,
    ) -> Result<Outcome, ConfirmError> {
        let view = {
            let mut state = self.state.lock().unwrap();
            let entry = state
                .entries
                .get_mut(&confirm_id)
                .filter(|e| e.outcome.is_open())
                .ok_or(ConfirmError::NotFound)?;

            let own = entry.view.requested_by == principal.id;
            let authorized = if own {
                // A companion answering its own voice turn is the user
                // confirming what was heard; anyone else asking for their
                // own command is not an approval.
                entry.lifetime == Lifetime::VoiceTurn
            } else {
                match nonce {
                    Some(n) => {
                        let handle = entry.view.handle.as_deref();
                        if !nonce_matches(&entry.token.0, confirm_id, handle, n) {
                            entry.bad_nonces += 1;
                            let remaining = MAX_BAD_NONCES.saturating_sub(entry.bad_nonces);
                            if remaining == 0 {
                                drop(state);
                                self.resolve_if_pending(
                                    confirm_id,
                                    Outcome::Denied,
                                    Some("too many wrong nonces".to_string()),
                                );
                            }
                            return Err(ConfirmError::BadNonce { remaining });
                        }
                        true
                    }
                    None => principal.can_approve,
                }
            };
            if !authorized {
                return Err(ConfirmError::MayNotApprove);
            }
            if approve {
                entry.outcome = Outcome::Approved;
            }
            entry.view.clone()
        };

        if !approve {
            self.resolve_if_pending(confirm_id, Outcome::Denied, None);
            return Ok(Outcome::Denied);
        }
        log::info!(
            "Confirmation {} approved by {}; sending '{}' to pane '{}'",
            confirm_id,
            principal.id,
            view.command,
            view.pane_id
        );
        let (outcome, detail) = match self.sink.send(&view.pane_id, &view.command).await {
            Ok(()) => (Outcome::Sent, None),
            Err(e) => {
                log::warn!("Confirmation {confirm_id}: sending failed: {e}");
                (Outcome::Failed, Some(e))
            }
        };
        self.finish(confirm_id, outcome, detail);
        Ok(outcome)
    }

    /// The requester withdraws its own pending confirmation.
    pub fn cancel(&self, requester: &Principal, confirm_id: u64) -> Result<Outcome, ConfirmError> {
        let mine = {
            let state = self.state.lock().unwrap();
            state
                .entries
                .get(&confirm_id)
                .is_some_and(|e| e.outcome.is_open() && e.view.requested_by == requester.id)
        };
        if !mine {
            return Err(ConfirmError::NotFound);
        }
        self.resolve_if_pending(confirm_id, Outcome::Cancelled, None);
        Ok(Outcome::Cancelled)
    }

    /// Cancel everything `principal_id` is waiting on: its connection closed.
    pub fn cancel_all(&self, principal_id: &str) {
        for id in self.pending_of(principal_id, None) {
            self.resolve_if_pending(
                id,
                Outcome::Cancelled,
                Some("requester disconnected".to_string()),
            );
        }
    }

    /// Deny the voice-turn confirmations `principal_id` is waiting on: its
    /// next turn has started, so they were not answered.
    pub fn end_voice_turns(&self, principal_id: &str) {
        for id in self.pending_of(principal_id, Some(Lifetime::VoiceTurn)) {
            self.resolve_if_pending(
                id,
                Outcome::Denied,
                Some("the next turn started".to_string()),
            );
        }
    }

    /// A pending confirmation's view and token, so it can be announced
    /// later than when it was opened (a companion's voice turn asks its
    /// question after the reply). Show the token only through
    /// `Token::reveal_to`.
    pub fn pending(&self, confirm_id: u64) -> Option<(ConfirmView, Token)> {
        let state = self.state.lock().unwrap();
        state
            .entries
            .get(&confirm_id)
            .filter(|e| e.outcome.is_open())
            .map(|e| (e.view.clone(), e.token.clone()))
    }

    /// Whether `principal_id` is waiting on any confirmation it asked for.
    pub fn has_pending(&self, principal_id: &str) -> bool {
        !self.pending_of(principal_id, None).is_empty()
    }

    /// Find one of `requester`'s own confirmations, pending or resolved
    /// within `RESOLVED_KEPT`. A handle not found as written is matched
    /// loosely against the requester's pending handles
    /// ([`best_handle_match`]).
    pub fn status(&self, requester: &Principal, key: &StatusKey) -> Option<(ConfirmView, Outcome)> {
        let mut state = self.state.lock().unwrap();
        state.prune();
        let mut found: Vec<&Entry> = state
            .entries
            .values()
            .filter(|e| e.view.requested_by == requester.id)
            .filter(|e| match key {
                StatusKey::Id(id) => e.view.confirm_id == *id,
                StatusKey::Handle(h) => e.view.handle.as_deref() == Some(h.as_str()),
            })
            .collect();
        if found.is_empty() {
            if let StatusKey::Handle(returned) = key {
                let pending: Vec<&Entry> = state
                    .entries
                    .values()
                    .filter(|e| e.outcome.is_open() && e.view.requested_by == requester.id)
                    .filter(|e| e.view.handle.is_some())
                    .collect();
                let handles: Vec<&str> = pending
                    .iter()
                    .filter_map(|e| e.view.handle.as_deref())
                    .collect();
                found.extend(best_handle_match(returned, &handles).map(|i| pending[i]));
            }
        }
        // A handle can be reused once its confirmation is resolved: the
        // newest one is the one meant.
        found.sort_by_key(|e| e.view.confirm_id);
        found.last().map(|e| (e.view.clone(), e.outcome))
    }

    /// The handles of `principal_id`'s pending confirmations, oldest first.
    pub fn pending_handles(&self, principal_id: &str) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut open: Vec<&Entry> = state
            .entries
            .values()
            .filter(|e| e.outcome.is_open() && e.view.requested_by == principal_id)
            .collect();
        open.sort_by_key(|e| e.view.confirm_id);
        open.iter().filter_map(|e| e.view.handle.clone()).collect()
    }

    fn pending_of(&self, principal_id: &str, lifetime: Option<Lifetime>) -> Vec<u64> {
        let state = self.state.lock().unwrap();
        state
            .entries
            .values()
            .filter(|e| e.outcome.is_open() && e.view.requested_by == principal_id)
            .filter(|e| lifetime.is_none_or(|l| e.lifetime == l))
            .map(|e| e.view.confirm_id)
            .collect()
    }

    fn resolve_if_pending(&self, id: u64, outcome: Outcome, detail: Option<String>) {
        let open = {
            let state = self.state.lock().unwrap();
            state.entries.get(&id).is_some_and(|e| e.outcome.is_open())
        };
        if open {
            self.finish(id, outcome, detail);
        }
    }

    fn finish(&self, id: u64, outcome: Outcome, detail: Option<String>) {
        let view = {
            let mut state = self.state.lock().unwrap();
            let Some(entry) = state.entries.get_mut(&id) else {
                return;
            };
            entry.outcome = outcome;
            entry.resolved_at = Some(Instant::now());
            entry.view.clone()
        };
        log::info!("Confirmation {} resolved: {}", id, outcome.as_str());
        let _ = self.events.send(ConfirmEvent::Resolved {
            view,
            outcome,
            detail,
        });
    }
}

impl State {
    fn prune(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, e| {
            e.resolved_at
                .is_none_or(|at| now.duration_since(at) < RESOLVED_KEPT)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libherdr::transport::HerdrTransport;

    fn paired_phone() -> Principal {
        Principal {
            id: "paired:s23".to_string(),
            agents: None,
            can_approve: true,
        }
    }

    fn mcp() -> Principal {
        Principal::local("mcp")
    }

    async fn sent(client: &HerdrClient) -> Vec<(String, String)> {
        match client.transport() {
            HerdrTransport::Mock { state } => state.read().await.command_log.clone(),
            _ => unreachable!("tests use the mock transport"),
        }
    }

    fn broker() -> (Arc<Broker>, HerdrClient) {
        let client = HerdrClient::mock();
        (Broker::new(Arc::new(client.clone()), DEFAULT_TTL), client)
    }

    fn token_of(event: ConfirmEvent) -> String {
        match event {
            ConfirmEvent::Requested { token, .. } => token.0,
            other => panic!("expected Requested, got {other:?}"),
        }
    }

    #[test]
    fn tokens_are_three_bip39_words() {
        let words: Vec<&str> = WORDS.lines().collect();
        assert_eq!(words.len(), 2048);
        let token = Token::generate();
        let parts: Vec<&str> = token.0.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| words.contains(p)));
        assert!(token.0.len() <= 26);
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
    }

    #[test]
    fn nonces_bind_the_id_and_handle() {
        let n = nonce("copper-lantern-ninety", 7, Some("compact-after-tests"));
        assert_eq!(n.len(), 64);
        assert!(nonce_matches(
            "copper-lantern-ninety",
            7,
            Some("compact-after-tests"),
            &n
        ));
        assert!(!nonce_matches(
            "copper-lantern-ninety",
            8,
            Some("compact-after-tests"),
            &n
        ));
        assert!(!nonce_matches("copper-lantern-ninety", 7, None, &n));
        assert!(!nonce_matches(
            "copper-lantern-nine",
            7,
            Some("compact-after-tests"),
            &n
        ));
        assert!(!nonce_matches("copper-lantern-ninety", 7, None, "not hex"));
    }

    #[tokio::test]
    async fn approval_sends_exactly_once() {
        let (broker, client) = broker();
        let id = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some("keep-going"),
                Lifetime::Standing,
            )
            .unwrap();
        let outcome = broker.reply(&paired_phone(), id, true, None).await.unwrap();
        assert_eq!(outcome, Outcome::Sent);
        assert_eq!(
            sent(&client).await,
            vec![("p1".to_string(), "continue".to_string())]
        );
        // A second reply finds nothing pending.
        let again = broker.reply(&paired_phone(), id, true, None).await;
        assert_eq!(again, Err(ConfirmError::NotFound));
        assert_eq!(again.unwrap_err().code(), -32006);
        assert_eq!(sent(&client).await.len(), 1);
    }

    #[tokio::test]
    async fn denial_sends_nothing() {
        let (broker, client) = broker();
        let id = broker
            .request(&mcp(), "p1", "claude", "continue", None, Lifetime::Standing)
            .unwrap();
        assert_eq!(
            broker.reply(&paired_phone(), id, false, None).await,
            Ok(Outcome::Denied)
        );
        assert!(sent(&client).await.is_empty());
    }

    #[tokio::test]
    async fn the_token_reaches_approvers_only() {
        let (broker, _client) = broker();
        let mut events = broker.subscribe();
        broker
            .request(&mcp(), "p1", "claude", "continue", None, Lifetime::Standing)
            .unwrap();
        let ConfirmEvent::Requested { token, .. } = events.recv().await.unwrap() else {
            panic!("expected Requested");
        };
        assert_eq!(token.reveal_to(&mcp()), None);
        assert!(token.reveal_to(&paired_phone()).is_some());
    }

    #[tokio::test]
    async fn a_local_principal_approves_with_the_right_nonce_only() {
        let (broker, client) = broker();
        let mut events = broker.subscribe();
        let ctl = Principal::local("ctl");
        let id = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "/compact",
                Some("compact-now"),
                Lifetime::Standing,
            )
            .unwrap();
        let token = token_of(events.recv().await.unwrap());

        let none = broker.reply(&ctl, id, true, None).await;
        assert_eq!(none, Err(ConfirmError::MayNotApprove));
        assert_eq!(none.unwrap_err().code(), -32007);
        let wrong = broker
            .reply(
                &ctl,
                id,
                true,
                Some(&nonce("abandon-abandon-zoo", id, Some("compact-now"))),
            )
            .await;
        assert_eq!(wrong, Err(ConfirmError::BadNonce { remaining: 4 }));
        assert_eq!(wrong.unwrap_err().code(), -32008);
        let right = nonce(&token, id, Some("compact-now"));
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&right)).await,
            Ok(Outcome::Sent)
        );
        assert_eq!(
            sent(&client).await,
            vec![("p1".to_string(), "/compact".to_string())]
        );
    }

    #[tokio::test]
    async fn five_wrong_nonces_deny() {
        let (broker, client) = broker();
        let ctl = Principal::local("ctl");
        let id = broker
            .request(&mcp(), "p1", "claude", "continue", None, Lifetime::Standing)
            .unwrap();
        let wrong = nonce("abandon-abandon-zoo", id, None);
        for remaining in (0..MAX_BAD_NONCES).rev() {
            let r = broker.reply(&ctl, id, true, Some(&wrong)).await;
            assert_eq!(r, Err(ConfirmError::BadNonce { remaining }));
        }
        assert_eq!(
            broker.status(&mcp(), &StatusKey::Id(id)).unwrap().1,
            Outcome::Denied
        );
        assert!(sent(&client).await.is_empty());
    }

    #[tokio::test]
    async fn a_requester_cannot_approve_its_own_command() {
        let (broker, client) = broker();
        let mut events = broker.subscribe();
        // Even an approving principal, even with the right nonce.
        let phone = paired_phone();
        let id = broker
            .request(&phone, "p1", "claude", "continue", None, Lifetime::Standing)
            .unwrap();
        let token = token_of(events.recv().await.unwrap());
        assert_eq!(
            broker.reply(&phone, id, true, None).await,
            Err(ConfirmError::MayNotApprove)
        );
        assert_eq!(
            broker
                .reply(&phone, id, true, Some(&nonce(&token, id, None)))
                .await,
            Err(ConfirmError::MayNotApprove)
        );
        assert!(sent(&client).await.is_empty());
    }

    #[tokio::test]
    async fn a_companion_answers_its_own_voice_turn() {
        let (broker, client) = broker();
        let phone = paired_phone();
        let id = broker
            .request(
                &phone,
                "p1",
                "claude",
                "continue",
                None,
                Lifetime::VoiceTurn,
            )
            .unwrap();
        assert_eq!(
            broker.reply(&phone, id, true, None).await,
            Ok(Outcome::Sent)
        );
        assert_eq!(sent(&client).await.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn voice_turns_expire_after_a_minute() {
        let (broker, client) = broker();
        let mut events = broker.subscribe();
        let id = broker
            .request(
                &paired_phone(),
                "p1",
                "claude",
                "continue",
                None,
                Lifetime::VoiceTurn,
            )
            .unwrap();
        let _requested = events.recv().await.unwrap();
        tokio::time::sleep(VOICE_TURN_TTL - Duration::from_secs(1)).await;
        assert_eq!(
            broker
                .status(&paired_phone(), &StatusKey::Id(id))
                .unwrap()
                .1,
            Outcome::Pending
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(matches!(
            events.recv().await.unwrap(),
            ConfirmEvent::Resolved {
                outcome: Outcome::Expired,
                ..
            }
        ));
        assert_eq!(
            broker.reply(&paired_phone(), id, true, None).await,
            Err(ConfirmError::NotFound)
        );
        assert!(sent(&client).await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn standing_confirmations_remind_once_then_expire() {
        let (broker, _client) = broker();
        let mut events = broker.subscribe();
        let id = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some("keep-going"),
                Lifetime::Standing,
            )
            .unwrap();
        assert!(matches!(
            events.recv().await.unwrap(),
            ConfirmEvent::Requested {
                reminder: false,
                ..
            }
        ));
        tokio::time::sleep(REMINDER_AFTER + Duration::from_secs(1)).await;
        assert!(matches!(
            events.recv().await.unwrap(),
            ConfirmEvent::Requested { reminder: true, .. }
        ));
        tokio::time::sleep(DEFAULT_TTL).await;
        assert!(matches!(
            events.recv().await.unwrap(),
            ConfirmEvent::Resolved {
                outcome: Outcome::Expired,
                ..
            }
        ));
        assert_eq!(
            broker.status(&mcp(), &StatusKey::Id(id)).unwrap().1,
            Outcome::Expired
        );
    }

    #[tokio::test]
    async fn requesters_cancel_and_disconnect() {
        let (broker, client) = broker();
        let a = broker
            .request(&mcp(), "p1", "claude", "continue", None, Lifetime::Standing)
            .unwrap();
        let b = broker
            .request(&mcp(), "p1", "claude", "/compact", None, Lifetime::Standing)
            .unwrap();
        // Only the requester can cancel.
        assert_eq!(
            broker.cancel(&paired_phone(), a),
            Err(ConfirmError::NotFound)
        );
        assert_eq!(broker.cancel(&mcp(), a), Ok(Outcome::Cancelled));
        broker.cancel_all(&mcp().id);
        assert_eq!(
            broker.status(&mcp(), &StatusKey::Id(b)).unwrap().1,
            Outcome::Cancelled
        );
        assert!(sent(&client).await.is_empty());
    }

    #[tokio::test]
    async fn the_next_turn_ends_a_voice_turn_as_denied() {
        let (broker, _client) = broker();
        let phone = paired_phone();
        let voice = broker
            .request(
                &phone,
                "p1",
                "claude",
                "continue",
                None,
                Lifetime::VoiceTurn,
            )
            .unwrap();
        let standing = broker
            .request(&phone, "p1", "claude", "/compact", None, Lifetime::Standing)
            .unwrap();
        broker.end_voice_turns(&phone.id);
        assert_eq!(
            broker.status(&phone, &StatusKey::Id(voice)).unwrap().1,
            Outcome::Denied
        );
        assert_eq!(
            broker.status(&phone, &StatusKey::Id(standing)).unwrap().1,
            Outcome::Pending
        );
    }

    #[tokio::test]
    async fn status_by_handle_is_private_to_the_requester() {
        let (broker, _client) = broker();
        let id = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some("keep-going"),
                Lifetime::Standing,
            )
            .unwrap();
        let (view, outcome) = broker
            .status(&mcp(), &StatusKey::Handle("keep-going".to_string()))
            .unwrap();
        assert_eq!((view.confirm_id, outcome), (id, Outcome::Pending));
        assert!(broker
            .status(
                &paired_phone(),
                &StatusKey::Handle("keep-going".to_string())
            )
            .is_none());
    }

    #[tokio::test]
    async fn handles_are_validated_and_unique_while_pending() {
        let (broker, _client) = broker();
        for bad in ["ab", "Has-Caps", "no spaces", "under_score"] {
            let r = broker.request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some(bad),
                Lifetime::Standing,
            );
            assert!(matches!(r, Err(ConfirmError::InvalidHandle(_))), "{bad}");
        }
        let first = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some("keep-going"),
                Lifetime::Standing,
            )
            .unwrap();
        let dup = broker.request(
            &mcp(),
            "p1",
            "claude",
            "continue",
            Some("keep-going"),
            Lifetime::Standing,
        );
        assert_eq!(
            dup,
            Err(ConfirmError::DuplicateHandle("keep-going".to_string()))
        );
        // Another requester may use the same handle.
        assert!(broker
            .request(
                &Principal::local("t3"),
                "p1",
                "claude",
                "continue",
                Some("keep-going"),
                Lifetime::Standing
            )
            .is_ok());
        // Once resolved, the requester may reuse it, and status finds the newest.
        broker.cancel(&mcp(), first).unwrap();
        let second = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some("keep-going"),
                Lifetime::Standing,
            )
            .unwrap();
        let (view, _) = broker
            .status(&mcp(), &StatusKey::Handle("keep-going".to_string()))
            .unwrap();
        assert_eq!(view.confirm_id, second);
    }

    // ── Handles written by the model (plan 20261022_08) ─────────────────

    fn no_word() -> String {
        panic!("no extra word expected")
    }

    fn req<'a>(given: Option<&'a str>, utterance: Option<&'a str>) -> HandleRequest<'a> {
        HandleRequest {
            given,
            utterance,
            command: "/usage",
            agent: "Claude",
            tab_code: None,
        }
    }

    #[test]
    fn handles_are_normalised() {
        assert_eq!(normalise_handle("Usage Claude!"), "usage-claude");
        assert_eq!(normalise_handle("--send__usage  to--"), "send-usage-to");
        assert_eq!(normalise_handle(&"a".repeat(80)).len(), 64);
        assert_eq!(match_words("kp-2 High,bay"), ["kp", "two", "high", "bay"]);
    }

    #[test]
    fn the_word_rule() {
        assert!(words_match("claude", "claud"));
        assert!(words_match("highbay", "hihbay"));
        assert!(words_match("compact", "compcat")); // two edits, 6+ letters
        assert!(!words_match("send", "seen")); // two edits, short words
        assert!(!words_match("usage", "continue"));
    }

    #[test]
    fn returned_handles_match_loosely() {
        let stored = ["send-usage-claude-highbay"];
        for returned in [
            "send usage claude high bay",
            "usage-claude-highbay",
            "Send-Usage-Claud-Highbay",
        ] {
            assert_eq!(best_handle_match(returned, &stored), Some(0), "{returned}");
        }
        assert_eq!(
            handle_score("send usage claude high bay", stored[0]),
            (4, 4)
        );
        // "high-bay" stored, "highbay" returned: the joined word counts for both.
        assert_eq!(
            handle_score("send-usage-highbay", "send-usage-high-bay"),
            (4, 4)
        );
        assert_eq!(best_handle_match("compact-codex", &stored), None);
    }

    #[test]
    fn an_ambiguous_handle_matches_neither() {
        let pending = ["send-usage-claude-highbay", "send-continue-claude-highbay"];
        assert_eq!(best_handle_match("send claude highbay", &pending), None);
        assert_eq!(
            best_handle_match("send continue claude highbay", &pending),
            Some(1)
        );
        // An exact match always wins, so `x` and `x-2` stay apart.
        let pending = ["send-usage-claude-highbay", "send-usage-claude-highbay-2"];
        assert_eq!(
            best_handle_match("send-usage-claude-highbay", &pending),
            Some(0)
        );
        assert_eq!(
            best_handle_match("Send Usage Claude Highbay 2", &pending),
            Some(1)
        );
    }

    #[test]
    fn a_handle_from_the_users_words_is_kept() {
        let r = req(
            Some("send-usage-claude-highbay"),
            Some("send usage to claude in high bay"),
        );
        assert_eq!(settle_handle(&r, &[], no_word), "send-usage-claude-highbay");
    }

    #[test]
    fn a_handle_not_from_the_utterance_is_replaced() {
        let r = req(
            Some("restart-codex-server"),
            Some("Send usage to Claude in Highbay."),
        );
        assert_eq!(settle_handle(&r, &[], no_word), "send-usage-claude-highbay");
        // Missing, or too short, the same.
        let r = req(None, Some("send usage to claude in highbay"));
        assert_eq!(settle_handle(&r, &[], no_word), "send-usage-claude-highbay");
        let r = req(Some("usage"), Some("send usage to claude in highbay"));
        assert_eq!(settle_handle(&r, &[], no_word), "send-usage-claude-highbay");
    }

    #[test]
    fn without_words_the_handle_comes_from_the_command_and_agent() {
        let r = HandleRequest {
            tab_code: Some("em bee"),
            ..req(None, None)
        };
        assert_eq!(settle_handle(&r, &[], no_word), "usage-claude-em-bee");
        assert_eq!(
            settle_handle(&req(Some("Usage Claude!"), None), &[], || "zebra".into()),
            "usage-claude-zebra"
        );
    }

    #[test]
    fn a_pending_handle_gets_a_number() {
        let pending = vec!["send-usage-claude-highbay".to_string()];
        let r = req(
            Some("send-usage-claude-highbay"),
            Some("send usage to claude in highbay"),
        );
        assert_eq!(
            settle_handle(&r, &pending, no_word),
            "send-usage-claude-highbay-2"
        );
        let pending = vec![
            "send-usage-claude-highbay".to_string(),
            "send-usage-claude-highbay-2".to_string(),
        ];
        assert_eq!(
            settle_handle(&r, &pending, no_word),
            "send-usage-claude-highbay-3"
        );
    }

    #[test]
    fn a_handle_too_close_to_a_pending_one_gets_a_distinguishing_word() {
        let pending = vec!["send-usage-claude-highbay".to_string()];
        let r = HandleRequest {
            tab_code: Some("em bee"),
            ..req(Some("send-usage-claude"), Some("send usage to claude"))
        };
        let handle = settle_handle(&r, &pending, no_word);
        assert_eq!(handle, "send-usage-claude-em-bee");
        assert_eq!(
            best_handle_match(&handle, &[pending[0].as_str(), &handle]),
            Some(1)
        );
        // No tab code: a BIP-39 word.
        let r = req(Some("send-usage-claude"), Some("send usage to claude"));
        assert_eq!(
            settle_handle(&r, &pending, || "zebra".into()),
            "send-usage-claude-zebra"
        );
        let word = random_word();
        assert!(WORDS.lines().any(|w| w == word), "{word}");
        // Far enough apart already: nothing added.
        let r = req(
            Some("send-continue-claude-highbay"),
            Some("send continue to claude in highbay"),
        );
        assert_eq!(
            settle_handle(&r, &pending, no_word),
            "send-continue-claude-highbay"
        );
    }

    #[tokio::test]
    async fn the_nonce_is_over_the_stored_handle_after_a_loose_match() {
        let (broker, client) = broker();
        let mut events = broker.subscribe();
        let ctl = Principal::local("ctl");
        let stored = "send-usage-claude-highbay";
        let id = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "/usage",
                Some(stored),
                Lifetime::Standing,
            )
            .unwrap();
        let token = token_of(events.recv().await.unwrap());
        assert_eq!(broker.pending_handles(&mcp().id), vec![stored.to_string()]);

        let returned = "send usage claude high bay";
        let (view, outcome) = broker
            .status(&mcp(), &StatusKey::Handle(returned.to_string()))
            .unwrap();
        assert_eq!((view.confirm_id, outcome), (id, Outcome::Pending));
        assert_eq!(view.handle.as_deref(), Some(stored));
        // Only the requester's pending handles are matched loosely.
        assert!(broker
            .status(&ctl, &StatusKey::Handle(returned.to_string()))
            .is_none());

        let over_returned = nonce(&token, id, Some(&normalise_handle(returned)));
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&over_returned)).await,
            Err(ConfirmError::BadNonce { remaining: 4 })
        );
        let over_stored = nonce(&token, id, view.handle.as_deref());
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&over_stored)).await,
            Ok(Outcome::Sent)
        );
        assert_eq!(
            sent(&client).await,
            vec![("p1".to_string(), "/usage".to_string())]
        );
        assert!(broker.pending_handles(&mcp().id).is_empty());
    }
}
