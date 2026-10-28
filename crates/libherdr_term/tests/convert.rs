//! Decision 8's conversions (plan 20261027_08, step 4).

mod common;

use common::{pane, rect, surface};
use libflankr_proto::common::{
    MouseButton, TerminalMouseKind, TerminalMouseParams, TerminalScrollParams,
};
use libflankr_term::{Color, Cursor, Grid, Run, flags, parse_ansi};
use libherdr_term::herdr::{
    self, CellData, ClientKeyCode, ClientKeyKind, ClientMouseButton, ClientMouseKind,
    ClientMousePosition, ClientPaneInputEvent,
};
use libherdr_term::{
    Color32, KeyToken, Modifier16, SurfaceView, UnknownMouseKind, key_events, row_from_cells,
};

fn c(symbol: &str, fg: u32, bg: u32, modifier: u16) -> CellData {
    common::cell(symbol, fg, bg, modifier)
}

fn skip() -> CellData {
    CellData {
        skip: true,
        ..c(" ", 0, 0, 0)
    }
}

#[test]
fn every_named_palette_and_rgb_colour() {
    assert_eq!(Color::from(Color32(0)), Color::DEFAULT);
    // herdr's named colours 1..=16 are ANSI 0..=15 in order.
    for n in 1..=16u32 {
        assert_eq!(
            Color::from(Color32(n)),
            Color::palette((n - 1) as u8),
            "{n}"
        );
    }
    assert_eq!(Color::from(Color32(17)), Color::DEFAULT);
    for i in 0..=255u32 {
        assert_eq!(
            Color::from(Color32(0x0100_0000 | i)),
            Color::palette(i as u8)
        );
    }
    assert_eq!(Color::from(Color32(0x02FF_8000)), Color::rgb(255, 128, 0));
    assert_eq!(Color::from(Color32(0x0200_0000)), Color::rgb(0, 0, 0));
    assert_eq!(Color::from(Color32(0x03FF_FFFF)), Color::DEFAULT);
    assert_eq!(Color::from(Color32(0x0001_0002)), Color::DEFAULT);
}

#[test]
fn each_modifier_bit() {
    let cases = [
        (Modifier16::BOLD, flags::BOLD),
        (Modifier16::DIM, flags::DIM),
        (Modifier16::ITALIC, flags::ITALIC),
        (Modifier16::UNDERLINED, flags::UNDERLINE),
        (Modifier16::SLOW_BLINK, 0),
        (Modifier16::RAPID_BLINK, 0),
        (Modifier16::REVERSED, flags::INVERSE),
        (Modifier16::HIDDEN, 0),
        (Modifier16::CROSSED_OUT, flags::STRIKETHROUGH),
        // An underline style alone (curly, 3 << 12) underlines.
        (3 << 12, flags::UNDERLINE),
    ];
    for (bits, expected) in cases {
        assert_eq!(u8::from(Modifier16(bits)), expected, "{bits:#x}");
    }
    assert_eq!(
        u8::from(Modifier16(Modifier16::BOLD | Modifier16::REVERSED)),
        flags::BOLD | flags::INVERSE
    );
}

fn run(text: &str, fg: Color, bg: Color, flags: u8) -> Run {
    Run {
        text: text.into(),
        fg,
        bg,
        flags,
    }
}

#[test]
fn a_wide_glyphs_skip_cell_emits_nothing_and_styles_merge() {
    let row = row_from_cells(&[c("🦀", 0, 0, 0), skip(), c("a", 0, 0, 0), c("b", 2, 0, 1)]);
    assert_eq!(
        row,
        vec![
            run("🦀a", Color::DEFAULT, Color::DEFAULT, 0),
            run("b", Color::palette(1), Color::DEFAULT, flags::BOLD),
        ]
    );
}

#[test]
fn a_hidden_run_is_spaces() {
    let row = row_from_cells(&[
        c("p", 0, 0, Modifier16::HIDDEN),
        c("w", 0, 0, Modifier16::HIDDEN),
        c("!", 0, 0, 0),
    ]);
    assert_eq!(row, vec![run("  !", Color::DEFAULT, Color::DEFAULT, 0)]);
}

#[test]
fn trailing_blanks_are_cut_but_visible_ones_stay() {
    let row = row_from_cells(&[c("x", 0, 0, 0), c(" ", 0, 0, 0), c(" ", 0, 0, 0)]);
    assert_eq!(row, vec![run("x", Color::DEFAULT, Color::DEFAULT, 0)]);
    // A coloured background or an inverse space is drawn.
    let row = row_from_cells(&[c("x", 0, 0, 0), c(" ", 0, 0x0100_0004, 0)]);
    assert_eq!(row.len(), 2);
    let row = row_from_cells(&[c(" ", 0, 0, Modifier16::REVERSED)]);
    assert_eq!(
        row,
        vec![run(" ", Color::DEFAULT, Color::DEFAULT, flags::INVERSE)]
    );
    assert_eq!(row_from_cells(&[c(" ", 0, 0, 0)]), vec![]);
}

/// A row of cells equals what `parse_ansi` makes of the ANSI herdr's
/// `visible_ansi` gives for the same content (fixture written by hand):
/// `\x1b[1;31mHi\x1b[0m \x1b[38;5;208;48;2;1;2;3mok\x1b[0m\x1b[7m!\x1b[0m   `.
#[test]
fn a_row_equals_parse_ansi_of_the_same_content() {
    let ansi = "\x1b[1;31mHi\x1b[0m \x1b[38;5;208;48;2;1;2;3mok\x1b[0m\x1b[7m!\x1b[0m   \n";
    let expected = parse_ansi(ansi);
    let cells = [
        c("H", 2, 0, Modifier16::BOLD),
        c("i", 2, 0, Modifier16::BOLD),
        c(" ", 0, 0, 0),
        c("o", 0x0100_00D0, 0x0201_0203, 0),
        c("k", 0x0100_00D0, 0x0201_0203, 0),
        c("!", 0, 0, Modifier16::REVERSED),
        c(" ", 0, 0, 0),
        c(" ", 0, 0, 0),
        c(" ", 0, 0, 0),
    ];
    assert_eq!(vec![row_from_cells(&cells)], expected.lines);
}

#[test]
fn a_pane_becomes_a_grid_with_its_cursor_and_modes() {
    let mut p = pane("w1:p1", rect(2, 1, 4, 2), true);
    p.mouse_reporting = true;
    p.alternate_screen_active = true;
    let mut frame = surface(
        1,
        8,
        4,
        " ",
        vec![p],
        Some(herdr::CursorState {
            x: 3,
            y: 2,
            visible: true,
            shape: 6,
        }),
    );
    frame.frame.cells[8 + 2] = c("$", 0, 0, 0);
    let view = SurfaceView::from_frame(frame);
    let grid = Grid::from(view.pane("w1:p1").unwrap());
    assert_eq!((grid.cols, grid.rows), (4, 2));
    assert_eq!(grid.text_lines(), ["$", ""]);
    assert_eq!(
        grid.cursor,
        Some(Cursor {
            col: 1,
            row: 1,
            visible: true,
            shape: 6,
            mouse_reporting: true,
            alternate_screen: true,
            stale: false,
        })
    );

    // The cursor outside the pane: none.
    let mut frame = surface(
        1,
        8,
        4,
        " ",
        vec![pane("w1:p1", rect(2, 1, 4, 2), true)],
        None,
    );
    frame.frame.cursor = Some(herdr::CursorState {
        x: 0,
        y: 0,
        visible: true,
        shape: 0,
    });
    let view = SurfaceView::from_frame(frame);
    assert_eq!(Grid::from(view.pane("w1:p1").unwrap()).cursor, None);
}

#[test]
fn a_cursor_state_becomes_a_cursor() {
    let cursor = Cursor::from(herdr::CursorState {
        x: 7,
        y: 3,
        visible: false,
        shape: 4,
    });
    assert_eq!(
        cursor,
        Cursor {
            col: 7,
            row: 3,
            visible: false,
            shape: 4,
            ..Cursor::default()
        }
    );
}

fn mouse(kind: TerminalMouseKind) -> TerminalMouseParams {
    TerminalMouseParams {
        terminal_id: 1,
        kind,
        col: 4,
        row: 2,
        modifiers: 2,
    }
}

#[test]
fn every_mouse_kind() {
    let cases = [
        (
            TerminalMouseKind::Down {
                button: MouseButton::Left,
            },
            ClientMouseKind::Down(ClientMouseButton::Left),
        ),
        (
            TerminalMouseKind::Up {
                button: MouseButton::Right,
            },
            ClientMouseKind::Up(ClientMouseButton::Right),
        ),
        (
            TerminalMouseKind::Drag {
                button: MouseButton::Middle,
            },
            ClientMouseKind::Drag(ClientMouseButton::Middle),
        ),
        (TerminalMouseKind::Moved, ClientMouseKind::Moved),
    ];
    for (kind, expected) in cases {
        let event = ClientPaneInputEvent::try_from(&mouse(kind)).unwrap();
        assert_eq!(
            event,
            ClientPaneInputEvent::Mouse {
                kind: expected,
                position: ClientMousePosition::Cell { column: 4, row: 2 },
                geometry: None,
                modifiers: 2,
                lines: 1,
            }
        );
    }
    assert_eq!(
        ClientPaneInputEvent::try_from(&mouse(TerminalMouseKind::Unknown)),
        Err(UnknownMouseKind)
    );
}

#[test]
fn both_wheel_directions() {
    let wheel = |lines| {
        ClientPaneInputEvent::from(TerminalScrollParams {
            terminal_id: 1,
            lines,
        })
    };
    let expect = |kind, lines| ClientPaneInputEvent::Mouse {
        kind,
        position: ClientMousePosition::Cell { column: 0, row: 0 },
        geometry: None,
        modifiers: 0,
        lines,
    };
    assert_eq!(wheel(3), expect(ClientMouseKind::ScrollUp, 3));
    assert_eq!(wheel(-5), expect(ClientMouseKind::ScrollDown, 5));
    assert_eq!(wheel(0), expect(ClientMouseKind::ScrollDown, 1));
    assert_eq!(wheel(i32::MAX), expect(ClientMouseKind::ScrollUp, 1000));
    assert_eq!(wheel(i32::MIN), expect(ClientMouseKind::ScrollDown, 1000));
}

fn press_release(code: ClientKeyCode, modifiers: u8) -> Vec<ClientPaneInputEvent> {
    [ClientKeyKind::Press, ClientKeyKind::Release]
        .into_iter()
        .map(|kind| ClientPaneInputEvent::Key {
            code: code.clone(),
            modifiers,
            kind,
            repeat_count: 1,
            shifted_codepoint: None,
            generated_text: None,
            tracks_release: true,
            physical_key_id: None,
            windows_record: None,
        })
        .collect()
}

#[test]
fn each_named_key_token() {
    let named = [
        ("enter", ClientKeyCode::Enter),
        ("escape", ClientKeyCode::Esc),
        ("esc", ClientKeyCode::Esc),
        ("tab", ClientKeyCode::Tab),
        ("backspace", ClientKeyCode::Backspace),
        ("up", ClientKeyCode::Up),
        ("down", ClientKeyCode::Down),
        ("left", ClientKeyCode::Left),
        ("right", ClientKeyCode::Right),
    ];
    for (token, code) in named {
        let parsed = KeyToken::parse(token).unwrap();
        assert_eq!(parsed, KeyToken::Named(code.clone()));
        assert_eq!(
            Vec::<ClientPaneInputEvent>::from(parsed),
            press_release(code, 0),
            "{token}"
        );
    }
    assert_eq!(
        Vec::<ClientPaneInputEvent>::from(KeyToken::parse("space").unwrap()),
        vec![ClientPaneInputEvent::TextCommit(" ".into())]
    );
    for (token, ch) in [
        ("ctrl+a", 'a'),
        ("ctrl+c", 'c'),
        ("ctrl+z", 'z'),
        ("ctrl+\\", '\\'),
    ] {
        assert_eq!(
            Vec::<ClientPaneInputEvent>::from(KeyToken::parse(token).unwrap()),
            press_release(ClientKeyCode::Char(ch), 2),
            "{token}"
        );
    }
    assert_eq!(
        Vec::<ClientPaneInputEvent>::from(KeyToken::parse("é").unwrap()),
        vec![ClientPaneInputEvent::TextCommit("é".into())]
    );
    for bad in ["", "ctrl+1", "ctrl+enter", "\u{1b}"] {
        assert_eq!(KeyToken::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn parse_keys_tokens_become_events_with_text_merged() {
    // What the daemon's parse_keys makes of "git space status enter".
    let tokens = ["g", "i", "t", "space", "s", "t", "enter", "ctrl+c"];
    let mut expected = vec![ClientPaneInputEvent::TextCommit("git st".into())];
    expected.extend(press_release(ClientKeyCode::Enter, 0));
    expected.extend(press_release(ClientKeyCode::Char('c'), 2));
    assert_eq!(key_events(&tokens).unwrap(), expected);
    assert_eq!(key_events(&["a", "ctrl+1"]), Err("ctrl+1".to_string()));
}
