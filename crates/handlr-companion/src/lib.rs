//! The companion client (docs/design/20261022_04_COMPANION_APP_PLAN.md):
//! the client side of `handlr.companion` apart from audio and UI.
//!
//! [`Companion`] is a worker in libvoicexn's style. A `net` thread keeps a
//! connection to the daemon and runs the logic of [`client::State`]; the app
//! [`send`](Companion::send)s it [`Input`]s and [`poll`](Companion::poll)s
//! its [`Event`]s.
//!
//! Steps 1 and 2 of the plan's 18:10 EDT amendment: the logic,
//! [`FileKeyStore`], and the connection. With a paired server in the
//! [`KeyStore`], the client reconnects to it over handlr link (`IK`, with
//! `hello` and its `session_id` in the handshake); without one, it uses the
//! daemon's Unix socket. [`Input::Pair`] pairs from the pairing URL, the
//! bare `handlr1…` string or the QR code's `data:` URL: the client stores
//! the server, shows its device code as [`Event::PairingCode`] for the user
//! to paste into `handlr pair`, and connects once the server knows its key
//! (plan 20261022_02, amendment of 23:07 EDT). `examples/text.rs` drives it
//! from typed lines.

pub mod client;
pub mod keys;
mod net;
pub mod scan;
pub mod view;
pub mod wire;

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub use client::{
    CACHED_VIEWS, DICTATION_SILENCE, Dictation, DictationPhase, Event, Input, Pressure, Settings,
    Speaker, Suggestion,
};
pub use keys::{FileKeyStore, KeyStore, ServerRecord};
pub use net::{Backoff, PAIRING_RETRY, PAIRING_WINDOW};
pub use view::{AgentCard, Bookend, CardKind, Shows, ViewSection};

/// How the client runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionConfig {
    /// The name the client gives in `hello`.
    pub client: String,
    /// The name the device asks to be paired as, in its device code: 1 to
    /// 32 ASCII letters, digits, `-`, `_` or `.`. The user may change it.
    pub device_name: String,
    /// The TTS voice, for the front end.
    pub voice: Option<String>,
    /// The daemon's socket, used while no server is paired.
    pub socket: PathBuf,
    /// The first delay before reconnecting, doubled after each failure.
    pub backoff_initial: Duration,
    /// The longest delay before reconnecting.
    pub backoff_max: Duration,
    /// The silence after which a dictated message is submitted (plan
    /// 20261023_02, the 01:57 EDT amendment). Shorter than
    /// [`DICTATION_SILENCE`] is raised to it.
    pub dictation_silence: Duration,
}

impl CompanionConfig {
    /// The defaults: the daemon's default socket, backoff of 1, 2, 4 …
    /// 30 s, and [`default_device_name`].
    pub fn new(client: impl Into<String>) -> Self {
        CompanionConfig {
            client: client.into(),
            device_name: default_device_name(),
            voice: None,
            socket: default_socket_path(),
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            dictation_silence: DICTATION_SILENCE,
        }
    }
}

/// The machine's hostname as a device name (see [`device_name_from`]), or
/// `companion`.
pub fn default_device_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most `buf.len()` bytes into `buf`,
    // which is valid and owned.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let host = if ok {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    } else {
        String::new()
    };
    device_name_from(&host).unwrap_or_else(|| "companion".to_string())
}

/// A device name from a hostname: its first label, lowercased, with any
/// character but ASCII letters, digits, `-` and `_` dropped, cut to 32
/// bytes. `None` if nothing is left.
pub fn device_name_from(host: &str) -> Option<String> {
    let label = host.trim().split('.').next().unwrap_or("");
    let name: String = label
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(libhandlr_proto::link::DEVICE_NAME_MAX)
        .collect();
    (!name.is_empty()).then_some(name)
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

#[cfg(test)]
mod tests {
    use super::*;
    use libhandlr_proto::link::DeviceCode;

    #[test]
    fn device_names_come_from_the_hostname() {
        assert_eq!(device_name_from("M4.local").as_deref(), Some("m4"));
        assert_eq!(
            device_name_from("Tim's MacBook Pro.local").as_deref(),
            Some("timsmacbookpro")
        );
        assert_eq!(device_name_from(&"x".repeat(40)).unwrap().len(), 32);
        assert_eq!(device_name_from(""), None);
        assert_eq!(device_name_from("..."), None);
        assert!(DeviceCode::valid_name(&default_device_name()));
    }
}
