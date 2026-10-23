//! The agent view's buttons as drawn: the square audio controls (play,
//! send, cancel, record) by glyph, and the rest with their labels.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use super::{
    AgentHit, CANCEL_GLYPH, CELL_EM, DIM, DOT, FG, FlowButton, PLAY_GLYPH, RECORDING, Row,
    SEND_GLYPH, SKIP_GLYPH, Terminal, button, draw_row, fill, term_h,
};

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

/// The state the buttons are drawn in.
#[derive(Clone, Copy, Debug, Default)]
pub struct Look {
    pub read_aloud: bool,
    pub speaking: bool,
    pub recording: bool,
    /// The button under a finger, drawn pressed in: 2 pt down and darker
    /// (owner, 2026-10-23: "a visual effect like pressing it should have
    /// like a depress effect").
    pub pressed: Option<AgentHit>,
}

/// How far a pressed button sinks, and how much darker it gets.
pub const DEPRESS: f32 = 2.0;
const DARKEN: f32 = 0.7;

/// `rect` and `bg` as drawn: moved down and darkened while pressed.
pub fn depressed(rect: Rect, bg: [f32; 4], pressed: bool, s: f32) -> (Rect, [f32; 4]) {
    if !pressed {
        return (rect, bg);
    }
    let rect = Rect::new(rect.x, rect.y + DEPRESS * s, rect.w, rect.h);
    (
        rect,
        [bg[0] * DARKEN, bg[1] * DARKEN, bg[2] * DARKEN, bg[3]],
    )
}

/// Draws `b` at `rect`: the take-a-message button as a white record dot on
/// blue, the play button as its glyph (bright when read aloud is on), and
/// the rest as labelled buttons; pressed in while a finger is on it.
pub struct SquareButton;

impl SquareButton {
    pub fn draw(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        look: &Look,
    ) {
        let Look {
            read_aloud,
            speaking,
            recording,
            pressed,
        } = *look;
        let (rect, bg) = depressed(rect, b.bg, pressed == Some(b.hit), s);
        let px = 26.0 * s;
        match b.hit {
            AgentHit::Message | AgentHit::Reply => {
                fill(list, rect, bg, 10.0 * s);
                let d = DOT * s;
                let dot = Rect::new(
                    rect.x + (rect.w - d) * 0.5,
                    rect.y + (rect.h - d) * 0.5,
                    d,
                    d,
                );
                fill(list, dot, if recording { RECORDING } else { FG }, d * 0.5);
            }
            AgentHit::ReadAloud => {
                fill(list, rect, bg, 10.0 * s);
                // Bright when on, dull when off; skip while a reply is read.
                let color = if read_aloud { FG } else { DIM };
                let (glyph, fallback) = if read_aloud && speaking {
                    (SKIP_GLYPH, ">|")
                } else {
                    (PLAY_GLYPH, ">")
                };
                draw_glyph(list, ctx, term, glyph, fallback, rect, px, color);
            }
            AgentHit::Submit if b.label.is_empty() => {
                fill(list, rect, bg, 10.0 * s);
                draw_glyph(list, ctx, term, SEND_GLYPH, ">", rect, px, FG);
            }
            AgentHit::Clear if b.label.is_empty() => {
                fill(list, rect, bg, 10.0 * s);
                draw_glyph(list, ctx, term, CANCEL_GLYPH, "x", rect, px, FG);
            }
            _ => button(list, ctx, b.label, rect, bg, s),
        }
    }
}
