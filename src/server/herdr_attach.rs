//! The daemon's one attachment to the local herdr as one of herdr's own
//! shell clients (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md,
//! steps 7 and 8; decisions 4, 5, 11 and 13 as the amendments of 23:24,
//! 23:32 and 23:42 EDT have them).
//!
//! * **One attachment, kept open.** While herdr is local and `[terminal]
//!   attach` is on, [`HerdrAttach::run`] holds one `libherdr_term`
//!   [`Attachment`] for the daemon's whole run and reconnects with backoff
//!   (1 s doubling to 30 s) when it closes. It carries only the terminal:
//!   the surface, the cursor and the size, keys, mouse and wheel, and the
//!   `pane.focus` request that points its own view at the session's pane.
//!   Everything else stays on herdr's JSON API.
//! * **Active only while a terminal is open.** Every terminal that may use
//!   the attachment is a *viewer*. With none, the surface is inactive
//!   (`client_shell.surface.set { active: false }`), so it claims no
//!   geometry. A herdr that does not serve that method (it ignores the
//!   hello's `surface_active` too) is detached instead, and attached again
//!   when a terminal opens.
//! * **The target.** The viewer that opened or touched its terminal last
//!   sets the attachment's pane and size: on open, `pane.focus` and one
//!   `ClientShellResize`; later resizes at most twice a second, coalesced.
//! * **A change made on the desktop is not forced back.** Once herdr has
//!   answered flankr's own requests and drawn a surface matching them
//!   (*settled*), a surface that shows another focus or size than flankr
//!   asked for ([`SurfaceView::desktop_change`]) makes the attachment
//!   *stale*: no `pane.focus` or resize of flankr's own until the next touch
//!   on a phone's terminal ([`HerdrAttach::touch`]), which sends both again
//!   before its input. Frames carry `Cursor.stale` meanwhile.
//! * **Fan-out.** Each surface is published, with the stale flag, on a
//!   `watch` channel; each terminal's task cuts its own pane from it.
//! * `pane.zoom` is never sent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use libherdr_term::herdr::ClientPaneInputEvent;
use libherdr_term::{
    AttachError, AttachOptions, Attachment, ENDPOINT_PROTOCOL_GENERATION, Event,
    HERDR_PROTOCOL_VERSION, SurfaceView,
};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;

/// The first reconnect delay, doubled after each failure up to
/// [`BACKOFF_MAX`] (the 23:32 EDT amendment, item 1).
pub const BACKOFF_INITIAL: Duration = Duration::from_secs(1);
/// The longest reconnect delay.
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// The least time between two `ClientShellResize`s: twice a second (the
/// 23:24 EDT amendment, item 1).
pub const RESIZE_GAP: Duration = Duration::from_millis(500);
/// The surface asked for before any phone has said its size.
pub const DEFAULT_SIZE: (u16, u16) = (80, 24);
/// How long a touch waits for herdr to answer the `pane.focus` it sent
/// before the input goes, so the input reaches the pane it focused.
const FOCUS_WAIT: Duration = Duration::from_secs(1);

/// Where and how the daemon attaches: herdr's client socket and the
/// reconnect backoff. Tests point it at a fake herdr with short delays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachSettings {
    /// herdr's client socket, `herdr-client.sock` beside the API socket.
    pub socket: PathBuf,
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
    /// How long herdr's welcome and first snapshot may take.
    pub handshake_timeout: Duration,
}

impl AttachSettings {
    /// `socket` with the plan's backoff and herdr's 4 s handshake.
    pub fn new(socket: impl Into<PathBuf>) -> AttachSettings {
        AttachSettings {
            socket: socket.into(),
            backoff_initial: BACKOFF_INITIAL,
            backoff_max: BACKOFF_MAX,
            handshake_timeout: Duration::from_secs(4),
        }
    }
}

/// A terminal using the attachment: its connection's id and its own.
pub type ViewerId = (u64, u32);

/// The latest surface, as the terminals read it.
#[derive(Debug, Clone, Default)]
pub struct Published {
    /// Counts connections; 0 before the first.
    pub generation: u64,
    /// The tab as herdr last drew it; `None` while not attached.
    pub view: Option<Arc<SurfaceView>>,
    /// The desktop has the tab (the 23:32 EDT amendment, item 3).
    pub stale: bool,
}

impl Published {
    /// Whether the surface shows `pane_id`.
    pub fn shows(&self, pane_id: &str) -> bool {
        self.view
            .as_ref()
            .is_some_and(|v| v.pane(pane_id).is_some())
    }
}

#[derive(Debug, Clone)]
struct Viewer {
    pane_id: String,
    cols: u16,
    rows: u16,
    /// When it last opened or touched, by [`Shared::clock`].
    touched: u64,
}

#[derive(Default)]
struct Shared {
    attachment: Option<Attachment>,
    generation: u64,
    /// Whether herdr serves `client_shell.surface.set`, once known.
    surface_set: Option<bool>,
    viewers: HashMap<ViewerId, Viewer>,
    clock: u64,
    target: Option<ViewerId>,
    stale: bool,
    /// herdr has drawn a surface matching flankr's last requests.
    settled: bool,
    last_resize: Option<Instant>,
    /// A coalesced resize is waiting for [`RESIZE_GAP`].
    resize_due: bool,
}

/// The daemon's attachment to herdr. See the module docs.
pub struct HerdrAttach {
    settings: AttachSettings,
    shared: Mutex<Shared>,
    published: watch::Sender<Published>,
    wake: Notify,
    started: AtomicBool,
    incompatible_logged: AtomicBool,
}

impl HerdrAttach {
    pub fn new(settings: AttachSettings) -> Arc<HerdrAttach> {
        Arc::new(HerdrAttach {
            settings,
            shared: Mutex::new(Shared::default()),
            published: watch::channel(Published::default()).0,
            wake: Notify::new(),
            started: AtomicBool::new(false),
            incompatible_logged: AtomicBool::new(false),
        })
    }

    pub fn settings(&self) -> &AttachSettings {
        &self.settings
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start [`HerdrAttach::run`] once.
    pub fn spawn(self: &Arc<Self>) {
        if !self.started.swap(true, Ordering::SeqCst) {
            tokio::spawn(Arc::clone(self).run());
        }
    }

    /// Whether the attachment is up: terminals that may use it read from
    /// it, and `terminal.mouse` is served.
    pub fn usable(&self) -> bool {
        self.lock().attachment.is_some()
    }

    /// Whether the desktop has the tab.
    pub fn stale(&self) -> bool {
        self.lock().stale
    }

    /// Whether herdr has drawn a surface after flankr's last requests.
    pub fn settled(&self) -> bool {
        let s = self.lock();
        s.settled && !s.stale
    }

    /// The surfaces as they come.
    pub fn subscribe(&self) -> watch::Receiver<Published> {
        self.published.subscribe()
    }

    /// The latest surface.
    pub fn current(&self) -> Published {
        self.published.borrow().clone()
    }

    /// The viewers' count (tests and logs).
    pub fn viewers(&self) -> usize {
        self.lock().viewers.len()
    }

    /// A terminal opens on `pane_id` at `cols` by `rows`, or opens again:
    /// it becomes the target. The surface goes active with the first
    /// viewer; then `pane.focus` and one resize (the 23:32 EDT amendment,
    /// item 2).
    pub fn open(self: &Arc<Self>, id: ViewerId, pane_id: &str, cols: u16, rows: u16) {
        let mut s = self.lock();
        s.clock += 1;
        let touched = s.clock;
        let first = s.viewers.is_empty();
        s.viewers.insert(
            id,
            Viewer {
                pane_id: pane_id.to_string(),
                cols,
                rows,
                touched,
            },
        );
        if s.attachment.is_none() {
            drop(s);
            // A herdr without surface.set waits detached for a viewer.
            self.wake.notify_one();
            return;
        }
        if first {
            Self::set_active(&s, true);
        }
        self.take(&mut s, id);
    }

    /// A terminal closes: with no viewer left the surface goes inactive (or
    /// is detached); if it was the target, the latest other viewer takes
    /// over, unless the desktop has the tab.
    pub fn leave(self: &Arc<Self>, id: ViewerId) {
        let mut s = self.lock();
        if s.viewers.remove(&id).is_none() {
            return;
        }
        if s.target == Some(id) {
            s.target = None;
        }
        if s.viewers.is_empty() {
            s.stale = false;
            s.settled = false;
            let detach = match s.attachment.as_ref().map(Attachment::supports_surface_set) {
                Some(true) => {
                    Self::set_active(&s, false);
                    None
                }
                Some(false) => s.attachment.take(),
                None => None,
            };
            let generation = s.generation;
            drop(s);
            self.published.send_replace(Published {
                generation,
                view: None,
                stale: false,
            });
            if let Some(attachment) = detach {
                log::info!(
                    "No terminal is open: detaching from herdr, which cannot make the surface inactive"
                );
                attachment.detach();
            }
            return;
        }
        if s.target.is_none()
            && !s.stale
            && let Some(next) = latest(&s)
        {
            self.take(&mut s, next);
        }
    }

    /// The phone's terminal area is `cols` by `rows` now: sent to herdr
    /// for the target, at most twice a second; kept for later while the
    /// desktop has the tab or another viewer is the target.
    pub fn resize(self: &Arc<Self>, id: ViewerId, cols: u16, rows: u16) {
        let mut s = self.lock();
        let Some(viewer) = s.viewers.get_mut(&id) else {
            return;
        };
        viewer.cols = cols;
        viewer.rows = rows;
        if s.target == Some(id) && !s.stale && s.attachment.is_some() {
            self.send_resize(&mut s);
        }
    }

    /// A touch on viewer `id`'s terminal (a key, a tap, a drag, a wheel):
    /// it becomes the target. While the desktop has the tab, or another
    /// viewer was the target, `pane.focus` and a resize go first, and the
    /// touch waits (up to 1 s) for herdr to answer the focus before its
    /// input is sent.
    pub async fn touch(self: &Arc<Self>, id: ViewerId) -> Result<(), AttachError> {
        if self.claim(id)? {
            self.wait_answered(FOCUS_WAIT).await;
        }
        Ok(())
    }

    /// A touch without input of its own (a tap that arms typing, the lead's
    /// ruling on the 23:32 EDT amendment): viewer `id` becomes the target,
    /// and while the desktop has the tab, or another viewer was the target,
    /// `pane.focus` and a resize are sent again. `Ok(true)` when they were.
    pub fn claim(self: &Arc<Self>, id: ViewerId) -> Result<bool, AttachError> {
        let retargeted = {
            let mut s = self.lock();
            if s.attachment.is_none() {
                return Err(AttachError::Closed);
            }
            s.clock += 1;
            let clock = s.clock;
            let Some(viewer) = s.viewers.get_mut(&id) else {
                return Err(AttachError::Closed);
            };
            viewer.touched = clock;
            if s.stale || s.target != Some(id) {
                if s.stale {
                    log::info!("A touch on the phone's terminal takes herdr's tab back");
                }
                self.take(&mut s, id);
                true
            } else {
                false
            }
        };
        Ok(retargeted)
    }

    /// Send `events` to viewer `id`'s pane.
    pub fn input(
        &self,
        id: ViewerId,
        events: Vec<ClientPaneInputEvent>,
    ) -> Result<(), AttachError> {
        let s = self.lock();
        let (Some(attachment), Some(viewer)) = (&s.attachment, s.viewers.get(&id)) else {
            return Err(AttachError::Closed);
        };
        attachment.input(&viewer.pane_id, events)
    }

    /// Wait until herdr has answered every request sent, or `limit`.
    pub async fn wait_answered(&self, limit: Duration) {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            let pending = match &self.lock().attachment {
                Some(a) => a.requested().unanswered,
                None => return,
            };
            if pending == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// The first surface showing `pane_id` once herdr has answered
    /// flankr's requests and drawn after them, within `limit`; else the
    /// latest one showing it, if any.
    pub async fn first_view(&self, pane_id: &str, limit: Duration) -> Option<Arc<SurfaceView>> {
        let mut rx = self.subscribe();
        let until = Instant::now() + limit;
        loop {
            let shown = {
                let p = rx.borrow_and_update();
                p.view.clone().filter(|v| v.pane(pane_id).is_some())
            };
            if shown.is_some() && self.settled() {
                return shown;
            }
            match tokio::time::timeout_at(until, rx.changed()).await {
                Ok(Ok(())) => {}
                _ => return shown,
            }
        }
    }

    /// Make `id` the target: clear stale, `pane.focus` its pane, and size
    /// the surface to it.
    fn take(self: &Arc<Self>, s: &mut Shared, id: ViewerId) {
        let Some(viewer) = s.viewers.get(&id).cloned() else {
            return;
        };
        s.target = Some(id);
        s.stale = false;
        s.settled = false;
        let Some(attachment) = &s.attachment else {
            return;
        };
        if let Err(e) = attachment.focus_pane(&viewer.pane_id) {
            log::debug!("herdr attachment: pane.focus not sent: {e}");
        }
        self.send_resize(s);
    }

    fn set_active(s: &Shared, active: bool) {
        if let Some(a) = &s.attachment
            && let Err(e) = a.set_active(active)
        {
            log::debug!("herdr attachment: surface.set not sent: {e}");
        }
    }

    /// The target's size to herdr now, or after [`RESIZE_GAP`] from the
    /// last one, coalesced.
    fn send_resize(self: &Arc<Self>, s: &mut Shared) {
        let now = Instant::now();
        match s.last_resize {
            Some(last) if now < last + RESIZE_GAP => {
                if !s.resize_due {
                    s.resize_due = true;
                    let me = Arc::clone(self);
                    tokio::spawn(async move {
                        tokio::time::sleep_until(last + RESIZE_GAP).await;
                        let mut s = me.lock();
                        s.resize_due = false;
                        if !s.stale {
                            me.send_resize(&mut s);
                        }
                    });
                }
            }
            _ => {
                let size = s
                    .target
                    .and_then(|t| s.viewers.get(&t))
                    .map(|v| (v.cols, v.rows));
                let (Some((cols, rows)), Some(attachment)) = (size, &s.attachment) else {
                    return;
                };
                let (cols, rows) = libherdr_term::clamp_surface(cols, rows);
                let requested = attachment.requested();
                s.last_resize = Some(now);
                if (requested.cols, requested.rows) == (cols, rows) {
                    return;
                }
                s.settled = false;
                if let Err(e) = attachment.resize(cols, rows) {
                    log::debug!("herdr attachment: resize not sent: {e}");
                }
            }
        }
    }

    /// Hold the attachment for the daemon's run (the 23:32 EDT amendment,
    /// item 1).
    pub async fn run(self: Arc<Self>) {
        log::info!(
            "herdr attachment on {}: endpoint generation {ENDPOINT_PROTOCOL_GENERATION}, \
             herdr protocol {HERDR_PROTOCOL_VERSION} at the pin",
            self.settings.socket.display()
        );
        let mut delay = self.settings.backoff_initial;
        let mut failures = 0u32;
        loop {
            // A herdr without surface.set is attached only while a
            // terminal is open.
            loop {
                let wait = {
                    let s = self.lock();
                    s.surface_set == Some(false) && s.viewers.is_empty()
                };
                if !wait {
                    break;
                }
                self.wake.notified().await;
            }
            let (active, (cols, rows)) = {
                let s = self.lock();
                let size = s
                    .target
                    .or_else(|| latest(&s))
                    .and_then(|t| s.viewers.get(&t))
                    .map_or(DEFAULT_SIZE, |v| (v.cols, v.rows));
                (!s.viewers.is_empty(), size)
            };
            let opts = AttachOptions {
                active,
                handshake_timeout: self.settings.handshake_timeout,
                ..AttachOptions::new(&self.settings.socket, cols, rows)
            };
            let connected = tokio::task::spawn_blocking(move || Attachment::connect(&opts))
                .await
                .unwrap_or_else(|e| Err(AttachError::Io(std::io::Error::other(e.to_string()))));
            let (attachment, events) = match connected {
                Ok(pair) => pair,
                Err(e) => {
                    failures += 1;
                    self.log_failure(&e, failures, delay);
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(self.settings.backoff_max);
                    continue;
                }
            };
            delay = self.settings.backoff_initial;
            failures = 0;
            log::info!(
                "Attached to herdr {} as a shell client ({})",
                attachment.server_version(),
                if attachment.supports_surface_set() {
                    "inactive while no terminal is open"
                } else {
                    "detached while no terminal is open"
                }
            );
            let (tx, mut rx) = mpsc::unbounded_channel();
            let forward = std::thread::Builder::new()
                .name("herdr-attach-events".into())
                .spawn(move || {
                    while let Ok(event) = events.recv() {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                });
            if let Err(e) = forward {
                log::warn!("herdr attachment: no event thread: {e}");
                attachment.detach();
                tokio::time::sleep(delay).await;
                continue;
            }
            let generation = {
                let mut s = self.lock();
                s.generation += 1;
                s.surface_set = Some(attachment.supports_surface_set());
                s.stale = false;
                s.settled = false;
                s.last_resize = None;
                if s.viewers.is_empty() && !attachment.supports_surface_set() {
                    // Such a herdr ignored `surface_active: false`.
                    drop(s);
                    attachment.detach();
                    continue;
                }
                s.attachment = Some(attachment);
                if let Some(target) = s.target.or_else(|| latest(&s)) {
                    self.take(&mut s, target);
                }
                s.generation
            };
            self.published.send_replace(Published {
                generation,
                view: None,
                stale: false,
            });
            let mut ended = (None, None);
            while let Some(event) = rx.recv().await {
                match event {
                    Event::Closed { reason, error } => {
                        ended = (reason, error);
                        break;
                    }
                    other => self.handle(generation, other),
                }
            }
            let held = {
                let mut s = self.lock();
                s.stale = false;
                s.settled = false;
                s.attachment.take()
            };
            self.published.send_replace(Published {
                generation,
                view: None,
                stale: false,
            });
            let Some(held) = held else {
                // Detached on purpose (no terminal open).
                continue;
            };
            held.detach();
            let (reason, error) = ended;
            if let Some(AttachError::Framing(e)) = &error {
                self.log_incompatible("framing", e, None);
            }
            log::warn!(
                "The herdr attachment closed ({}); terminals read herdr's pane.read until it is back, in {:?}",
                reason.as_deref().unwrap_or("no reason given"),
                delay
            );
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(self.settings.backoff_max);
        }
    }

    fn handle(&self, generation: u64, event: Event) {
        match event {
            Event::Surface(view) => {
                let stale = {
                    let mut guard = self.lock();
                    let s = &mut *guard;
                    let Some(attachment) = &s.attachment else {
                        return;
                    };
                    let requested = attachment.requested();
                    if s.target.is_some()
                        && !s.stale
                        && !s.resize_due
                        && requested.active
                        && requested.unanswered == 0
                        && let Some(pane_id) = &requested.pane_id
                    {
                        match view.desktop_change(pane_id, requested.cols, requested.rows) {
                            None => s.settled = true,
                            Some(change) if s.settled => {
                                log::info!(
                                    "The desktop took herdr's tab ({change:?}); not forced back until the phone's terminal is touched"
                                );
                                s.stale = true;
                            }
                            Some(_) => {}
                        }
                    }
                    s.stale
                };
                self.published.send_replace(Published {
                    generation,
                    view: Some(view),
                    stale,
                });
            }
            Event::FocusAnswer {
                pane_id,
                ok: false,
                code,
                message,
            } => log::warn!(
                "herdr refused pane.focus for '{pane_id}' ({}): {}",
                code.as_deref().unwrap_or("?"),
                message.as_deref().unwrap_or("")
            ),
            Event::ActiveAnswer {
                active,
                ok: false,
                code,
                message,
            } => log::warn!(
                "herdr refused surface.set {{ active: {active} }} ({}): {}",
                code.as_deref().unwrap_or("?"),
                message.as_deref().unwrap_or("")
            ),
            Event::ServerError { message } => log::warn!("herdr attachment: {message}"),
            _ => {}
        }
    }

    fn log_failure(&self, e: &AttachError, failures: u32, delay: Duration) {
        match e {
            AttachError::Incompatible {
                code,
                message,
                server_version,
            } => self.log_incompatible(code, message, server_version.as_deref()),
            _ if failures == 1 => log::info!(
                "Cannot attach to herdr on {} ({e}); terminals read herdr's pane.read, retrying in {delay:?}",
                self.settings.socket.display()
            ),
            _ => log::debug!("Cannot attach to herdr ({e}); retrying in {delay:?}"),
        }
    }

    /// Once per run (decision 11 and the 23:42 EDT amendment, item 3).
    fn log_incompatible(&self, code: &str, message: &str, server_version: Option<&str>) {
        if self.incompatible_logged.swap(true, Ordering::SeqCst) {
            log::debug!("herdr is still not compatible ({code}): {message}");
            return;
        }
        log::warn!(
            "herdr {} is not compatible with flankr's attachment ({code}: {message}); flankr speaks \
             endpoint generation {ENDPOINT_PROTOCOL_GENERATION} with herdr protocol \
             {HERDR_PROTOCOL_VERSION}'s types. Terminals read herdr's pane.read instead.",
            server_version.unwrap_or("(version unknown)")
        );
    }
}

/// The viewer that opened or touched last.
fn latest(s: &Shared) -> Option<ViewerId> {
    s.viewers
        .iter()
        .max_by_key(|(_, v)| v.touched)
        .map(|(id, _)| *id)
}
