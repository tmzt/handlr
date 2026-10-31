//! `speech.summarize`: what to say for a reply about to be read aloud
//! (docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md, as amended at 15:45
//! EDT, the daemon decides what is complex, and at 16:28 EDT, reading rules
//! first and Gemini only in speaking mode).
//!
//! The client names the reply by its pane and time. The daemon reads the
//! whole reply from the agent's session through the same reader and
//! redaction as `session.peek` and measures it with
//! [`flankr_speech::complexity`]. A complex reply, on a connection whose
//! `client.options` turned spoken output on, gets the `spoken_summary`
//! prompt of `ADDITIONAL_PROMPTS.md`: one system text, one user message, no
//! tools and no history. Every other reply is answered with
//! [`flankr_speech::read_aloud`] at the daemon's local time. Summaries are
//! kept by `(pane_id, ts)`, so the same reply is never summarised twice.
//!
//! `speech.catch_up` (the 2026-10-30 13:52 EDT amendment) is one spoken
//! catch-up for the speech a companion held while the user spoke: the
//! `spoken_catch_up` prompt on the same spoken summaries client, with the
//! held replies read whole from their sessions. No model, spoken output
//! off, a failed or empty answer: no catch-up, and the client says what it
//! held as it is.
//!
//! With `[summary] cards` (the CHANGES.md departure of 2026-10-24 12:06
//! EDT), the same prompt also summarises each new Finished card, asked for
//! points as well by its `points` member: [`DaemonCards`] is the hub's
//! [`CardSummarizer`].

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use flankr_speech::LocalTime;
use libflankr_proto::ErrorObject;
use libflankr_proto::common::{
    CARD_POINTS_MAX, SpeechCatchUpParams, SpeechCatchUpResult, SpeechSummarizeParams,
    SpeechSummarizeResult,
};
use libflankr_proto::envelope::codes;
use serde_json::Value;

use super::conn::{self, Conn};
use crate::llm::{LlmService, Profile, Use};
use crate::prompts::{Prompt, Prompts, SPOKEN_CATCH_UP, SPOKEN_SUMMARY};
use crate::reasoning::{Message, ReasoningEngine, ToolExecutor};
use crate::sessions::hub::{CardSummarizer, CardSummary};

/// How long a summary may take before it is given up and the reply is
/// read through the rules (decision 5, as amended at 16:56 EDT).
pub const SUMMARY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a catch-up may take: the client says the held speech as it is
/// after 8 s (the 2026-10-30 13:52 EDT amendment, item 4), so the daemon
/// gives up then too.
pub const CATCH_UP_TIMEOUT: Duration = Duration::from_secs(8);
/// The words of a catch-up when its prompt has no `max_words`.
pub const CATCH_UP_WORDS_DEFAULT: usize = 40;
/// How many answers are kept, by `(pane_id, ts)` (decision 5).
pub const CACHE_SIZE: usize = 64;
/// The words of a summary when the prompt has no `max_words`.
pub const MAX_WORDS_DEFAULT: usize = 60;
/// The longest point of a card's summary, in characters.
pub const POINT_MAX: usize = 160;
/// The line that starts a card summary's points.
pub const POINTS_MARK: &str = "POINTS:";

/// A card summary's answer split into its spoken text, cut to `max_words`,
/// and its points: the lines after a `POINTS:` line, their list marks
/// taken off, at most `CARD_POINTS_MAX` of them, each at most `POINT_MAX`
/// characters. An answer without the line is all spoken text.
pub fn split_card_answer(answer: &str, max_words: usize) -> CardSummary {
    let lines: Vec<&str> = answer.lines().collect();
    let mark = lines
        .iter()
        .position(|l| l.trim().trim_matches('*').eq_ignore_ascii_case(POINTS_MARK))
        .unwrap_or(lines.len());
    let spoken = cut_at_sentence(&lines[..mark].join("\n"), max_words);
    let points = lines
        .iter()
        .skip(mark + 1)
        .map(|l| list_item(l.trim()))
        .filter(|l| !l.is_empty())
        .take(CARD_POINTS_MAX)
        .map(|l| crate::sessions::cut(l, POINT_MAX))
        .collect();
    CardSummary {
        points,
        spoken: (!spoken.is_empty()).then_some(spoken),
    }
}

/// `line` without its list mark: `- `, `* `, `• ` or `1. `.
fn list_item(line: &str) -> &str {
    for mark in ["- ", "* ", "• "] {
        if let Some(rest) = line.strip_prefix(mark) {
            return rest.trim();
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = line[digits..].strip_prefix(". ")
    {
        return rest.trim();
    }
    line
}

/// `text` cut to at most `max` words, at the end of a sentence where one
/// is within them (decision 5).
pub fn cut_at_sentence(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max {
        return text.trim().to_string();
    }
    let kept = &words[..max];
    match kept.iter().rposition(|w| w.ends_with(['.', '?', '!'])) {
        Some(last) => kept[..=last].join(" "),
        None => format!("{}.", kept.join(" ").trim_end_matches([',', ';', ':'])),
    }
}

/// How `speech.summarize` is set up. The default is `[llm]`'s clients for
/// `spoken_summaries` and `card_summaries` (plan 20261027_02), the shipped
/// prompts and a 10 s timeout.
#[derive(Clone, Default)]
pub struct SpeechOptions {
    /// The engine to use for both summaries instead of `[llm]`'s clients,
    /// which counts them as available unless their own `enabled = false`
    /// (tests use a mock).
    pub engine: Option<Arc<dyn ReasoningEngine>>,
    /// `--prompts <path>`: this file in place of the shipped
    /// `ADDITIONAL_PROMPTS.md` (decision 7).
    pub prompts_file: Option<PathBuf>,
    /// Instead of `SUMMARY_TIMEOUT`.
    pub timeout: Option<Duration>,
}

/// The summary prompt, its clients and the answers kept.
pub struct SpeechService {
    llm: Arc<LlmService>,
    /// `spoken_summary` and its filled system text, or why it is not
    /// there.
    prompt: Result<(Prompt, String), String>,
    /// `spoken_catch_up` and its filled system text, or why it is not
    /// there.
    catch_up: Result<(Prompt, String), String>,
    timeout: Duration,
    cache: Mutex<VecDeque<((String, String), String)>>,
}

impl SpeechService {
    pub fn new(llm: Arc<LlmService>, options: SpeechOptions) -> Self {
        let prompts = load_prompts(options.prompts_file.as_deref());
        SpeechService {
            llm,
            prompt: prompts
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|p| prompt_named(p, SPOKEN_SUMMARY)),
            catch_up: prompts.and_then(|p| prompt_named(&p, SPOKEN_CATCH_UP)),
            timeout: options.timeout.unwrap_or(SUMMARY_TIMEOUT),
            cache: Mutex::new(VecDeque::new()),
        }
    }

    /// The summary's system text, with its slots filled.
    pub fn system_text(&self) -> Option<&str> {
        self.prompt.as_ref().ok().map(|(_, system)| system.as_str())
    }

    /// `use_`'s client from `[llm]`: the summary's system text and no
    /// tools, built on first use and shared by the two summaries when they
    /// run on the same backend and model.
    fn engine(&self, use_: Use, system: &str) -> Result<Arc<dyn ReasoningEngine>, String> {
        self.llm.client(use_, Profile::Summary, Some(system))
    }

    fn cached(&self, key: &(String, String)) -> Option<String> {
        let cache = self.cache.lock().unwrap();
        cache.iter().find(|(k, _)| k == key).map(|(_, r)| r.clone())
    }

    fn keep(&self, key: (String, String), result: String) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|(k, _)| k != &key);
        if cache.len() >= CACHE_SIZE {
            cache.pop_front();
        }
        cache.push_back((key, result));
    }

    /// What to say for `reply`: Gemini's summary when it is complex and
    /// `spoken_output` is on (A4), otherwise the reply under the reading
    /// rules at `now` (A3).
    pub async fn answer(
        &self,
        agent: &str,
        project: &str,
        reply: &str,
        spoken_output: bool,
        now: LocalTime,
        cached: Option<String>,
    ) -> Result<SpeechSummarizeResult, ErrorObject> {
        let complex = flankr_speech::complexity(reply).is_complex();
        // The rules' version is made fresh every time: its clock times
        // are relative to now.
        let read = flankr_speech::read_aloud(reply, now);
        // Both versions: a failed summary leaves `read` to be said.
        let summary = if !(complex && spoken_output) {
            None
        } else if cached.is_some() {
            cached
        } else if let Err(e) = self.llm.available(Use::SpokenSummaries) {
            // Read through the local rules, as offline (decision 8).
            log::debug!("speech.summarize: {e}; the reply is read as it is");
            None
        } else {
            match self.summarize(agent, project, reply).await {
                Ok(summary) => {
                    // What is said, so a summary can be checked (plan
                    // 20261023_05, the 2026-10-30 22:08 EDT amendment).
                    log::info!(
                        "speech.summarize: said {}",
                        crate::sessions::redact::redact(&summary)
                    );
                    Some(summary)
                }
                Err(e) => {
                    log::warn!(
                        "speech.summarize: {}; the reply is read as it is",
                        e.message
                    );
                    None
                }
            }
        };
        Ok(SpeechSummarizeResult {
            read,
            summary,
            complex,
        })
    }

    /// Gemini's spoken summary of `reply`. `agent` and `project` fill the
    /// prompt's user message.
    async fn summarize(
        &self,
        agent: &str,
        project: &str,
        reply: &str,
    ) -> Result<String, ErrorObject> {
        let answer = self
            .ask(Use::SpokenSummaries, agent, project, reply, false)
            .await?;
        Ok(cut_at_sentence(&answer, self.max_words()))
    }

    /// A Finished card's summary of `reply`: the spoken text and its points,
    /// asked for by the prompt's `points` member after its user message.
    pub async fn card_summary(
        &self,
        agent: &str,
        project: &str,
        reply: &str,
    ) -> Result<CardSummary, ErrorObject> {
        let answer = self
            .ask(Use::CardSummaries, agent, project, reply, true)
            .await?;
        Ok(split_card_answer(&answer, self.max_words()))
    }

    fn max_words(&self) -> usize {
        self.prompt
            .as_ref()
            .ok()
            .and_then(|(p, _)| p.number("max_words"))
            .map_or(MAX_WORDS_DEFAULT, |n| n as usize)
    }

    /// Gemini's answer to the prompt for `reply`, with the `points` request
    /// after the user message when `points`.
    async fn ask(
        &self,
        use_: Use,
        agent: &str,
        project: &str,
        reply: &str,
        points: bool,
    ) -> Result<String, ErrorObject> {
        let (prompt, system) = self.prompt.as_ref().map_err(unavailable)?;
        let values: BTreeMap<&str, String> = [
            ("agent", agent.to_string()),
            ("project", project.to_string()),
            ("reply", reply.to_string()),
        ]
        .into_iter()
        .collect();
        let user_text = prompt
            .user
            .as_deref()
            .ok_or_else(|| unavailable(format!("the prompt `{SPOKEN_SUMMARY}` has no `user`")))?;
        let mut user = prompt.fill(user_text, &values).map_err(unavailable)?;
        if points {
            let request = prompt
                .settings
                .get("points")
                .ok_or_else(|| unavailable("the prompt has no `points`"))?;
            let request = prompt.fill(request, &values).map_err(unavailable)?;
            user = format!("{}\n\n{}", user.trim_end(), request.trim());
        }
        let engine = self.engine(use_, system).map_err(unavailable)?;
        let turns = [Message {
            role: "user".to_string(),
            content: user,
            name: None,
            thought_signature: None,
        }];
        let answer = tokio::time::timeout(self.timeout, engine.process_turns(&turns, &NoTools))
            .await
            .map_err(|_| unavailable("the summary timed out"))?
            .map_err(unavailable)?
            .filter(|a| !a.trim().is_empty())
            .ok_or_else(|| unavailable("the summary was empty"))?;
        Ok(answer)
    }
}

/// One held item for a catch-up: where it came from, and its text (a
/// reply's whole text when found).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchUpItem {
    pub from: CatchUpFrom,
    pub text: String,
}

/// Where a held item came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUpFrom {
    /// This agent in this project, as shown.
    Agent { agent: String, project: String },
    /// A pane that could not be named.
    Pane,
    /// No pane: a general agent (Gemini) line.
    General,
}

/// What the voice input was, for a catch-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUpSaid {
    /// A message sent to this agent in this project.
    Message {
        agent: String,
        project: String,
        text: String,
    },
    /// A message to a pane that could not be named.
    MessageTo { text: String },
    /// Gemini's prompt.
    Prompt { text: String },
}

impl SpeechService {
    /// The catch-up's system text, with its slots filled.
    pub fn catch_up_system_text(&self) -> Option<&str> {
        self.catch_up
            .as_ref()
            .ok()
            .map(|(_, system)| system.as_str())
    }

    /// One spoken catch-up for `held`, in light of `said` (the 2026-10-30
    /// 13:52 EDT amendment, items 2 to 4). `summary: None` when none is
    /// made, logged once with the reason; never an error.
    pub async fn catch_up(
        &self,
        held: &[CatchUpItem],
        said: Option<&CatchUpSaid>,
        spoken_output: bool,
    ) -> SpeechCatchUpResult {
        let why = if held.is_empty() {
            Some("nothing was held".to_string())
        } else if !spoken_output {
            Some("spoken output is off".to_string())
        } else {
            self.llm.available(Use::SpokenSummaries).err()
        };
        if let Some(why) = why {
            log::info!("speech.catch_up: {why}; the held speech is said as it is");
            return SpeechCatchUpResult { summary: None };
        }
        match self.ask_catch_up(held, said).await {
            Ok(summary) => {
                log::info!(
                    "speech.catch_up: said {}",
                    crate::sessions::redact::redact(&summary)
                );
                SpeechCatchUpResult {
                    summary: Some(summary),
                }
            }
            Err(e) => {
                log::warn!(
                    "speech.catch_up: {}; the held speech is said as it is",
                    e.message
                );
                SpeechCatchUpResult { summary: None }
            }
        }
    }

    async fn ask_catch_up(
        &self,
        held: &[CatchUpItem],
        said: Option<&CatchUpSaid>,
    ) -> Result<String, ErrorObject> {
        let (prompt, system) = self.catch_up.as_ref().map_err(unavailable)?;
        let user_text = prompt
            .user
            .as_deref()
            .ok_or_else(|| unavailable(format!("the prompt `{SPOKEN_CATCH_UP}` has no `user`")))?;
        let values: BTreeMap<&str, String> = [("held", held_text(held)), ("said", said_text(said))]
            .into_iter()
            .collect();
        let user = prompt.fill(user_text, &values).map_err(unavailable)?;
        let engine = self
            .llm
            .client(Use::SpokenSummaries, Profile::CatchUp, Some(system))
            .map_err(unavailable)?;
        let turns = [Message {
            role: "user".to_string(),
            content: user,
            name: None,
            thought_signature: None,
        }];
        let wait = self.timeout.min(CATCH_UP_TIMEOUT);
        let answer = tokio::time::timeout(wait, engine.process_turns(&turns, &NoTools))
            .await
            .map_err(|_| unavailable("the catch-up timed out"))?
            .map_err(unavailable)?
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .ok_or_else(|| unavailable("the catch-up was empty"))?;
        let words = prompt
            .number("max_words")
            .map_or(CATCH_UP_WORDS_DEFAULT, |n| n as usize);
        Ok(cut_at_sentence(&answer, words))
    }
}

/// The held items for the catch-up's user text, one per line, oldest
/// first.
pub fn held_text(held: &[CatchUpItem]) -> String {
    held.iter()
        .map(|item| {
            let text = item.text.trim();
            match &item.from {
                CatchUpFrom::Agent { agent, project } => {
                    format!("- The {agent} agent in {project}: {text}")
                }
                CatchUpFrom::Pane => format!("- A coding agent: {text}"),
                CatchUpFrom::General => format!("- The general agent: {text}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the user said, for the catch-up's user text.
pub fn said_text(said: Option<&CatchUpSaid>) -> String {
    match said {
        Some(CatchUpSaid::Message {
            agent,
            project,
            text,
        }) => format!(
            "Then the user sent the {agent} agent in {project} this message:\n\n{}",
            text.trim()
        ),
        Some(CatchUpSaid::MessageTo { text }) => format!(
            "Then the user sent a coding agent this message:\n\n{}",
            text.trim()
        ),
        Some(CatchUpSaid::Prompt { text }) => {
            format!("Then the user asked the general agent:\n\n{}", text.trim())
        }
        None => "Then the user stopped without sending anything.".to_string(),
    }
}

/// The hub's card summariser (`[llm.uses.card_summaries]`): the daemon's
/// [`SpeechService::card_summary`]. Card summaries unavailable, no answer
/// or an error leaves the card's fields empty.
pub struct DaemonCards(pub Weak<super::Daemon>);

#[async_trait]
impl CardSummarizer for DaemonCards {
    async fn summarize_card(&self, agent: &str, project: &str, reply: &str) -> Option<CardSummary> {
        let daemon = self.0.upgrade()?;
        // Unavailable: cards carry no points or spoken text (decision 8).
        if let Err(e) = daemon.llm.available(Use::CardSummaries) {
            log::debug!("[llm] card_summaries: {e}");
            return None;
        }
        match daemon.speech.card_summary(agent, project, reply).await {
            Ok(summary) => Some(summary),
            Err(e) => {
                log::warn!("[llm] card_summaries: {}", e.message);
                None
            }
        }
    }
}

/// The daemon's local time of day, for reading clock times as relative.
pub fn local_now() -> LocalTime {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let now = secs as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which is a valid, owned
    // value; `now` is a valid time_t.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        // UTC, then.
        let seconds = secs % 86_400;
        return LocalTime {
            hour: (seconds / 3600) as u8,
            minute: (seconds / 60 % 60) as u8,
            second: (seconds % 60) as u8,
            utc_offset_minutes: 0,
        };
    }
    LocalTime {
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
        second: tm.tm_sec as u8,
        utc_offset_minutes: (tm.tm_gmtoff / 60) as i16,
    }
}

/// The prompts of `file`, or the shipped ones. Every error in the file is
/// logged, once.
fn load_prompts(file: Option<&std::path::Path>) -> Result<Prompts, String> {
    let (prompts, errors) = match file {
        Some(path) => Prompts::load(path).inspect_err(|e| log::warn!("{e}"))?,
        None => Prompts::shipped(),
    };
    for e in &errors {
        log::warn!("ADDITIONAL_PROMPTS: {e}");
    }
    Ok(prompts)
}

/// The prompt `name` and its filled system text; unavailable only when its
/// own prompt has an error.
fn prompt_named(prompts: &Prompts, name: &str) -> Result<(Prompt, String), String> {
    let prompt = prompts
        .get(name)
        .cloned()
        .ok_or_else(|| format!("the prompt `{name}` did not load"))?;
    if prompt.user.is_none() {
        let e = format!("the prompt `{name}` has no `user`");
        log::warn!("ADDITIONAL_PROMPTS: {e}");
        return Err(e);
    }
    let system = prompt
        .fill(&prompt.system, &BTreeMap::new())
        .inspect_err(|e| log::warn!("ADDITIONAL_PROMPTS: {e}"))?;
    Ok((prompt, system))
}

fn unavailable(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(codes::UNAVAILABLE, format!("no summary: {e}"))
}

/// The summary runs with no tools: any call is refused.
struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute_tool(&self, name: &str, _args: &Value) -> Result<Value, String> {
        Err(format!("no tools are available here ({name})"))
    }
}

/// `speech.summarize` on a companion connection: the pane resolved and
/// permitted as `session.peek` does, the reply read from the session (the
/// client's text when it is not found), then measured and answered. One
/// runs at a time on a connection; a second waits. Only Gemini's summaries
/// are kept: the rules' text reads clock times relative to now.
pub(super) async fn speech_summarize(
    conn: &Conn,
    p: SpeechSummarizeParams,
) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    let snap = conn::snapshot(conn).await?;
    let pane_id = conn::resolve_target(daemon, &snap, &p.pane_id)?;
    let names = daemon.names(&snap);
    let Some(named) = names.get(&pane_id) else {
        return Err(ErrorObject::new(
            codes::UNKNOWN_AGENT,
            format!("no pane '{pane_id}'"),
        ));
    };
    let pane = &named.agent;
    if pane.agent == "shell" {
        return Err(ErrorObject::new(
            codes::UNKNOWN_AGENT,
            format!("{} runs no agent", named.shown),
        ));
    }
    if !conn.principal().may_reach(&pane.agent) {
        return Err(ErrorObject::new(
            codes::NOT_PERMITTED,
            format!("{} may not reach {}", conn.principal().id, named.shown),
        ));
    }
    // A subagent's reply is found in its transcript (the subagents plan,
    // decision 2); one the session lacks is unknown.
    let found = match conn::sessions_here(daemon).then(|| {
        daemon
            .sessions
            .reply_text_in(pane, p.child.as_deref(), &p.ts)
    }) {
        Some(Ok(found)) => found,
        Some(Err(crate::sessions::UnknownChild)) | None if p.child.is_some() => {
            return Err(conn::unknown_child(&named.shown, p.child.as_deref()));
        }
        _ => None,
    };
    // A subagent's replies are kept apart from its pane's.
    let key = match &p.child {
        Some(child) => (format!("{pane_id}/{child}"), p.ts.clone()),
        None => (pane_id.clone(), p.ts.clone()),
    };
    let _one = conn.speech.lock().await;
    let spoken_output = conn.spoken_output();
    let cached = spoken_output.then(|| daemon.speech.cached(&key)).flatten();
    let reply = found.unwrap_or_else(|| crate::sessions::redact::redact(&p.text));
    let project = named
        .project
        .as_ref()
        .map(|p| p.shown.clone())
        .or_else(|| pane.project.clone())
        .unwrap_or_else(|| "this project".to_string());
    let result = daemon
        .speech
        .answer(
            &named.agent_part.shown,
            &project,
            &reply,
            spoken_output,
            local_now(),
            cached,
        )
        .await?;
    if let Some(summary) = &result.summary {
        daemon.speech.keep(key, summary.clone());
    }
    conn::ok(result)
}

/// `speech.catch_up` on a companion connection (the 2026-10-30 13:52 EDT
/// amendment): each held item named by its pane, a reply read whole from
/// its session when the pane may be reached (the client's text when it is
/// not found), and the message named by its target. herdr out of reach
/// leaves the items unnamed; it is never an error.
pub(super) async fn speech_catch_up(
    conn: &Conn,
    p: SpeechCatchUpParams,
) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    let names = match conn::snapshot(conn).await {
        Ok(snap) => Some(daemon.names(&snap)),
        Err(e) => {
            log::debug!("speech.catch_up: {}; the items are not named", e.message);
            None
        }
    };
    let here = conn::sessions_here(daemon);
    let named = |pane_id: Option<&str>| {
        let pane_id = pane_id.filter(|id| !id.is_empty())?;
        names.as_ref()?.get(pane_id)
    };
    let project_of = |n: &crate::core::naming::NamedAgent| {
        n.project
            .as_ref()
            .map(|p| p.shown.clone())
            .or_else(|| n.agent.project.clone())
            .unwrap_or_else(|| "this project".to_string())
    };
    let held: Vec<CatchUpItem> = p
        .held
        .iter()
        .map(|h| {
            let Some(n) = named(h.pane_id.as_deref()) else {
                let from = match h.pane_id.as_deref() {
                    Some(id) if !id.is_empty() => CatchUpFrom::Pane,
                    _ => CatchUpFrom::General,
                };
                return CatchUpItem {
                    from,
                    text: crate::sessions::redact::redact(&h.text),
                };
            };
            let pane = &n.agent;
            let reachable = pane.agent != "shell" && conn.principal().may_reach(&pane.agent);
            let found = match (&h.ts, reachable && here) {
                (Some(ts), true) => daemon.sessions.reply_text_in(pane, None, ts).ok().flatten(),
                _ => None,
            };
            CatchUpItem {
                from: CatchUpFrom::Agent {
                    agent: n.agent_part.shown.clone(),
                    project: project_of(n),
                },
                text: found.unwrap_or_else(|| crate::sessions::redact::redact(&h.text)),
            }
        })
        .collect();
    let said = p.said.as_ref().map(|s| {
        let text = crate::sessions::redact::redact(&s.text);
        match (&s.pane_id, named(s.pane_id.as_deref())) {
            (_, Some(n)) => CatchUpSaid::Message {
                agent: n.agent_part.shown.clone(),
                project: project_of(n),
                text,
            },
            (Some(_), None) => CatchUpSaid::MessageTo { text },
            (None, None) => CatchUpSaid::Prompt { text },
        }
    });
    let _one = conn.speech.lock().await;
    let result = daemon
        .speech
        .catch_up(&held, said.as_ref(), conn.spoken_output())
        .await;
    conn::ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::llm::{LlmOptions, Overrides};
    use crate::reasoning::MockReasoningEngine;

    /// A service whose summaries are `engine`'s, with `[llm]` as in
    /// `config`.
    fn service(config: &AppConfig, options: SpeechOptions) -> SpeechService {
        let llm = LlmService::new(
            config,
            LlmOptions {
                key_env: Some(crate::llm::KeyEnv::in_dir("/nonexistent")),
                ..LlmOptions::default()
            },
            Overrides {
                summary: options.engine.clone(),
                ..Overrides::default()
            },
        );
        SpeechService::new(llm, options)
    }

    /// Replies as agents gave them.
    const PLAIN: &str = "I ran the tests and they all pass. Shall I go on with the next step?";
    const BACKQUOTES: &str = "Done: `Sessions::reply_text` now joins the parts.";
    const PATHS: &str = "I changed three files:\n\
        - src/server/speech.rs\n\
        - src/sessions/mod.rs\n\
        - crates/libflankr_proto/src/common.rs\n\
        All tests pass.";

    const NOON: LocalTime = LocalTime {
        hour: 12,
        minute: 0,
        second: 0,
        utc_offset_minutes: 0,
    };

    #[test]
    fn a_long_answer_is_cut_at_a_sentence() {
        assert_eq!(cut_at_sentence(" Short. ", 60), "Short.");
        let long = "One two three. Four five six. Seven eight nine.";
        assert_eq!(cut_at_sentence(long, 7), "One two three. Four five six.");
        assert_eq!(cut_at_sentence("a b c d e", 3), "a b c.");
    }

    #[tokio::test]
    async fn a_complex_reply_is_summarised_only_with_spoken_output_on() {
        let mock = Arc::new(MockReasoningEngine::with_response("The tests pass."));
        let service = service(
            &AppConfig::default(),
            SpeechOptions {
                engine: Some(mock.clone()),
                ..SpeechOptions::default()
            },
        );
        assert!(service.system_text().unwrap().contains("at most 60 words"));
        // Not complex: the rules, on or off.
        for on in [false, true] {
            let r = service
                .answer("Claude", "flankr", BACKQUOTES, on, NOON, None)
                .await
                .unwrap();
            assert_eq!(r.read, "Done: sessions reply text now joins the parts.");
            assert_eq!((r.summary, r.complex), (None, false));
        }
        let r = service
            .answer("Claude", "flankr", PLAIN, true, NOON, None)
            .await
            .unwrap();
        assert_eq!(
            (r.read.as_str(), r.summary, r.complex),
            (PLAIN, None, false)
        );
        // Complex, off: the rules, and no call.
        let r = service
            .answer("Claude", "flankr", PATHS, false, NOON, None)
            .await
            .unwrap();
        let rules = "I changed three files: the speech the mod the common All tests pass.";
        assert_eq!((r.read.as_str(), r.summary), (rules, None));
        assert!(r.complex);
        assert!(mock.recorded_transcripts().is_empty());
        // Complex, on: Gemini.
        let r = service
            .answer("Claude", "flankr", PATHS, true, NOON, None)
            .await
            .unwrap();
        // Both versions.
        assert_eq!(r.read, rules);
        assert_eq!(r.summary.as_deref(), Some("The tests pass."));
        assert!(r.complex);
        let sent = mock.recorded_transcripts();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].starts_with("The Claude agent in flankr replied:\n\n"));
        assert!(sent[0].ends_with("All tests pass."));
    }

    #[tokio::test]
    async fn a_missing_prompt_leaves_the_rules_version() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prompts.md");
        std::fs::write(&file, "# [Prompt] spoken_summary\n\n- system: |\n    s\n").unwrap();
        let service = service(
            &AppConfig::default(),
            SpeechOptions {
                engine: Some(Arc::new(MockReasoningEngine::with_response("x"))),
                prompts_file: Some(file),
                ..SpeechOptions::default()
            },
        );
        assert!(service.system_text().is_none());
        assert!(
            service
                .answer("Claude", "h", PLAIN, true, NOON, None)
                .await
                .is_ok()
        );
        assert!(
            service
                .answer("Claude", "h", PATHS, false, NOON, None)
                .await
                .is_ok()
        );
        // A failed summary: the rules' version alone, not an error.
        let r = service
            .answer("Claude", "h", PATHS, true, NOON, None)
            .await
            .unwrap();
        assert_eq!(r.summary, None);
        assert!(r.complex && !r.read.is_empty());
    }

    #[tokio::test]
    async fn a_catch_up_lists_the_held_items_then_what_was_said() {
        let mock = Arc::new(MockReasoningEngine::with_response(" Caught up. "));
        let service = service(
            &AppConfig::default(),
            SpeechOptions {
                engine: Some(mock.clone()),
                ..SpeechOptions::default()
            },
        );
        assert!(
            service
                .catch_up_system_text()
                .unwrap()
                .contains("at most 40 words")
        );
        let held = [
            CatchUpItem {
                from: CatchUpFrom::Agent {
                    agent: "Claude".to_string(),
                    project: "flankr".to_string(),
                },
                text: "The tests pass. ".to_string(),
            },
            CatchUpItem {
                from: CatchUpFrom::General,
                text: "Codex is working.".to_string(),
            },
        ];
        let said = CatchUpSaid::Prompt {
            text: "what now".to_string(),
        };
        // Off, or nothing held: no call.
        assert_eq!(
            service.catch_up(&held, Some(&said), false).await.summary,
            None
        );
        assert_eq!(service.catch_up(&[], Some(&said), true).await.summary, None);
        assert!(mock.recorded_transcripts().is_empty());
        let r = service.catch_up(&held, Some(&said), true).await;
        assert_eq!(r.summary.as_deref(), Some("Caught up."));
        assert_eq!(
            mock.recorded_transcripts(),
            ["While the user spoke, these arrived, oldest first:\n\n\
              - The Claude agent in flankr: The tests pass.\n\
              - The general agent: Codex is working.\n\n\
              Then the user asked the general agent:\n\nwhat now"]
        );
    }

    #[tokio::test]
    async fn without_its_prompt_no_catch_up_is_made_and_summaries_go_on() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prompts.md");
        let shipped = crate::prompts::ADDITIONAL_PROMPTS_RAW;
        let start = shipped.find("# [Prompt] spoken_catch_up").unwrap();
        let end = shipped.find("# [Prompt] planning").unwrap();
        std::fs::write(&file, format!("{}{}", &shipped[..start], &shipped[end..])).unwrap();
        let mock = Arc::new(MockReasoningEngine::with_response("Summary."));
        let service = service(
            &AppConfig::default(),
            SpeechOptions {
                engine: Some(mock.clone()),
                prompts_file: Some(file),
                ..SpeechOptions::default()
            },
        );
        assert!(service.catch_up_system_text().is_none());
        assert!(service.system_text().is_some());
        let held = [CatchUpItem {
            from: CatchUpFrom::Pane,
            text: "Done.".to_string(),
        }];
        assert_eq!(service.catch_up(&held, None, true).await.summary, None);
        assert!(mock.recorded_transcripts().is_empty());
        let r = service
            .answer("Claude", "flankr", PATHS, true, NOON, None)
            .await
            .unwrap();
        assert_eq!(r.summary.as_deref(), Some("Summary."));
    }

    #[test]
    fn a_card_answer_splits_into_spoken_text_and_points() {
        let answer = "It wrote the plan and pushed it.\n\
            POINTS:\n\
            - Wrote docs/design/X_PLAN.md\n\
            * Pushed to main\n\
            \n\
            3. Tests pass\n\
            - four\n- five\n- six\n- seven";
        let s = split_card_answer(answer, 60);
        assert_eq!(
            s.spoken.as_deref(),
            Some("It wrote the plan and pushed it.")
        );
        assert_eq!(
            s.points,
            [
                "Wrote docs/design/X_PLAN.md",
                "Pushed to main",
                "Tests pass",
                "four",
                "five",
                "six"
            ]
        );
        // No mark: all spoken, no points.
        let s = split_card_answer("All done.", 60);
        assert_eq!(s.spoken.as_deref(), Some("All done."));
        assert!(s.points.is_empty());
        // A bold mark, and a long point cut.
        let long = "x".repeat(400);
        let s = split_card_answer(&format!("Done.\n**POINTS:**\n- {long}"), 60);
        assert_eq!(s.points[0].chars().count(), POINT_MAX);
    }

    #[tokio::test]
    async fn a_card_summary_asks_for_points_after_the_user_message() {
        let mock = Arc::new(MockReasoningEngine::with_response(
            "Written and pushed.\nPOINTS:\n- Wrote it\n- Pushed it",
        ));
        let service = service(
            &AppConfig::default(),
            SpeechOptions {
                engine: Some(mock.clone()),
                ..SpeechOptions::default()
            },
        );
        let s = service
            .card_summary("Claude", "flankr", PATHS)
            .await
            .unwrap();
        assert_eq!(s.spoken.as_deref(), Some("Written and pushed."));
        assert_eq!(s.points, ["Wrote it", "Pushed it"]);
        let sent = mock.recorded_transcripts();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].starts_with("The Claude agent in flankr replied:\n\n"));
        assert!(sent[0].contains("All tests pass.\n\nThen, after the spoken text"));
        assert!(sent[0].contains("at most 6 short lines"), "{}", sent[0]);
    }

    #[test]
    fn the_local_time_is_a_time_of_day() {
        let now = local_now();
        assert!(now.hour < 24 && now.minute < 60 && now.second < 61);
        assert!(now.utc_offset_minutes.abs() <= 14 * 60);
    }

    #[test]
    fn the_cache_keeps_the_last_64() {
        let service = service(&AppConfig::default(), SpeechOptions::default());
        let answer = |n: usize| n.to_string();
        for n in 0..=CACHE_SIZE {
            service.keep(("p1".to_string(), n.to_string()), answer(n));
        }
        assert_eq!(service.cached(&("p1".to_string(), "0".to_string())), None);
        assert_eq!(
            service.cached(&("p1".to_string(), "1".to_string())),
            Some(answer(1))
        );
        assert_eq!(service.cache.lock().unwrap().len(), CACHE_SIZE);
    }
}
