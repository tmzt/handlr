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

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use blake2::Blake2sMac256;
use blake2::digest::{KeyInit, Mac};
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

const NONCE_CONTEXT: &[u8] = b"flankr.confirm/1";

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

/// What an approved confirmation runs in place of sending its command to
/// a pane: a plugin tool that changes something (plan 20261028_04, the
/// 02:10 EDT amendment of 2026-10-29, item 5).
#[async_trait]
pub trait ConfirmAction: Send + Sync {
    async fn run(&self) -> Result<(), String>;
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

    /// The token, for the daemon's own terminal (`flankr serve`) only.
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

/// Words dropped when a handle is built from the command and the agent.
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

// ── The entropy measure (plan 20261023_01, section 3) ───────────────────

/// Words that say nothing about which confirmation is meant: worth 0 bits,
/// and never needed for a full match.
pub const STOP_WORDS: [&str; 21] = [
    "a", "an", "the", "to", "in", "on", "and", "of", "for", "it", "that", "this", "please", "send",
    "run", "tell", "ask", "agent", "pane", "now", "again",
];
/// The fewest bits a match on bits alone needs.
pub const MATCH_FLOOR_BITS: f64 = 6.0;
/// How many more bits than the runner-up a match on bits alone needs.
pub const MATCH_MARGIN_BITS: f64 = 3.0;
/// The fewest bits for a candidate to be listed as one an ambiguous
/// handle might mean.
pub const AMBIGUOUS_LISTED_BITS: f64 = 3.0;
/// Bits compared with this much slack, so `1 + 1 + 1 + 3` is 6.
const BITS_EPSILON: f64 = 1e-9;

const PRIOR_STOP: f64 = 1.0;
const PRIOR_NAMED: f64 = 1.0 / 4.0;
const PRIOR_DIGIT: f64 = 1.0 / 8.0;
const PRIOR_LETTER: f64 = 1.0 / 26.0;
const PRIOR_OTHER: f64 = 1.0 / 64.0;
const PRIOR_DISTINGUISHER: f64 = 1.0 / 2048.0;

/// How likely a word is to turn up in a handle by chance, by its class
/// (plan 20261023_01, section 3.2). Built in: the stop words and the digit
/// words; the server adds the command, agent, project and letter words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordPriors {
    /// Command, agent, project and workspace words: 1/4 each.
    named: HashSet<String>,
    /// The spoken names of letters: 1/26 each.
    letters: HashSet<String>,
}

impl WordPriors {
    /// Only the built-in stop words and digits.
    pub fn new() -> Self {
        Self::default()
    }

    fn add_named<S: AsRef<str>>(mut self, texts: impl IntoIterator<Item = S>) -> Self {
        for text in texts {
            for w in match_words(text.as_ref()) {
                if !STOP_WORDS.contains(&w.as_str()) && !DIGIT_WORDS.contains(&w.as_str()) {
                    self.named.insert(w);
                }
            }
        }
        self
    }

    /// Every word of every allowed command (`/usage` -> `usage`).
    pub fn with_commands<S: AsRef<str>>(self, commands: impl IntoIterator<Item = S>) -> Self {
        self.add_named(commands)
    }

    /// Agents' `[[agents]]` names and spoken names.
    pub fn with_agents<S: AsRef<str>>(self, agents: impl IntoIterator<Item = S>) -> Self {
        self.add_named(agents)
    }

    /// Project and workspace names.
    pub fn with_projects<S: AsRef<str>>(self, projects: impl IntoIterator<Item = S>) -> Self {
        self.add_named(projects)
    }

    /// The names letters are spelled with (`aitch`, `bee`).
    pub fn with_letter_names<S: AsRef<str>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        for name in names {
            self.letters.extend(match_words(name.as_ref()));
        }
        self
    }

    /// The chance of `word` in a handle whose recorded distinguishing words
    /// are `distinguisher`. A stop word is always 1; a distinguishing word
    /// 1/2048; otherwise the most common class the word is in.
    pub fn prior(&self, word: &str, distinguisher: Option<&str>) -> f64 {
        if STOP_WORDS.contains(&word) {
            return PRIOR_STOP;
        }
        if distinguisher.is_some_and(|d| handle_words(d).contains(&word)) {
            return PRIOR_DISTINGUISHER;
        }
        let mut prior = PRIOR_OTHER;
        if self.named.contains(word) {
            prior = prior.max(PRIOR_NAMED);
        }
        if DIGIT_WORDS.contains(&word) {
            prior = prior.max(PRIOR_DIGIT);
        }
        if self.letters.contains(word) {
            prior = prior.max(PRIOR_LETTER);
        }
        prior
    }
}

/// A handle as matching sees it: the stored handle and the distinguishing
/// words the daemon appended to it, if any (joined by `-`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandleInfo {
    pub handle: String,
    pub distinguisher: Option<String>,
}

impl HandleInfo {
    pub fn new(handle: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            distinguisher: None,
        }
    }
}

/// Which candidate a returned handle means, and how it was decided.
#[derive(Debug, Clone, PartialEq)]
pub enum HandleMatch {
    /// Equal to this candidate once normalised.
    Exact(usize),
    /// Every non-stop word of this candidate, and of no other, matched.
    Coverage(usize),
    /// This candidate, with the bits its matched words carry.
    Entropy(usize, f64),
    /// No one candidate: those scoring at least `AMBIGUOUS_LISTED_BITS`,
    /// most bits first.
    Ambiguous(Vec<usize>),
    None,
}

impl HandleMatch {
    /// The candidate chosen, if one was.
    pub fn chosen(&self) -> Option<usize> {
        match self {
            HandleMatch::Exact(i) | HandleMatch::Coverage(i) | HandleMatch::Entropy(i, _) => {
                Some(*i)
            }
            HandleMatch::Ambiguous(_) | HandleMatch::None => None,
        }
    }
}

/// For each of `stored`'s words, whether `returned` matches it: `None`, or
/// `Some(exact)`. A stored word matches a returned word, or two adjacent
/// returned words joined ("high bay" for `highbay`); two adjacent stored
/// words both match a returned word that matches them joined. Each stored
/// word counts once.
fn word_matches(returned: &[String], stored: &[String]) -> Vec<Option<bool>> {
    let joined_returned: Vec<String> = returned.windows(2).map(|p| p.concat()).collect();
    let mut out: Vec<Option<bool>> = stored
        .iter()
        .map(|s| {
            let all = returned.iter().chain(&joined_returned);
            if all.clone().any(|r| r == s) {
                Some(true)
            } else if all.clone().any(|r| words_match(r, s)) {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    for i in 0..stored.len().saturating_sub(1) {
        let joined = format!("{}{}", stored[i], stored[i + 1]);
        let exact = returned.contains(&joined);
        if exact || returned.iter().any(|r| words_match(r, &joined)) {
            for m in &mut out[i..=i + 1] {
                *m = Some(m.unwrap_or(false) || exact);
            }
        }
    }
    out
}

/// Whether any word of `words`, or two adjacent ones joined, matches `w`.
fn contains_match(words: &[String], w: &str) -> bool {
    words.iter().any(|x| words_match(x, w))
        || words.windows(2).any(|pair| words_match(&pair.concat(), w))
}

/// What `returned` scores against each candidate: `(bits, covered)`.
fn score_candidates(
    returned: &str,
    candidates: &[HandleInfo],
    priors: &WordPriors,
) -> Vec<(f64, bool)> {
    let returned = match_words(returned);
    let words: Vec<Vec<String>> = candidates.iter().map(|c| match_words(&c.handle)).collect();
    let n = candidates.len() as f64;
    words
        .iter()
        .enumerate()
        .map(|(i, mine)| {
            let matches = word_matches(&returned, mine);
            let mut bits = 0.0;
            let mut covered = false;
            let mut uncovered = false;
            for (w, m) in mine.iter().zip(&matches) {
                let stop = STOP_WORDS.contains(&w.as_str());
                match m {
                    None => uncovered |= !stop,
                    Some(exact) => {
                        covered |= !stop;
                        let shared = words
                            .iter()
                            .enumerate()
                            .filter(|(j, other)| *j != i && contains_match(other, w))
                            .count() as f64;
                        let prior = priors.prior(w, candidates[i].distinguisher.as_deref());
                        let fuzzy = if *exact { 0.0 } else { 1.0 };
                        bits += (-prior.max(shared / n).log2() - fuzzy).max(0.0);
                    }
                }
            }
            (bits, covered && !uncovered)
        })
        .collect()
}

/// Which of `candidates` a `returned` handle means (plan 20261023_01,
/// section 3.3): an exact match after normalising; else the one candidate
/// whose every non-stop word is matched; else the one with the most bits,
/// if it has at least `MATCH_FLOOR_BITS` and, among two or more,
/// `MATCH_MARGIN_BITS` more than the runner-up.
pub fn entropy_match(
    returned: &str,
    candidates: &[HandleInfo],
    priors: &WordPriors,
) -> HandleMatch {
    let exact = normalise_handle(returned);
    if let Some(i) = candidates.iter().position(|c| c.handle == exact) {
        return HandleMatch::Exact(i);
    }
    inexact_match(returned, candidates, priors)
}

/// [`entropy_match`] without its exact step.
fn inexact_match(returned: &str, candidates: &[HandleInfo], priors: &WordPriors) -> HandleMatch {
    let scores = score_candidates(returned, candidates, priors);
    let covered: Vec<usize> = (0..scores.len()).filter(|i| scores[*i].1).collect();
    if let [only] = covered[..] {
        return HandleMatch::Coverage(only);
    }
    let mut ranked: Vec<usize> = (0..scores.len()).collect();
    ranked.sort_by(|a, b| scores[*b].0.total_cmp(&scores[*a].0));
    let Some(&best) = ranked.first() else {
        return HandleMatch::None;
    };
    let bits = scores[best].0;
    let runner = ranked.get(1).map_or(0.0, |i| scores[*i].0);
    let margin_ok = ranked.len() < 2 || bits - runner >= MATCH_MARGIN_BITS - BITS_EPSILON;
    if bits >= MATCH_FLOOR_BITS - BITS_EPSILON && margin_ok {
        return HandleMatch::Entropy(best, bits);
    }
    let listed: Vec<usize> = ranked
        .into_iter()
        .filter(|i| scores[*i].0 >= AMBIGUOUS_LISTED_BITS - BITS_EPSILON)
        .collect();
    if listed.is_empty() {
        HandleMatch::None
    } else {
        HandleMatch::Ambiguous(listed)
    }
}

/// Whether a new `handle` stays apart from the `pending` ones: returned as
/// written, it is the one the measure picks among them and itself, by
/// coverage or bits (an exact match on itself proves nothing).
fn stands_apart(handle: &HandleInfo, pending: &[HandleInfo], priors: &WordPriors) -> bool {
    if pending.iter().any(|p| p.handle == handle.handle) {
        return false;
    }
    let mut all = pending.to_vec();
    all.push(handle.clone());
    inexact_match(&handle.handle, &all, priors).chosen() == Some(all.len() - 1)
}

/// A random word of the BIP-39 list.
pub fn random_word() -> String {
    let words: Vec<&str> = WORDS.lines().collect();
    let mut bytes = [0u8; 2];
    getrandom::getrandom(&mut bytes).expect("the OS random source is unavailable");
    words[(u16::from_be_bytes(bytes) & 0x7ff) as usize].to_string()
}

/// What the daemon knows when the model asks for a confirmation, to settle
/// its handle (plan 20261022_08, 01:26 EDT amendment).
#[derive(Debug, Clone, Default)]
pub struct HandleRequest<'a> {
    /// The handle the model gave, if any.
    pub given: Option<&'a str>,
    /// The command and the agent's spoken name, used when the model gave
    /// no handle.
    pub command: &'a str,
    pub agent: &'a str,
    /// The agent's tab code spelled out ("em bee"), the first distinguishing
    /// word to try.
    pub tab_code: Option<&'a str>,
}

/// The handle to store for `req`, given the requester's `pending` handles:
/// * the model's handle, normalised, as given: the model has the utterance
///   and resolves what speech-to-text misheard;
/// * only when it is missing or empty, the command and the agent;
/// * at least [`MIN_HANDLE_WORDS`] words;
/// * `-2`, `-3`, ... when it equals a pending handle;
/// * a distinguishing word (the tab code, then words from `extra_word`)
///   until it stands apart from the pending handles under
///   [`entropy_match`].
///
/// The words taken from `extra_word` are returned as its `distinguisher`.
pub fn settle_handle(
    req: &HandleRequest<'_>,
    pending: &[HandleInfo],
    priors: &WordPriors,
    mut extra_word: impl FnMut() -> String,
) -> HandleInfo {
    let long_enough = |handle: &str| handle_words(handle).len() >= MIN_HANDLE_WORDS;
    let given = req.given.map(normalise_handle).unwrap_or_default();
    let handle = if !given.is_empty() {
        given
    } else {
        let mut words: Vec<String> =
            handle_words(&normalise_handle(&format!("{} {}", req.command, req.agent)))
                .into_iter()
                .filter(|w| !ARTICLES.contains(w))
                .map(str::to_string)
                .collect();
        words.dedup();
        join_handle(&words)
    };
    let mut info = HandleInfo::new(handle);
    let mut tab_code = req.tab_code.map(normalise_handle).filter(|t| !t.is_empty());
    let mut add_word = |info: &mut HandleInfo| {
        let word = match tab_code.take() {
            Some(code) => code,
            None => {
                let word = normalise_handle(&extra_word());
                info.distinguisher = Some(match info.distinguisher.take() {
                    Some(d) => format!("{d}-{word}"),
                    None => word.clone(),
                });
                word
            }
        };
        // Keep the new word even when 64 characters would cut it off.
        let mut words: Vec<&str> = handle_words(&info.handle);
        while !words.is_empty() && words.join("-").len() + 1 + word.len() > MAX_HANDLE_LEN {
            words.pop();
        }
        words.push(&word);
        info.handle = words.join("-");
    };
    while !long_enough(&info.handle) {
        add_word(&mut info);
    }
    for _ in 0..16 {
        if pending.iter().any(|p| p.handle == info.handle) {
            let base = info.handle.clone();
            info.handle = (2..)
                .map(|n| {
                    let suffix = format!("-{n}");
                    let mut stem = base.clone();
                    stem.truncate(MAX_HANDLE_LEN - suffix.len());
                    format!("{}{suffix}", stem.trim_end_matches('-'))
                })
                .find(|h| !pending.iter().any(|p| p.handle == *h))
                .expect("some suffix is free");
        }
        if stands_apart(&info, pending, priors) {
            break;
        }
        add_word(&mut info);
    }
    info
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
    /// A dictated message (plan 20261023_02), not a command: `command` is
    /// its text, and its question does not quote it.
    pub message: bool,
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
    /// A handle that could mean more than one pending confirmation: the
    /// handles it might mean, most likely first (`-32009`).
    AmbiguousHandle(Vec<String>),
}

impl ConfirmError {
    pub fn code(&self) -> i64 {
        match self {
            ConfirmError::NotFound => -32006,
            ConfirmError::MayNotApprove => -32007,
            ConfirmError::BadNonce { .. } => -32008,
            ConfirmError::AmbiguousHandle(_) => -32009,
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
            ConfirmError::AmbiguousHandle(candidates) if candidates.is_empty() => {
                f.write_str("ambiguous handle")
            }
            ConfirmError::AmbiguousHandle(candidates) => {
                write!(
                    f,
                    "ambiguous handle; it may mean: {}",
                    candidates.join(", ")
                )
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

/// How `Broker::find` found a confirmation (plan 20261023_01, section 4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Matched {
    /// By its id.
    Id,
    /// By its handle, exactly once normalised.
    Exact,
    /// By a returned handle that matched every non-stop word of it alone.
    Coverage,
    /// By a returned handle whose matched words carry these many bits.
    Entropy(f64),
}

impl Matched {
    pub fn as_str(self) -> &'static str {
        match self {
            Matched::Id => "id",
            Matched::Exact => "exact",
            Matched::Coverage => "coverage",
            Matched::Entropy(_) => "entropy",
        }
    }

    /// The bits of an entropy match.
    pub fn bits(self) -> Option<f64> {
        match self {
            Matched::Entropy(bits) => Some(bits),
            _ => None,
        }
    }
}

/// A confirmation `Broker::find` found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub view: ConfirmView,
    pub outcome: Outcome,
    pub matched: Matched,
}

struct Entry {
    view: ConfirmView,
    /// What is typed on approval in place of `view.command`: the general
    /// agent's request with its provenance prefix (plan 20261027_05,
    /// decision 5).
    typed: Option<String>,
    /// Run on approval instead of sending anything to the pane.
    action: Option<Arc<dyn ConfirmAction>>,
    /// The words the daemon appended to the handle to set it apart, which
    /// are worth more than their class says.
    distinguisher: Option<String>,
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
    /// What handle words are worth (plan 20261023_01, section 3.2).
    priors: Mutex<Arc<WordPriors>>,
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
            priors: Mutex::new(Arc::new(WordPriors::new())),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConfirmEvent> {
        self.events.subscribe()
    }

    /// Set what handle words are worth: the server builds them from the
    /// config and the herdr snapshot.
    pub fn set_word_priors(&self, priors: WordPriors) {
        let mut current = self.priors.lock().unwrap();
        if **current != priors {
            *current = Arc::new(priors);
        }
    }

    /// What handle words are worth now.
    pub fn word_priors(&self) -> Arc<WordPriors> {
        Arc::clone(&self.priors.lock().unwrap())
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
        let handle = handle.map(HandleInfo::new);
        self.request_with(
            requester,
            pane_id,
            agent,
            command,
            handle.as_ref(),
            lifetime,
        )
    }

    /// [`Broker::request`] with a handle that may carry the distinguishing
    /// words the daemon added (`settle_handle`).
    pub fn request_with(
        &self,
        requester: &Principal,
        pane_id: &str,
        agent: &str,
        command: &str,
        handle_info: Option<&HandleInfo>,
        lifetime: Lifetime,
    ) -> Result<u64, ConfirmError> {
        self.open(
            requester,
            pane_id,
            agent,
            command,
            None,
            handle_info,
            lifetime,
            false,
            None,
        )
    }

    /// A confirmation whose approval runs `action` rather than sending
    /// `command` to a pane: `command` and `agent` are what the card and
    /// its question show (a plugin tool and its plugin's name).
    #[allow(clippy::too_many_arguments)]
    pub fn request_action(
        &self,
        requester: &Principal,
        pane_id: &str,
        agent: &str,
        command: &str,
        handle_info: Option<&HandleInfo>,
        lifetime: Lifetime,
        action: Arc<dyn ConfirmAction>,
    ) -> Result<u64, ConfirmError> {
        self.open(
            requester,
            pane_id,
            agent,
            command,
            None,
            handle_info,
            lifetime,
            false,
            Some(action),
        )
    }

    /// [`Broker::request_with`] for a dictated message, already checked by
    /// `core::policy::check_message`: approved, `text` is sent as a command
    /// is, as text and then Enter (plan 20261023_02, decision 6).
    pub fn request_message(
        &self,
        requester: &Principal,
        pane_id: &str,
        agent: &str,
        text: &str,
        handle_info: Option<&HandleInfo>,
        lifetime: Lifetime,
    ) -> Result<u64, ConfirmError> {
        self.open(
            requester,
            pane_id,
            agent,
            text,
            None,
            handle_info,
            lifetime,
            true,
            None,
        )
    }

    /// [`request_message`](Self::request_message) whose approval types
    /// `typed` rather than `text`; the card and its question show `text`
    /// (plan 20261027_05, decision 5: the general agent's `via flankr: `).
    #[allow(clippy::too_many_arguments)]
    pub fn request_message_typed(
        &self,
        requester: &Principal,
        pane_id: &str,
        agent: &str,
        text: &str,
        typed: &str,
        handle_info: Option<&HandleInfo>,
        lifetime: Lifetime,
    ) -> Result<u64, ConfirmError> {
        self.open(
            requester,
            pane_id,
            agent,
            text,
            Some(typed),
            handle_info,
            lifetime,
            true,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn open(
        &self,
        requester: &Principal,
        pane_id: &str,
        agent: &str,
        command: &str,
        typed: Option<&str>,
        handle_info: Option<&HandleInfo>,
        lifetime: Lifetime,
        message: bool,
        action: Option<Arc<dyn ConfirmAction>>,
    ) -> Result<u64, ConfirmError> {
        let handle = handle_info.map(|h| h.handle.as_str());
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
                message,
            };
            let token = Token::generate();
            state.entries.insert(
                view.confirm_id,
                Entry {
                    view: view.clone(),
                    typed: typed.map(str::to_string),
                    action,
                    distinguisher: handle_info.and_then(|h| h.distinguisher.clone()),
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
        let (view, typed, action) = {
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
            (
                entry.view.clone(),
                entry.typed.clone(),
                entry.action.clone(),
            )
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
        let text = typed.as_deref().unwrap_or(&view.command);
        let done = match &action {
            Some(action) => action.run().await,
            None => self.sink.send(&view.pane_id, text).await,
        };
        let (outcome, detail) = match done {
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
    /// within `RESOLVED_KEPT`; `None` when [`Broker::find`] finds none.
    pub fn status(&self, requester: &Principal, key: &StatusKey) -> Option<(ConfirmView, Outcome)> {
        self.find(requester, key)
            .ok()
            .map(|found| (found.view, found.outcome))
    }

    /// Find one of `requester`'s own confirmations, pending or resolved
    /// within `RESOLVED_KEPT`, and say how it was found. A handle not equal
    /// to one of them once normalised is matched against the requester's
    /// pending handles by [`entropy_match`]; when that picks none, the
    /// error is `AmbiguousHandle` (with what it might mean) or `NotFound`.
    pub fn find(&self, requester: &Principal, key: &StatusKey) -> Result<Found, ConfirmError> {
        let priors = self.word_priors();
        let mut state = self.state.lock().unwrap();
        state.prune();
        let normalised = match key {
            StatusKey::Handle(h) => Some(normalise_handle(h)),
            StatusKey::Id(_) => None,
        };
        let mut found: Vec<&Entry> = state
            .entries
            .values()
            .filter(|e| e.view.requested_by == requester.id)
            .filter(|e| match key {
                StatusKey::Id(id) => e.view.confirm_id == *id,
                StatusKey::Handle(_) => e.view.handle.is_some() && e.view.handle == normalised,
            })
            .collect();
        let mut matched = match key {
            StatusKey::Id(_) => Matched::Id,
            StatusKey::Handle(_) => Matched::Exact,
        };
        if found.is_empty()
            && let StatusKey::Handle(returned) = key
        {
            let mut pending: Vec<&Entry> = state
                .entries
                .values()
                .filter(|e| e.outcome.is_open() && e.view.requested_by == requester.id)
                .filter(|e| e.view.handle.is_some())
                .collect();
            pending.sort_by_key(|e| e.view.confirm_id);
            let candidates: Vec<HandleInfo> = pending
                .iter()
                .map(|e| HandleInfo {
                    handle: e.view.handle.clone().unwrap_or_default(),
                    distinguisher: e.distinguisher.clone(),
                })
                .collect();
            match entropy_match(returned, &candidates, &priors) {
                HandleMatch::Exact(i) => found.push(pending[i]),
                HandleMatch::Coverage(i) => {
                    matched = Matched::Coverage;
                    found.push(pending[i]);
                }
                HandleMatch::Entropy(i, bits) => {
                    matched = Matched::Entropy(bits);
                    found.push(pending[i]);
                }
                HandleMatch::Ambiguous(listed) => {
                    return Err(ConfirmError::AmbiguousHandle(
                        listed
                            .into_iter()
                            .map(|i| candidates[i].handle.clone())
                            .collect(),
                    ));
                }
                HandleMatch::None => {}
            }
        }
        // A handle can be reused once its confirmation is resolved: the
        // newest one is the one meant.
        found.sort_by_key(|e| e.view.confirm_id);
        found
            .last()
            .map(|e| Found {
                view: e.view.clone(),
                outcome: e.outcome,
                matched,
            })
            .ok_or(ConfirmError::NotFound)
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

    /// `principal_id`'s pending handles with their distinguishing words,
    /// oldest first: what `settle_handle` keeps a new handle apart from.
    pub fn pending_handle_infos(&self, principal_id: &str) -> Vec<HandleInfo> {
        let state = self.state.lock().unwrap();
        let mut open: Vec<&Entry> = state
            .entries
            .values()
            .filter(|e| e.outcome.is_open() && e.view.requested_by == principal_id)
            .collect();
        open.sort_by_key(|e| e.view.confirm_id);
        open.iter()
            .filter_map(|e| {
                e.view.handle.as_ref().map(|handle| HandleInfo {
                    handle: handle.clone(),
                    distinguisher: e.distinguisher.clone(),
                })
            })
            .collect()
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
        assert!(
            broker
                .status(
                    &paired_phone(),
                    &StatusKey::Handle("keep-going".to_string())
                )
                .is_none()
        );
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
        assert!(
            broker
                .request(
                    &Principal::local("t3"),
                    "p1",
                    "claude",
                    "continue",
                    Some("keep-going"),
                    Lifetime::Standing
                )
                .is_ok()
        );
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

    // ── Handles written by the model (plan 20261022_08) and matched by
    //    entropy (plan 20261023_01) ──────────────────────────────────────

    fn no_word() -> String {
        panic!("no extra word expected")
    }

    fn req(given: Option<&str>) -> HandleRequest<'_> {
        HandleRequest {
            given,
            command: "/usage",
            agent: "Claude",
            tab_code: None,
        }
    }

    fn infos(handles: &[&str]) -> Vec<HandleInfo> {
        handles.iter().map(|h| HandleInfo::new(*h)).collect()
    }

    /// The priors of section 3.4: commands, agents and projects at 1/4,
    /// letters at 1/26.
    fn priors() -> WordPriors {
        WordPriors::new()
            .with_commands(["continue", "/compact", "/usage", "/clear"])
            .with_agents(["claude", "omp", "Agy", "flankr"])
            .with_projects(["highbay", "bubble"])
            .with_letter_names(crate::core::naming::LETTER_NAMES)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
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
    fn word_priors_by_class() {
        let p = priors();
        assert_eq!(p.prior("send", None), 1.0);
        assert_eq!(p.prior("usage", None), 1.0 / 4.0);
        assert_eq!(p.prior("agy", None), 1.0 / 4.0);
        assert_eq!(p.prior("highbay", None), 1.0 / 4.0);
        assert_eq!(p.prior("two", None), 1.0 / 8.0);
        assert_eq!(p.prior("aitch", None), 1.0 / 26.0);
        assert_eq!(p.prior("tests", None), 1.0 / 64.0);
        assert_eq!(p.prior("ripple", Some("ripple")), 1.0 / 2048.0);
        // A stop word never counts, even as a distinguishing word.
        assert_eq!(p.prior("now", Some("now")), 1.0);
        // Without priors, only the stop words and digits apply.
        assert_eq!(WordPriors::new().prior("claude", None), 1.0 / 64.0);
        assert_eq!(WordPriors::new().prior("three", None), 1.0 / 8.0);
    }

    #[test]
    fn one_pending_handle_matches_on_coverage() {
        let stored = infos(&["send-usage-claude-highbay"]);
        let p = WordPriors::new();
        for returned in [
            "send usage claude high bay",
            "usage-claude-highbay",
            "Send-Usage-Claud-Highbay",
        ] {
            assert_eq!(
                entropy_match(returned, &stored, &p),
                HandleMatch::Coverage(0),
                "{returned}"
            );
        }
        assert_eq!(
            entropy_match("Send Usage Claude Highbay", &stored, &p),
            HandleMatch::Exact(0)
        );
        // "high-bay" stored, "highbay" returned: the joined word counts for both.
        assert_eq!(
            entropy_match("send-usage-highbay", &infos(&["send-usage-high-bay"]), &p),
            HandleMatch::Coverage(0)
        );
        assert_eq!(
            entropy_match("compact-codex", &stored, &p),
            HandleMatch::None
        );
        // Section 3.4, n = 1: a vague reference is not taken.
        assert_eq!(
            entropy_match("the usage", &stored, &priors()),
            HandleMatch::None
        );
    }

    #[test]
    fn worked_examples_with_two_pending() {
        // Section 3.4: (A) and (B).
        let pending = infos(&["send-continue-claude-highbay", "send-usage-claude-highbay"]);
        let p = priors();
        let m = |r: &str| entropy_match(r, &pending, &p);
        assert_eq!(m("send usage claude highbay"), HandleMatch::Exact(1));
        assert_eq!(m("send usage claude high bay"), HandleMatch::Coverage(1));
        assert_eq!(m("usage claude highbay"), HandleMatch::Coverage(1));
        assert_eq!(m("send claude highbay"), HandleMatch::None);
        assert_eq!(m("the usage one"), HandleMatch::None);
        // 3 bits for A, 1 for B: under the floor, and A is listed.
        assert_eq!(m("continue claude"), HandleMatch::Ambiguous(vec![0]));
        let scores = score_candidates("continue claude", &pending, &p);
        assert!(
            close(scores[0].0, 3.0) && close(scores[1].0, 1.0),
            "{scores:?}"
        );
        let scores = score_candidates("usage claude highbay", &pending, &p);
        assert!(
            close(scores[0].0, 2.0) && close(scores[1].0, 4.0),
            "{scores:?}"
        );
    }

    #[test]
    fn a_digit_word_tells_a_duplicate_apart() {
        let pending = infos(&[
            "send-continue-claude-highbay",
            "send-continue-claude-highbay-2",
        ]);
        let p = priors();
        // Both covered: bits decide, 6 against 3.
        match entropy_match("continue claude highbay two", &pending, &p) {
            HandleMatch::Entropy(1, bits) => assert!(close(bits, 6.0), "{bits}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            entropy_match("continue claude highbay", &pending, &p),
            HandleMatch::Coverage(0)
        );
        // An exact match always wins, so `x` and `x-2` stay apart.
        assert_eq!(
            entropy_match("send-continue-claude-highbay", &pending, &p),
            HandleMatch::Exact(0)
        );
        assert_eq!(
            entropy_match("Send Continue Claude Highbay 2", &pending, &p),
            HandleMatch::Exact(1)
        );
    }

    #[test]
    fn a_distinguishing_word_carries_eleven_bits() {
        let p = priors();
        let pending = vec![
            HandleInfo::new("send-continue-claude-highbay"),
            HandleInfo {
                handle: "continue-claude-highbay-ripple".to_string(),
                distinguisher: Some("ripple".to_string()),
            },
        ];
        match entropy_match("the ripple one", &pending, &p) {
            HandleMatch::Entropy(1, bits) => assert!(close(bits, 11.0), "{bits}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            entropy_match("continue claude highbay", &pending, &p),
            HandleMatch::Coverage(0)
        );
        // Not recorded as distinguishing: an ordinary word, 6 bits.
        let plain = infos(&[
            "send-continue-claude-highbay",
            "continue-claude-highbay-ripple",
        ]);
        match entropy_match("the ripple one", &plain, &p) {
            HandleMatch::Entropy(1, bits) => assert!(close(bits, 6.0), "{bits}"),
            other => panic!("{other:?}"),
        }
        // A spelled tab code: 4.7 bits a letter.
        let tab = infos(&[
            "send-continue-claude-highbay",
            "continue-claude-highbay-aitch-bee",
        ]);
        match entropy_match("aitch bee", &tab, &p) {
            HandleMatch::Entropy(1, bits) => assert!(close(bits, 9.4), "{bits}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_near_miss_is_worth_a_bit_less_and_never_below_zero() {
        let p = priors();
        let one = infos(&["send-tests-claude"]);
        // `tests` alone: 6 bits; `test`, a near miss, 5.
        let exact = score_candidates("tests", &one, &p)[0].0;
        let fuzzy = score_candidates("test", &one, &p)[0].0;
        assert!(close(exact, 6.0) && close(fuzzy, 5.0), "{exact} {fuzzy}");
        // A near miss of a stop word (`sen` for `send`) is 0, not -1; `sin`
        // does not match `send` at all.
        assert_eq!(score_candidates("sen", &one, &p)[0].0, 0.0);
        assert_eq!(score_candidates("sin", &one, &p)[0].0, 0.0);
        // And a stop word alone never covers a handle.
        assert_eq!(entropy_match("send", &one, &p), HandleMatch::None);
    }

    #[test]
    fn a_handle_that_covers_a_pending_one_does_not_stand_apart() {
        let p = priors();
        let pending = infos(&["send-continue-claude-highbay"]);
        assert!(!stands_apart(
            &HandleInfo::new("continue-claude-highbay"),
            &pending,
            &p
        ));
        assert!(stands_apart(
            &HandleInfo::new("send-usage-claude-highbay"),
            &pending,
            &p
        ));
        assert!(!stands_apart(
            &HandleInfo::new("send-continue-claude-highbay"),
            &pending,
            &p
        ));
        assert!(stands_apart(
            &HandleInfo::new("send-continue-claude-highbay-2"),
            &pending,
            &p
        ));
    }

    #[test]
    fn a_garbled_utterance_keeps_geminis_handle() {
        // Speech-to-text heard "sin did again"; Gemini's handle is kept.
        let settled = settle_handle(
            &req(Some("send-continue-again-flankr")),
            &[],
            &priors(),
            no_word,
        );
        assert_eq!(settled, HandleInfo::new("send-continue-again-flankr"));
        let settled = settle_handle(
            &req(Some("Send-Usage-Claude-Highbay")),
            &[],
            &priors(),
            no_word,
        );
        assert_eq!(settled.handle, "send-usage-claude-highbay");
    }

    #[test]
    fn without_a_handle_it_comes_from_the_command_and_agent() {
        let r = HandleRequest {
            tab_code: Some("em bee"),
            ..req(None)
        };
        let settled = settle_handle(&r, &[], &priors(), no_word);
        // The tab code is not recorded as distinguishing: letters keep their prior.
        assert_eq!(settled, HandleInfo::new("usage-claude-em-bee"));
        let settled = settle_handle(&req(Some("  ")), &[], &priors(), || "zebra".into());
        assert_eq!(settled.handle, "usage-claude-zebra");
        assert_eq!(settled.distinguisher.as_deref(), Some("zebra"));
        // A short handle is padded, not replaced.
        let settled = settle_handle(&req(Some("Usage Claude!")), &[], &priors(), || {
            "zebra".into()
        });
        assert_eq!(settled.handle, "usage-claude-zebra");
    }

    #[test]
    fn a_pending_handle_gets_a_number() {
        let p = priors();
        let pending = infos(&["send-usage-claude-highbay"]);
        let r = req(Some("send-usage-claude-highbay"));
        assert_eq!(
            settle_handle(&r, &pending, &p, no_word).handle,
            "send-usage-claude-highbay-2"
        );
        // A third: `-3` alone is under the floor among three, so it gets a word.
        let pending = infos(&["send-usage-claude-highbay", "send-usage-claude-highbay-2"]);
        let settled = settle_handle(&r, &pending, &p, || "zebra".into());
        assert_eq!(settled.handle, "send-usage-claude-highbay-3-zebra");
        assert_eq!(settled.distinguisher.as_deref(), Some("zebra"));
    }

    #[test]
    fn a_handle_too_close_to_a_pending_one_gets_a_distinguishing_word() {
        let p = priors();
        let pending = infos(&["send-continue-claude-highbay"]);
        let given = Some("continue-claude-highbay");
        let r = HandleRequest {
            tab_code: Some("em bee"),
            ..req(given)
        };
        let settled = settle_handle(&r, &pending, &p, no_word);
        assert_eq!(settled.handle, "continue-claude-highbay-em-bee");
        let both = [pending[0].clone(), settled.clone()];
        assert_eq!(entropy_match("em bee", &both, &p).chosen(), Some(1));
        // No tab code: a BIP-39 word, recorded as distinguishing.
        let settled = settle_handle(&req(given), &pending, &p, || "zebra".into());
        assert_eq!(settled.handle, "continue-claude-highbay-zebra");
        assert_eq!(settled.distinguisher.as_deref(), Some("zebra"));
        let word = random_word();
        assert!(WORDS.lines().any(|w| w == word), "{word}");
        // Far enough apart already: nothing added.
        let pending = infos(&["send-usage-claude-highbay"]);
        let settled = settle_handle(
            &req(Some("send-continue-claude-highbay")),
            &pending,
            &p,
            no_word,
        );
        assert_eq!(settled.handle, "send-continue-claude-highbay");
    }

    #[tokio::test]
    async fn status_by_handle_says_when_it_is_ambiguous() {
        let (broker, _client) = broker();
        broker.set_word_priors(priors());
        for handle in ["send-continue-claude-highbay", "send-usage-claude-highbay"] {
            broker
                .request(
                    &mcp(),
                    "p1",
                    "claude",
                    "continue",
                    Some(handle),
                    Lifetime::Standing,
                )
                .unwrap();
        }
        let r = broker.find(&mcp(), &StatusKey::Handle("continue claude".to_string()));
        let err = r.unwrap_err();
        assert_eq!(
            err,
            ConfirmError::AmbiguousHandle(vec!["send-continue-claude-highbay".to_string()])
        );
        assert_eq!(err.code(), -32009);
        assert_eq!(
            broker.find(
                &mcp(),
                &StatusKey::Handle("send claude highbay".to_string())
            ),
            Err(ConfirmError::NotFound)
        );
        let found = broker
            .find(
                &mcp(),
                &StatusKey::Handle("usage claude highbay".to_string()),
            )
            .unwrap();
        assert_eq!(found.matched, Matched::Coverage);
        assert_eq!(
            found.view.handle.as_deref(),
            Some("send-usage-claude-highbay")
        );
    }

    #[tokio::test]
    async fn the_nonce_is_over_the_stored_handle_after_a_loose_match() {
        let (broker, client) = broker();
        broker.set_word_priors(priors());
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

        // A coverage match.
        let returned = "send usage claude high bay";
        let found = broker
            .find(&mcp(), &StatusKey::Handle(returned.to_string()))
            .unwrap();
        assert_eq!(found.matched, Matched::Coverage);
        assert_eq!(
            (found.view.confirm_id, found.outcome),
            (id, Outcome::Pending)
        );
        assert_eq!(found.view.handle.as_deref(), Some(stored));
        // Only the requester's pending handles are matched loosely.
        assert!(
            broker
                .status(&ctl, &StatusKey::Handle(returned.to_string()))
                .is_none()
        );

        let over_returned = nonce(&token, id, Some(&normalise_handle(returned)));
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&over_returned)).await,
            Err(ConfirmError::BadNonce { remaining: 4 })
        );
        let over_stored = nonce(&token, id, found.view.handle.as_deref());
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&over_stored)).await,
            Ok(Outcome::Sent)
        );
        let _resolved = events.recv().await.unwrap();

        // An entropy match, on the distinguishing word.
        let first = broker
            .request(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some("send-continue-claude-highbay"),
                Lifetime::Standing,
            )
            .unwrap();
        let _requested = events.recv().await.unwrap();
        let rippled = HandleInfo {
            handle: "continue-claude-highbay-ripple".to_string(),
            distinguisher: Some("ripple".to_string()),
        };
        let id = broker
            .request_with(
                &mcp(),
                "p1",
                "claude",
                "continue",
                Some(&rippled),
                Lifetime::Standing,
            )
            .unwrap();
        let token = token_of(events.recv().await.unwrap());
        assert_eq!(
            broker.pending_handle_infos(&mcp().id),
            vec![
                HandleInfo::new("send-continue-claude-highbay"),
                rippled.clone()
            ]
        );
        let found = broker
            .find(&mcp(), &StatusKey::Handle("the ripple one".to_string()))
            .unwrap();
        assert!(matches!(found.matched, Matched::Entropy(b) if close(b, 11.0)));
        assert_eq!(found.view.confirm_id, id);
        assert_ne!(found.view.confirm_id, first);
        let over_returned = nonce(&token, id, Some("the-ripple-one"));
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&over_returned)).await,
            Err(ConfirmError::BadNonce { remaining: 4 })
        );
        let over_stored = nonce(&token, id, found.view.handle.as_deref());
        assert_eq!(
            broker.reply(&ctl, id, true, Some(&over_stored)).await,
            Ok(Outcome::Sent)
        );
        assert_eq!(
            sent(&client).await,
            vec![
                ("p1".to_string(), "/usage".to_string()),
                ("p1".to_string(), "continue".to_string())
            ]
        );
        assert_eq!(
            broker.pending_handles(&mcp().id),
            vec!["send-continue-claude-highbay".to_string()]
        );
    }
}
