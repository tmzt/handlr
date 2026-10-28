//! The extracted herdr types encode as herdr's own (plan 20261027_08,
//! step 2): herdr's golden hashes, its frozen message tags, and its
//! generation-1 fixtures from the submodule.

mod common;

use libherdr_term::herdr::{self, *};
use sha2::{Digest, Sha256};

fn encode(value: &impl serde::Serialize) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

fn sha256(value: &impl serde::Serialize) -> String {
    Sha256::digest(encode(value))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn decode<T: for<'de> serde::Deserialize<'de>>(bytes: &[u8]) -> T {
    let (value, used) =
        bincode::serde::decode_from_slice(bytes, bincode::config::standard()).unwrap();
    assert_eq!(used, bytes.len());
    value
}

// herdr's `color_to_u32` of the ratatui colours its tests use.
const RESET: u32 = 0;
const BLACK: u32 = 1;
const RED: u32 = 2;
const GREEN: u32 = 3;
const YELLOW: u32 = 4;
const BLUE: u32 = 5;
const MAGENTA: u32 = 6;
const CYAN: u32 = 7;
// ratatui's `Modifier` bits.
const BOLD: u16 = 1;
const ITALIC: u16 = 4;
const UNDERLINED: u16 = 8;
const REVERSED: u16 = 64;

fn cell(symbol: &str, fg: u32, bg: u32, modifier: u16, skip: bool, link: Option<u32>) -> CellData {
    CellData {
        symbol: symbol.into(),
        fg,
        bg,
        modifier,
        skip,
        hyperlink: link,
    }
}

#[test]
fn protocol_constants_come_from_the_pin() {
    assert_eq!(libherdr_term::HERDR_PROTOCOL_VERSION, 22);
    assert_eq!(libherdr_term::ENDPOINT_PROTOCOL_GENERATION, 1);
    assert_eq!(herdr::MAX_FRAME_SIZE, 2 * 1024 * 1024);
    assert_eq!(herdr::MAX_GRAPHICS_FRAME_SIZE, 32 * 1024 * 1024);
    assert!(libherdr_term::HERDR_WIRE_RS.contains("pub enum ServerMessage"));
}

/// herdr's `pane_surface_roundtrip` (wire.rs:2504-2595), built from the
/// same values.
#[test]
fn pane_surface_matches_herdrs_golden_hash() {
    let frame = FrameData {
        cells: vec![
            cell("H", RED, BLACK, BOLD, false, None),
            cell("i", GREEN, RESET, ITALIC, false, None),
            cell(
                "!",
                0x02FF_8000,
                0x0100_00DC,
                BOLD | UNDERLINED,
                false,
                Some(0),
            ),
            cell(" ", RESET, RESET, 0, true, None),
            cell("→", CYAN, BLUE, REVERSED, false, None),
            cell("🦀", YELLOW, MAGENTA, 0, false, None),
        ],
        width: 3,
        height: 2,
        cursor: Some(CursorState {
            x: 0,
            y: 0,
            visible: true,
            shape: 6,
        }),
        hyperlinks: vec!["https://example.com".to_owned()],
        graphics: Vec::new(),
    };
    let msg = ServerMessage::PaneSurface(PaneSurfaceFrame {
        boot_id: "boot-1".into(),
        projection_revision: 1,
        surface_revision: 1,
        frame,
        panes: Vec::new(),
        splits: Vec::new(),
        popup: None,
        graphics: SurfaceGraphicsScene::default(),
    });
    assert_eq!(
        sha256(&msg),
        "7c016f7b21ddb5ac79212cf65a968b93eb292b5305b941263e89ffaa40158ee3"
    );
    assert_eq!(decode::<ServerMessage>(&encode(&msg)), msg);
}

/// herdr's `pane_surface_patch_roundtrip` (wire.rs:2597-2631).
#[test]
fn pane_surface_patch_matches_herdrs_golden_hash() {
    let msg = ServerMessage::PaneSurfacePatch(PaneSurfacePatch {
        boot_id: "boot-1".into(),
        projection_revision: 3,
        base_surface_revision: 7,
        surface_revision: 8,
        rows: vec![PaneSurfacePatchRow {
            x: 2,
            y: 4,
            cells: vec![cell("x", 1, 2, 3, false, None)],
        }],
        panes: Vec::new(),
        cursor: Some(CursorState {
            x: 2,
            y: 4,
            visible: true,
            shape: 2,
        }),
    });
    assert_eq!(
        sha256(&msg),
        "0814b99a1dc6eaf7918424aa416c066509cbfb73b72344a809c27cde78cb6dbd"
    );
    assert_eq!(decode::<ServerMessage>(&encode(&msg)), msg);
}

fn tag(value: &impl serde::Serialize) -> u8 {
    encode(value)[0]
}

/// herdr's `client_message_wire_tags_reflect_current_order` (wire.rs:1957).
#[test]
fn client_message_tags_are_herdrs() {
    let size = ClientSurfaceSize { cols: 80, rows: 29 };
    let cases: Vec<(ClientMessage, u8)> = vec![
        (
            ClientMessage::TerminalHello {
                version: PROTOCOL_VERSION,
                cols: 80,
                rows: 24,
                cell_width_px: 8,
                cell_height_px: 16,
                pixel_mouse: false,
            },
            0,
        ),
        (ClientMessage::Input { data: Vec::new() }, 1),
        (
            ClientMessage::ClipboardImage {
                target: ClientClipboardImageTarget::DirectTerminal,
                extension: "png".into(),
                data: Vec::new(),
            },
            2,
        ),
        (
            ClientMessage::Resize {
                cols: 80,
                rows: 24,
                cell_width_px: 8,
                cell_height_px: 16,
                pixel_mouse: false,
            },
            3,
        ),
        (ClientMessage::Detach, 4),
        (
            ClientMessage::AttachTerminal {
                terminal_id: "term".into(),
                takeover: false,
            },
            5,
        ),
        (
            ClientMessage::AttachScroll {
                source: AttachScrollSource::Wheel,
                direction: AttachScrollDirection::Up,
                lines: 1,
                column: None,
                row: None,
                modifiers: 0,
            },
            6,
        ),
        (
            ClientMessage::ObserveTerminal {
                target: "w1:p1".into(),
            },
            7,
        ),
        (
            ClientMessage::ControlTerminal {
                target: "w1:p1".into(),
                takeover: false,
            },
            8,
        ),
        (
            ClientMessage::GraphicsTransmissionResult {
                transfer_id: 1,
                image_id: 2,
                success: true,
            },
            9,
        ),
        (
            ClientMessage::GraphicsTransmissionStarted {
                transfer_id: 1,
                image_id: 2,
            },
            10,
        ),
        (
            ClientMessage::ClientShellHello {
                version: PROTOCOL_VERSION,
                cell_width_px: 8,
                cell_height_px: 16,
                surface_size: size,
                pixel_mouse: false,
                direct_graphics: false,
                endpoint_keybindings: false,
                mouse_capture: false,
            },
            11,
        ),
        (
            ClientMessage::ClientShellResize {
                cell_width_px: 8,
                cell_height_px: 16,
                surface_size: size,
                pixel_mouse: false,
            },
            12,
        ),
        (
            ClientMessage::ClientShellPaneInput {
                pane_id: "pane".into(),
                events: Vec::new(),
            },
            13,
        ),
        (
            ClientMessage::ClientShellPopupInput {
                terminal_id: "popup".into(),
                events: Vec::new(),
            },
            14,
        ),
        (
            ClientMessage::ClientShellEndpointRequest {
                boot_id: "boot".into(),
                request: "{}".into(),
            },
            15,
        ),
        (
            ClientMessage::AttachMouse {
                kind: ClientMouseKind::Down(ClientMouseButton::Left),
                position: ClientMousePosition::Cell { column: 10, row: 5 },
                geometry: None,
                modifiers: 0,
                lines: 1,
            },
            16,
        ),
        (
            ClientMessage::ClientShellHostTheme {
                update: ClientHostThemeUpdate::Appearance(ClientHostAppearance::Dark),
            },
            17,
        ),
        (ClientMessage::ClientShellFocus { focused: true }, 18),
        (ClientMessage::ClientShellMouseCapture { enabled: true }, 19),
        (
            ClientMessage::EndpointControl {
                kind: String::new(),
                data: String::new(),
            },
            20,
        ),
    ];
    for (msg, expected) in cases {
        assert_eq!(tag(&msg), expected, "{msg:?}");
    }
}

/// herdr's `client_shell_server_message_tags_are_frozen` (wire.rs:2720) and
/// the two-string `EndpointControl` payload.
#[test]
fn server_message_tags_are_herdrs() {
    let empty = PaneSurfaceFrame {
        boot_id: "boot".into(),
        projection_revision: 1,
        surface_revision: 1,
        frame: FrameData {
            cells: Vec::new(),
            width: 0,
            height: 0,
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        },
        panes: Vec::new(),
        splits: Vec::new(),
        popup: None,
        graphics: SurfaceGraphicsScene::default(),
    };
    assert_eq!(tag(&ServerMessage::PaneSurface(empty)), 13);
    assert_eq!(
        tag(&ServerMessage::ClientShellError {
            message: String::new()
        }),
        15
    );
    assert_eq!(
        tag(&ServerMessage::ClientShellKeyboardReportAll { enabled: false }),
        17
    );
    assert_eq!(
        tag(&ServerMessage::ClientShellEndpointResponseChunk {
            boot_id: String::new(),
            request_id: String::new(),
            final_chunk: true,
            data: Vec::new(),
        }),
        18
    );
    assert_eq!(
        tag(&ServerMessage::PaneSurfacePatch(PaneSurfacePatch {
            boot_id: String::new(),
            projection_revision: 0,
            base_surface_revision: 0,
            surface_revision: 0,
            rows: Vec::new(),
            panes: Vec::new(),
            cursor: None,
        })),
        19
    );
    assert_eq!(
        encode(&ServerMessage::EndpointControl {
            kind: String::new(),
            data: String::new(),
        }),
        [20, 0, 0]
    );
}

fn fixture(name: &str) -> String {
    let path = common::herdr_root().join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// herdr's `frozen_generation_one_handshake_decodes` and
/// `frozen_generation_one_snapshot_decodes` (endpoint.rs:262-300), on the
/// fixtures in the submodule.
#[test]
fn herdrs_generation_one_fixtures_decode() {
    let hello: EndpointClientHello =
        serde_json::from_str(&fixture("endpoint-hello-v1.json")).unwrap();
    assert_eq!(hello.generation, ENDPOINT_PROTOCOL_GENERATION);
    assert!(hello.snapshot_codecs.iter().any(|c| c == SNAPSHOT_CODEC_V1));
    assert!(hello.surface_codecs.iter().any(|c| c == SURFACE_CODEC_V1));
    assert!(hello.input_codecs.iter().any(|c| c == INPUT_CODEC_V1));
    assert!(hello.blob_codecs.iter().any(|c| c == BLOB_CODEC_V1));

    let welcome: EndpointServerWelcome =
        serde_json::from_str(&fixture("endpoint-welcome-v1.json")).unwrap();
    assert_eq!(welcome.generation, ENDPOINT_PROTOCOL_GENERATION);
    assert_eq!(welcome.snapshot_codec, SNAPSHOT_CODEC_V1);
    assert_eq!(welcome.surface_codec, SURFACE_CODEC_V1);
    assert_eq!(welcome.input_codec, INPUT_CODEC_V1);
    assert_eq!(welcome.blob_codec, BLOB_CODEC_V1);
    assert!(welcome.capabilities.is_empty());

    let snapshot: ClientShellSnapshot =
        serde_json::from_str(&fixture("endpoint-snapshot-v1.json")).unwrap();
    assert_eq!(snapshot.boot_id, "boot-v1");
    assert_eq!(snapshot.workspaces[0].agent_status, AgentStatus::Unknown);
}

#[test]
fn framing_round_trips_and_refuses_trailing_bytes() {
    let msg = ClientMessage::Detach;
    let mut buf = Vec::new();
    write_message(&mut buf, &msg).unwrap();
    assert_eq!(buf, [1, 0, 0, 0, 4]);
    let back: ClientMessage = read_message(&mut buf.as_slice(), MAX_FRAME_SIZE).unwrap();
    assert_eq!(back, msg);

    let bad = [2u8, 0, 0, 0, 4, 0];
    let err = read_message::<_, ClientMessage>(&mut bad.as_slice(), MAX_FRAME_SIZE).unwrap_err();
    assert!(matches!(err, FramingError::Bincode(_)), "{err:?}");
}
