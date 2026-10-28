//! The companion client (docs/design/20261022_04_COMPANION_APP_PLAN.md):
//! the client side of `flankr.companion` apart from audio and UI.
//!
//! [`Companion`] is a worker in libvoicexn's style. A `net` thread keeps a
//! connection to the daemon and runs the logic of [`client::State`]; the app
//! [`send`](Companion::send)s it [`Input`]s and [`poll`](Companion::poll)s
//! its [`Event`]s.
//!
//! Steps 1 and 2 of the plan's 18:10 EDT amendment: the logic,
//! [`FileKeyStore`], and the connection. With a paired server in the
//! [`KeyStore`], the client reconnects to it over flankr link (`IK`, with
//! `hello` and its `session_id` in the handshake); without one, it uses the
//! daemon's Unix socket. [`Input::Pair`] pairs from the pairing URL, the
//! bare `flankr1…` string or the QR code's `data:` URL: the client shows
//! its device code as [`Event::PairingCode`] for the user to paste into
//! `flankr pair`, and adds the remote once the server knows its key (plan
//! 20261022_02, amendment of 23:07 EDT). The phone keeps several paired
//! remotes ([`Remotes`]), connects to the active one, and removes one with
//! [`Input::RemoveRemote`] (plan 20261028_01). `examples/text.rs` drives it
//! from typed lines.

pub mod client;
pub mod cross;
pub mod keys;
mod net;
pub mod remote_state;
pub mod scan;
pub mod terminal;
pub mod view;
pub mod wire;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

pub use client::{
    CACHED_VIEWS, DICTATION_SILENCE, Dictation, DictationPhase, Event, Input, Pressure, PromptMode,
    Settings, Speaker, Suggestion,
};
pub use cross::{SessionCard, SessionCardKind, is_terminal_only};
pub use keys::{
    FileKeyStore, KeyStore, LEGACY_KEY_NAME, MAX_REMOTES, RemoteId, RemoteInfo, Remotes,
    ServerRecord,
};
pub use net::{Backoff, PAIRING_RETRY, PAIRING_WINDOW, REVOKE_TIMEOUT};
pub use remote_state::RemoteState;
pub use terminal::{TerminalStatus, Terminals, key_batches};
// What the app draws and reads of a terminal.
pub use libflankr_proto::common::{
    MouseButton, TerminalClosedReason, TerminalMouseKind, TerminalSource,
};
pub use libflankr_term::{Cursor, Grid, Screen};
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
    /// The app's data directory, where each remote's state lives in
    /// `remotes/<id>/state.redb` ([`RemoteState`]; plan 20261028_01,
    /// decision 14). `None`: no state is kept.
    pub data_dir: Option<PathBuf>,
    /// How long a pairing waits for its code to be added
    /// ([`PAIRING_WINDOW`]; shorter in tests).
    pub pairing_window: Duration,
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
            data_dir: None,
            pairing_window: PAIRING_WINDOW,
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
        .take(libflankr_proto::link::DEVICE_NAME_MAX)
        .collect();
    (!name.is_empty()).then_some(name)
}

/// `~/.config/flankr/flankr.sock`, where `flankr serve` listens by default.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("flankr")
        .join("flankr.sock")
}

/// The client: the `net` thread and its two channels. Dropping it stops
/// the thread.
pub struct Companion {
    inputs: tokio::sync::mpsc::UnboundedSender<Input>,
    events: mpsc::Receiver<Event>,
    /// The `net` thread's count of bytes over the link.
    bytes: Arc<AtomicU64>,
    /// The terminals the `net` thread keeps (plan 20261027_07, step 4).
    terminals: Arc<Terminals>,
}

impl Companion {
    /// Starts the `net` thread, which connects at once and keeps
    /// reconnecting.
    pub fn spawn(config: CompanionConfig, keys: Box<dyn KeyStore>) -> Self {
        Self::spawn_tapped(config, keys, |_, _| {})
    }

    /// As [`spawn`](Self::spawn), with `tap` called on the `net` thread with
    /// each event, and the bytes over the link so far
    /// ([`link_bytes`](Self::link_bytes)), just before the event is queued
    /// for [`poll`](Self::poll). It sees the events while the app's own loop
    /// does not run, such as an Android app in the background with a
    /// foreground service (plan 20261024_02, the notification's content and
    /// alerts). It must be quick: the link waits on it.
    pub fn spawn_tapped(
        config: CompanionConfig,
        keys: Box<dyn KeyStore>,
        tap: impl Fn(&Event, u64) + Send + 'static,
    ) -> Self {
        let (inputs, inputs_rx) = tokio::sync::mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::channel();
        let bytes = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&bytes);
        let terminals = Arc::new(Terminals::default());
        let kept = Arc::clone(&terminals);
        thread::Builder::new()
            .name("net".into())
            .spawn(move || {
                net::run(
                    config,
                    keys,
                    inputs_rx,
                    events_tx,
                    counted,
                    kept,
                    Box::new(tap),
                )
            })
            .expect("spawning the net thread");
        Companion {
            inputs,
            events,
            bytes,
            terminals,
        }
    }

    /// The terminals (plan 20261027_07, step 4): read from any thread.
    /// Whether the daemon offers them ([`Terminals::supported`]), each
    /// session's [`Screen`] and [`TerminalStatus`], and a change counter
    /// ([`Terminals::changes`]) to redraw on. The `net` thread writes them;
    /// each read takes a short lock.
    pub fn terminals(&self) -> &Terminals {
        &self.terminals
    }

    /// Opens the session's terminal ([`Input::TerminalOpen`]): frames
    /// follow, each acknowledged as it is shown.
    pub fn terminal_open(&self, session: &str, cols_hint: Option<u16>, rows_hint: Option<u16>) {
        self.send(Input::TerminalOpen {
            session: session.to_string(),
            cols_hint,
            rows_hint,
        });
    }

    /// Closes the session's terminal ([`Input::TerminalClose`]).
    pub fn terminal_close(&self, session: &str) {
        self.send(Input::TerminalClose(session.to_string()));
    }

    /// Arms keyboard input on the session's terminal
    /// ([`Input::TerminalArm`]); [`TerminalStatus::armed_for`] is set once
    /// the daemon answers, and stays until the terminal is closed.
    pub fn terminal_arm_input(&self, session: &str) {
        self.send(Input::TerminalArm(session.to_string()));
    }

    /// Sends keys into the session's armed terminal
    /// ([`Input::TerminalKeys`]).
    pub fn terminal_send_keys(&self, session: &str, keys: &str) {
        self.send(Input::TerminalKeys {
            session: session.to_string(),
            keys: keys.to_string(),
        });
    }

    /// Scrolls the session's armed terminal by `lines`, as a mouse wheel
    /// does: positive back into the scrollback ([`Input::TerminalScroll`]).
    /// Scrolls asked for while one is in flight are summed;
    /// [`TerminalStatus::scroll_offset`] says where the pane is once the
    /// daemon answers.
    pub fn terminal_scroll(&self, session: &str, lines: i32) {
        if lines != 0 {
            self.send(Input::TerminalScroll {
                session: session.to_string(),
                lines,
            });
        }
    }

    /// The columns and rows that fit the phone's terminal area now, keyboard
    /// included ([`Input::TerminalResize`]; plan 20261027_08, the 23:24 EDT
    /// amendment). Call it whenever the fit changes; the same size twice
    /// sends nothing, one `terminal.resize` is in flight at a time and the
    /// newest fit wins. A reopen after a reconnect asks for this fit.
    pub fn terminal_resize(&self, session: &str, cols: u16, rows: u16) {
        self.send(Input::TerminalResize {
            session: session.to_string(),
            cols,
            rows,
        });
    }

    /// A mouse event at pane-local cell `col`, `row` of the session's armed
    /// terminal ([`Input::TerminalMouse`]), for a TUI that reports the mouse
    /// ([`Cursor::mouse_reporting`]) on a daemon that serves it
    /// ([`Terminals::mouse_allowed`]). Refused here unless armed.
    pub fn terminal_mouse(
        &self,
        session: &str,
        kind: TerminalMouseKind,
        col: u16,
        row: u16,
        modifiers: u8,
    ) {
        self.send(Input::TerminalMouse {
            session: session.to_string(),
            kind,
            col,
            row,
            modifiers,
        });
    }

    /// The bytes sent to and received from the daemon since the client
    /// started, over every connection: each socket line with its newline,
    /// each link message's plaintext (CHANGES.md, 2026-10-24 12:59 EDT,
    /// item 3, "n MB this session").
    pub fn link_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
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
    use libflankr_proto::link::DeviceCode;

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
