//! `Attachment` against an in-process fake herdr speaking the extracted
//! types (plan 20261027_08, step 3). No real herdr socket is touched.

mod common;

use std::sync::mpsc::Receiver;
use std::time::Duration;

use common::{Fake, cell, fake_server, pane, rect, surface};
use libherdr_term::herdr::{self, ClientMessage, ServerMessage};
use libherdr_term::{AttachError, AttachOptions, Attachment, DesktopChange, Event};

const WAIT: Duration = Duration::from_secs(10);

fn next(rx: &Receiver<Event>) -> Event {
    rx.recv_timeout(WAIT).expect("an event")
}

/// The next event that is not a snapshot or mouse capture.
fn next_main(rx: &Receiver<Event>) -> Event {
    loop {
        match next(rx) {
            Event::Snapshot(_) | Event::MouseCapture { .. } => continue,
            other => return other,
        }
    }
}

type Connected = Result<(Attachment, Receiver<Event>), AttachError>;

fn attach(
    serve: impl FnOnce(Fake) + Send + 'static,
    opts: impl FnOnce(&std::path::Path) -> AttachOptions,
) -> (Connected, std::thread::JoinHandle<()>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (path, server) = fake_server(dir.path(), serve);
    (Attachment::connect(&opts(&path)), server, dir)
}

fn ready(rx: &Receiver<Event>) {
    match next(rx) {
        Event::Ready {
            boot_id,
            server_version,
        } => {
            assert_eq!(boot_id, "boot-1");
            assert_eq!(server_version, "0.9.5-fake");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
    match next(rx) {
        Event::Snapshot(s) => assert_eq!(s.focused_pane_id.as_deref(), Some("w1:p1")),
        other => panic!("expected the first snapshot, got {other:?}"),
    }
}

#[test]
fn the_hello_is_generation_one_with_the_four_codecs_and_no_optional_encodings() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (result, server, _dir) = attach(
        move |mut fake| {
            let hello = fake.handshake();
            tx.send(hello).unwrap();
            let _ = fake.try_read();
        },
        |p| AttachOptions {
            active: false,
            ..AttachOptions::new(p, 1000, 2)
        },
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    let hello = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(hello.generation, 1);
    assert_eq!(hello.snapshot_codecs, ["shell.snapshot.v1"]);
    assert_eq!(hello.surface_codecs, ["shell.surface.v1"]);
    assert_eq!(hello.input_codecs, ["shell.input.semantic.v1"]);
    assert_eq!(hello.blob_codecs, ["shell.blob.v1"]);
    assert!(!hello.surface_active);
    assert!(!hello.surface_reuse && !hello.surface_delta && !hello.surface_scroll);
    assert!(!hello.pixel_mouse && !hello.mouse_capture && !hello.direct_graphics);
    // Clamped to 20-500 by 5-200 (decision 6).
    assert_eq!(
        hello.surface_size,
        herdr::ClientSurfaceSize { cols: 500, rows: 5 }
    );
    assert_eq!(attachment.boot_id(), "boot-1");
    assert!(attachment.supports_surface_set());
    drop(attachment);
    server.join().unwrap();
}

#[test]
fn a_refused_welcome_is_incompatible_with_herdrs_code() {
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.read_hello();
            let welcome = herdr::EndpointServerWelcome {
                generation: 1,
                server_version: "0.9.9".into(),
                snapshot_codec: "shell.snapshot.v1".into(),
                surface_codec: "shell.surface.v1".into(),
                input_codec: "shell.input.semantic.v1".into(),
                blob_codec: "shell.blob.v1".into(),
                methods: Vec::new(),
                capabilities: Vec::new(),
                error: Some(herdr::EndpointHandshakeError {
                    code: "unsupported_generation".into(),
                    message: "generation 1 is retired".into(),
                }),
            };
            fake.send(&ServerMessage::EndpointControl {
                kind: herdr::ENDPOINT_WELCOME_KIND.into(),
                data: serde_json::to_string(&welcome).unwrap(),
            });
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    match result {
        Err(AttachError::Incompatible {
            code,
            message,
            server_version,
        }) => {
            assert_eq!(code, "unsupported_generation");
            assert_eq!(message, "generation 1 is retired");
            assert_eq!(server_version.as_deref(), Some("0.9.9"));
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
    server.join().unwrap();
}

#[test]
fn a_herdr_without_generation_one_is_incompatible() {
    // The connection ends after the hello (a herdr that cannot decode it).
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.read_hello();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    match result {
        Err(AttachError::Incompatible { code, .. }) => assert_eq!(code, "no_welcome"),
        other => panic!("expected Incompatible, got {other:?}"),
    }
    server.join().unwrap();

    // The direct-terminal welcome.
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.read_hello();
            fake.send(&ServerMessage::Welcome {
                version: 22,
                encoding: herdr::RenderEncoding::TerminalAnsi,
                error: Some("this client predates the stable endpoint protocol".into()),
            });
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    match result {
        Err(AttachError::Incompatible { code, message, .. }) => {
            assert_eq!(code, "direct_terminal_welcome");
            assert!(message.contains("predates"));
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
    server.join().unwrap();

    // Bytes that are not a ServerMessage.
    let (result, server, _dir) = attach(
        |mut fake| {
            use std::io::Write;
            fake.read_hello();
            fake.stream.write_all(&[2, 0, 0, 0, 0xFF, 0xFF]).unwrap();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    assert!(
        matches!(result, Err(AttachError::Incompatible { ref code, .. }) if code == "no_welcome"),
        "{result:?}"
    );
    server.join().unwrap();
}

fn patch(
    base: u64,
    revision: u64,
    rows: Vec<herdr::PaneSurfacePatchRow>,
    panes: Vec<herdr::PaneSurfacePane>,
    cursor: Option<herdr::CursorState>,
) -> ServerMessage {
    ServerMessage::PaneSurfacePatch(herdr::PaneSurfacePatch {
        boot_id: "boot-1".into(),
        projection_revision: 1,
        base_surface_revision: base,
        surface_revision: revision,
        rows,
        panes,
        cursor,
    })
}

fn cursor(x: u16, y: u16) -> Option<herdr::CursorState> {
    Some(herdr::CursorState {
        x,
        y,
        visible: true,
        shape: 2,
    })
}

#[test]
fn a_full_surface_then_two_patches_gives_the_panes_cells_and_cursor() {
    // A 10x4 surface split in two: w1:p1 on columns 0-4, w1:p2 on 5-9
    // with a one-row header above them.
    let panes = || {
        vec![
            pane("w1:p1", rect(0, 1, 5, 3), false),
            pane("w1:p2", rect(5, 1, 5, 3), true),
        ]
    };
    let (result, server, _dir) = attach(
        move |mut fake| {
            fake.handshake();
            fake.send(&ServerMessage::PaneSurface(surface(
                1,
                10,
                4,
                ".",
                panes(),
                cursor(6, 1),
            )));
            fake.send(&patch(
                1,
                2,
                vec![herdr::PaneSurfacePatchRow {
                    x: 5,
                    y: 2,
                    cells: vec![cell("a", 0, 0, 0), cell("b", 0, 0, 0)],
                }],
                Vec::new(),
                cursor(7, 2),
            ));
            let mut moved = pane("w1:p2", rect(5, 1, 5, 3), true);
            moved.mouse_reporting = true;
            fake.send(&patch(
                2,
                3,
                vec![herdr::PaneSurfacePatchRow {
                    x: 9,
                    y: 3,
                    cells: vec![cell("z", 0, 0, 0), cell("past the edge", 0, 0, 0)],
                }],
                vec![moved],
                cursor(9, 3),
            ));
            let _ = fake.try_read();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);

    let Event::Surface(full) = next_main(&events) else {
        panic!("expected the full surface")
    };
    assert_eq!(full.surface_revision, 1);
    let p2 = full.pane("w1:p2").unwrap();
    assert_eq!(p2.cells.len(), 3);
    assert!(p2.cells.iter().all(|row| row.len() == 5));
    assert_eq!(
        p2.cursor,
        Some(herdr::CursorState {
            x: 1,
            y: 0,
            visible: true,
            shape: 2
        })
    );
    assert_eq!(
        full.pane("w1:p1").unwrap().cursor,
        None,
        "the cursor is in p2"
    );

    let Event::Surface(first) = next_main(&events) else {
        panic!("expected the first patch")
    };
    assert_eq!(first.surface_revision, 2);
    let p2 = first.pane("w1:p2").unwrap();
    let text: Vec<String> = p2
        .cells
        .iter()
        .map(|row| row.iter().map(|c| c.symbol.as_str()).collect())
        .collect();
    assert_eq!(text, [".....", "ab...", "....."]);
    assert_eq!(p2.cursor.as_ref().map(|c| (c.x, c.y)), Some((2, 1)));
    // The earlier view the receiver held is unchanged.
    assert_eq!(full.pane("w1:p2").unwrap().cells[1][0].symbol, ".");

    let Event::Surface(second) = next_main(&events) else {
        panic!("expected the second patch")
    };
    assert_eq!(second.surface_revision, 3);
    let p2 = second.pane("w1:p2").unwrap();
    assert_eq!(p2.cells[2][4].symbol, "z");
    assert!(p2.pane.mouse_reporting, "the patch's pane metadata applied");
    assert_eq!(p2.cursor.as_ref().map(|c| (c.x, c.y)), Some((4, 2)));
    assert_eq!(
        second.frame.cells.len(),
        40,
        "cells past the edge are dropped"
    );
    assert!(second.pane("w1:p9").is_none());

    drop(attachment);
    server.join().unwrap();
}

#[test]
fn a_patch_on_the_wrong_base_closes_the_attachment() {
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.handshake();
            fake.send(&ServerMessage::PaneSurface(surface(
                5,
                20,
                5,
                " ",
                vec![pane("w1:p1", rect(0, 0, 20, 5), true)],
                None,
            )));
            fake.send(&patch(4, 6, Vec::new(), Vec::new(), None));
            let _ = fake.try_read();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    assert!(matches!(next_main(&events), Event::Surface(_)));
    match next_main(&events) {
        Event::Closed {
            error: Some(AttachError::Framing(e)),
            ..
        } => assert!(e.contains("revision 4") && e.contains("held 5"), "{e}"),
        other => panic!("expected Closed with Framing, got {other:?}"),
    }
    assert!(matches!(
        attachment.focus_pane("w1:p1"),
        Err(AttachError::Closed)
    ));
    drop(attachment);
    server.join().unwrap();
}

fn request(msg: ClientMessage) -> (String, serde_json::Value) {
    match msg {
        ClientMessage::ClientShellEndpointRequest { boot_id, request } => {
            (boot_id, serde_json::from_str(&request).unwrap())
        }
        other => panic!("expected an endpoint request, got {other:?}"),
    }
}

#[test]
fn focus_pane_sends_pane_focus_with_the_boot_id_one_at_a_time_and_reports_the_answer() {
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.handshake();
            // The first request; the second waits for its answer.
            let (boot, first) = request(fake.read());
            assert_eq!(boot, "boot-1");
            assert_eq!(first["method"], "pane.focus");
            assert_eq!(first["params"]["pane_id"], "w1:p2");
            let id = first["id"].as_str().unwrap().to_owned();
            // Busy once: the library resends the same request.
            fake.answer(
                "boot-1",
                &id,
                &format!(r#"{{"id":"{id}","error":{{"code":"endpoint_busy","message":"busy"}}}}"#),
            );
            let (_, again) = request(fake.read());
            assert_eq!(again, first);
            fake.answer(
                "boot-1",
                &id,
                &format!(r#"{{"id":"{id}","result":{{"type":"ok"}}}}"#),
            );
            let (_, second) = request(fake.read());
            assert_eq!(second["params"]["pane_id"], "w1:p3");
            let id = second["id"].as_str().unwrap().to_owned();
            fake.answer(
                "boot-1",
                &id,
                &format!(r#"{{"id":"{id}","error":{{"code":"pane_not_found","message":"no pane w1:p3"}}}}"#),
            );
            let _ = fake.try_read();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    attachment.focus_pane("w1:p2").unwrap();
    attachment.focus_pane("w1:p3").unwrap();
    assert_eq!(attachment.requested().unanswered, 2);
    match next_main(&events) {
        Event::FocusAnswer {
            pane_id, ok, code, ..
        } => {
            assert_eq!(pane_id, "w1:p2");
            assert!(ok);
            assert_eq!(code, None);
        }
        other => panic!("expected FocusAnswer, got {other:?}"),
    }
    match next_main(&events) {
        Event::FocusAnswer {
            pane_id,
            ok,
            code,
            message,
        } => {
            assert_eq!(pane_id, "w1:p3");
            assert!(!ok);
            assert_eq!(code.as_deref(), Some("pane_not_found"));
            assert_eq!(message.as_deref(), Some("no pane w1:p3"));
        }
        other => panic!("expected FocusAnswer, got {other:?}"),
    }
    let requested = attachment.requested();
    assert_eq!(requested.pane_id.as_deref(), Some("w1:p3"));
    assert_eq!(requested.unanswered, 0);
    drop(attachment);
    server.join().unwrap();
}

#[test]
fn set_active_sends_surface_set_and_is_a_no_op_on_a_herdr_without_it() {
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.handshake();
            let (_, req) = request(fake.read());
            assert_eq!(req["method"], "client_shell.surface.set");
            assert_eq!(req["params"]["active"], true);
            let id = req["id"].as_str().unwrap().to_owned();
            fake.answer(
                "boot-1",
                &id,
                &format!(r#"{{"id":"{id}","result":{{"active":true}}}}"#),
            );
            let _ = fake.try_read();
        },
        |p| AttachOptions {
            active: false,
            ..AttachOptions::new(p, 80, 24)
        },
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    assert!(!attachment.requested().active);
    assert!(attachment.set_active(true).unwrap());
    match next_main(&events) {
        Event::ActiveAnswer { active, ok, .. } => assert!(active && ok),
        other => panic!("expected ActiveAnswer, got {other:?}"),
    }
    assert!(attachment.requested().active);
    drop(attachment);
    server.join().unwrap();

    // herdr 2026-09-02..05 serves no client_shell.surface.set.
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.read_hello();
            fake.welcome(&["pane.focus"]);
            fake.snapshot("boot-1", Some("w1:p1"));
            // Nothing but the Detach follows.
            assert_eq!(fake.read(), ClientMessage::Detach);
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    assert!(!attachment.supports_surface_set());
    assert!(!attachment.set_active(false).unwrap());
    attachment.detach();
    server.join().unwrap();
}

#[test]
fn resize_input_and_the_health_ping() {
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.handshake();
            fake.send(&ServerMessage::EndpointControl {
                kind: herdr::HEALTH_PING_KIND.into(),
                data: "probe".into(),
            });
            assert_eq!(
                fake.read(),
                ClientMessage::EndpointControl {
                    kind: herdr::HEALTH_PONG_KIND.into(),
                    data: "probe".into(),
                }
            );
            assert_eq!(
                fake.read(),
                ClientMessage::ClientShellResize {
                    cell_width_px: 0,
                    cell_height_px: 0,
                    surface_size: herdr::ClientSurfaceSize {
                        cols: 20,
                        rows: 200
                    },
                    pixel_mouse: false,
                }
            );
            let mut total = 0;
            for expected in [4096, 4] {
                match fake.read() {
                    ClientMessage::ClientShellPaneInput { pane_id, events } => {
                        assert_eq!(pane_id, "w1:p1");
                        assert_eq!(events.len(), expected);
                        total += events.len();
                    }
                    other => panic!("expected pane input, got {other:?}"),
                }
            }
            assert_eq!(total, 4100);
            let _ = fake.try_read();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    // Wait for the pong to go out before the resize, so the order is fixed.
    std::thread::sleep(Duration::from_millis(200));
    attachment.resize(3, 900).unwrap();
    assert_eq!(
        (attachment.requested().cols, attachment.requested().rows),
        (20, 200)
    );
    let text = herdr::ClientPaneInputEvent::TextCommit("x".into());
    attachment.input("w1:p1", vec![text; 4100]).unwrap();
    drop(attachment);
    server.join().unwrap();
}

#[test]
fn detach_is_sent_on_drop_and_the_reader_ends() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (result, server, _dir) = attach(
        move |mut fake| {
            fake.handshake();
            tx.send(fake.read()).unwrap();
            // Then the socket closes.
            assert!(fake.try_read().is_err());
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    drop(attachment);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), ClientMessage::Detach);
    match next_main(&events) {
        Event::Closed { reason, error } => {
            assert!(reason.is_none() && error.is_none(), "{reason:?}");
        }
        other => panic!("expected Closed, got {other:?}"),
    }
    server.join().unwrap();
}

#[test]
fn server_shutdown_and_mouse_capture_are_reported() {
    let (result, server, _dir) = attach(
        |mut fake| {
            fake.handshake();
            fake.send(&ServerMessage::MouseCapture {
                enabled: true,
                sgr_pixels: false,
            });
            fake.send(&ServerMessage::ServerShutdown {
                reason: Some("restarting".into()),
            });
            let _ = fake.try_read();
        },
        |p| AttachOptions::new(p, 80, 24),
    );
    let (attachment, events) = result.expect("attach");
    ready(&events);
    assert!(matches!(
        next(&events),
        Event::MouseCapture { enabled: true }
    ));
    match next(&events) {
        Event::Closed { reason, error } => {
            assert_eq!(reason.as_deref(), Some("restarting"));
            assert!(error.is_none());
        }
        other => panic!("expected Closed, got {other:?}"),
    }
    drop(attachment);
    server.join().unwrap();
}

#[test]
fn a_change_made_on_the_desktop_is_detected() {
    let base = surface(
        1,
        40,
        10,
        " ",
        vec![
            pane("w1:p1", rect(0, 0, 20, 10), true),
            pane("w1:p2", rect(20, 0, 20, 10), false),
        ],
        None,
    );
    let view = libherdr_term::SurfaceView::from_frame(base.clone());
    assert_eq!(view.desktop_change("w1:p1", 40, 10), None);
    assert_eq!(
        view.desktop_change("w1:p1", 80, 24),
        Some(DesktopChange::SurfaceSize { cols: 40, rows: 10 })
    );
    assert_eq!(
        view.desktop_change("w1:p2", 40, 10),
        Some(DesktopChange::FocusMoved {
            focused: Some("w1:p1".into())
        })
    );
    assert_eq!(
        view.desktop_change("w1:p9", 40, 10),
        Some(DesktopChange::PaneNotShown)
    );

    // The desktop took the tab's geometry: the PTY has its rows.
    let mut taken = base;
    taken.panes[0].scroll.as_mut().unwrap().viewport_rows = 50;
    let view = libherdr_term::SurfaceView::from_frame(taken);
    assert_eq!(
        view.desktop_change("w1:p1", 40, 10),
        Some(DesktopChange::PaneRows {
            pty_rows: 50,
            shown_rows: 10
        })
    );
}

#[test]
fn the_client_socket_is_beside_the_api_socket() {
    assert_eq!(
        libherdr_term::client_socket_for(std::path::Path::new("/home/u/.config/herdr/herdr.sock")),
        std::path::PathBuf::from("/home/u/.config/herdr/herdr-client.sock")
    );
}
