//! One herdr shell-client connection: the generation-1 handshake, the
//! reader thread, endpoint requests one at a time, input and resizes
//! (decisions 3, 5, 6 and 7; the 23:32 EDT amendment).

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::herdr::{self, ClientMessage, ServerMessage};
use crate::surface::SurfaceView;

/// The endpoint method that moves this client's view, herdr's active tab
/// and herdr's focused pane to a pane (findings 2.1.7).
pub const METHOD_PANE_FOCUS: &str = "pane.focus";
/// The endpoint method that makes this client's surface active or inactive
/// (findings 2.1.6). herdr before 9e9bc8a1 does not serve it.
pub const METHOD_SURFACE_SET: &str = "client_shell.surface.set";

/// herdr's handshake timeout (`client_transport.rs:42`).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
/// herdr's cap on input events in one message (`client_transport.rs:118`).
const MAX_INPUT_EVENT_BATCH: usize = 4096;
/// How often an `endpoint_busy` answer is retried before it is reported.
const BUSY_RETRIES: u8 = 20;
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Where and how to attach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOptions {
    /// herdr's client socket, `herdr-client.sock` ([`client_socket_for`]).
    pub socket: PathBuf,
    /// The surface size asked for, clamped by [`clamp_surface`].
    pub cols: u16,
    pub rows: u16,
    /// Whether the surface starts active (`surface_active` in the hello).
    /// An inactive surface claims no geometry and is drawn no surfaces
    /// until [`Attachment::set_active`] (the 23:32 EDT amendment, item 1).
    pub active: bool,
    /// How long the welcome and the first snapshot may take.
    pub handshake_timeout: Duration,
}

impl AttachOptions {
    /// Attach to `socket` with an active `cols` by `rows` surface.
    pub fn new(socket: impl Into<PathBuf>, cols: u16, rows: u16) -> AttachOptions {
        AttachOptions {
            socket: socket.into(),
            cols,
            rows,
            active: true,
            handshake_timeout: HANDSHAKE_TIMEOUT,
        }
    }
}

/// herdr's client socket beside its API socket: `-client` inserted before
/// `.sock` (`herdr.sock` is `herdr-client.sock`), as herdr derives it
/// (`src/server/socket_paths.rs`).
pub fn client_socket_for(api_socket: &Path) -> PathBuf {
    let stem = api_socket
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("herdr");
    let parent = api_socket.parent().unwrap_or_else(|| Path::new(""));
    parent.join(format!("{stem}-client.sock"))
}

/// The surface size herdr is asked for (decision 6): 20-500 columns, 5-200
/// rows.
pub fn clamp_surface(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(20, 500), rows.clamp(5, 200))
}

/// What flankr last asked herdr for on this attachment, for telling a
/// change made on the desktop from its own (the 23:32 EDT amendment,
/// item 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    /// The pane of the last `pane.focus`, if any was sent.
    pub pane_id: Option<String>,
    /// The surface size of the hello or the last resize, clamped.
    pub cols: u16,
    pub rows: u16,
    /// The surface's activity as last set.
    pub active: bool,
    /// Requests sent or queued whose answer has not arrived. While it is
    /// not zero, herdr's view may not show flankr's own changes yet.
    pub unanswered: usize,
}

/// What the attachment reports, in order, on the [`Receiver`] of
/// [`Attachment::connect`].
#[derive(Debug)]
pub enum Event {
    /// The handshake is done: always the first event.
    Ready {
        boot_id: String,
        server_version: String,
    },
    /// herdr's `shell.snapshot.v1`: the first one right after `Ready`, then
    /// each time herdr's projection changes (focused workspace, tab and
    /// pane among it).
    Snapshot(Box<herdr::ClientShellSnapshot>),
    /// The surface after a full `PaneSurface` or a `PaneSurfacePatch`. The
    /// attachment clones its held view only while the receiver still holds
    /// an earlier `Arc`.
    Surface(Arc<SurfaceView>),
    /// herdr's answer to [`Attachment::focus_pane`].
    FocusAnswer {
        pane_id: String,
        ok: bool,
        code: Option<String>,
        message: Option<String>,
    },
    /// herdr's answer to [`Attachment::set_active`].
    ActiveAnswer {
        active: bool,
        ok: bool,
        code: Option<String>,
        message: Option<String>,
    },
    /// herdr's `MouseCapture`: whether the focused pane reports the mouse.
    MouseCapture { enabled: bool },
    /// herdr's `ClientShellError`.
    ServerError { message: String },
    /// The connection ended: the last event. `reason` is herdr's shutdown
    /// reason or the error's text; `error` is set when it ended on an error
    /// (`Framing` for a patch on the wrong base or an undecodable message).
    /// `None`/`None` after [`Attachment::detach`].
    Closed {
        reason: Option<String>,
        error: Option<AttachError>,
    },
}

/// Why attaching or a call failed.
#[derive(Debug)]
pub enum AttachError {
    Io(io::Error),
    /// herdr's `FramingError`, or a surface stream that cannot be followed.
    Framing(String),
    /// herdr does not speak endpoint generation 1 to flankr: it refused the
    /// hello (`code` is herdr's: `unsupported_generation`, `no_common_core`
    /// …), or answered with something that is not a generation-1 welcome
    /// (`no_welcome`, `invalid_welcome`, `unexpected_message`,
    /// `direct_terminal_welcome`, `invalid_snapshot`). The daemon falls back
    /// to the poll.
    Incompatible {
        code: String,
        message: String,
        server_version: Option<String>,
    },
    /// An endpoint request's error.
    Endpoint {
        code: String,
        message: String,
    },
    /// The attachment is closed.
    Closed,
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachError::Io(e) => write!(f, "herdr client socket: {e}"),
            AttachError::Framing(e) => write!(f, "herdr framing: {e}"),
            AttachError::Incompatible {
                code,
                message,
                server_version,
            } => {
                write!(f, "herdr is not compatible ({code}): {message}")?;
                if let Some(v) = server_version {
                    write!(f, " (herdr {v})")?;
                }
                Ok(())
            }
            AttachError::Endpoint { code, message } => {
                write!(f, "herdr endpoint request failed ({code}): {message}")
            }
            AttachError::Closed => f.write_str("herdr attachment closed"),
        }
    }
}

impl std::error::Error for AttachError {}

impl From<herdr::FramingError> for AttachError {
    fn from(e: herdr::FramingError) -> Self {
        match e {
            herdr::FramingError::Io(e) => AttachError::Io(e),
            other => AttachError::Framing(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
enum RequestKind {
    Focus(String),
    SurfaceSet(bool),
}

#[derive(Debug, Clone)]
struct Pending {
    id: String,
    kind: RequestKind,
    request: String,
    retries: u8,
}

struct WriterState {
    stream: UnixStream,
    next_id: u64,
    in_flight: Option<Pending>,
    queue: VecDeque<Pending>,
    requested: Requested,
    closed: bool,
    detached: bool,
}

struct Inner {
    boot_id: String,
    welcome: herdr::EndpointServerWelcome,
    writer: Mutex<WriterState>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, WriterState> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write(state: &mut WriterState, msg: &ClientMessage) -> Result<(), AttachError> {
        if state.closed {
            return Err(AttachError::Closed);
        }
        herdr::write_message(&mut &state.stream, msg).map_err(AttachError::from)
    }

    /// Send the next queued request when none is in flight.
    fn pump(&self, state: &mut WriterState) -> Result<(), AttachError> {
        if state.in_flight.is_some() {
            return Ok(());
        }
        let Some(next) = state.queue.pop_front() else {
            return Ok(());
        };
        let msg = ClientMessage::ClientShellEndpointRequest {
            boot_id: self.boot_id.clone(),
            request: next.request.clone(),
        };
        state.in_flight = Some(next);
        Self::write(state, &msg)
    }

    fn enqueue(&self, kind: RequestKind) -> Result<(), AttachError> {
        let mut state = self.lock();
        if state.closed {
            return Err(AttachError::Closed);
        }
        state.next_id += 1;
        let id = format!("flankr-{}", state.next_id);
        let (method, params) = match &kind {
            RequestKind::Focus(pane_id) => {
                state.requested.pane_id = Some(pane_id.clone());
                (METHOD_PANE_FOCUS, serde_json::json!({ "pane_id": pane_id }))
            }
            RequestKind::SurfaceSet(active) => {
                state.requested.active = *active;
                (METHOD_SURFACE_SET, serde_json::json!({ "active": active }))
            }
        };
        let request =
            serde_json::json!({ "id": id, "method": method, "params": params }).to_string();
        state.queue.push_back(Pending {
            id,
            kind,
            request,
            retries: 0,
        });
        state.requested.unanswered += 1;
        self.pump(&mut state)
    }
}

/// One attachment to herdr as a shell client.
pub struct Attachment {
    inner: Arc<Inner>,
    reader: Option<JoinHandle<()>>,
}

impl fmt::Debug for Attachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachment")
            .field("boot_id", &self.inner.boot_id)
            .field("server_version", &self.inner.welcome.server_version)
            .finish_non_exhaustive()
    }
}

fn incompatible(
    code: &str,
    message: impl Into<String>,
    server_version: Option<String>,
) -> AttachError {
    AttachError::Incompatible {
        code: code.to_owned(),
        message: message.into(),
        server_version,
    }
}

fn timed_out(e: &herdr::FramingError) -> bool {
    matches!(e, herdr::FramingError::Io(e)
        if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
}

impl Attachment {
    /// Connect, say the generation-1 hello (decision 3), and wait for
    /// herdr's welcome and first snapshot. Anything but a generation-1
    /// welcome, including the connection ending or an undecodable answer,
    /// is [`AttachError::Incompatible`]: a `PROTOCOL_VERSION` alone does
    /// not prove generation 1 (docs/HERDR_WIRE.md).
    pub fn connect(opts: &AttachOptions) -> Result<(Attachment, Receiver<Event>), AttachError> {
        let stream = UnixStream::connect(&opts.socket).map_err(AttachError::Io)?;
        stream
            .set_read_timeout(Some(opts.handshake_timeout))
            .map_err(AttachError::Io)?;
        let (cols, rows) = clamp_surface(opts.cols, opts.rows);
        let hello = herdr::EndpointClientHello {
            generation: herdr::ENDPOINT_PROTOCOL_GENERATION,
            cell_width_px: 0,
            cell_height_px: 0,
            surface_size: herdr::ClientSurfaceSize { cols, rows },
            pixel_mouse: false,
            direct_graphics: false,
            endpoint_keybindings: false,
            mouse_capture: false,
            surface_active: opts.active,
            // The optional encodings would arrive inside EndpointControl;
            // flankr asks for none of them.
            surface_reuse: false,
            surface_delta: false,
            surface_scroll: false,
            snapshot_codecs: vec![herdr::SNAPSHOT_CODEC_V1.into()],
            surface_codecs: vec![herdr::SURFACE_CODEC_V1.into()],
            input_codecs: vec![herdr::INPUT_CODEC_V1.into()],
            blob_codecs: vec![herdr::BLOB_CODEC_V1.into()],
        };
        let hello = ClientMessage::EndpointControl {
            kind: herdr::ENDPOINT_HELLO_KIND.into(),
            data: serde_json::to_string(&hello).expect("hello encodes"),
        };
        herdr::write_message(&mut &stream, &hello)?;

        let first: ServerMessage =
            herdr::read_message(&mut &stream, herdr::MAX_GRAPHICS_FRAME_SIZE).map_err(|e| {
                if timed_out(&e) {
                    AttachError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "no welcome from herdr",
                    ))
                } else {
                    incompatible("no_welcome", e.to_string(), None)
                }
            })?;
        let welcome = match first {
            ServerMessage::EndpointControl { kind, data }
                if kind == herdr::ENDPOINT_WELCOME_KIND =>
            {
                serde_json::from_str::<herdr::EndpointServerWelcome>(&data)
                    .map_err(|e| incompatible("invalid_welcome", e.to_string(), None))?
            }
            ServerMessage::Welcome { version, error, .. } => {
                return Err(incompatible(
                    "direct_terminal_welcome",
                    error.unwrap_or_else(|| {
                        format!(
                            "herdr answered with the direct-terminal welcome (protocol {version})"
                        )
                    }),
                    None,
                ));
            }
            other => {
                return Err(incompatible(
                    "unexpected_message",
                    format!("herdr answered the hello with {}", message_name(&other)),
                    None,
                ));
            }
        };
        if let Some(error) = welcome.error {
            return Err(incompatible(
                &error.code,
                error.message,
                Some(welcome.server_version),
            ));
        }
        if welcome.generation != herdr::ENDPOINT_PROTOCOL_GENERATION {
            return Err(incompatible(
                "unsupported_generation",
                format!("herdr speaks endpoint generation {}", welcome.generation),
                Some(welcome.server_version),
            ));
        }

        // Up to the first snapshot; what comes before it is replayed by the
        // reader thread.
        let mut early = Vec::new();
        let snapshot = loop {
            let msg: ServerMessage =
                herdr::read_message(&mut &stream, herdr::MAX_GRAPHICS_FRAME_SIZE).map_err(|e| {
                    if timed_out(&e) {
                        AttachError::Io(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "no snapshot from herdr",
                        ))
                    } else {
                        AttachError::from(e)
                    }
                })?;
            match msg {
                ServerMessage::EndpointControl { kind, data }
                    if kind == herdr::SNAPSHOT_CODEC_V1 =>
                {
                    break serde_json::from_str::<herdr::ClientShellSnapshot>(&data).map_err(
                        |e| {
                            incompatible(
                                "invalid_snapshot",
                                e.to_string(),
                                Some(welcome.server_version.clone()),
                            )
                        },
                    )?;
                }
                other => early.push(other),
            }
        };
        stream.set_read_timeout(None).map_err(AttachError::Io)?;

        let (tx, rx) = channel();
        let _ = tx.send(Event::Ready {
            boot_id: snapshot.boot_id.clone(),
            server_version: welcome.server_version.clone(),
        });
        let boot_id = snapshot.boot_id.clone();
        let _ = tx.send(Event::Snapshot(Box::new(snapshot)));

        let read_stream = stream.try_clone().map_err(AttachError::Io)?;
        let inner = Arc::new(Inner {
            boot_id,
            welcome,
            writer: Mutex::new(WriterState {
                stream,
                next_id: 0,
                in_flight: None,
                queue: VecDeque::new(),
                requested: Requested {
                    pane_id: None,
                    cols,
                    rows,
                    active: opts.active,
                    unanswered: 0,
                },
                closed: false,
                detached: false,
            }),
        });
        let reader_inner = Arc::clone(&inner);
        let reader = std::thread::Builder::new()
            .name("herdr-term-attach".into())
            .spawn(move || Reader::new(reader_inner, tx).run(read_stream, early))
            .map_err(AttachError::Io)?;
        Ok((
            Attachment {
                inner,
                reader: Some(reader),
            },
            rx,
        ))
    }

    /// The server boot, from the first snapshot.
    pub fn boot_id(&self) -> &str {
        &self.inner.boot_id
    }

    /// herdr's welcome: its version, methods and capabilities.
    pub fn welcome(&self) -> &herdr::EndpointServerWelcome {
        &self.inner.welcome
    }

    /// herdr's `server_version` from the welcome.
    pub fn server_version(&self) -> &str {
        &self.inner.welcome.server_version
    }

    /// Whether herdr serves `client_shell.surface.set` (it lists it in the
    /// welcome's `methods`; herdr before 9e9bc8a1 does not).
    pub fn supports_surface_set(&self) -> bool {
        self.inner
            .welcome
            .methods
            .iter()
            .any(|m| m == METHOD_SURFACE_SET)
    }

    /// What flankr last asked for.
    pub fn requested(&self) -> Requested {
        self.inner.lock().requested.clone()
    }

    /// Send `pane.focus` for `pane_id` (decision 5): it moves this client's
    /// view, herdr's active tab and focused pane, and claims the tab's
    /// geometry. One request is in flight at a time; later ones wait, and
    /// `endpoint_busy` is retried. The answer is [`Event::FocusAnswer`].
    /// herdr refuses it (`surface_inactive`) while the surface is inactive.
    pub fn focus_pane(&self, pane_id: &str) -> Result<(), AttachError> {
        self.inner.enqueue(RequestKind::Focus(pane_id.to_owned()))
    }

    /// Make the surface active or inactive (`client_shell.surface.set`, the
    /// 23:32 EDT amendment, item 1). `Ok(true)` when sent, answered by
    /// [`Event::ActiveAnswer`]; `Ok(false)`, and nothing sent, when this
    /// herdr does not serve the method ([`Attachment::supports_surface_set`]):
    /// such a herdr also ignored the hello's `surface_active`, so the
    /// surface is active, and the caller detaches to give the tab back.
    pub fn set_active(&self, active: bool) -> Result<bool, AttachError> {
        if !self.supports_surface_set() {
            return Ok(false);
        }
        self.inner.enqueue(RequestKind::SurfaceSet(active))?;
        Ok(true)
    }

    /// Ask for a `cols` by `rows` surface (`ClientShellResize`, decision 6),
    /// clamped by [`clamp_surface`].
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), AttachError> {
        let (cols, rows) = clamp_surface(cols, rows);
        let mut state = self.inner.lock();
        state.requested.cols = cols;
        state.requested.rows = rows;
        Inner::write(
            &mut state,
            &ClientMessage::ClientShellResize {
                cell_width_px: 0,
                cell_height_px: 0,
                surface_size: herdr::ClientSurfaceSize { cols, rows },
                pixel_mouse: false,
            },
        )
    }

    /// Send `events` to `pane_id` (`ClientShellPaneInput`), in batches of
    /// herdr's 4096. Mouse positions are pane-local.
    pub fn input(
        &self,
        pane_id: &str,
        events: Vec<herdr::ClientPaneInputEvent>,
    ) -> Result<(), AttachError> {
        let mut state = self.inner.lock();
        let mut events = events;
        while !events.is_empty() {
            let rest = events.split_off(events.len().min(MAX_INPUT_EVENT_BATCH));
            Inner::write(
                &mut state,
                &ClientMessage::ClientShellPaneInput {
                    pane_id: pane_id.to_owned(),
                    events,
                },
            )?;
            events = rest;
        }
        Ok(())
    }

    /// Leave cleanly: `Detach`, then the socket is closed and the reader
    /// joined. Dropping the attachment does the same.
    pub fn detach(self) {
        drop(self);
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        {
            let mut state = self.inner.lock();
            if !state.closed && !state.detached {
                state.detached = true;
                let _ = Inner::write(&mut state, &ClientMessage::Detach);
            }
            state.detached = true;
            state.closed = true;
            let _ = state.stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn message_name(msg: &ServerMessage) -> &'static str {
    match msg {
        ServerMessage::Welcome { .. } => "Welcome",
        ServerMessage::Terminal(_) => "Terminal",
        ServerMessage::Graphics { .. } => "Graphics",
        ServerMessage::ServerShutdown { .. } => "ServerShutdown",
        ServerMessage::Notify { .. } => "Notify",
        ServerMessage::Clipboard { .. } => "Clipboard",
        ServerMessage::WindowTitle { .. } => "WindowTitle",
        ServerMessage::ReloadSoundConfig => "ReloadSoundConfig",
        ServerMessage::MouseCapture { .. } => "MouseCapture",
        ServerMessage::TerminalBell { .. } => "TerminalBell",
        ServerMessage::GraphicsFile { .. } => "GraphicsFile",
        ServerMessage::GraphicsTransmissionRetired { .. } => "GraphicsTransmissionRetired",
        ServerMessage::ClientShellSnapshot(_) => "ClientShellSnapshot",
        ServerMessage::PaneSurface(_) => "PaneSurface",
        ServerMessage::SemanticNotification(_) => "SemanticNotification",
        ServerMessage::ClientShellError { .. } => "ClientShellError",
        ServerMessage::DirectTerminalKeyboardProtocol { .. } => "DirectTerminalKeyboardProtocol",
        ServerMessage::ClientShellKeyboardReportAll { .. } => "ClientShellKeyboardReportAll",
        ServerMessage::ClientShellEndpointResponseChunk { .. } => {
            "ClientShellEndpointResponseChunk"
        }
        ServerMessage::PaneSurfacePatch(_) => "PaneSurfacePatch",
        ServerMessage::EndpointControl { .. } => "EndpointControl",
    }
}

/// An endpoint answer: herdr's API response JSON, `{"id", "result"}` or
/// `{"id", "error": {"code", "message"}}`.
fn parse_answer(data: &[u8]) -> (bool, Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return (
            false,
            Some("invalid_response".into()),
            Some(String::from_utf8_lossy(data).into_owned()),
        );
    };
    match value.get("error") {
        Some(error) if !error.is_null() => (
            false,
            error
                .get("code")
                .and_then(|c| c.as_str())
                .map(str::to_owned),
            error
                .get("message")
                .and_then(|c| c.as_str())
                .map(str::to_owned),
        ),
        _ => (true, None, None),
    }
}

struct Reader {
    inner: Arc<Inner>,
    tx: Sender<Event>,
    view: Option<Arc<SurfaceView>>,
    chunks: HashMap<String, Vec<u8>>,
}

enum Flow {
    Continue,
    Stop {
        reason: Option<String>,
        error: Option<AttachError>,
    },
}

impl Reader {
    fn new(inner: Arc<Inner>, tx: Sender<Event>) -> Reader {
        Reader {
            inner,
            tx,
            view: None,
            chunks: HashMap::new(),
        }
    }

    fn run(mut self, stream: UnixStream, early: Vec<ServerMessage>) {
        let mut flow = Flow::Continue;
        for msg in early {
            flow = self.handle(msg);
            if !matches!(flow, Flow::Continue) {
                break;
            }
        }
        while matches!(flow, Flow::Continue) {
            flow = match herdr::read_message::<_, ServerMessage>(
                &mut &stream,
                herdr::MAX_GRAPHICS_FRAME_SIZE,
            ) {
                Ok(msg) => self.handle(msg),
                Err(e) => {
                    if self.inner.lock().detached {
                        Flow::Stop {
                            reason: None,
                            error: None,
                        }
                    } else {
                        let error = AttachError::from(e);
                        Flow::Stop {
                            reason: Some(error.to_string()),
                            error: Some(error),
                        }
                    }
                }
            };
        }
        let Flow::Stop { reason, error } = flow else {
            unreachable!()
        };
        {
            let mut state = self.inner.lock();
            state.closed = true;
            let _ = state.stream.shutdown(std::net::Shutdown::Both);
        }
        let _ = self.tx.send(Event::Closed { reason, error });
    }

    fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    fn handle(&mut self, msg: ServerMessage) -> Flow {
        match msg {
            ServerMessage::PaneSurface(frame) => {
                let view = Arc::new(SurfaceView::from_frame(frame));
                self.view = Some(Arc::clone(&view));
                self.send(Event::Surface(view));
            }
            ServerMessage::PaneSurfacePatch(patch) => {
                let Some(view) = self.view.as_mut() else {
                    let error = AttachError::Framing(format!(
                        "herdr patch on surface revision {} before any full surface",
                        patch.base_surface_revision
                    ));
                    return Flow::Stop {
                        reason: Some(error.to_string()),
                        error: Some(error),
                    };
                };
                if let Err(e) = Arc::make_mut(view).apply_patch(patch) {
                    let error = AttachError::Framing(e);
                    return Flow::Stop {
                        reason: Some(error.to_string()),
                        error: Some(error),
                    };
                }
                let view = Arc::clone(view);
                self.send(Event::Surface(view));
            }
            ServerMessage::EndpointControl { kind, data } => {
                if kind == herdr::SNAPSHOT_CODEC_V1 {
                    // A snapshot that no longer decodes is skipped; the
                    // surface stream does not depend on it.
                    if let Ok(snapshot) = serde_json::from_str::<herdr::ClientShellSnapshot>(&data)
                    {
                        self.send(Event::Snapshot(Box::new(snapshot)));
                    }
                } else if kind == herdr::HEALTH_PING_KIND {
                    let mut state = self.inner.lock();
                    let _ = Inner::write(
                        &mut state,
                        &ClientMessage::EndpointControl {
                            kind: herdr::HEALTH_PONG_KIND.into(),
                            data,
                        },
                    );
                }
                // Every other named control is optional and ignored
                // (endpoint.rs:8).
            }
            ServerMessage::ClientShellEndpointResponseChunk {
                request_id,
                final_chunk,
                data,
                ..
            } => {
                let mut buf = self.chunks.remove(&request_id).unwrap_or_default();
                buf.extend_from_slice(&data);
                if final_chunk {
                    self.answer(&request_id, &buf);
                } else {
                    self.chunks.insert(request_id, buf);
                }
            }
            ServerMessage::MouseCapture { enabled, .. } => {
                self.send(Event::MouseCapture { enabled });
            }
            ServerMessage::ClientShellError { message } => {
                self.send(Event::ServerError { message });
            }
            ServerMessage::ServerShutdown { reason } => {
                return Flow::Stop {
                    reason: reason.or_else(|| Some("herdr shut down".into())),
                    error: None,
                };
            }
            _ => {}
        }
        Flow::Continue
    }

    fn answer(&mut self, request_id: &str, data: &[u8]) {
        let (ok, code, message) = parse_answer(data);
        let mut state = self.inner.lock();
        let Some(pending) = state.in_flight.take_if(|p| p.id == request_id) else {
            return;
        };
        if code.as_deref() == Some("endpoint_busy") && pending.retries < BUSY_RETRIES {
            let mut retry = pending;
            retry.retries += 1;
            state.queue.push_front(retry);
            drop(state);
            std::thread::sleep(BUSY_RETRY_DELAY);
            let mut state = self.inner.lock();
            let _ = self.inner.pump(&mut state);
            return;
        }
        state.requested.unanswered = state.requested.unanswered.saturating_sub(1);
        let _ = self.inner.pump(&mut state);
        drop(state);
        self.send(match pending.kind {
            RequestKind::Focus(pane_id) => Event::FocusAnswer {
                pane_id,
                ok,
                code,
                message,
            },
            RequestKind::SurfaceSet(active) => Event::ActiveAnswer {
                active,
                ok,
                code,
                message,
            },
        });
    }
}
