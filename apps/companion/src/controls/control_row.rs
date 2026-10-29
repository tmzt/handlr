//! The bottom row, which controls the audio, on the agent view and the
//! overview (owner,
//! 2026-10-23: "the bottom row to control the audio, with the left having
//! a square play button, the right a square record button … with an X
//! button in the middle to cancel"; "play doesn't become send, record
//! becomes send").
//!
//! Evenly spaced slots (docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md,
//! decision 1): `n` 52 pt squares between `x0` and `x1`, slot `i` at
//! `x0 + i * (x1 - x0 - sq) / (n - 1)`. Four on the agent view (volume,
//! terminal switch, centre, right), three on the overview and in a card
//! being replied to (volume, centre, right). A slot whose button is not
//! shown stays empty; nothing shifts.

use libcontrols::Rect;

use crate::agent::{AgentHit, BUTTON_H, FlowButton};
use crate::theme::theme;

/// What the row shows (decision 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowState {
    /// The number of slots: 4 on the agent view, 3 elsewhere.
    pub slots: usize,
    /// A prompt with an input area is open: the centre is the ✕ and the
    /// right the send.
    pub dictating: bool,
    /// Gemini listens: the centre is the listening button.
    pub general_agent_listening: bool,
    /// The terminal switch's label (`TERMINAL_LABEL` or `LIST_LABEL`), in
    /// slot 1 of 4; `None` for no switch (a terminal-only session, or a
    /// row of three). Not drawn while dictating.
    pub terminal: Option<&'static str>,
    /// The record button: `false` for a terminal-only session, which
    /// takes no messages.
    pub record: bool,
}

impl RowState {
    /// The overview's row, and a replied card's: three slots.
    pub fn three(dictating: bool, general_agent_listening: bool) -> Self {
        RowState {
            slots: 3,
            dictating,
            general_agent_listening,
            terminal: None,
            record: true,
        }
    }
}

/// The control row between `x0` and `x1`, its bottom at `bottom`.
pub struct ControlRow;

impl ControlRow {
    /// Slot `i` of `n`'s left edge, for squares `sq` wide.
    pub fn slot_x(x0: f32, x1: f32, sq: f32, n: usize, i: usize) -> f32 {
        if n < 2 {
            return x0;
        }
        x0 + i as f32 * (x1 - x0 - sq) / (n - 1) as f32
    }

    pub fn lay_out(x0: f32, x1: f32, bottom: f32, s: f32, st: &RowState) -> Vec<FlowButton> {
        let sq = BUTTON_H * s;
        let y = bottom - sq;
        let n = st.slots.max(3);
        let at = |i: usize| Self::slot_x(x0, x1, sq, n, i);
        let square = |hit, i: usize, bg| FlowButton {
            hit,
            rect: Rect::new(at(i), y, sq, sq),
            label: "".into(),
            bg,
        };
        let (centre, right) = (n - 2, n - 1);
        let mut row = vec![square(AgentHit::Volume, 0, theme().back)];
        // The terminal switch, with no background (plan 20261027_07,
        // decision 7), slot 1 of 4; not while a message is dictated.
        if n == 4
            && !st.dictating
            && let Some(label) = st.terminal
        {
            row.push(FlowButton {
                hit: AgentHit::TerminalToggle,
                rect: Rect::new(at(1), y, sq, sq),
                label: label.into(),
                bg: [0.0; 4],
            });
        }
        if st.general_agent_listening {
            // Centre button shows pulsing/growing-shrinking and alternating
            // dingbats.
            row.push(FlowButton {
                hit: AgentHit::GeneralAgent,
                rect: Rect::new(at(centre), y, sq, sq),
                label: "listening".into(),
                bg: theme().accent,
            });
        } else if st.dictating {
            row.push(square(AgentHit::Clear, centre, theme().back));
        } else if crate::settings::llm().general_agent {
            // The centre button: general agent (Gemini) idle in neutral
            // background, empty while the daemon offers none (plan
            // 20261027_02, decision 9).
            row.push(square(AgentHit::GeneralAgent, centre, theme().back));
        }
        // Record, and send in its place, always at the bottom right, the
        // easiest place to reach (owner: "the record/send action should
        // always be in the bottom right per Fitz").
        if st.dictating {
            row.push(square(AgentHit::Submit, right, theme().approve));
        } else if st.record {
            row.push(square(AgentHit::Message, right, theme().message));
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::TERMINAL_LABEL;

    /// The S23 Ultra's 411 pt width with the 12 pt pad, at scale 1.
    const X0: f32 = 12.0;
    const X1: f32 = 411.0 - 12.0;

    fn agent(dictating: bool, listening: bool, terminal: bool, record: bool) -> RowState {
        RowState {
            slots: 4,
            dictating,
            general_agent_listening: listening,
            terminal: terminal.then_some(TERMINAL_LABEL),
            record,
        }
    }

    fn xs(row: &[FlowButton]) -> Vec<(AgentHit, f32)> {
        row.iter()
            .map(|b| (b.hit, (b.rect.x * 10.0).round() / 10.0))
            .collect()
    }

    #[test]
    fn four_slots_on_the_agent_view_at_411_pt() {
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &agent(false, false, true, true));
        assert_eq!(
            xs(&row),
            [
                (AgentHit::Volume, 12.0),
                (AgentHit::TerminalToggle, 123.7),
                (AgentHit::GeneralAgent, 235.3),
                (AgentHit::Message, 347.0),
            ]
        );
        for b in &row {
            assert_eq!((b.rect.w, b.rect.h), (BUTTON_H, BUTTON_H));
            assert_eq!(b.rect.y, 500.0 - BUTTON_H);
        }
        // Dictating: no switch; the ✕ and send in the centre's and the
        // right's slots.
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &agent(true, false, true, true));
        assert_eq!(
            xs(&row),
            [
                (AgentHit::Volume, 12.0),
                (AgentHit::Clear, 235.3),
                (AgentHit::Submit, 347.0),
            ]
        );
        // Listening: the listening button in slot 2.
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &agent(false, true, true, true));
        assert_eq!(row[2].hit, AgentHit::GeneralAgent);
        assert_eq!(row[2].label, "listening");
        assert_eq!(row[2].bg, theme().accent);
        assert_eq!(row[2].rect.x, ControlRow::slot_x(X0, X1, BUTTON_H, 4, 2));
        // Listening while dictating: listening and send.
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &agent(true, true, true, true));
        assert_eq!(
            row.iter().map(|b| b.hit).collect::<Vec<_>>(),
            [AgentHit::Volume, AgentHit::GeneralAgent, AgentHit::Submit]
        );
        // Terminal only: no switch, no record; the others stay put.
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &agent(false, false, false, false));
        assert_eq!(
            xs(&row),
            [(AgentHit::Volume, 12.0), (AgentHit::GeneralAgent, 235.3)]
        );
    }

    #[test]
    fn three_slots_on_the_overview_at_411_pt() {
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &RowState::three(false, false));
        assert_eq!(
            xs(&row),
            [
                (AgentHit::Volume, 12.0),
                (AgentHit::GeneralAgent, 179.5),
                (AgentHit::Message, 347.0),
            ]
        );
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &RowState::three(true, false));
        assert_eq!(
            xs(&row),
            [
                (AgentHit::Volume, 12.0),
                (AgentHit::Clear, 179.5),
                (AgentHit::Submit, 347.0),
            ]
        );
    }

    #[test]
    fn without_the_general_agent_the_centre_is_empty() {
        crate::settings::set_llm(flankr_companion::wire::LlmInfo {
            general_agent: false,
            summaries: true,
            reason: None,
        });
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &agent(false, false, true, true));
        assert_eq!(
            xs(&row),
            [
                (AgentHit::Volume, 12.0),
                (AgentHit::TerminalToggle, 123.7),
                (AgentHit::Message, 347.0),
            ]
        );
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &RowState::three(false, false));
        assert_eq!(
            xs(&row),
            [(AgentHit::Volume, 12.0), (AgentHit::Message, 347.0)]
        );
        // The ✕ while dictating still.
        let row = ControlRow::lay_out(X0, X1, 500.0, 1.0, &RowState::three(true, false));
        assert_eq!(row[1].hit, AgentHit::Clear);
        crate::settings::set_llm(flankr_companion::wire::LlmInfo::available());
    }
}
