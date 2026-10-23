//! The agent view's bottom row, which controls the audio (owner,
//! 2026-10-23: "the bottom row to control the audio, with the left having
//! a square play button, the right a square record button, the play button
//! should be the submit button … with an X button in the middle to
//! cancel").

use libcontrols::Rect;

use super::{APPROVE_BG, AgentHit, BACK_BG, BUTTON_H, FlowButton, MESSAGE_BG};

/// The control row between `x0` and `x1`, its bottom at `bottom`: play and
/// record, or, while `dictating`, send, cancel and record. All square.
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
        let mut row = Vec::new();
        if dictating {
            row.push(square(AgentHit::Submit, x0, APPROVE_BG));
            row.push(square(AgentHit::Clear, (x0 + x1 - sq) * 0.5, BACK_BG));
        } else {
            row.push(square(AgentHit::ReadAloud, x0, BACK_BG));
        }
        row.push(square(AgentHit::Message, x1 - sq, MESSAGE_BG));
        row
    }
}
