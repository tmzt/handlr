//! `speech.summarize`: a reply about to be read aloud, measured and, when
//! complex, summarised for the ear by Gemini
//! (docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md, as amended at 15:45
//! EDT: the daemon decides what is complex).
//!
//! The client names the reply by its pane and time. The daemon reads the
//! whole reply from the agent's session through the same reader and
//! redaction as `session.peek`, measures it with [`complexity`], and runs
//! the `spoken_summary` prompt of `ADDITIONAL_PROMPTS.md` on a complex one:
//! one system text, one user message, no tools and no history. Answers
//! are kept by `(pane_id, ts)`, so the same reply is never summarised
//! twice.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use libhandlr_proto::ErrorObject;
use libhandlr_proto::common::{SpeechSummarizeParams, SpeechSummarizeResult};
use libhandlr_proto::envelope::codes;
use serde_json::Value;

use super::conn::{self, Conn};
use crate::config::AppConfig;
use crate::prompts::{Prompt, Prompts, SPOKEN_SUMMARY};
use crate::reasoning::{GeminiReasoningEngine, Message, ReasoningEngine, ToolExecutor};

/// How long a summary may take before the call is `UNAVAILABLE`
/// (decision 5).
pub const SUMMARY_TIMEOUT: Duration = Duration::from_secs(10);
/// How many answers are kept, by `(pane_id, ts)` (decision 5).
pub const CACHE_SIZE: usize = 64;
/// The words of a summary when the prompt has no `max_words`.
pub const MAX_WORDS_DEFAULT: usize = 60;

/// Decision 2's thresholds: complex when code-like words are at least
/// `CODE_WORDS_PERCENT` of the words or at least `CODE_WORDS_MIN` of them,
/// when there are `LIST_ITEMS_MIN` list items or a table row, or when the
/// reply is longer than `LENGTH_MAX` characters.
pub const CODE_WORDS_PERCENT: usize = 10;
pub const CODE_WORDS_MIN: usize = 4;
pub const LIST_ITEMS_MIN: usize = 3;
pub const LENGTH_MAX: usize = 600;

/// A reply's measures (decision 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Complexity {
    pub backquotes: bool,
    /// Words, split on whitespace.
    pub words: usize,
    /// Words that look like code.
    pub code_words: usize,
    /// Lines starting `- `, `* ` or `<n>. `.
    pub list_items: usize,
    /// Lines starting `|`.
    pub table_rows: usize,
    /// Characters.
    pub chars: usize,
}

impl Complexity {
    /// Whether the reply goes to Gemini rather than being read as it is.
    pub fn is_complex(&self) -> bool {
        self.backquotes
            || (self.words > 0 && self.code_words * 100 >= self.words * CODE_WORDS_PERCENT)
            || self.code_words >= CODE_WORDS_MIN
            || self.list_items >= LIST_ITEMS_MIN
            || self.table_rows > 0
            || self.chars > LENGTH_MAX
    }
}

/// Measure `text` by decision 2's basic coding measure.
pub fn complexity(text: &str) -> Complexity {
    let mut c = Complexity {
        backquotes: text.contains('`'),
        chars: text.chars().count(),
        ..Complexity::default()
    };
    for word in text.split_whitespace() {
        c.words += 1;
        if code_like(word) {
            c.code_words += 1;
        }
    }
    for line in text.lines().map(str::trim_start) {
        if line.starts_with('|') {
            c.table_rows += 1;
        } else if is_list_item(line) {
            c.list_items += 1;
        }
    }
    c
}

/// Whether a word looks like code: it has `/`, `::`, `_`, `=`, `{`, `}`,
/// `<`, `>`, `#` or `$`, a `.` between two letters, a capital after a
/// lower-case letter, or both letters and digits.
fn code_like(word: &str) -> bool {
    if word.contains("::") || word.contains(['/', '_', '=', '{', '}', '<', '>', '#', '$']) {
        return true;
    }
    let chars: Vec<char> = word.chars().collect();
    let dotted = chars
        .windows(3)
        .any(|w| w[1] == '.' && w[0].is_alphabetic() && w[2].is_alphabetic());
    let camel = chars
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase());
    let mixed = chars.iter().any(|c| c.is_alphabetic()) && chars.iter().any(|c| c.is_ascii_digit());
    dotted || camel || mixed
}

fn is_list_item(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") {
        return true;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
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
    cache: Mutex<VecDeque<((String, String), SpeechSummarizeResult)>>,
}

impl SpeechService {
    pub fn new(config: &AppConfig, options: SpeechOptions) -> Self {
        SpeechService {
            given: options.engine,
            gemini_key_file: super::companion::gemini_key_file(config),
            gemini_model: config.gemini.model.clone(),
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
        let api_key = super::companion::read_key_file(&self.gemini_key_file)?;
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

    fn cached(&self, key: &(String, String)) -> Option<SpeechSummarizeResult> {
        let cache = self.cache.lock().unwrap();
        cache.iter().find(|(k, _)| k == key).map(|(_, r)| r.clone())
    }

    fn keep(&self, key: (String, String), result: SpeechSummarizeResult) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|(k, _)| k != &key);
        if cache.len() >= CACHE_SIZE {
            cache.pop_front();
        }
        cache.push_back((key, result));
    }

    /// Measure `reply` and, when it is complex, summarise it. `agent` and
    /// `project` fill the prompt's user message.
    pub async fn summarize(
        &self,
        agent: &str,
        project: &str,
        reply: &str,
    ) -> Result<SpeechSummarizeResult, ErrorObject> {
        if !complexity(reply).is_complex() {
            return Ok(SpeechSummarizeResult {
                spoken: None,
                complex: false,
            });
        }
        let (prompt, system) = self.prompt.as_ref().map_err(unavailable)?;
        let values: BTreeMap<&str, String> = [
            ("agent", agent.to_string()),
            ("project", project.to_string()),
            ("reply", reply.to_string()),
        ]
        .into_iter()
        .collect();
        let user = prompt.fill(&prompt.user, &values).map_err(unavailable)?;
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
        let max = prompt
            .number("max_words")
            .map_or(MAX_WORDS_DEFAULT, |n| n as usize);
        Ok(SpeechSummarizeResult {
            spoken: Some(cut_at_sentence(&answer, max)),
            complex: true,
        })
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
/// client's text when it is not found), then measured and summarised.
/// One summary runs at a time on a connection; a second waits.
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
    if let Some(result) = daemon.speech.cached(&key) {
        return conn::ok(result);
    }
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
        .summarize(&named.agent_part.shown, &project, &reply)
        .await?;
    daemon.speech.keep(key, result.clone());
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
        - crates/libhandlr_proto/src/common.rs\n\
        All tests pass.";

    #[test]
    fn a_plain_reply_is_not_complex() {
        let c = complexity(PLAIN);
        assert_eq!(c.code_words, 0);
        assert!(!c.is_complex(), "{c:?}");
        assert!(!complexity("Yes, that works.").is_complex());
    }

    #[test]
    fn backquotes_are_complex() {
        assert!(complexity(BACKQUOTES).backquotes);
        assert!(complexity(BACKQUOTES).is_complex());
    }

    #[test]
    fn a_list_of_paths_is_complex() {
        let c = complexity(PATHS);
        assert_eq!(c.list_items, 3);
        assert_eq!(c.code_words, 3);
        assert!(c.is_complex(), "{c:?}");
    }

    #[test]
    fn the_measures_each_count() {
        // Code-like words: each kind.
        for word in [
            "a/b",
            "a::b",
            "snake_case",
            "x=1",
            "{",
            "}",
            "<T>",
            "#3",
            "$HOME",
            "foo.rs",
            "AgentInfo",
            "v0",
            "355f5f0",
        ] {
            assert!(code_like(word), "{word}");
        }
        for word in ["plain", "Hello,", "end.", "3"] {
            assert!(!code_like(word), "{word}");
        }
        // 1 code word in 10 is 10%.
        let one_in_ten = "one two three four five six seven eight nine foo.rs";
        assert!(complexity(one_in_ten).is_complex());
        let one_in_eleven = "one two three four five six seven eight nine ten foo.rs";
        assert!(!complexity(one_in_eleven).is_complex());
        // Two list items are not enough; a table row is.
        assert!(!complexity("Two things:\n- one\n- two").is_complex());
        assert!(complexity("1. one\n2. two\n10. three").is_complex());
        assert!(complexity("| a | b |").is_complex());
        // Length.
        assert!(!complexity(&"word ".repeat(120)).is_complex());
        assert!(complexity(&"word ".repeat(121)).is_complex());
    }

    #[test]
    fn a_long_answer_is_cut_at_a_sentence() {
        assert_eq!(cut_at_sentence(" Short. ", 60), "Short.");
        let long = "One two three. Four five six. Seven eight nine.";
        assert_eq!(cut_at_sentence(long, 7), "One two three. Four five six.");
        assert_eq!(cut_at_sentence("a b c d e", 3), "a b c.");
    }

    #[tokio::test]
    async fn a_complex_reply_is_summarised_and_a_plain_one_is_not() {
        let mock = Arc::new(MockReasoningEngine::with_response("The tests pass."));
        let service = SpeechService::new(
            &AppConfig::default(),
            SpeechOptions {
                engine: Some(mock.clone()),
                ..SpeechOptions::default()
            },
        );
        assert!(service.system_text().unwrap().contains("at most 60 words"));
        let plain = service.summarize("Claude", "handlr", PLAIN).await.unwrap();
        assert_eq!((plain.spoken, plain.complex), (None, false));
        assert!(mock.recorded_transcripts().is_empty());
        let r = service.summarize("Claude", "handlr", PATHS).await.unwrap();
        assert_eq!(r.spoken.as_deref(), Some("The tests pass."));
        assert!(r.complex);
        let sent = mock.recorded_transcripts();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].starts_with("The Claude agent in handlr replied:\n\n"));
        assert!(sent[0].ends_with("All tests pass."));
    }

    #[tokio::test]
    async fn a_missing_prompt_is_unavailable_for_complex_replies_only() {
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
        assert!(service.summarize("Claude", "h", PLAIN).await.is_ok());
        let err = service.summarize("Claude", "h", PATHS).await.unwrap_err();
        assert_eq!(err.code, codes::UNAVAILABLE);
    }

    #[test]
    fn the_cache_keeps_the_last_64() {
        let service = SpeechService::new(&AppConfig::default(), SpeechOptions::default());
        let answer = |n: usize| SpeechSummarizeResult {
            spoken: Some(n.to_string()),
            complex: true,
        };
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
