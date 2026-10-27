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
    pub fn lay_out(
        x0: f32,
        x1: f32,
        bottom: f32,
        s: f32,
        dictating: bool,
        general_agent_listening: bool,
    ) -> Vec<FlowButton> {
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
        if general_agent_listening {
            // General agent voice recognition is active:
            // Centre button shows pulsing/growing-shrinking and alternating dingbats.
            row.push(FlowButton {
                hit: AgentHit::GeneralAgent,
                rect: Rect::new((x0 + x1 - sq) * 0.5, y, sq, sq),
                label: "listening",
                bg: theme().accent,
            });
            if dictating {
                row.push(square(AgentHit::Submit, x1 - sq, theme().approve));
            } else {
                let gap = 12.0 * s;
                row.push(square(AgentHit::Keyboard, x1 - 2.0 * sq - gap, [0.0; 4]));
                row.push(square(AgentHit::Message, x1 - sq, theme().message));
            }
        } else if dictating {
            row.push(square(AgentHit::Clear, (x0 + x1 - sq) * 0.5, theme().back));
            row.push(square(AgentHit::Submit, x1 - sq, theme().approve));
        } else {
            // The centre button: general agent (Gemini) idle in neutral
            // background, hidden while the daemon offers none (plan
            // 20261027_02, decision 9).
            if crate::settings::llm().general_agent {
                row.push(square(
                    AgentHit::GeneralAgent,
                    (x0 + x1 - sq) * 0.5,
                    theme().back,
                ));
            }
            // The keyboard glyph, left of record with record's hit box and
            // no background.
            let gap = 12.0 * s;
            row.push(square(AgentHit::Keyboard, x1 - 2.0 * sq - gap, [0.0; 4]));
            row.push(square(AgentHit::Message, x1 - sq, theme().message));
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_row_layout_with_general_agent_listening() {
        let buttons = ControlRow::lay_out(10.0, 390.0, 500.0, 1.0, false, true);
        assert_eq!(buttons.len(), 4);
        let center = &buttons[1];
        assert_eq!(center.hit, AgentHit::GeneralAgent);
        assert_eq!(center.label, "listening");
        assert_eq!(center.rect.w, BUTTON_H);
        assert_eq!(center.rect.h, BUTTON_H);
        assert_eq!(center.rect.x, (10.0 + 390.0 - BUTTON_H) * 0.5);

        // Even with dictating active, general agent listening keeps the centre button as listening
        let dictating_buttons = ControlRow::lay_out(10.0, 390.0, 500.0, 1.0, true, true);
        assert_eq!(dictating_buttons.len(), 3);
        assert_eq!(dictating_buttons[1].hit, AgentHit::GeneralAgent);
        assert_eq!(dictating_buttons[1].label, "listening");
        assert_eq!(dictating_buttons[1].rect.w, BUTTON_H);
        assert_eq!(dictating_buttons[2].hit, AgentHit::Submit);

        // Idle state: general_agent_listening is false, centre button is idle with theme().back
        let idle_buttons = ControlRow::lay_out(10.0, 390.0, 500.0, 1.0, false, false);
        assert_eq!(idle_buttons.len(), 4);
        assert_eq!(idle_buttons[1].hit, AgentHit::GeneralAgent);
        assert_eq!(idle_buttons[1].label, "");
        assert_eq!(idle_buttons[1].bg, theme().back);

        // Without the daemon's general agent, no centre button.
        crate::settings::set_llm(flankr_companion::wire::LlmInfo {
            general_agent: false,
            summaries: true,
            reason: None,
        });
        let hidden = ControlRow::lay_out(10.0, 390.0, 500.0, 1.0, false, false);
        assert_eq!(hidden.len(), 3);
        assert!(hidden.iter().all(|b| b.hit != AgentHit::GeneralAgent));
        crate::settings::set_llm(flankr_companion::wire::LlmInfo::available());
    }
}
