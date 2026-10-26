//! The bottom row's buttons and the agent view's as drawn: the square
//! audio controls (play, send, cancel, record) by glyph, and the rest with
//! their labels.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{
    AgentHit, CANCEL_GLYPH, CELL_EM, DOT, FlowButton, GEMINI_GLYPH, PLAY_GLYPH, Row, SEND_GLYPH,
    SKIP_GLYPH, Terminal, button, draw_row, fill, term_h,
};
use crate::settings;
use crate::theme::theme;

/// `glyph` centred in `rect` at `px`, from the terminal face; `fallback`
/// in the context's font without it.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    glyph: &str,
    fallback: &str,
    rect: Rect,
    px: f32,
    color: [f32; 4],
) {
    let row = Row {
        text: if term.is_some() { glyph } else { fallback }.to_string(),
        x: 0.0,
        y: 0.0,
        px,
        color,
        mono: true,
    };
    let pos = [
        rect.x + (rect.w - CELL_EM * px * row.text.chars().count() as f32) * 0.5,
        rect.y + (rect.h - term_h(px)) * 0.5,
    ];
    draw_row(list, ctx, term, &row, pos);
}

/// The general agent button in active voice recognition mode:
/// fixed outer button size, inner pulsing glow ring, and glyph alternating
/// between the Gemini star and dingbats symbols.
fn draw_general_agent_listening(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    rect: Rect,
    bg: [f32; 4],
    s: f32,
) {
    // Outer button keeps the exact same size.
    fill(list, rect, bg, 10.0 * s);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let millis = now.as_millis();

    // 1. Pulsing / growing and shrinking inner ring (oscillates with ~1200ms period).
    let phase = ((millis % 1200) as f32 / 1200.0 * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let ring_pad = (2.0 + (1.0 - phase) * 6.0) * s;
    let ring_rect = Rect::new(
        rect.x + ring_pad,
        rect.y + ring_pad,
        rect.w - 2.0 * ring_pad,
        rect.h - 2.0 * ring_pad,
    );
    let glow_alpha = 0.20 + 0.35 * phase;
    fill(
        list,
        ring_rect,
        [1.0, 1.0, 1.0, glow_alpha],
        (8.0 - phase * 2.0) * s,
    );

    // 2. Glyph alternates with dingbats symbols across animation steps (~300ms each).
    const DINGBATS: &[(&str, &str)] = &[
        (GEMINI_GLYPH, "*"), // ✦ (Black four-pointed star)
        ("\u{2727}", "*"),   // ✧ (White four-pointed star)
        ("\u{2736}", "*"),   // ✶ (Six-pointed black star)
        ("\u{273B}", "*"),   // ✻ (Teardrop-spoked asterisk)
    ];
    let step = ((millis / 300) % (DINGBATS.len() as u128)) as usize;
    let (glyph, fallback) = DINGBATS[step];

    // Inner glyph gently grows and shrinks in sync with the pulse.
    let px = (21.0 + phase * 6.0) * s;
    draw_glyph(list, ctx, term, glyph, fallback, rect, px, theme().fg);
}

/// A keyboard drawn as strokes in `rect`, with no background: an outline
/// and three rows of keys, the last a space bar. The faces have no
/// keyboard character.
fn keyboard_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let w = rect.w * 0.62;
    let h = w * 0.62;
    let x = rect.x + (rect.w - w) * 0.5;
    let y = rect.y + (rect.h - h) * 0.5;
    let t = 1.8 * s;
    for (a, b) in [
        ([x, y], [x + w, y]),
        ([x + w, y], [x + w, y + h]),
        ([x + w, y + h], [x, y + h]),
        ([x, y + h], [x, y]),
    ] {
        list.push_line(a, b, t, color);
    }
    let key = w * 0.09;
    let cols = 6;
    let step = (w - 2.0 * key) / cols as f32;
    for row in 0..2 {
        let ky = y + h * (0.22 + 0.24 * row as f32);
        for c in 0..cols {
            let kx = x + key + step * c as f32 + (step - key) * 0.5;
            fill(list, Rect::new(kx, ky, key, key), color, key * 0.25);
        }
    }
    let bar = Rect::new(x + w * 0.28, y + h * 0.72, w * 0.44, key);
    fill(list, bar, color, key * 0.25);
}

/// Draws `b` at `rect`: the take-a-message button as a white record dot on
/// blue, the play button as its glyph (bright when `read_aloud`), and the
/// rest as labelled buttons. A button the settings turn off (plan
/// 20261024_02, decision 4) is drawn at 35%, showing nothing live.
pub struct SquareButton;

impl SquareButton {
    /// Draws `b`, live or not as [`settings::allows`] says.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        read_aloud: bool,
        speaking: bool,
        recording: bool,
    ) {
        let enabled = settings::allows(b.hit);
        Self::draw_enabled(
            list, ctx, term, b, rect, s, read_aloud, speaking, recording, enabled,
        );
    }

    /// Draws `b`, dim and showing nothing live unless `enabled`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_enabled(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        read_aloud: bool,
        speaking: bool,
        recording: bool,
        enabled: bool,
    ) {
        let start = list.instances.len();
        let (read_aloud, speaking, recording) = if enabled {
            (read_aloud, speaking, recording)
        } else {
            (false, false, false)
        };
        Self::draw_live(list, ctx, term, b, rect, s, read_aloud, speaking, recording);
        if !enabled {
            settings::dim_since(list, start);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_live(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        read_aloud: bool,
        speaking: bool,
        recording: bool,
    ) {
        let px = 26.0 * s;
        match b.hit {
            AgentHit::Keyboard => keyboard_glyph(list, rect, s, theme().fg),
            AgentHit::Message | AgentHit::Reply => {
                fill(list, rect, b.bg, 10.0 * s);
                let d = DOT * s;
                let dot = Rect::new(
                    rect.x + (rect.w - d) * 0.5,
                    rect.y + (rect.h - d) * 0.5,
                    d,
                    d,
                );
                fill(
                    list,
                    dot,
                    if recording {
                        theme().recording
                    } else {
                        theme().fg
                    },
                    d * 0.5,
                );
            }
            AgentHit::ReadAloud => {
                fill(list, rect, b.bg, 10.0 * s);
                // Bright when on, dull when off; skip while a reply is read.
                let color = if read_aloud { theme().fg } else { theme().dim };
                let (glyph, fallback) = if read_aloud && speaking {
                    (SKIP_GLYPH, ">|")
                } else {
                    (PLAY_GLYPH, ">")
                };
                draw_glyph(list, ctx, term, glyph, fallback, rect, px, color);
            }
            // A summary card's replay: the play glyph alone, dim.
            AgentHit::Replay(_) => {
                draw_glyph(
                    list,
                    ctx,
                    term,
                    PLAY_GLYPH,
                    ">",
                    rect,
                    18.0 * s,
                    theme().dim,
                );
            }
            AgentHit::Submit if b.label.is_empty() => {
                fill(list, rect, b.bg, 10.0 * s);
                draw_glyph(list, ctx, term, SEND_GLYPH, ">", rect, px, theme().fg);
            }
            AgentHit::Clear if b.label.is_empty() => {
                fill(list, rect, b.bg, 10.0 * s);
                draw_glyph(list, ctx, term, CANCEL_GLYPH, "x", rect, px, theme().fg);
            }
            AgentHit::GeneralAgent if b.label == "listening" => {
                draw_general_agent_listening(list, ctx, term, rect, b.bg, s);
            }
            AgentHit::GeneralAgent if b.label.is_empty() => {
                fill(list, rect, b.bg, 10.0 * s);
                let px = 24.0 * s;
                draw_glyph(list, ctx, term, GEMINI_GLYPH, "*", rect, px, theme().accent);
            }
            // A plan question's words are plain text in their colour (the
            // colour stands in `bg`), not buttons.
            AgentHit::Suggest(
                flankr_companion::Suggestion::Approve
                | flankr_companion::Suggestion::Discuss
                | flankr_companion::Suggestion::Implement,
            ) => {
                let px = 19.0 * s;
                let y = rect.y + (rect.h - crate::agent::line_h(px)) * 0.5;
                ctx.draw_text(list, b.label, [rect.x + 4.0 * s, y], px, b.bg);
            }
            _ => button(list, ctx, b.label, rect, b.bg, s),
        }
    }
}
