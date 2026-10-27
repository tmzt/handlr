//! The daemon's LLM settings while it runs (plan 20261027_02, decisions
//! 4, 6 and 7): each use's client, the 5 s re-read of `[llm]`, and what
//! the phone is told.
//!
//! - **Clients:** one per distinct (backend, model, key) and profile,
//!   built on first use and shared by the uses on it. The profile is the
//!   general agent's (its tools, system prompt and flankr's routes) or a
//!   summary's (one system text, no tools), since a Gemini engine carries
//!   both from when it is built.
//! - **The watch:** every [`RELOAD_INTERVAL`] the config file's
//!   modification time and length are compared; on a change only `[llm]`,
//!   `[gemini]` and `[summary]` are read again. Keys are looked for on
//!   every check, so a key file appearing or going is noticed too. A
//!   change applies from the next call; a call already running keeps its
//!   client. A section that no longer parses keeps the last good settings.
//! - **The phone:** [`LlmService::subscribe`] gives the [`LlmInfo`] as it
//!   changes, for `server.capabilities`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

use libflankr_proto::companion::LlmInfo;
use tokio::sync::watch;

use super::{KeyEnv, Kind, LlmSettings, LlmSources, Target, Use, UseState, resolve_sources};
use crate::config::AppConfig;
use crate::reasoning::{
    GeminiReasoningEngine, ReasoningEngine, declare_gemini_tools, default_system_prompt,
    with_flankr_routes,
};

/// How often the config file and the keys are checked (decision 6).
pub const RELOAD_INTERVAL: Duration = Duration::from_secs(5);

/// The sections the watch re-reads; a change anywhere else needs a
/// restart.
const SECTIONS: [&str; 3] = ["llm", "gemini", "summary"];

/// What a client is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    /// The general agent: flankr's tools and system prompt, and its routes.
    Agent,
    /// A summary: the given system text, no tools, one turn.
    Summary,
}

/// What to build a client for.
#[derive(Debug, Clone, Copy)]
pub struct ClientSpec<'a> {
    pub use_: Use,
    pub profile: Profile,
    pub target: &'a Target,
    /// A summary's system text.
    pub system: Option<&'a str>,
}

/// Builds clients; [`GeminiFactory`] by default, a mock in tests.
pub trait ClientFactory: Send + Sync {
    fn build(&self, spec: &ClientSpec<'_>) -> Result<Arc<dyn ReasoningEngine>, String>;
}

/// The real clients: libinfer-gemini's.
pub struct GeminiFactory;

impl ClientFactory for GeminiFactory {
    fn build(&self, spec: &ClientSpec<'_>) -> Result<Arc<dyn ReasoningEngine>, String> {
        match spec.target.kind {
            Kind::Gemini => {
                let builder = GeminiReasoningEngine::builder()
                    .api_key(Some(spec.target.key.expose().to_string()))
                    .model(spec.target.model.clone());
                match spec.profile {
                    Profile::Agent => {
                        let mut builder = builder.tools(declare_gemini_tools());
                        if let Some(prompt) = default_system_prompt() {
                            builder = builder.system_prompt(prompt);
                        }
                        let gemini = builder.build()?;
                        Ok(with_flankr_routes(Arc::new(gemini)))
                    }
                    Profile::Summary => {
                        let builder = builder.tools(Vec::new()).max_turns(1);
                        let builder = match spec.system {
                            Some(system) => builder.system_prompt(system),
                            None => builder,
                        };
                        Ok(Arc::new(builder.build()?))
                    }
                }
            }
        }
    }
}

/// How the service is set up.
#[derive(Clone, Default)]
pub struct LlmOptions {
    /// The config file to watch; without it, nothing is re-read.
    pub config_path: Option<PathBuf>,
    /// Where keys are looked for, instead of the process's working folder
    /// and environment.
    pub key_env: Option<KeyEnv>,
    /// The clients' builder, instead of [`GeminiFactory`].
    pub factory: Option<Arc<dyn ClientFactory>>,
    /// Instead of [`RELOAD_INTERVAL`].
    pub interval: Option<Duration>,
}

/// Engines that stand in for a use's client whatever `[llm]` says, but
/// for the use's own `enabled = false` (tests: `TurnOptions::engine` and
/// `SpeechOptions::engine`).
#[derive(Clone, Default)]
pub struct Overrides {
    pub agent: Option<Arc<dyn ReasoningEngine>>,
    pub summary: Option<Arc<dyn ReasoningEngine>>,
}

impl Overrides {
    fn for_use(&self, use_: Use) -> Option<&Arc<dyn ReasoningEngine>> {
        match use_ {
            Use::GeneralAgent => self.agent.as_ref(),
            Use::CardSummaries | Use::SpokenSummaries => self.summary.as_ref(),
        }
    }
}

/// The file as last read.
struct Watched {
    sources: LlmSources,
    /// The file's other sections, to tell a change that needs a restart.
    rest: Option<toml::Table>,
    stamp: Option<(SystemTime, u64)>,
    settings: LlmSettings,
}

type ClientKey = (Target, Profile);

/// `[llm]` while the daemon runs.
pub struct LlmService {
    config_path: Option<PathBuf>,
    env: KeyEnv,
    factory: Arc<dyn ClientFactory>,
    interval: Duration,
    overrides: Overrides,
    watched: Mutex<Watched>,
    clients: Mutex<HashMap<ClientKey, Arc<dyn ReasoningEngine>>>,
    info: watch::Sender<LlmInfo>,
}

impl LlmService {
    /// The service for `config`, as loaded at start.
    pub fn new(config: &AppConfig, options: LlmOptions, overrides: Overrides) -> Arc<Self> {
        let env = options.key_env.unwrap_or_else(KeyEnv::from_process);
        let sources = LlmSources::of(config);
        let settings = resolve_sources(&sources, &env);
        log_settings(None, &settings);
        let (stamp, rest) = match &options.config_path {
            Some(path) => (
                stamp(path),
                std::fs::read_to_string(path)
                    .ok()
                    .and_then(|t| t.parse::<toml::Table>().ok())
                    .map(without_sections),
            ),
            None => (None, None),
        };
        let info = info_of(&settings, &overrides);
        Arc::new(LlmService {
            config_path: options.config_path,
            env,
            factory: options.factory.unwrap_or_else(|| Arc::new(GeminiFactory)),
            interval: options.interval.unwrap_or(RELOAD_INTERVAL),
            overrides,
            watched: Mutex::new(Watched {
                sources,
                rest,
                stamp,
                settings,
            }),
            clients: Mutex::new(HashMap::new()),
            info: watch::Sender::new(info),
        })
    }

    /// The settings in force.
    pub fn settings(&self) -> LlmSettings {
        self.watched.lock().unwrap().settings.clone()
    }

    /// Whether `use_` may run now, or why not.
    pub fn available(&self, use_: Use) -> Result<(), String> {
        let watched = self.watched.lock().unwrap();
        match (watched.settings.state(use_), self.overrides.for_use(use_)) {
            (
                UseState::Unavailable {
                    reason,
                    turned_off: true,
                },
                _,
            ) => Err(reason.clone()),
            (_, Some(_)) | (UseState::Available(_), None) => Ok(()),
            (UseState::Unavailable { reason, .. }, None) => Err(reason.clone()),
        }
    }

    /// `use_`'s client for `profile`, built on first use; `Err` is why the
    /// use is unavailable, or why its client could not be built.
    pub fn client(
        &self,
        use_: Use,
        profile: Profile,
        system: Option<&str>,
    ) -> Result<Arc<dyn ReasoningEngine>, String> {
        self.available(use_)?;
        if let Some(engine) = self.overrides.for_use(use_) {
            return Ok(Arc::clone(engine));
        }
        let target = self
            .watched
            .lock()
            .unwrap()
            .settings
            .target(use_)
            .cloned()
            .ok_or_else(|| "unavailable".to_string())?;
        let key = (target, profile);
        if let Some(engine) = self.clients.lock().unwrap().get(&key) {
            return Ok(Arc::clone(engine));
        }
        let engine = self.factory.build(&ClientSpec {
            use_,
            profile,
            target: &key.0,
            system,
        })?;
        log::info!(
            "[llm] {}: a client on backend {}, model {}",
            use_.name(),
            key.0.backend,
            key.0.model
        );
        let mut clients = self.clients.lock().unwrap();
        Ok(Arc::clone(clients.entry(key).or_insert(engine)))
    }

    /// How many clients are kept.
    pub fn client_count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }

    /// What the phone is told now.
    pub fn info(&self) -> LlmInfo {
        self.info.borrow().clone()
    }

    /// The [`LlmInfo`], as it changes.
    pub fn subscribe(&self) -> watch::Receiver<LlmInfo> {
        self.info.subscribe()
    }

    /// One check of the watch (decision 6): the file read again if it
    /// changed, the keys looked for, and the settings resolved. Returns
    /// whether the settings changed.
    pub fn check(&self) -> bool {
        let mut watched = self.watched.lock().unwrap();
        if let Some(path) = &self.config_path {
            let now = stamp(path);
            if now != watched.stamp {
                watched.stamp = now;
                self.reread(&mut watched, path);
            }
        }
        let settings = resolve_sources(&watched.sources, &self.env);
        if settings == watched.settings {
            return false;
        }
        log_settings(Some(&watched.settings), &settings);
        watched.settings = settings;
        let in_use: Vec<Target> = Use::ALL
            .iter()
            .filter_map(|&u| watched.settings.target(u).cloned())
            .collect();
        let info = info_of(&watched.settings, &self.overrides);
        drop(watched);
        // Clients no longer used are dropped; a call running on one keeps it.
        self.clients
            .lock()
            .unwrap()
            .retain(|(target, _), _| in_use.contains(target));
        self.info.send_if_modified(|old| {
            let changed = *old != info;
            *old = info;
            changed
        });
        true
    }

    /// The file read again: its sections, or the last good ones.
    fn reread(&self, watched: &mut Watched, path: &std::path::Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                log::warn!("[llm] {}: {e}; the last good settings stay", path.display());
                return;
            }
        };
        let table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(e) => {
                log::warn!(
                    "[llm] {} no longer parses ({e}); the last good settings stay",
                    path.display()
                );
                return;
            }
        };
        match sources_of(&table) {
            Ok(sources) => {
                if sources != watched.sources {
                    log::info!("[llm] {} changed; reading [llm] again", path.display());
                }
                watched.sources = sources;
            }
            Err(e) => {
                log::warn!("[llm] {}: {e}; the last good settings stay", path.display());
            }
        }
        let rest = without_sections(table);
        if watched.rest.as_ref().is_some_and(|old| *old != rest) {
            log::warn!(
                "{} changed outside [llm]: those settings take effect when the daemon restarts",
                path.display()
            );
        }
        watched.rest = Some(rest);
    }

    /// Checks every interval while the service is in use.
    pub fn spawn_watch(self: &Arc<Self>) {
        if self.config_path.is_none() {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        let interval = self.interval;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(service) = weak.upgrade() else {
                    return;
                };
                service.check();
            }
        });
    }
}

/// The sections `[llm]` is resolved from, out of a parsed file.
fn sources_of(table: &toml::Table) -> Result<LlmSources, String> {
    fn section<T: serde::de::DeserializeOwned>(
        table: &toml::Table,
        name: &str,
    ) -> Result<Option<T>, String> {
        table
            .get(name)
            .map(|v| {
                v.clone()
                    .try_into::<T>()
                    .map_err(|e| format!("[{name}]: {e}"))
            })
            .transpose()
    }
    Ok(LlmSources {
        llm: section(table, "llm")?,
        gemini: section(table, "gemini")?,
        summary: section(table, "summary")?,
    })
}

fn without_sections(mut table: toml::Table) -> toml::Table {
    for name in SECTIONS {
        table.remove(name);
    }
    table
}

fn stamp(path: &std::path::Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// What the phone is told for `settings`, the overrides counted as
/// available.
fn info_of(settings: &LlmSettings, overrides: &Overrides) -> LlmInfo {
    let state = |u: Use| match (settings.state(u), overrides.for_use(u)) {
        (UseState::Unavailable { reason, turned_off }, o) if *turned_off || o.is_none() => {
            Err(reason.clone())
        }
        _ => Ok(()),
    };
    let general = state(Use::GeneralAgent);
    let cards = state(Use::CardSummaries);
    let spoken = state(Use::SpokenSummaries);
    let summaries = cards.is_ok() || spoken.is_ok();
    let reason = match (&general, summaries) {
        (Err(e), _) => Some(e.clone()),
        (Ok(()), false) => cards.err(),
        _ => None,
    };
    LlmInfo {
        general_agent: general.is_ok(),
        summaries,
        reason,
    }
}

/// The log for new settings: each use's backend and model or reason, and
/// the notes and problems not logged before (each once).
fn log_settings(old: Option<&LlmSettings>, new: &LlmSettings) {
    let seen = |list: fn(&LlmSettings) -> &Vec<String>, line: &String| {
        old.is_some_and(|o| list(o).contains(line))
    };
    for note in &new.notes {
        if !seen(|s| &s.notes, note) {
            log::info!("{note}");
        }
    }
    for problem in &new.problems {
        if !seen(|s| &s.problems, problem) {
            log::warn!("[llm] {problem}");
        }
    }
    let old_lines = old.map(LlmSettings::describe).unwrap_or_default();
    for line in new.describe() {
        if !old_lines.contains(&line) {
            log::info!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::MockReasoningEngine;

    /// Records what it was asked to build, and builds a mock answering
    /// with the backend and model.
    #[derive(Default)]
    pub struct Recording {
        pub built: Mutex<Vec<(Use, Profile, String, String)>>,
    }

    impl ClientFactory for Recording {
        fn build(&self, spec: &ClientSpec<'_>) -> Result<Arc<dyn ReasoningEngine>, String> {
            self.built.lock().unwrap().push((
                spec.use_,
                spec.profile,
                spec.target.backend.clone(),
                spec.target.model.clone(),
            ));
            Ok(Arc::new(MockReasoningEngine::with_response(format!(
                "{}/{}",
                spec.target.backend, spec.target.model
            ))))
        }
    }

    fn write_key(path: &std::path::Path, key: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, key).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    struct Setup {
        dir: tempfile::TempDir,
        path: PathBuf,
        factory: Arc<Recording>,
        service: Arc<LlmService>,
    }

    fn setup(text: &str, key: bool) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        if key {
            write_key(&dir.path().join(super::super::gemini::KEY_FILE), "k");
        }
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let config: AppConfig = toml::from_str(text).unwrap();
        let factory = Arc::new(Recording::default());
        let service = LlmService::new(
            &config,
            LlmOptions {
                config_path: Some(path.clone()),
                key_env: Some(KeyEnv::in_dir(dir.path())),
                factory: Some(factory.clone()),
                interval: None,
            },
            Overrides::default(),
        );
        Setup {
            dir,
            path,
            factory,
            service,
        }
    }

    /// Rewrites the file so its stamp changes.
    fn rewrite(s: &Setup, text: &str) {
        std::fs::write(&s.path, text).unwrap();
        let later = SystemTime::now() + Duration::from_secs(2);
        let f = std::fs::File::options().write(true).open(&s.path).unwrap();
        f.set_modified(later).unwrap();
    }

    const TWO_BACKENDS: &str = r#"
[llm]
enabled = true
default = "main"
[llm.backends.main]
kind = "gemini"
[llm.backends.cheap]
kind = "gemini"
model = "gemini-3.8-flash-lite"
[llm.uses.card_summaries]
backend = "cheap"
[llm.uses.spoken_summaries]
backend = "cheap"
"#;

    #[test]
    fn each_use_builds_on_its_backend_and_model_and_pairs_share() {
        let s = setup(TWO_BACKENDS, true);
        s.service
            .client(Use::GeneralAgent, Profile::Agent, None)
            .unwrap();
        let a = s
            .service
            .client(Use::CardSummaries, Profile::Summary, Some("sys"))
            .unwrap();
        let b = s
            .service
            .client(Use::SpokenSummaries, Profile::Summary, Some("sys"))
            .unwrap();
        assert!(Arc::ptr_eq(&a, &b), "one client for the shared pair");
        let built = s.factory.built.lock().unwrap().clone();
        assert_eq!(
            built,
            [
                (
                    Use::GeneralAgent,
                    Profile::Agent,
                    "main".into(),
                    "gemini-3.8-flash".into()
                ),
                (
                    Use::CardSummaries,
                    Profile::Summary,
                    "cheap".into(),
                    "gemini-3.8-flash-lite".into()
                ),
            ]
        );
        assert_eq!(s.service.client_count(), 2);
        assert_eq!(s.service.info(), LlmInfo::available());
    }

    #[test]
    fn off_is_refused_with_the_reason() {
        let s = setup("", true);
        let err = s
            .service
            .client(Use::GeneralAgent, Profile::Agent, None)
            .err()
            .unwrap();
        assert!(err.contains("[llm] enabled"), "{err}");
        assert!(s.factory.built.lock().unwrap().is_empty());
        let info = s.service.info();
        assert!(!info.general_agent && !info.summaries);
        assert!(info.reason.unwrap().contains("[llm] enabled"));
    }

    #[test]
    fn a_reload_turns_a_use_off_for_the_next_call_and_drops_its_client() {
        let s = setup("[llm]\nenabled = true\n", true);
        let mut rx = s.service.subscribe();
        let running = s
            .service
            .client(Use::GeneralAgent, Profile::Agent, None)
            .unwrap();
        assert!(!s.service.check(), "nothing changed");
        rewrite(
            &s,
            "[llm]\nenabled = true\n[llm.uses.general_agent]\nenabled = false\n",
        );
        assert!(s.service.check());
        assert!(rx.has_changed().unwrap());
        let info = rx.borrow_and_update().clone();
        assert!(!info.general_agent && info.summaries);
        assert!(info.reason.unwrap().contains("general_agent"));
        assert!(
            s.service
                .client(Use::GeneralAgent, Profile::Agent, None)
                .is_err()
        );
        assert_eq!(s.service.client_count(), 0, "its client is dropped");
        // What was running keeps its client.
        assert_eq!(Arc::strong_count(&running), 1);
    }

    #[test]
    fn a_broken_section_keeps_the_last_good_settings() {
        let s = setup("[llm]\nenabled = true\n", true);
        rewrite(&s, "[llm]\nenabled = \"yes please\"\n");
        assert!(!s.service.check(), "kept");
        assert!(s.service.available(Use::GeneralAgent).is_ok());
        rewrite(&s, "[llm\nenabled = false\n");
        assert!(!s.service.check(), "a file that does not parse");
        assert!(s.service.available(Use::GeneralAgent).is_ok());
        // Fixed: read.
        rewrite(&s, "[llm]\nenabled = false\n");
        assert!(s.service.check());
        assert!(s.service.available(Use::GeneralAgent).is_err());
        // A change elsewhere is only logged.
        rewrite(&s, "[llm]\nenabled = false\n[server]\nsocket = \"/x\"\n");
        assert!(!s.service.check());
    }

    #[test]
    fn a_key_file_appearing_makes_the_uses_available() {
        let s = setup("[llm]\nenabled = true\n", false);
        let mut rx = s.service.subscribe();
        assert!(!s.service.info().general_agent);
        assert!(s.service.info().reason.unwrap().contains("no Gemini key"));
        write_key(&s.dir.path().join(super::super::gemini::KEY_FILE), "k");
        assert!(s.service.check(), "noticed without a config change");
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), LlmInfo::available());
        assert!(
            s.service
                .client(Use::SpokenSummaries, Profile::Summary, Some("s"))
                .is_ok()
        );
        // And going.
        std::fs::remove_file(s.dir.path().join(super::super::gemini::KEY_FILE)).unwrap();
        assert!(s.service.check());
        assert!(!s.service.info().summaries);
        assert_eq!(s.service.client_count(), 0);
    }

    #[test]
    fn overrides_stand_in_but_a_use_turned_off_stays_off() {
        let config: AppConfig = toml::from_str("[summary]\ncards = false\n").unwrap();
        let mock: Arc<dyn ReasoningEngine> = Arc::new(MockReasoningEngine::with_response("x"));
        let service = LlmService::new(
            &config,
            LlmOptions {
                key_env: Some(KeyEnv::in_dir("/nonexistent")),
                ..LlmOptions::default()
            },
            Overrides {
                agent: Some(Arc::clone(&mock)),
                summary: Some(Arc::clone(&mock)),
            },
        );
        assert!(
            service
                .client(Use::GeneralAgent, Profile::Agent, None)
                .is_ok()
        );
        assert!(service.available(Use::SpokenSummaries).is_ok());
        assert!(service.available(Use::CardSummaries).is_err());
        assert_eq!(service.info(), LlmInfo::available());
    }
}
