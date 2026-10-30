//! The `gemini` backend kind's defaults and models (plan 20261027_02,
//! the 2026-10-30 13:54 EDT amendment): each use's model asked for by
//! tier and age (Flash, newest, for the general agent; Flash-Lite, oldest,
//! for the summaries, which run often; never Pro unless a config names
//! it), and resolved once against the key's model list, fetched at start.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

pub use libinfer_gemini::{Age, How, ModelCatalog, ModelChoice, Resolved, Tier};

use super::{ApiKey, Use};

/// The discovered backend's key file, against the daemon's working folder
/// (`.private` is gitignored).
pub const KEY_FILE: &str = ".private/gemini_api_key";
/// The environment variable read when the key file is not there.
pub const KEY_ENV: &str = "GEMINI_API_KEY";
/// How long the model list may take at start (item 3), all pages.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The default choice for `use_` (item 1): `flash:newest` for the general
/// agent and planning, `flash-lite:oldest` for the summaries.
pub fn default_choice(use_: Use) -> ModelChoice {
    match use_ {
        Use::GeneralAgent | Use::Planning => ModelChoice::Rule {
            tier: Tier::Flash,
            age: Age::Newest,
        },
        Use::CardSummaries | Use::SpokenSummaries => ModelChoice::Rule {
            tier: Tier::FlashLite,
            age: Age::Oldest,
        },
    }
}

/// How a choice was resolved, as the start log says it (item 4).
pub fn how_name(how: How) -> &'static str {
    match how {
        How::Alias => "alias",
        How::Matched => "matched",
        How::Named => "named",
        How::NamedMissing => "named, not in the key's model list",
        How::Fallback => "fallback",
        _ => "resolved",
    }
}

/// The `gemini` backends' model lists, fetched once at start (item 3), and
/// each (backend, choice) resolved against them once (item 5): a reload
/// resolves a new choice against the same list, and a backend without one
/// (its fetch failed, or it got its key after start) resolves offline.
#[derive(Debug, Default)]
pub struct Catalogs {
    /// By backend name; an empty catalog counts as no list.
    by_backend: BTreeMap<String, ModelCatalog>,
    memo: Mutex<HashMap<(String, String), Resolved>>,
}

impl Catalogs {
    /// No lists: every choice resolved offline (`resolve_offline`).
    pub fn none() -> Self {
        Catalogs::default()
    }

    /// These backends' lists (tests: [`ModelCatalog::from_names`]).
    pub fn of<I, S>(catalogs: I) -> Self
    where
        I: IntoIterator<Item = (S, ModelCatalog)>,
        S: Into<String>,
    {
        Catalogs {
            by_backend: catalogs
                .into_iter()
                .map(|(name, catalog)| (name.into(), catalog))
                .collect(),
            memo: Mutex::default(),
        }
    }

    /// The backends with a list.
    pub fn backends(&self) -> Vec<String> {
        self.by_backend.keys().cloned().collect()
    }

    /// `backend`'s list, if it has one that is not empty: what its clients
    /// are built with, so a refused model is replaced inside the client
    /// (the 2026-10-30 15:39 EDT amendment, item 1).
    pub fn catalog(&self, backend: &str) -> Option<&ModelCatalog> {
        self.by_backend
            .get(backend)
            .filter(|catalog| !catalog.names().is_empty())
    }

    /// `choice` for `backend`, resolved once and then remembered; the
    /// first resolution is logged by libinfer-gemini.
    pub fn resolve(&self, backend: &str, choice: &ModelChoice) -> Resolved {
        let key = (backend.to_string(), choice.to_string());
        if let Some(resolved) = self.memo.lock().unwrap().get(&key) {
            return resolved.clone();
        }
        let resolved = match self.by_backend.get(backend) {
            Some(catalog) => catalog.resolve(choice),
            None => libinfer_gemini::resolve_offline(choice),
        };
        self.memo
            .lock()
            .unwrap()
            .entry(key)
            .or_insert(resolved)
            .clone()
    }

    /// Fetches the list for each backend in `keys` (backend name, key),
    /// once per distinct key, each within [`FETCH_TIMEOUT`]. A failure is
    /// one warning naming the backends and the reason, never the key, and
    /// leaves those backends without a list.
    pub async fn fetch(keys: Vec<(String, ApiKey)>) -> Self {
        let mut by_key: Vec<(ApiKey, Vec<String>)> = Vec::new();
        for (backend, key) in keys {
            match by_key.iter_mut().find(|(k, _)| *k == key) {
                Some((_, names)) => names.push(backend),
                None => by_key.push((key, vec![backend])),
            }
        }
        let mut by_backend = BTreeMap::new();
        for (key, names) in by_key {
            let fetched = match tokio::time::timeout(
                FETCH_TIMEOUT,
                ModelCatalog::fetch(key.expose()),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(format!("no answer within {} s", FETCH_TIMEOUT.as_secs())),
            };
            match fetched {
                Ok(catalog) => {
                    log::info!(
                        "[llm] {}: the key's model list has {} model(s)",
                        names.join(", "),
                        catalog.names().len()
                    );
                    for name in names {
                        by_backend.insert(name, catalog.clone());
                    }
                }
                Err(e) => log::warn!(
                    "[llm] {}: the key's model list could not be fetched ({e}); models are \
                     chosen without it (a tier rule gives {})",
                    names.join(", "),
                    libinfer_gemini::DEFAULT_MODEL
                ),
            }
        }
        Catalogs {
            by_backend,
            memo: Mutex::default(),
        }
    }
}
