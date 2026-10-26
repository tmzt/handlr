//! The bottom row, which controls the audio, on the agent view and the
//! overview (owner,
//! 2026-10-23: "the bottom row to control the audio, with the left having
//! a square play button, the right a square record button … with an X
//! button in the middle to cancel"; "play doesn't become send, record
//! becomes send").

use libcontrols::Rect;

use crate::agent::{AgentHit, BUTTON_H, FlowButton};
use crate::theme::theme;

/// The control row between `x0` and `x1`, its bottom at `bottom`: play and
/// record, or, while `dictating`, play, cancel and send. All square.
pub struct ControlRow;

impl ControlRow {
    pub fn lay_out(x0: f32, x1: f32, bottom: f32, s: f32, dictating: bool) -> Vec<FlowButton> {
        let sq = BUTTON_H * s;
        let y = bottom - sq;
        let square = |hit, x: f32, bg| FlowButton {
            hit,
            rect: Rect::new(x, y, sq, sq),
            label: "",
            bg,
        };
        // Record, and send in its place, always at the bottom right, the
        // easiest place to reach (owner: "the record/send action should
        // always be in the bottom right per Fitz").
        let mut row = vec![square(AgentHit::ReadAloud, x0, theme().back)];
        if dictating {
            row.push(square(AgentHit::Clear, (x0 + x1 - sq) * 0.5, theme().back));
            row.push(square(AgentHit::Submit, x1 - sq, theme().approve));
        } else {
            // The centre button: general agent (Gemini) in the Gemini accent.
            row.push(square(
                AgentHit::GeneralAgent,
                (x0 + x1 - sq) * 0.5,
                theme().accent,
            ));
            // The keyboard glyph, left of record with record's hit box and
            // no background.
            let gap = 12.0 * s;
            row.push(square(AgentHit::Keyboard, x1 - 2.0 * sq - gap, [0.0; 4]));
            row.push(square(AgentHit::Message, x1 - sq, theme().message));
        }
        row
    }
}
