//! Shared by the tests: herdr's checkout, and an in-process fake herdr
//! server on one end of a `UnixStream` pair that speaks the extracted types.
//! Nothing here touches a real herdr socket.

#![allow(dead_code)]

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use libherdr_term::herdr::{self, ClientMessage, ServerMessage};

/// herdr's checkout: `HERDR_SRC`, else the submodule (as build.rs finds it).
pub fn herdr_root() -> PathBuf {
    match std::env::var("HERDR_SRC") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/herdr"),
    }
}

/// The server side of one fake connection.
pub struct Fake {
    pub stream: UnixStream,
}

impl Fake {
    pub fn read(&mut self) -> ClientMessage {
        herdr::read_message(&mut &self.stream, herdr::MAX_FRAME_SIZE).expect("client message")
    }

    pub fn try_read(&mut self) -> Result<ClientMessage, herdr::FramingError> {
        herdr::read_message(&mut &self.stream, herdr::MAX_FRAME_SIZE)
    }

    pub fn send(&mut self, msg: &ServerMessage) {
        herdr::write_message(&mut &self.stream, msg).expect("server message");
    }

    /// Read the hello and decode its JSON.
    pub fn read_hello(&mut self) -> herdr::EndpointClientHello {
        match self.read() {
            ClientMessage::EndpointControl { kind, data } => {
                assert_eq!(kind, herdr::ENDPOINT_HELLO_KIND);
                serde_json::from_str(&data).expect("hello json")
            }
            other => panic!("expected the endpoint hello, got {other:?}"),
        }
    }

    pub fn welcome(&mut self, methods: &[&str]) {
        let welcome = herdr::EndpointServerWelcome {
            generation: 1,
            server_version: "0.9.5-fake".into(),
            snapshot_codec: herdr::SNAPSHOT_CODEC_V1.into(),
            surface_codec: herdr::SURFACE_CODEC_V1.into(),
            input_codec: herdr::INPUT_CODEC_V1.into(),
            blob_codec: herdr::BLOB_CODEC_V1.into(),
            methods: methods.iter().map(|m| m.to_string()).collect(),
            capabilities: Vec::new(),
            error: None,
        };
        self.send(&ServerMessage::EndpointControl {
            kind: herdr::ENDPOINT_WELCOME_KIND.into(),
            data: serde_json::to_string(&welcome).unwrap(),
        });
    }

    pub fn snapshot(&mut self, boot_id: &str, focused_pane: Option<&str>) {
        let snapshot = snapshot(boot_id, focused_pane);
        self.send(&ServerMessage::EndpointControl {
            kind: herdr::SNAPSHOT_CODEC_V1.into(),
            data: serde_json::to_string(&snapshot).unwrap(),
        });
    }

    /// Hello, welcome (serving both endpoint methods) and snapshot.
    pub fn handshake(&mut self) -> herdr::EndpointClientHello {
        let hello = self.read_hello();
        self.welcome(&["pane.focus", "client_shell.surface.set"]);
        self.snapshot("boot-1", Some("w1:p1"));
        hello
    }

    /// Answer the endpoint request `request_id` with `json` in two chunks.
    pub fn answer(&mut self, boot_id: &str, request_id: &str, json: &str) {
        let (a, b) = json.as_bytes().split_at(json.len() / 2);
        for (data, final_chunk) in [(a, false), (b, true)] {
            self.send(&ServerMessage::ClientShellEndpointResponseChunk {
                boot_id: boot_id.into(),
                request_id: request_id.into(),
                final_chunk,
                data: data.to_vec(),
            });
        }
    }
}

pub fn snapshot(boot_id: &str, focused_pane: Option<&str>) -> herdr::ClientShellSnapshot {
    herdr::ClientShellSnapshot {
        boot_id: boot_id.into(),
        revision: 1,
        config_diagnostic: None,
        product_announcement: None,
        update_available: None,
        update_install_command: "herdr update".into(),
        server_keybindings_toml: None,
        latest_release_notes_available: false,
        integration_updates_available: false,
        worktree_directory: String::new(),
        release_notes: None,
        focused_workspace_id: Some("w1".into()),
        focused_tab_id: Some("w1:t1".into()),
        focused_pane_id: focused_pane.map(str::to_owned),
        tab_bar_right: Vec::new(),
        tab_bar_right_separator: String::new(),
        agent_view_label: None,
        agent_order: Vec::new(),
        workspaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        agents: Vec::new(),
        commands: Vec::new(),
    }
}

/// A fake herdr listening on a socket in `dir`; `serve` runs on its own
/// thread with the one accepted connection.
pub fn fake_server(
    dir: &std::path::Path,
    serve: impl FnOnce(Fake) + Send + 'static,
) -> (PathBuf, std::thread::JoinHandle<()>) {
    let path = dir.join("herdr-client.sock");
    let listener = UnixListener::bind(&path).expect("bind fake socket");
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        serve(Fake { stream });
    });
    (path, handle)
}

pub fn cell(symbol: &str, fg: u32, bg: u32, modifier: u16) -> herdr::CellData {
    herdr::CellData {
        symbol: symbol.into(),
        fg,
        bg,
        modifier,
        skip: false,
        hyperlink: None,
    }
}

pub fn rect(x: u16, y: u16, width: u16, height: u16) -> herdr::SurfaceRect {
    herdr::SurfaceRect {
        x,
        y,
        width,
        height,
    }
}

pub fn pane(pane_id: &str, inner: herdr::SurfaceRect, focused: bool) -> herdr::PaneSurfacePane {
    herdr::PaneSurfacePane {
        pane_id: pane_id.into(),
        content_revision: 0,
        rect: inner,
        inner_rect: inner,
        scrollbar_rect: None,
        scroll: Some(herdr::PaneSurfaceScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 0,
            viewport_rows: u64::from(inner.height),
        }),
        focused,
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        alternate_screen_active: false,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// A `width` by `height` surface of `fill` cells.
pub fn surface(
    revision: u64,
    width: u16,
    height: u16,
    fill: &str,
    panes: Vec<herdr::PaneSurfacePane>,
    cursor: Option<herdr::CursorState>,
) -> herdr::PaneSurfaceFrame {
    herdr::PaneSurfaceFrame {
        boot_id: "boot-1".into(),
        projection_revision: 1,
        surface_revision: revision,
        frame: herdr::FrameData {
            cells: (0..usize::from(width) * usize::from(height))
                .map(|_| cell(fill, 0, 0, 0))
                .collect(),
            width,
            height,
            cursor,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        },
        panes,
        splits: Vec::new(),
        popup: None,
        graphics: herdr::SurfaceGraphicsScene::default(),
    }
}
