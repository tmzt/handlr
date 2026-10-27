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
//! With `[summary] cards` (the CHANGES.md departure of 2026-10-24 12:06
//! EDT), the same prompt also summarises each new Finished card, asked for
//! points as well by its `points` member: [`DaemonCards`] is the hub's
//! [`CardSummarizer`].

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use flankr_speech::LocalTime;
use libflankr_proto::ErrorObject;
use libflankr_proto::common::{CARD_POINTS_MAX, SpeechSummarizeParams, SpeechSummarizeResult};
use libflankr_proto::envelope::codes;
use serde_json::Value;

use super::conn::{self, Conn};
use crate::config::AppConfig;
use crate::prompts::{Prompt, Prompts, SPOKEN_SUMMARY};
use crate::reasoning::{GeminiReasoningEngine, Message, ReasoningEngine, ToolExecutor};
use crate::sessions::hub::{CardSummarizer, CardSummary};

/// How long a summary may take before it is given up and the reply is
/// read through the rules (decision 5, as amended at 16:56 EDT).
pub const SUMMARY_TIMEOUT: Duration = Duration::from_secs(10);
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

/// How `speech.summarize` is set up. The default is Gemini, with the key
/// the turn service uses, the shipped prompts and a 10 s timeout.
#[derive(Clone, Default)]
pub struct SpeechOptions {
    /// The engine to use instead of Gemini (tests use a mock).
    pub engine: Option<Arc<dyn ReasoningEngine>>,
    /// `--prompts <path>`: this file in place of the shipped
    /// `ADDITIONAL_PROMPTS.md` (decision 7).
    pub prompts_file: Option<PathBuf>,
    /// Instead of `SUMMARY_TIMEOUT`.
    pub timeout: Option<Duration>,
}

/// The summary prompt, its engine and the answers kept.
pub struct SpeechService {
    given: Option<Arc<dyn ReasoningEngine>>,
    gemini_key_file: PathBuf,
    gemini_model: Option<String>,
    gemini: OnceLock<Arc<dyn ReasoningEngine>>,
    /// `spoken_summary` and its filled system text, or why it is not
    /// there.
    prompt: Result<(Prompt, String), String>,
    timeout: Duration,
    cache: Mutex<VecDeque<((String, String), String)>>,
}

impl SpeechService {
    pub fn new(config: &AppConfig, options: SpeechOptions) -> Self {
        SpeechService {
            given: options.engine,
            gemini_key_file: super::companion::gemini_key_file(config),
            gemini_model: config.gemini.as_ref().and_then(|g| g.model.clone()),
            gemini: OnceLock::new(),
            prompt: load_prompt(options.prompts_file.as_deref()),
            timeout: options.timeout.unwrap_or(SUMMARY_TIMEOUT),
            cache: Mutex::new(VecDeque::new()),
        }
    }

    /// The summary's system text, with its slots filled.
    pub fn system_text(&self) -> Option<&str> {
        self.prompt.as_ref().ok().map(|(_, system)| system.as_str())
    }

    /// The engine: Gemini with the summary's system text and no tools,
    /// built on first use.
    fn engine(&self, system: &str) -> Result<Arc<dyn ReasoningEngine>, String> {
        if let Some(engine) = &self.given {
            return Ok(Arc::clone(engine));
        }
        if let Some(engine) = self.gemini.get() {
            return Ok(Arc::clone(engine));
        }
        let api_key = crate::llm::read_key_file(&self.gemini_key_file)?;
        let mut builder = GeminiReasoningEngine::builder()
            .api_key(api_key)
            .system_prompt(system)
            .tools(Vec::new())
            .max_turns(1);
        if let Some(model) = &self.gemini_model {
            builder = builder.model(model.clone());
        }
        let engine: Arc<dyn ReasoningEngine> = Arc::new(builder.build()?);
        Ok(Arc::clone(self.gemini.get_or_init(|| engine)))
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
        } else {
            match self.summarize(agent, project, reply).await {
                Ok(summary) => Some(summary),
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
        let answer = self.ask(agent, project, reply, false).await?;
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
        let answer = self.ask(agent, project, reply, true).await?;
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
        let mut user = prompt.fill(&prompt.user, &values).map_err(unavailable)?;
        if points {
            let request = prompt
                .settings
                .get("points")
                .ok_or_else(|| unavailable("the prompt has no `points`"))?;
            let request = prompt.fill(request, &values).map_err(unavailable)?;
            user = format!("{}\n\n{}", user.trim_end(), request.trim());
        }
        let engine = self.engine(system).map_err(unavailable)?;
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

/// The hub's card summariser (`[summary] cards`): the daemon's
/// [`SpeechService::card_summary`]. No answer, no Gemini key or an error
/// leaves the card's fields empty.
pub struct DaemonCards(pub Weak<super::Daemon>);

#[async_trait]
impl CardSummarizer for DaemonCards {
    async fn summarize_card(&self, agent: &str, project: &str, reply: &str) -> Option<CardSummary> {
        let daemon = self.0.upgrade()?;
        match daemon.speech.card_summary(agent, project, reply).await {
            Ok(summary) => Some(summary),
            Err(e) => {
                log::warn!("[summary] cards: {}", e.message);
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

/// `spoken_summary` from `file`, or from the shipped prompts. Every error
/// in the file is logged; the summary is unavailable only when its own
/// prompt has one.
fn load_prompt(file: Option<&std::path::Path>) -> Result<(Prompt, String), String> {
    let (prompts, errors) = match file {
        Some(path) => Prompts::load(path).inspect_err(|e| log::warn!("{e}"))?,
        None => Prompts::shipped(),
    };
    for e in &errors {
        log::warn!("ADDITIONAL_PROMPTS: {e}");
    }
    let prompt = prompts
        .get(SPOKEN_SUMMARY)
        .cloned()
        .ok_or_else(|| format!("the prompt `{SPOKEN_SUMMARY}` did not load"))?;
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
    let key = (pane_id.clone(), p.ts.clone());
    let _one = conn.speech.lock().await;
    let spoken_output = conn.spoken_output();
    let cached = spoken_output.then(|| daemon.speech.cached(&key)).flatten();
    let reply = conn::sessions_here(daemon)
        .then(|| daemon.sessions.reply_text(pane, &p.ts))
        .flatten()
        .unwrap_or_else(|| crate::sessions::redact::redact(&p.text));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::MockReasoningEngine;

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
        let service = SpeechService::new(
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
        let service = SpeechService::new(
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
        let service = SpeechService::new(
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
        let service = SpeechService::new(&AppConfig::default(), SpeechOptions::default());
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
