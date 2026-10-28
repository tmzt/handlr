//! Attach to the local herdr as a shell client, focus one pane at a fixed
//! surface size, and print the pane's rows and cursor on every surface
//! (plan 20261027_08, step 3's check, run by the owner):
//!
//! ```text
//! cargo run -p libherdr_term --example attach -- <pane_id> 80 24 [herdr-client.sock]
//! ```
//!
//! While it runs, herdr lays the pane's tab out at the given size (the
//! takeover, findings 2.1.4); Ctrl-C ends the process, herdr sees the
//! connection close and gives the tab back. The socket defaults to the
//! client socket beside `$HERDR_SOCKET_PATH`, else beside
//! `~/.config/herdr/herdr.sock`.

use std::path::PathBuf;

use libherdr_term::{AttachOptions, Attachment, Event, client_socket_for};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: attach <pane_id> <cols> <rows> [herdr-client.sock]");
        std::process::exit(2);
    }
    let pane_id = args[0].clone();
    let cols: u16 = args[1].parse().expect("cols");
    let rows: u16 = args[2].parse().expect("rows");
    let socket = args.get(3).map(PathBuf::from).unwrap_or_else(|| {
        let api = std::env::var("HERDR_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
                    .join(".config/herdr/herdr.sock")
            });
        client_socket_for(&api)
    });
    eprintln!(
        "attaching to {} (herdr protocol {} pinned, endpoint generation {})",
        socket.display(),
        libherdr_term::HERDR_PROTOCOL_VERSION,
        libherdr_term::ENDPOINT_PROTOCOL_GENERATION
    );
    let (attachment, events) = match Attachment::connect(&AttachOptions::new(socket, cols, rows)) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    attachment.focus_pane(&pane_id).expect("pane.focus");
    for event in events {
        match event {
            Event::Ready {
                boot_id,
                server_version,
            } => eprintln!("ready: herdr {server_version}, boot {boot_id}"),
            Event::Snapshot(s) => eprintln!(
                "snapshot {}: focused pane {:?}",
                s.revision, s.focused_pane_id
            ),
            Event::FocusAnswer {
                ok, code, message, ..
            } => eprintln!("pane.focus: ok={ok} {code:?} {message:?}"),
            Event::Surface(view) => {
                let Some(pane) = view.pane(&pane_id) else {
                    eprintln!(
                        "surface {}: {pane_id} not in the viewed tab",
                        view.surface_revision
                    );
                    continue;
                };
                let grid: libflankr_term::Grid = (&pane).into();
                println!(
                    "\x1b[2J\x1b[H-- surface {} {}x{}, pane {}x{}, pty rows {:?}, cursor {:?}",
                    view.surface_revision,
                    view.cols(),
                    view.rows(),
                    grid.cols,
                    grid.rows,
                    pane.pty_rows(),
                    grid.cursor
                );
                for line in grid.text_lines() {
                    println!("{line}");
                }
                if let Some(change) = view.desktop_change(&pane_id, cols, rows) {
                    println!("-- desktop change: {change:?}");
                }
            }
            Event::Closed { reason, .. } => {
                eprintln!("closed: {reason:?}");
                break;
            }
            other => eprintln!("{other:?}"),
        }
    }
    attachment.detach();
}
