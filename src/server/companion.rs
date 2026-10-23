//! `handlr.companion` version 1 in the daemon: sessions, the turn service,
//! notices, vocabulary and interrupt keys
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, decisions 8–13,
//! with the amendments of 12:06, 17:59, 18:03, 18:05 and 18:07 EDT).
//!
//! A turn runs the reasoning engine with today's system prompt, tools and
//! `HerdrContextContributor`, every tool call going through the policy core
//! as the session's principal. A command that needs confirmation opens a
//! `Lifetime::VoiceTurn` confirmation; its question is sent after the
//! reply, and the turn ends once every confirmation it opened is resolved,
//! so `ran` lists what was approved.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use libhandlr_proto::common::methods as common_methods;
use libhandlr_proto::companion::{self, methods, TurnOutcome};
use libhandlr_proto::envelope::codes;
use libhandlr_proto::{ErrorObject, Id, Notification};
use libherdr::events::PaneAgentStatusChanged;
use libherdr::protocol::AgentStatus;
use libinfer::TurnContributor;
use serde_json::Value;
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

use super::conn::Conn;
use super::Daemon;
use crate::config::AppConfig;
use crate::core::agents::extract_agent_name_for_pane;
use crate::core::confirm::{ConfirmEvent, ConfirmView, Lifetime, Outcome, StatusKey, Token};
use crate::core::naming::{self, Names};
use crate::core::policy::Denied;
use crate::core::principal::Principal;
use crate::reasoning::{
    declare_gemini_tools, default_system_prompt, with_handlr_routes, GeminiReasoningEngine,
    Message, ReasoningEngine, SessionManager,
};
use crate::transport::{HerdrContextContributor, HerdrToolExecutor, TurnLog, VoiceTurn};

/// The model entry an interrupted turn leaves in the history.
pub const INTERRUPTED_ENTRY: &str = "[interrupted by user]";
/// How long a turn's engine call may take before the turn is an error.
pub const ENGINE_TIMEOUT: Duration = Duration::from_secs(120);
/// How long an interrupt may spend sending keys, so the `turn.start`
/// response goes out within 500 ms.
const INTERRUPT_KEYS_TIMEOUT: Duration = Duration::from_millis(250);
/// How long an interrupt waits for the snapshot that names and checks the
/// agents it stops.
const INTERRUPT_SNAPSHOT_TIMEOUT: Duration = Duration::from_millis(150);
/// How long a `turn.start` waits for a turn that is only waiting on the
/// confirmations the new turn just denied.
const CLAIM_WAIT: Duration = Duration::from_secs(2);
/// How often a companion connection checks its vocabulary for changes,
/// besides after every turn.
pub const VOCABULARY_INTERVAL: Duration = Duration::from_secs(30);

/// How the turn service is set up. The default is Gemini, with the key from
/// `[gemini] api_key_file` (default `.private/gemini_api_key`) or
/// `GEMINI_API_KEY`, and sessions in `~/.cache/libinfer`.
#[derive(Clone, Default)]
pub struct TurnOptions {
    /// The engine to use instead of Gemini (tests use a scripted
    /// `MockReasoningEngine`).
    pub engine: Option<Arc<dyn ReasoningEngine>>,
    /// Where session histories are kept, instead of `~/.cache/libinfer`.
    pub session_dir: Option<PathBuf>,
}

/// The engine, the session histories, and who is in a turn.
pub struct TurnService {
    given: Option<Arc<dyn ReasoningEngine>>,
    gemini_key_file: PathBuf,
    /// `[gemini] model`; `None` or empty is libinfer-gemini's default.
    gemini_model: Option<String>,
    gemini: OnceLock<Arc<dyn ReasoningEngine>>,
    session_dir: Option<PathBuf>,
    manager: OnceLock<SessionManager>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl TurnService {
    pub fn new(config: &AppConfig, options: TurnOptions) -> Self {
        TurnService {
            given: options.engine,
            gemini_key_file: gemini_key_file(config),
            gemini_model: config.gemini.model.clone(),
            gemini: OnceLock::new(),
            session_dir: options.session_dir,
            manager: OnceLock::new(),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The engine. Gemini is built on first use, so a daemon without a key
    /// still serves everything else, and a turn says why it cannot think.
    /// It is wrapped in handlr's routes, so "never mind" needs no request.
    fn engine(&self) -> Result<Arc<dyn ReasoningEngine>, String> {
        if let Some(engine) = &self.given {
            return Ok(Arc::clone(engine));
        }
        if let Some(engine) = self.gemini.get() {
            return Ok(Arc::clone(engine));
        }
        // Read on first use, so a key added later needs no restart.
        let api_key = read_key_file(&self.gemini_key_file)?;
        let mut builder = GeminiReasoningEngine::builder()
            .api_key(api_key)
            .tools(declare_gemini_tools());
        if let Some(model) = &self.gemini_model {
            builder = builder.model(model.clone());
        }
        if let Some(prompt) = default_system_prompt() {
            builder = builder.system_prompt(prompt);
        }
        let gemini = builder.build()?;
        log::info!("The reasoning engine is Gemini, model {}", gemini.model());
        let engine = with_handlr_routes(Arc::new(gemini));
        Ok(Arc::clone(self.gemini.get_or_init(|| engine)))
    }

    fn manager(&self) -> Result<SessionManager, String> {
        if let Some(manager) = self.manager.get() {
            return Ok(manager.clone());
        }
        let manager = match &self.session_dir {
            Some(dir) => SessionManager::with_dir(dir.clone())?,
            None => SessionManager::new()?,
        };
        Ok(self.manager.get_or_init(|| manager).clone())
    }

    /// The session `id`, shared by every connection of its principal.
    pub(super) fn session(&self, id: &str) -> Arc<Session> {
        let mut sessions = self.sessions.lock().unwrap();
        Arc::clone(
            sessions
                .entry(id.to_string())
                .or_insert_with(|| Arc::new(Session::new(id))),
        )
    }
}

/// The session id of a companion principal (decision 13):
/// `companion:<name>` for a paired device, `companion:local:<client>` on
/// the daemon socket. Paired names cannot contain `:`, so the two never
/// meet.
pub fn session_id(principal: &Principal) -> String {
    match principal.id.strip_prefix("paired:") {
        Some(name) => format!("companion:{name}"),
        None => format!("companion:{}", principal.id),
    }
}

/// One conversation: its running turn and its current agent.
pub struct Session {
    pub id: String,
    turn: Mutex<Option<Running>>,
    /// The pane the conversation last addressed (18:03 EDT amendment).
    current: Mutex<Option<String>>,
    serial: AtomicU64,
}

struct Running {
    serial: u64,
    turn_id: u64,
    request_id: Id,
    stop: CancellationToken,
    /// The engine is done; the turn waits on its confirmations.
    awaiting: Arc<AtomicBool>,
    finished: watch::Receiver<bool>,
}

/// Holds a session's turn slot; dropping it frees the slot.
struct TurnGuard {
    session: Arc<Session>,
    serial: u64,
    stop: CancellationToken,
    awaiting: Arc<AtomicBool>,
    finished: watch::Sender<bool>,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        let mut slot = self.session.turn.lock().unwrap();
        if slot.as_ref().is_some_and(|r| r.serial == self.serial) {
            *slot = None;
        }
        let _ = self.finished.send(true);
    }
}

impl Session {
    fn new(id: &str) -> Self {
        Session {
            id: id.to_string(),
            turn: Mutex::new(None),
            current: Mutex::new(None),
            serial: AtomicU64::new(0),
        }
    }

    /// Take the turn slot, or `-32020` while another turn runs. A turn
    /// that is only waiting on its confirmations (which this `turn.start`
    /// has just denied) is given a moment to finish.
    async fn claim(
        self: &Arc<Self>,
        turn_id: u64,
        request_id: &Id,
    ) -> Result<TurnGuard, ErrorObject> {
        for attempt in 0..2 {
            let mut finished = {
                let mut slot = self.turn.lock().unwrap();
                match &*slot {
                    None => {
                        let serial = self.serial.fetch_add(1, Ordering::Relaxed) + 1;
                        let stop = CancellationToken::new();
                        let awaiting = Arc::new(AtomicBool::new(false));
                        let (finished_tx, finished_rx) = watch::channel(false);
                        *slot = Some(Running {
                            serial,
                            turn_id,
                            request_id: request_id.clone(),
                            stop: stop.clone(),
                            awaiting: Arc::clone(&awaiting),
                            finished: finished_rx,
                        });
                        return Ok(TurnGuard {
                            session: Arc::clone(self),
                            serial,
                            stop,
                            awaiting,
                            finished: finished_tx,
                        });
                    }
                    Some(running) if attempt == 0 && running.awaiting.load(Ordering::SeqCst) => {
                        running.finished.clone()
                    }
                    Some(_) => break,
                }
            };
            let _ = tokio::time::timeout(CLAIM_WAIT, finished.wait_for(|done| *done)).await;
        }
        Err(ErrorObject::new(
            codes::BUSY,
            "a turn is already running in this session",
        ))
    }

    /// Interrupt the running turn if it is `turn_id`. Returns whether one
    /// was.
    pub(super) fn interrupt(&self, turn_id: u64) -> bool {
        let slot = self.turn.lock().unwrap();
        match &*slot {
            Some(running) if running.turn_id == turn_id => {
                running.stop.cancel();
                true
            }
            _ => false,
        }
    }

    /// Interrupt the running turn if `request_id` started it (`cancel` on
    /// the `turn.start` id). Returns whether one did.
    pub(super) fn interrupt_request(&self, request_id: &Id) -> bool {
        let slot = self.turn.lock().unwrap();
        match &*slot {
            Some(running) if &running.request_id == request_id => {
                running.stop.cancel();
                true
            }
            _ => false,
        }
    }
}

/// Withdraws a turn's unanswered confirmations when dropped.
struct Withdraw {
    daemon: Arc<Daemon>,
    principal: Principal,
    log: Arc<TurnLog>,
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        withdraw(&self.daemon, &self.principal, &self.log);
    }
}

fn withdraw(daemon: &Daemon, principal: &Principal, log: &TurnLog) {
    let opened: Vec<u64> = log.confirmations.lock().unwrap().clone();
    for confirm_id in opened {
        // Already resolved ones are not found, which is fine.
        let _ = daemon.broker.cancel(principal, confirm_id);
    }
}

/// Aborts a task when dropped: the turn's work, if the connection closes
/// while it runs.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// `Approve sending "<command>" to <spoken>?`, then ` For <handle, with "-"
/// read as spaces>.` when there is a handle (12:06 EDT amendment).
pub fn confirm_text(command: &str, spoken: &str, handle: Option<&str>) -> String {
    let mut text = format!("Approve sending \"{command}\" to {spoken}?");
    if let Some(handle) = handle.filter(|h| !h.is_empty()) {
        text.push_str(&format!(" For {}.", handle.replace('-', " ")));
    }
    text
}

/// The default key file, under the gitignored `.private`.
const GEMINI_KEY_FILE: &str = ".private/gemini_api_key";

/// `[gemini] api_key_file`, or [`GEMINI_KEY_FILE`], made absolute against
/// the working directory at startup.
fn gemini_key_file(config: &AppConfig) -> PathBuf {
    let path = PathBuf::from(
        config
            .gemini
            .api_key_file
            .as_deref()
            .unwrap_or(GEMINI_KEY_FILE),
    );
    match std::env::current_dir() {
        Ok(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

/// The key in `path`, trimmed; `None` when the file does not exist or is
/// empty, which leaves `GEMINI_API_KEY` to libinfer. A file anyone but its
/// owner may access is refused. Errors name the file, never its contents.
fn read_key_file(path: &Path) -> Result<Option<String>, String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("the Gemini key file {}: {e}", path.display())),
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "the Gemini key file {} has mode {mode:04o}; only its owner may have access \
             (chmod 600)",
            path.display()
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("the Gemini key file {}: {e}", path.display()))?;
    let key = text.trim();
    Ok((!key.is_empty()).then(|| key.to_string()))
}

/// A sentence to speak for a turn that failed with `error`.
pub fn spoken_error(error: &str) -> String {
    if error.contains("GEMINI_API_KEY") {
        "Sorry, I can't think right now: the server has no Gemini API key.".to_string()
    } else if error.contains("Gemini key file") {
        "Sorry, I can't think right now: the server can't read its Gemini key.".to_string()
    } else if error.contains("took too long") {
        "Sorry, that took too long. Please try again.".to_string()
    } else if error.contains("session") {
        "Sorry, I couldn't open our conversation history.".to_string()
    } else {
        "Sorry, something went wrong reaching Gemini. Please try again.".to_string()
    }
}

/// The `confirm.requested` a companion is sent.
pub(super) fn confirm_requested(
    view: &ConfirmView,
    token: &Token,
    principal: &Principal,
    reminder: bool,
    names: Option<super::PaneNames>,
) -> companion::ConfirmRequested {
    let (name, spoken, project) = match names {
        Some(n) => (Some(n.name), Some(n.spoken), n.project),
        None => (None, None, None),
    };
    let say = spoken
        .clone()
        .unwrap_or_else(|| naming::capitalise(&view.agent));
    companion::ConfirmRequested {
        confirm_id: view.confirm_id,
        pane_id: view.pane_id.clone(),
        agent: view.agent.clone(),
        name,
        spoken,
        project,
        command: view.command.clone(),
        requested_by: view.requested_by.clone(),
        handle: view.handle.clone(),
        token: token.reveal_to(principal).map(str::to_string),
        reminder,
        text: confirm_text(&view.command, &say, view.handle.as_deref()),
    }
}

/// Why an interrupt key was not sent.
#[derive(Debug)]
pub enum InterruptError {
    /// No `[[agents]]` entry, or out of this principal's reach.
    Denied(Denied),
    /// herdr refused or could not be reached.
    Herdr(String),
}

/// Send `pane_id`'s agent its interrupt key, as `principal`: only to an
/// agent with an `[[agents]]` entry the principal can reach. Never gated by
/// confirmation (18:03 EDT amendment). `Ok(false)` when the key is empty.
pub async fn send_interrupt_key(
    daemon: &Daemon,
    principal: &Principal,
    agent: &str,
    pane_id: &str,
) -> Result<bool, InterruptError> {
    let Some(config) = daemon
        .agents
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(agent))
    else {
        return Err(InterruptError::Denied(Denied::UnconfiguredAgent {
            agent: agent.to_string(),
            pane_id: pane_id.to_string(),
            permitted: daemon.agents.iter().map(|a| a.name.clone()).collect(),
        }));
    };
    if !principal.may_reach(agent) {
        return Err(InterruptError::Denied(Denied::OutOfReach {
            agent: agent.to_string(),
            principal: principal.id.clone(),
        }));
    }
    if config.interrupt_key.is_empty() {
        return Ok(false);
    }
    log::info!(
        "Sending the interrupt key '{}' to pane '{}' for {}",
        config.interrupt_key,
        pane_id,
        principal.id
    );
    daemon
        .client
        .send_keys(pane_id, &config.interrupt_key)
        .await
        .map(|_| true)
        .map_err(|e| InterruptError::Herdr(e.to_string()))
}

/// `agents.interrupt {target}`, a common method.
pub(super) async fn agents_interrupt(
    daemon: &Daemon,
    principal: &Principal,
    snap: &Value,
    target: &str,
) -> Result<libhandlr_proto::common::AgentsInterruptResult, ErrorObject> {
    let names = daemon.names(snap);
    let named = names.resolve(target).map_err(super::conn::resolve_error)?;
    let pane_id = named.agent.pane_id.clone();
    let agent = extract_agent_name_for_pane(snap, &pane_id);
    let interrupted = send_interrupt_key(daemon, principal, &agent, &pane_id)
        .await
        .map_err(|e| match e {
            InterruptError::Denied(d) => ErrorObject::new(codes::NOT_PERMITTED, d.to_string()),
            InterruptError::Herdr(e) => {
                ErrorObject::new(codes::HERDR_UNAVAILABLE, format!("herdr: {e}"))
            }
        })?;
    Ok(libhandlr_proto::common::AgentsInterruptResult {
        interrupted,
        name: named.shown.clone(),
        spoken: named.spoken.clone(),
    })
}

/// How the turn's work ended, short of an interrupt.
enum WorkEnd {
    Done,
    Error(String),
}

/// `turn.start`: run one turn and answer when it ends.
pub(super) async fn turn_start(
    conn: &Arc<Conn>,
    request_id: &Id,
    params: companion::TurnStartParams,
) -> Result<Value, ErrorObject> {
    let daemon = Arc::clone(&conn.daemon);
    let principal = conn.principal().clone();
    let session = Arc::clone(conn.session());
    let text = params.text.trim().to_string();
    if text.is_empty() {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            "turn.start needs some text",
        ));
    }
    let turn_id = params.turn_id;

    // A new turn ends the voice-turn confirmations still pending, as
    // denied (plan 01's lifetime rule, plan 02's 12:06 EDT amendment).
    daemon.broker.end_voice_turns(&principal.id);
    let guard = session.claim(turn_id, request_id).await?;
    log::info!("Turn {turn_id} of {} started", session.id);

    let log = Arc::new(TurnLog::default());
    // However the turn ends, even by its connection closing, what it asked
    // and nobody answered is withdrawn.
    let _withdraw = Withdraw {
        daemon: Arc::clone(&daemon),
        principal: principal.clone(),
        log: Arc::clone(&log),
    };
    let said: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let work = tokio::spawn(work(
        Arc::clone(conn),
        Arc::clone(&session),
        principal.clone(),
        turn_id,
        text.clone(),
        Arc::clone(&log),
        Arc::clone(&said),
        Arc::clone(&guard.awaiting),
    ));
    let _abort = AbortOnDrop(work.abort_handle());
    let ended = tokio::select! {
        ended = work => Some(ended.unwrap_or_else(|e| WorkEnd::Error(format!("the turn failed: {e}")))),
        _ = guard.stop.cancelled() => None,
    };
    drop(_abort);

    // An interrupted or failed turn's open questions are withdrawn now,
    // before its keys and its answer.
    if !matches!(ended, Some(WorkEnd::Done)) {
        withdraw(&daemon, &principal, &log);
    }
    let addressed = log.addressed.lock().unwrap().clone();
    if addressed.is_some() {
        *session.current.lock().unwrap() = addressed;
    }
    let current = session.current.lock().unwrap().clone();
    let ran_raw: Vec<(String, String)> = log.ran.lock().unwrap().clone();
    // An interrupt answers within 500 ms, so its snapshot waits less.
    let lookup = if ended.is_none() {
        INTERRUPT_SNAPSHOT_TIMEOUT
    } else {
        super::NAME_LOOKUP_TIMEOUT
    };
    let snap = tokio::time::timeout(lookup, daemon.client.list_panes())
        .await
        .ok()
        .and_then(Result::ok);
    let names = snap.as_ref().map(|s| daemon.names(s));

    let mut interrupted = Vec::new();
    let outcome = match &ended {
        Some(WorkEnd::Done) => TurnOutcome::Done,
        Some(WorkEnd::Error(_)) => TurnOutcome::Error,
        None => {
            log::info!("Turn {turn_id} of {} interrupted", session.id);
            if let Some(snap) = &snap {
                let keys = interrupt_keys(
                    &daemon,
                    &principal,
                    snap,
                    names.as_ref(),
                    &ran_raw,
                    current.as_deref(),
                );
                match tokio::time::timeout(INTERRUPT_KEYS_TIMEOUT, keys).await {
                    Ok(stopped) => interrupted = stopped,
                    Err(_) => log::warn!("Sending interrupt keys took too long; gave up"),
                }
            }
            TurnOutcome::Interrupted
        }
    };

    // The history: the user's text, what was said, and how it ended.
    let said = said.lock().unwrap().clone();
    let mut entries = vec![message("user", &text)];
    match outcome {
        TurnOutcome::Done => entries.extend(said.as_deref().map(|s| message("model", s))),
        TurnOutcome::Interrupted => {
            entries.extend(said.as_deref().map(|s| message("model", s)));
            entries.push(message("model", INTERRUPTED_ENTRY));
        }
        // A failed turn leaves no trace, so the next turn is not confused
        // by a question that was never answered.
        TurnOutcome::Error => entries.clear(),
    }
    if !entries.is_empty() {
        match daemon.turns.manager() {
            Ok(manager) => {
                for entry in &entries {
                    if let Err(e) = manager.append_message(&session.id, entry) {
                        log::warn!("Could not write the history of {}: {e}", session.id);
                        break;
                    }
                }
            }
            Err(e) => log::warn!("Could not write the history of {}: {e}", session.id),
        }
    }

    let detail = match ended {
        Some(WorkEnd::Error(e)) => {
            log::warn!("Turn {turn_id} of {} failed: {e}", session.id);
            conn.send(&Notification::new(
                methods::TURN_SAY,
                companion::TurnSay {
                    turn_id,
                    seq: 0,
                    text: spoken_error(&e),
                },
            ));
            Some(e)
        }
        _ => None,
    };
    let ran = ran_raw
        .into_iter()
        .map(|(pane_id, command)| {
            let named = names.as_ref().and_then(|n| n.get(&pane_id));
            companion::Ran {
                name: named.map(|a| a.shown.clone()),
                spoken: named.map(|a| a.spoken.clone()),
                pane_id,
                command,
            }
        })
        .collect();
    drop(guard);
    tokio::spawn(check_vocabulary(Arc::clone(conn)));
    super::conn::ok(companion::TurnResult {
        outcome,
        ran,
        interrupted,
        detail,
    })
}

fn message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_string(),
        content: content.to_string(),
        name: None,
        thought_signature: None,
    }
}

/// The interrupt (18:03 EDT amendment): the key to every pane the turn
/// sent a command to, whatever its status, and to the session's current
/// agent if herdr reports it working.
async fn interrupt_keys(
    daemon: &Daemon,
    principal: &Principal,
    snap: &Value,
    names: Option<&Names>,
    ran: &[(String, String)],
    current: Option<&str>,
) -> Vec<companion::Interrupted> {
    let mut targets: Vec<String> = Vec::new();
    for (pane_id, _) in ran {
        if !targets.contains(pane_id) {
            targets.push(pane_id.clone());
        }
    }
    if let Some(current) = current {
        let working = names
            .and_then(|n| n.get(current))
            .is_some_and(|a| a.agent.status == AgentStatus::Working);
        if working && !targets.iter().any(|t| t == current) {
            targets.push(current.to_string());
        }
    }
    let mut stopped = Vec::new();
    for pane_id in targets {
        let agent = extract_agent_name_for_pane(snap, &pane_id);
        match send_interrupt_key(daemon, principal, &agent, &pane_id).await {
            Ok(true) => {
                let named = names.and_then(|n| n.get(&pane_id));
                stopped.push(companion::Interrupted {
                    name: named
                        .map(|a| a.shown.clone())
                        .unwrap_or_else(|| naming::capitalise(&agent)),
                    spoken: named
                        .map(|a| a.spoken.clone())
                        .unwrap_or_else(|| naming::capitalise(&agent)),
                    pane_id,
                });
            }
            Ok(false) => log::info!("Pane '{pane_id}' has no interrupt key; left alone"),
            Err(e) => log::warn!("No interrupt key sent to pane '{pane_id}': {e:?}"),
        }
    }
    stopped
}

/// The turn's work: the engine, the reply, then the confirmations it
/// opened, each asked and waited on.
#[allow(clippy::too_many_arguments)]
async fn work(
    conn: Arc<Conn>,
    session: Arc<Session>,
    principal: Principal,
    turn_id: u64,
    text: String,
    log: Arc<TurnLog>,
    said: Arc<Mutex<Option<String>>>,
    awaiting: Arc<AtomicBool>,
) -> WorkEnd {
    let daemon = &conn.daemon;
    let engine = match daemon.turns.engine() {
        Ok(engine) => engine,
        Err(e) => return WorkEnd::Error(format!("the reasoning engine: {e}")),
    };
    let manager = match daemon.turns.manager() {
        Ok(manager) => manager,
        Err(e) => return WorkEnd::Error(format!("the session history: {e}")),
    };
    let mut messages = match manager.load_session(&session.id) {
        Ok(history) => history,
        Err(e) => return WorkEnd::Error(format!("the session history: {e}")),
    };
    // The user's text goes in before the context, so the contributor sees
    // it, and is moved last, where the engine reads it.
    let user = message("user", &text);
    messages.push(user.clone());
    let contributor = HerdrContextContributor::new(daemon.client.clone(), daemon.agents.clone())
        .with_watches(Arc::clone(&daemon.watches))
        .with_naming(daemon.naming.clone());
    if let Err(e) = contributor.contribute(&mut messages).await {
        log::warn!("The herdr context could not be added: {e}");
    }
    if let Some(i) = messages.iter().rposition(|m| m == &user) {
        let user = messages.remove(i);
        messages.push(user);
    }

    let executor = HerdrToolExecutor::new(daemon.client.clone(), daemon.agents.clone())
        .with_principal(principal.clone())
        .with_watches(Arc::clone(&daemon.watches))
        .with_naming(daemon.naming.clone())
        .with_voice_turn(VoiceTurn {
            broker: Arc::clone(&daemon.broker),
            log: Arc::clone(&log),
        });
    // Subscribed before any confirmation can be opened, so no resolution
    // is missed.
    let mut events = daemon.broker.subscribe();
    let reply = match tokio::time::timeout(
        ENGINE_TIMEOUT,
        engine.process_turns(&messages, &executor),
    )
    .await
    {
        Ok(Ok(reply)) => reply,
        Ok(Err(e)) => return WorkEnd::Error(e),
        Err(_) => {
            return WorkEnd::Error(format!(
                "the reasoning engine took too long (over {} s)",
                ENGINE_TIMEOUT.as_secs()
            ))
        }
    };

    if let Some(reply) = reply
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
    {
        conn.send(&Notification::new(
            methods::TURN_SAY,
            companion::TurnSay {
                turn_id,
                seq: 0,
                text: reply.clone(),
            },
        ));
        *said.lock().unwrap() = Some(reply);
    }

    // Ask each confirmation after the reply, then wait for its answer.
    let opened: Vec<u64> = log.confirmations.lock().unwrap().clone();
    if opened.is_empty() {
        return WorkEnd::Done;
    }
    awaiting.store(true, Ordering::SeqCst);
    conn.opened(&opened);
    for confirm_id in &opened {
        if let Some((view, token)) = daemon.broker.pending(*confirm_id) {
            let names = daemon.names_for(&view.pane_id).await;
            conn.send(&Notification::new(
                common_methods::CONFIRM_REQUESTED,
                confirm_requested(&view, &token, &principal, false, names),
            ));
        }
    }
    for confirm_id in opened {
        loop {
            let outcome = daemon
                .broker
                .status(&principal, &StatusKey::Id(confirm_id))
                .map(|(_, outcome)| outcome);
            match outcome {
                Some(Outcome::Pending) | Some(Outcome::Approved) => {}
                Some(Outcome::Sent) => {
                    if let Some((view, _)) =
                        daemon.broker.status(&principal, &StatusKey::Id(confirm_id))
                    {
                        log.ran.lock().unwrap().push((view.pane_id, view.command));
                    }
                    break;
                }
                _ => break,
            }
            match events.recv().await {
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    WorkEnd::Done
}

/// Send `vocabulary.changed` if the vocabulary differs from what this
/// connection was last given (on each turn and every
/// `VOCABULARY_INTERVAL`).
pub(super) async fn check_vocabulary(conn: Arc<Conn>) {
    let Ok(Ok(snap)) =
        tokio::time::timeout(super::NAME_LOOKUP_TIMEOUT, conn.daemon.client.list_panes()).await
    else {
        return;
    };
    let vocabulary = conn.daemon.names(&snap).vocabulary();
    let changed = {
        let mut last = conn.vocabulary.lock().unwrap();
        if *last == vocabulary {
            false
        } else {
            *last = vocabulary.clone();
            true
        }
    };
    if changed {
        conn.send(&Notification::new(
            methods::VOCABULARY_CHANGED,
            companion::VocabularyChanged { vocabulary },
        ));
    }
}

/// The vocabulary for `hello`, or none when herdr cannot be read.
pub(super) async fn vocabulary(daemon: &Daemon) -> Vec<String> {
    match tokio::time::timeout(super::NAME_LOOKUP_TIMEOUT, daemon.client.list_panes()).await {
        Ok(Ok(snap)) => daemon.names(&snap).vocabulary(),
        _ => Vec::new(),
    }
}

/// A status change of a watched agent: queue its notice if it is worth
/// announcing, then send what may be sent.
pub(super) async fn agent_status(
    conn: &Arc<Conn>,
    pane_id: &str,
    agent: &str,
    status: &AgentStatus,
) {
    let change = PaneAgentStatusChanged {
        pane_id: pane_id.to_string(),
        workspace_id: String::new(),
        agent_status: status.clone(),
        agent: Some(agent.to_string()),
        display_agent: None,
        title: None,
    };
    let noted = conn.notices.lock().unwrap().note(&change);
    let Some(status) = noted else { return };
    let names = conn.daemon.names_for(pane_id).await;
    let spoken = names
        .as_ref()
        .map(|n| n.spoken.clone())
        .unwrap_or_else(|| naming::capitalise(agent));
    let Some(text) = crate::notify::announcement(&status, &spoken) else {
        return;
    };
    let id = conn.next_notice.fetch_add(1, Ordering::Relaxed) + 1;
    let notice = companion::Notice {
        id,
        kind: companion::notice_kinds::AGENT_STATUS.to_string(),
        pane_id: pane_id.to_string(),
        name: names.as_ref().map(|n| n.name.clone()),
        spoken: Some(spoken),
        text,
    };
    conn.notices.lock().unwrap().queue(notice);
    flush_notices(conn);
}

/// Send the held notices, oldest first, unless a confirmation this
/// principal asked for is pending (decision 12).
pub(super) fn flush_notices(conn: &Conn) {
    let pending = conn.daemon.broker.has_pending(&conn.principal().id);
    loop {
        let next = conn.notices.lock().unwrap().take_speakable(pending);
        let Some(notice) = next else { break };
        conn.send(&Notification::new(methods::NOTICE, notice));
    }
}

/// Whether a `Requested` event is a companion's own voice-turn question,
/// which its turn sends after the reply rather than the forwarder.
pub(super) fn is_own_voice_turn(event: &ConfirmEvent, principal: &Principal) -> bool {
    matches!(event, ConfirmEvent::Requested { view, .. }
        if view.requested_by == principal.id && view.lifetime == Lifetime::VoiceTurn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_name_the_command_the_agent_and_the_handle() {
        assert_eq!(
            confirm_text("continue", "the handlr agent", None),
            "Approve sending \"continue\" to the handlr agent?"
        );
        assert_eq!(
            confirm_text("/compact", "Claude", Some("compact-after-tests")),
            "Approve sending \"/compact\" to Claude? For compact after tests."
        );
    }

    #[test]
    fn session_ids_follow_the_principal() {
        let paired = Principal {
            id: "paired:s23".to_string(),
            agents: None,
            can_approve: true,
        };
        assert_eq!(session_id(&paired), "companion:s23");
        assert_eq!(
            session_id(&Principal::local("text")),
            "companion:local:text"
        );
    }

    #[test]
    fn errors_are_spoken_plainly() {
        let s = spoken_error("the reasoning engine: GEMINI_API_KEY not set");
        assert!(s.contains("no Gemini API key"), "{s}");
        assert!(!s.contains("GEMINI_API_KEY"));
    }

    #[test]
    fn the_key_file_is_read_trimmed_and_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gemini_api_key");
        assert_eq!(read_key_file(&path).unwrap(), None);

        std::fs::write(&path, "  not-a-real-key\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = read_key_file(&path).unwrap_err();
        assert!(err.contains("0644"), "{err}");
        assert!(!err.contains("not-a-real-key"), "{err}");
        assert!(spoken_error(&err).contains("can't read its Gemini key"));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_key_file(&path).unwrap().as_deref(),
            Some("not-a-real-key")
        );

        std::fs::write(&path, "\n").unwrap();
        assert_eq!(read_key_file(&path).unwrap(), None);
    }
}
