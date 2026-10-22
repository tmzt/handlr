//! The companion client (docs/design/20261022_04_COMPANION_APP_PLAN.md):
//! the client side of `handlr.companion` apart from audio and UI.
//!
//! [`Companion`] is a worker in libvoicexn's style. A `net` thread keeps a
//! connection to the daemon and runs the logic of [`client::State`]; the app
//! [`send`](Companion::send)s it [`Input`]s and [`poll`](Companion::poll)s
//! its [`Event`]s.
//!
//! This is the plan's step 1: the logic, [`FileKeyStore`] and the local
//! connection to the daemon's Unix socket, with reconnecting and the
//! `session_id`. Pairing and the link are step 2.

pub mod client;
pub mod keys;
mod net;
pub mod wire;

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub use client::{Event, Input};
pub use keys::{FileKeyStore, KeyStore, ServerRecord};
pub use net::Backoff;

/// How the client runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionConfig {
    /// The name the client gives in `hello`.
    pub client: String,
    /// The TTS voice, for the front end.
    pub voice: Option<String>,
    /// The daemon's socket.
    pub socket: PathBuf,
    /// The first delay before reconnecting, doubled after each failure.
    pub backoff_initial: Duration,
    /// The longest delay before reconnecting.
    pub backoff_max: Duration,
}

impl CompanionConfig {
    /// The defaults: the daemon's default socket, and backoff of 1, 2, 4 …
    /// 30 s.
    pub fn new(client: impl Into<String>) -> Self {
        CompanionConfig {
            client: client.into(),
            voice: None,
            socket: default_socket_path(),
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
        }
    }
}

/// `~/.config/handlr/handlr.sock`, where `handlr serve` listens by default.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("handlr")
        .join("handlr.sock")
}

/// The client: the `net` thread and its two channels. Dropping it stops
/// the thread.
pub struct Companion {
    inputs: tokio::sync::mpsc::UnboundedSender<Input>,
    events: mpsc::Receiver<Event>,
}

impl Companion {
    /// Starts the `net` thread, which connects at once and keeps
    /// reconnecting.
    pub fn spawn(config: CompanionConfig, keys: Box<dyn KeyStore>) -> Self {
        let (inputs, inputs_rx) = tokio::sync::mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("net".into())
            .spawn(move || net::run(config, keys, inputs_rx, events_tx))
            .expect("spawning the net thread");
        Companion { inputs, events }
    }

    pub fn send(&self, input: Input) {
        // The thread only exits once this sender is dropped.
        let _ = self.inputs.send(input);
    }

    /// The events so far, without waiting.
    pub fn poll(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }

    /// Waits up to `timeout` for the next event.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}
