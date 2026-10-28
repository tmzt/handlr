//! The ANSI normaliser: herdr's `format: "ansi"` text to a [`Grid`].

use crate::{Color, Grid, Row, Run, flags};

/// The most rows [`parse_ansi`] keeps; rows past it are cut.
pub const MAX_ROWS: usize = 1000;
/// The most columns [`parse_ansi`] keeps in a row; characters past it are
/// cut.
pub const MAX_COLS: usize = 1000;

const TAB_STOP: usize = 8;

/// Parse a terminal frame rendered with SGR escapes into a [`Grid`].
///
/// * Rows are split on `\n`; `\r` is dropped. A final `\n` does not add a
///   row, but empty rows before it are kept as herdr gives them.
/// * SGR (`ESC [ … m`) sets the style: 0 reset; 1 bold, 2 dim, 3 italic,
///   4 underline (`4:0` none, `4:n` any other style), 7 inverse,
///   9 strikethrough, and their resets 22 (bold and dim), 23, 24, 27, 29;
///   foreground 30-37, 90-97, 39 default, `38;5;n`, `38;2;r;g;b`;
///   background 40-47, 100-107, 49 default, `48;5;n`, `48;2;r;g;b`; and
///   the colon forms (`38:5:n`, `38:2:r:g:b`, `38:2:id:r:g:b`). Other
///   codes are ignored. The style carries across rows.
/// * Every other escape (CSI, OSC ended by BEL or ST, DCS/SOS/PM/APC
///   strings, two- and three-byte ESC sequences) and every other C0 or C1
///   control is dropped. A `\n` inside an unfinished escape ends it, so a
///   broken escape cannot swallow rows.
/// * Tabs expand to spaces up to the next 8-column stop, in the current
///   style.
/// * Trailing blank spaces are cut: spaces with the default background and
///   no underline, inverse or strikethrough, which draw nothing.
/// * Each `char` counts as one column. Wide (CJK, emoji) and zero-width
///   (combining) characters are not measured; a row holding them is
///   narrower or wider on screen than its count.
/// * At most [`MAX_ROWS`] rows of [`MAX_COLS`] columns are kept.
///
/// `cols` is the widest row, `rows` the number of rows.
pub fn parse_ansi(text: &str) -> Grid {
    let mut parser = Parser::default();
    for c in text.chars() {
        parser.feed(c);
        if parser.lines.len() >= MAX_ROWS {
            break;
        }
    }
    // A final `\n` (or an empty input) leaves an empty, unstarted row.
    if !(text.is_empty() || text.ends_with('\n')) && parser.lines.len() < MAX_ROWS {
        parser.end_line();
    }
    Grid::from_lines(parser.lines)
}

#[derive(Default)]
enum State {
    #[default]
    Ground,
    /// After ESC.
    Escape,
    /// After ESC and an intermediate byte, waiting for the final byte.
    EscapeIntermediate,
    /// After `ESC [`: parameter and intermediate bytes so far, and whether
    /// any byte disqualifies it as SGR (a private marker or intermediate).
    Csi { params: String, plain: bool },
    /// An OSC, DCS, SOS, PM or APC string, until BEL or ST.
    String,
    /// ESC seen inside a string: `\` ends it.
    StringEscape,
}

#[derive(Default)]
struct Parser {
    state: State,
    fg: Color,
    bg: Color,
    flags: u8,
    line: Row,
    col: usize,
    lines: Vec<Row>,
}

impl Parser {
    fn feed(&mut self, c: char) {
        match std::mem::take(&mut self.state) {
            State::Ground => self.ground(c),
            State::Escape => match c {
                '[' => {
                    self.state = State::Csi {
                        params: String::new(),
                        plain: true,
                    }
                }
                ']' | 'P' | 'X' | '^' | '_' => self.state = State::String,
                '\x1b' => self.state = State::Escape,
                ' '..='/' => self.state = State::EscapeIntermediate,
                '0'..='~' => {}
                _ => self.ground(c),
            },
            State::EscapeIntermediate => match c {
                ' '..='/' => self.state = State::EscapeIntermediate,
                '0'..='~' => {}
                _ => self.ground(c),
            },
            State::Csi { mut params, plain } => match c {
                '0'..='9' | ';' | ':' => {
                    params.push(c);
                    self.state = State::Csi { params, plain };
                }
                // Private markers and intermediates: not SGR.
                '<'..='?' | ' '..='/' => {
                    self.state = State::Csi {
                        params,
                        plain: false,
                    }
                }
                'm' if plain => self.sgr(&params),
                '@'..='~' => {}
                _ => self.ground(c),
            },
            State::String => match c {
                '\x07' => {}
                '\x1b' => self.state = State::StringEscape,
                '\n' => self.ground(c),
                _ => self.state = State::String,
            },
            State::StringEscape => match c {
                '\\' => {}
                '\x1b' => self.state = State::StringEscape,
                '\n' => self.ground(c),
                _ => self.state = State::String,
            },
        }
    }

    fn ground(&mut self, c: char) {
        match c {
            '\n' => self.end_line(),
            '\t' => {
                let next = (self.col / TAB_STOP + 1) * TAB_STOP;
                for _ in self.col..next {
                    self.put(' ');
                }
            }
            '\x1b' => self.state = State::Escape,
            c if c.is_control() => {}
            c => self.put(c),
        }
    }

    fn put(&mut self, c: char) {
        if self.col >= MAX_COLS {
            return;
        }
        self.col += 1;
        if let Some(last) = self.line.last_mut()
            && last.fg == self.fg
            && last.bg == self.bg
            && last.flags == self.flags
        {
            last.text.push(c);
            return;
        }
        self.line.push(Run {
            text: c.to_string(),
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        });
    }

    fn end_line(&mut self) {
        let mut line = std::mem::take(&mut self.line);
        trim_blank_end(&mut line);
        self.lines.push(line);
        self.col = 0;
    }

    fn sgr(&mut self, params: &str) {
        let groups: Vec<Vec<Option<u32>>> = params
            .split(';')
            .map(|group| group.split(':').map(number).collect())
            .collect();
        let mut i = 0;
        while i < groups.len() {
            let group = &groups[i];
            i += 1;
            let code = group[0].unwrap_or(0);
            if group.len() > 1 {
                // A colon form: everything this code needs is in its group.
                match code {
                    4 => self.set(flags::UNDERLINE, group[1].unwrap_or(0) != 0),
                    38 | 48 | 58 => {
                        let color = colon_color(&group[1..]);
                        self.set_color(code, color);
                    }
                    _ => self.simple(code),
                }
                continue;
            }
            match code {
                38 | 48 | 58 => {
                    let rest: Vec<Option<u32>> = groups[i..]
                        .iter()
                        .map(|g| if g.len() == 1 { g[0] } else { None })
                        .collect();
                    let (color, used) = semicolon_color(&rest);
                    i += used;
                    self.set_color(code, color);
                }
                code => self.simple(code),
            }
        }
    }

    fn set_color(&mut self, code: u32, color: Option<Color>) {
        match (code, color) {
            (38, Some(color)) => self.fg = color,
            (48, Some(color)) => self.bg = color,
            // 58 is the underline colour, not drawn.
            _ => {}
        }
    }

    fn simple(&mut self, code: u32) {
        match code {
            0 => {
                self.fg = Color::DEFAULT;
                self.bg = Color::DEFAULT;
                self.flags = 0;
            }
            1 => self.set(flags::BOLD, true),
            2 => self.set(flags::DIM, true),
            3 => self.set(flags::ITALIC, true),
            4 | 21 => self.set(flags::UNDERLINE, true),
            7 => self.set(flags::INVERSE, true),
            9 => self.set(flags::STRIKETHROUGH, true),
            22 => self.set(flags::BOLD | flags::DIM, false),
            23 => self.set(flags::ITALIC, false),
            24 => self.set(flags::UNDERLINE, false),
            27 => self.set(flags::INVERSE, false),
            29 => self.set(flags::STRIKETHROUGH, false),
            30..=37 => self.fg = Color::palette((code - 30) as u8),
            39 => self.fg = Color::DEFAULT,
            40..=47 => self.bg = Color::palette((code - 40) as u8),
            49 => self.bg = Color::DEFAULT,
            90..=97 => self.fg = Color::palette((code - 90 + 8) as u8),
            100..=107 => self.bg = Color::palette((code - 100 + 8) as u8),
            _ => {}
        }
    }

    fn set(&mut self, bits: u8, on: bool) {
        if on {
            self.flags |= bits;
        } else {
            self.flags &= !bits;
        }
    }
}

/// A parameter: empty is `None` (read as 0 where a default applies);
/// values past `u32` saturate.
fn number(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    Some(s.bytes().fold(0u32, |n, b| {
        n.saturating_mul(10).saturating_add(u32::from(b - b'0'))
    }))
}

fn byte(n: Option<u32>) -> Option<u8> {
    u8::try_from(n.unwrap_or(0)).ok()
}

/// `5:n`, `2:r:g:b` or `2:id:r:g:b` after a colon-form 38/48/58.
fn colon_color(sub: &[Option<u32>]) -> Option<Color> {
    match sub.first().copied().flatten() {
        Some(5) => Some(Color::palette(byte(*sub.get(1)?)?)),
        Some(2) => {
            let rgb = if sub.len() >= 5 {
                &sub[2..5]
            } else {
                &sub[1..]
            };
            if rgb.len() < 3 {
                return None;
            }
            Some(Color::rgb(byte(rgb[0])?, byte(rgb[1])?, byte(rgb[2])?))
        }
        _ => None,
    }
}

/// `5;n` or `2;r;g;b` after a semicolon-form 38/48/58: the colour and how
/// many groups it used.
fn semicolon_color(rest: &[Option<u32>]) -> (Option<Color>, usize) {
    match rest.first().copied().flatten() {
        Some(5) if rest.len() >= 2 => (byte(rest[1]).map(Color::palette), 2),
        Some(2) if rest.len() >= 4 => {
            let color = match (byte(rest[1]), byte(rest[2]), byte(rest[3])) {
                (Some(r), Some(g), Some(b)) => Some(Color::rgb(r, g, b)),
                _ => None,
            };
            (color, 4)
        }
        // Malformed: drop the rest of the sequence, as xterm does.
        _ => (None, rest.len()),
    }
}

/// True when a space in `run`'s style draws nothing.
fn blank_style(run: &Run) -> bool {
    run.bg.is_default()
        && run.flags & (flags::UNDERLINE | flags::INVERSE | flags::STRIKETHROUGH) == 0
}

fn trim_blank_end(line: &mut Row) {
    while let Some(last) = line.last_mut() {
        if !blank_style(last) {
            break;
        }
        let kept = last.text.trim_end_matches(' ').len();
        last.text.truncate(kept);
        if !last.text.is_empty() {
            break;
        }
        line.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row_width;

    fn run(text: &str, fg: Color, bg: Color, flags: u8) -> Run {
        Run {
            text: text.into(),
            fg,
            bg,
            flags,
        }
    }

    fn plain(text: &str) -> Run {
        run(text, Color::DEFAULT, Color::DEFAULT, 0)
    }

    #[test]
    fn rows_split_trimmed_and_sized() {
        let g = parse_ansi("ab  \r\n\nlonger row   \n");
        assert_eq!(g.rows, 3);
        assert_eq!(g.cols, 10);
        assert_eq!(
            g.lines,
            vec![vec![plain("ab")], vec![], vec![plain("longer row")]]
        );
        assert_eq!(parse_ansi(""), Grid::default());
        assert_eq!(parse_ansi("x").rows, 1);
        assert_eq!(parse_ansi("x\n\n\n").rows, 3);
    }

    #[test]
    fn rows_and_columns_are_cut_at_the_limits() {
        let wide = "x".repeat(MAX_COLS + 50);
        let g = parse_ansi(&wide);
        assert_eq!(g.cols as usize, MAX_COLS);
        assert_eq!(row_width(&g.lines[0]), MAX_COLS);
        let tall = "y\n".repeat(MAX_ROWS + 50);
        let g = parse_ansi(&tall);
        assert_eq!(g.rows as usize, MAX_ROWS);
        assert_eq!(g.lines.len(), MAX_ROWS);
    }

    #[test]
    fn tabs_expand_to_stops() {
        let g = parse_ansi("a\tb\n12345678\tc\n\tx");
        assert_eq!(
            g.text_lines(),
            ["a       b", "12345678        c", "        x"]
        );
    }

    #[test]
    fn sgr_attributes_and_resets() {
        let g = parse_ansi("\x1b[1ma\x1b[2mb\x1b[22;3mc\x1b[4;7;9md\x1b[23;24;27;29me\x1b[0mf");
        let b = flags::BOLD;
        assert_eq!(
            g.lines[0],
            vec![
                run("a", Color::DEFAULT, Color::DEFAULT, b),
                run("b", Color::DEFAULT, Color::DEFAULT, b | flags::DIM),
                run("c", Color::DEFAULT, Color::DEFAULT, flags::ITALIC),
                run(
                    "d",
                    Color::DEFAULT,
                    Color::DEFAULT,
                    flags::ITALIC | flags::UNDERLINE | flags::INVERSE | flags::STRIKETHROUGH
                ),
                plain("ef"),
            ]
        );
        // ESC [ m is a reset; 4:0 clears an underline, 4:3 sets one.
        let g = parse_ansi("\x1b[1;4:3mx\x1b[4:0my\x1b[mz");
        assert_eq!(
            g.lines[0],
            vec![
                run("x", Color::DEFAULT, Color::DEFAULT, b | flags::UNDERLINE),
                run("y", Color::DEFAULT, Color::DEFAULT, b),
                plain("z"),
            ]
        );
    }

    #[test]
    fn sgr_colours_in_every_form() {
        let d = Color::DEFAULT;
        let cases: &[(&str, Color, Color)] = &[
            ("\x1b[31m", Color::palette(1), d),
            ("\x1b[97m", Color::palette(15), d),
            ("\x1b[42m", d, Color::palette(2)),
            ("\x1b[104m", d, Color::palette(12)),
            ("\x1b[38;5;208m", Color::palette(208), d),
            ("\x1b[48;5;17m", d, Color::palette(17)),
            ("\x1b[38;2;1;2;3m", Color::rgb(1, 2, 3), d),
            ("\x1b[48;2;250;128;0m", d, Color::rgb(250, 128, 0)),
            ("\x1b[38:5:99m", Color::palette(99), d),
            ("\x1b[48:2:10:20:30m", d, Color::rgb(10, 20, 30)),
            ("\x1b[38:2::10:20:30m", Color::rgb(10, 20, 30), d),
            ("\x1b[38:2:0:10:20:30m", Color::rgb(10, 20, 30), d),
            ("\x1b[31;39m", d, d),
            ("\x1b[41;49m", d, d),
            ("\x1b[38;5;300m", d, d),
            ("\x1b[58;5;3;31m", Color::palette(1), d),
            (
                "\x1b[38;2;1;2;3;48;5;4m",
                Color::rgb(1, 2, 3),
                Color::palette(4),
            ),
        ];
        for (seq, fg, bg) in cases {
            let g = parse_ansi(&format!("{seq}x"));
            assert_eq!(g.lines[0], vec![run("x", *fg, *bg, 0)], "{seq:?}");
        }
    }

    #[test]
    fn style_carries_across_rows_and_runs_merge() {
        let g = parse_ansi("\x1b[32mab\x1b[32mcd\nef\x1b[0m");
        let green = Color::palette(2);
        assert_eq!(g.lines[0], vec![run("abcd", green, Color::DEFAULT, 0)]);
        assert_eq!(g.lines[1], vec![run("ef", green, Color::DEFAULT, 0)]);
    }

    #[test]
    fn styled_trailing_spaces_are_kept_only_when_visible() {
        let g = parse_ansi("a\x1b[1m   \n\x1b[0mb\x1b[41m  \x1b[0m  ");
        assert_eq!(g.lines[0], vec![plain("a")]);
        assert_eq!(
            g.lines[1],
            vec![plain("b"), run("  ", Color::DEFAULT, Color::palette(1), 0)]
        );
        assert_eq!(g.cols, 3);
    }

    #[test]
    fn other_escapes_and_controls_are_dropped() {
        let text = concat!(
            "\x1b[2J\x1b[H\x1b[?25l",     // CSI: clear, home, private mode
            "\x1b]0;title\x07",           // OSC ended by BEL
            "\x1b]8;;http://x\x1b\\link", // OSC ended by ST
            "\x1b]8;;\x1b\\",
            "\x1bPdcs\x1b\\",     // DCS
            "\x1b(B\x1b=",        // ESC sequences
            "\x08\x00\x7f\u{9b}", // C0, DEL and C1 controls
            "\x1b[>4;1m",         // a private-marker `m`: not SGR
            "ok\x1b[1 q"          // CSI with an intermediate
        );
        let g = parse_ansi(text);
        assert_eq!(g.lines, vec![vec![plain("linkok")]]);
    }

    #[test]
    fn a_broken_escape_ends_at_the_row() {
        let g = parse_ansi("a\x1b]unterminated\nb\x1b[12\nc");
        assert_eq!(g.text_lines(), ["a", "b", "c"]);
    }
}
