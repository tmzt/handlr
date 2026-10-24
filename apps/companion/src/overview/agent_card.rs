//! One agent's card on the overview (plan 20261023_04, the 17:04 EDT
//! amendment, A2 and A3): the agent, its project and tab on one line;
//! herdr's focus as a bar at the left edge; and its turn, from the
//! session read, never herdr's status:
//! - **asking:** "Asking", the question (three lines at most) and its Yes
//!   and No drawn inactive, outlined; they show what is asked, and a tap
//!   anywhere on the card opens the agent view, where they are live;
//! - **stopped:** "Stopped" and the first line of the last reply;
//! - **working:** the newest line, dim;
//! - no session: herdr's status word, dim.
//!
//! A card that needs input (a question, or a handlr confirmation pending
//! for the pane) says so on itself, with an outline and its label in the
//! same colour: amber for an approval, blue for a question. It never moves.

use handlr_companion::CardKind;
use libcontrols::{DrawContext, Rect};
use libhandlr_proto::common::{AgentInfo, Turn};
use libmsdf::drawlist::{DrawList, SdfInstance, SdfKind};

use crate::agent::{
    CARD_BG, DIM, FG, Fill, Flow, GREEN, Row, accent, columns, fill, line_h, one_line, wrap_hard,
};
use crate::controls::OutlineButton;

/// The chevron after the focused agent's heading.
pub const CHEVRON: char = '\u{203a}';

/// What a card needs from the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeedsInput {
    /// A handlr confirmation is pending for the pane.
    Approval,
    /// The agent asks a question.
    Question,
}

impl NeedsInput {
    pub fn label(self) -> &'static str {
        match self {
            NeedsInput::Approval => "Approval needed",
            NeedsInput::Question => "Asking",
        }
    }

    /// As the agent view's cards: approval amber, question blue.
    pub fn colour(self) -> [f32; 4] {
        match self {
            NeedsInput::Approval => accent(CardKind::Approval),
            NeedsInput::Question => accent(CardKind::Question),
        }
    }
}

/// One agent's card, as data: the agent as `agents.list` gave it, and
/// whether a confirmation waits for its pane.
#[derive(Clone, Copy, Debug)]
pub struct AgentCard<'a> {
    pub agent: &'a AgentInfo,
    pub approval: bool,
}

/// A card as laid out, in the coordinates it was laid out at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardLayout {
    pub rect: Rect,
    /// The panel, the focus bar and the text.
    pub flow: Flow,
    /// The needing-input outline, and its colour.
    pub outline: Option<([f32; 4], Rect)>,
    /// A question's Yes and No, inactive.
    pub buttons: Vec<OutlineButton>,
}

impl CardLayout {
    /// Draws the card moved down by `dy`, at scale `s`.
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext, dy: f32, s: f32) {
        let moved = |r: Rect| Rect::new(r.x, r.y + dy, r.w, r.h);
        for f in &self.flow.fills {
            fill(list, moved(f.rect), f.color, f.radius);
        }
        if let Some((color, rect)) = self.outline {
            let r = moved(rect);
            list.push(SdfInstance {
                kind: SdfKind::Outline {
                    radius: AgentCard::RADIUS * s,
                    thickness: AgentCard::OUTLINE * s,
                },
                position: r.pos(),
                size: r.size(),
                color,
                anim: 0,
            });
        }
        for r in &self.flow.rows {
            ctx.draw_text(list, &r.text, [r.x, r.y + dy], r.px, r.color);
        }
        for b in &self.buttons {
            b.draw(list, ctx, dy, s);
        }
    }
}

/// `agent` with its first letter in capitals: "claude" is "Claude".
fn shown_agent(agent: &str) -> String {
    let mut chars = agent.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl AgentCard<'_> {
    /// The least height, the padding, the focus bar's width, the outline's
    /// width and the corner radius, in points.
    pub const MIN_HEIGHT: f32 = 112.0;
    pub const PAD: f32 = 10.0;
    pub const BAR: f32 = 4.0;
    pub const OUTLINE: f32 = 2.0;
    pub const RADIUS: f32 = 10.0;
    /// Type sizes, in points.
    pub const HEADING_PT: f32 = 15.0;
    pub const TEXT_PT: f32 = 14.0;
    /// The inactive buttons' size, in points.
    pub const BUTTON_W: f32 = 64.0;
    pub const BUTTON_H: f32 = 30.0;
    /// Lines of a question shown at most, and of any other line.
    pub const QUESTION_LINES: usize = 3;
    pub const LINE_LINES: usize = 2;

    /// What the card needs from the user, if anything: an approval first.
    pub fn needs_input(&self) -> Option<NeedsInput> {
        if self.approval {
            Some(NeedsInput::Approval)
        } else if matches!(self.agent.turn, Some(Turn::Asking { .. })) {
            Some(NeedsInput::Question)
        } else {
            None
        }
    }

    /// The top line: "Claude · handlr · 17-hr".
    pub fn heading(&self) -> String {
        let a = self.agent;
        let mut parts = vec![shown_agent(&a.agent)];
        if let Some(p) = a.project.as_deref().filter(|p| !p.is_empty()) {
            parts.push(p.to_string());
        }
        let tab = a
            .tab_label
            .clone()
            .filter(|l| !l.is_empty())
            .or_else(|| a.tab_number.map(|n| format!("tab {n}")));
        parts.extend(tab);
        parts.join(" \u{b7} ")
    }

    /// The state line and its colour.
    pub fn state(&self) -> (String, [f32; 4]) {
        if let Some(need) = self.needs_input() {
            return (need.label().to_string(), need.colour());
        }
        match &self.agent.turn {
            Some(Turn::Stopped { .. }) => ("Stopped".to_string(), GREEN),
            Some(Turn::Working { .. }) => ("Working".to_string(), DIM),
            // Asking is a need, above.
            Some(Turn::Asking { .. }) => ("Asking".to_string(), accent(CardKind::Question)),
            None => (self.agent.status.clone(), DIM),
        }
    }

    /// The text under the state line, its colour and its most lines.
    fn body(&self) -> Option<(&str, [f32; 4], usize)> {
        match self.agent.turn.as_ref()? {
            Turn::Asking { question } => Some((question, FG, Self::QUESTION_LINES)),
            Turn::Stopped { line } => Some((line, FG, Self::LINE_LINES)),
            Turn::Working { line } => Some((line, DIM, Self::LINE_LINES)),
        }
    }

    /// Lays the card out at `(x, y)`, `w` wide and at least `min_h` high,
    /// at scale `s`.
    pub fn lay_out(&self, x: f32, y: f32, w: f32, min_h: f32, s: f32) -> CardLayout {
        let pad = Self::PAD * s;
        let inner_x = x + Self::BAR * s + pad;
        let inner_w = w - Self::BAR * s - 2.0 * pad;
        let (head_px, text_px) = (Self::HEADING_PT * s, Self::TEXT_PT * s);
        let mut rows = Vec::new();
        let mut ty = y + pad;
        let mut row = |text: String, px: f32, color, ty: &mut f32| {
            rows.push(Row {
                text,
                x: inner_x,
                y: *ty,
                px,
                color,
                mono: false,
            });
            *ty += line_h(px);
        };
        // herdr's focused agent is a visible control: its heading ends in a
        // chevron, and a tap opens its view (plan 20261023_03, section 7).
        let heading = if self.agent.focused {
            let cols = columns(inner_w, head_px).saturating_sub(2);
            format!("{} {CHEVRON}", one_line(&self.heading(), cols))
        } else {
            one_line(&self.heading(), columns(inner_w, head_px))
        };
        row(heading, head_px, FG, &mut ty);
        ty += 2.0 * s;
        let (state, state_color) = self.state();
        row(state, text_px, state_color, &mut ty);
        if let Some((text, color, most)) = self.body() {
            let mut lines = wrap_hard(text.trim(), columns(inner_w, text_px));
            lines.retain(|l| !l.is_empty());
            if lines.len() > most {
                lines.truncate(most);
                if let Some(last) = lines.last_mut() {
                    last.push('\u{2026}');
                }
            }
            for l in lines {
                row(l, text_px, color, &mut ty);
            }
        }
        let mut buttons = Vec::new();
        if matches!(self.agent.turn, Some(Turn::Asking { .. })) {
            ty += 6.0 * s;
            let (bw, bh) = (Self::BUTTON_W * s, Self::BUTTON_H * s);
            for (i, label) in ["Yes", "No"].into_iter().enumerate() {
                let bx = inner_x + i as f32 * (bw + 8.0 * s);
                buttons.push(OutlineButton::new(label, Rect::new(bx, ty, bw, bh)));
            }
            ty += bh;
        }
        let h = (ty + pad - y).max(min_h);
        let rect = Rect::new(x, y, w, h);
        let mut fills = vec![Fill {
            rect,
            color: CARD_BG,
            radius: Self::RADIUS * s,
        }];
        if self.agent.focused {
            fills.push(Fill {
                rect: Rect::new(x, y, Self::BAR * s, h),
                color: FG,
                radius: 2.0 * s,
            });
        }
        CardLayout {
            rect,
            flow: Flow {
                rows,
                fills,
                height: h,
                ..Flow::default()
            },
            outline: self.needs_input().map(|n| (n.colour(), rect)),
            buttons,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An agent in tab `tab` of workspace `ws`, with `turn`.
    pub(crate) fn agent(pane: &str, ws: &str, tab: u32, turn: Option<Turn>) -> AgentInfo {
        AgentInfo {
            pane_id: pane.to_string(),
            name: Some(format!("Claude in handlr, tab {tab}-hr")),
            spoken: None,
            agent: "claude".to_string(),
            project: Some("handlr".to_string()),
            cwd: None,
            workspace_id: Some(ws.to_string()),
            tab_id: None,
            tab_label: Some(format!("{tab}-hr")),
            tab_number: Some(tab),
            status: "working".to_string(),
            focused: false,
            session: None,
            turn,
        }
    }

    pub(crate) fn asking(q: &str) -> Option<Turn> {
        Some(Turn::Asking {
            question: q.to_string(),
        })
    }

    fn texts(c: &CardLayout) -> Vec<&str> {
        c.flow.rows.iter().map(|r| r.text.as_str()).collect()
    }

    fn colour_of(c: &CardLayout, text: &str) -> [f32; 4] {
        c.flow
            .rows
            .iter()
            .find(|r| r.text == text)
            .unwrap_or_else(|| panic!("no row {text:?} in {:?}", texts(c)))
            .color
    }

    /// At the phone's scale, a card of the two-per-row width.
    fn lay(card: AgentCard) -> CardLayout {
        card.lay_out(24.0, 300.0, 504.0, AgentCard::MIN_HEIGHT * 2.0, 2.0)
    }

    #[test]
    fn an_asking_card_shows_the_question_and_inactive_yes_and_no() {
        let a = agent("p1", "w1", 17, asking("Install it on the S23?"));
        let c = lay(AgentCard {
            agent: &a,
            approval: false,
        });
        assert_eq!(
            texts(&c),
            [
                "Claude \u{b7} handlr \u{b7} 17-hr",
                "Asking",
                "Install it on the S23?"
            ]
        );
        let blue = accent(CardKind::Question);
        assert_eq!(colour_of(&c, "Asking"), blue);
        // Needing input: a blue outline round the card.
        assert_eq!(c.outline, Some((blue, c.rect)));
        // Yes and No, outlined and dim, inside the card and under the text.
        let labels: Vec<&str> = c.buttons.iter().map(|b| b.label).collect();
        assert_eq!(labels, ["Yes", "No"]);
        let question = c.flow.rows.last().unwrap();
        for b in &c.buttons {
            assert_eq!(b.color, DIM);
            assert!(b.rect.y > question.y);
            assert!(b.rect.y + b.rect.h <= c.rect.y + c.rect.h);
            assert!(b.rect.x + b.rect.w <= c.rect.x + c.rect.w);
        }
        // No filled button: only the panel is filled.
        assert_eq!(c.flow.fills.len(), 1);
    }

    #[test]
    fn a_long_question_is_cut_to_three_lines() {
        let a = agent(
            "p1",
            "w1",
            17,
            asking(&"Shall I go on with it? ".repeat(20)),
        );
        let c = lay(AgentCard {
            agent: &a,
            approval: false,
        });
        assert_eq!(c.flow.rows.len(), 2 + AgentCard::QUESTION_LINES);
        assert!(c.flow.rows.last().unwrap().text.ends_with('\u{2026}'));
        // Nothing runs past the card.
        let inner_w = 504.0 - (AgentCard::BAR + 2.0 * AgentCard::PAD) * 2.0;
        for r in &c.flow.rows {
            assert!(
                r.text.chars().count() <= columns(inner_w, r.px) + 1,
                "{r:?}"
            );
        }
    }

    #[test]
    fn an_approval_is_outlined_amber() {
        let a = agent("p1", "w1", 17, asking("Push it?"));
        let c = lay(AgentCard {
            agent: &a,
            approval: true,
        });
        let amber = accent(CardKind::Approval);
        assert_eq!(c.outline, Some((amber, c.rect)));
        assert_eq!(colour_of(&c, "Approval needed"), amber);
        // The question still shows, with its buttons.
        assert!(texts(&c).contains(&"Push it?"));
        assert_eq!(c.buttons.len(), 2);
    }

    #[test]
    fn a_stopped_card_shows_the_first_line_of_the_last_reply() {
        let a = agent(
            "p1",
            "w1",
            17,
            Some(Turn::Stopped {
                line: "All 12 tests pass.".to_string(),
            }),
        );
        let c = lay(AgentCard {
            agent: &a,
            approval: false,
        });
        assert_eq!(&texts(&c)[1..], ["Stopped", "All 12 tests pass."]);
        assert_eq!(colour_of(&c, "All 12 tests pass."), FG);
        assert_eq!(c.outline, None);
        assert!(c.buttons.is_empty());
    }

    #[test]
    fn a_working_card_is_dim_and_one_without_a_session_shows_the_status() {
        let a = agent(
            "p1",
            "w1",
            17,
            Some(Turn::Working {
                line: "Bash cargo test".to_string(),
            }),
        );
        let c = lay(AgentCard {
            agent: &a,
            approval: false,
        });
        assert_eq!(&texts(&c)[1..], ["Working", "Bash cargo test"]);
        assert_eq!(colour_of(&c, "Working"), DIM);
        assert_eq!(colour_of(&c, "Bash cargo test"), DIM);
        assert_eq!(c.outline, None);
        let none = AgentInfo {
            status: "idle".to_string(),
            ..agent("p2", "w1", 3, None)
        };
        let c = lay(AgentCard {
            agent: &none,
            approval: false,
        });
        assert_eq!(&texts(&c)[1..], ["idle"]);
        assert_eq!(colour_of(&c, "idle"), DIM);
        // At least 112 pt high.
        assert_eq!(c.rect.h, AgentCard::MIN_HEIGHT * 2.0);
    }

    #[test]
    fn herdrs_focus_is_a_bar_at_the_left_edge() {
        let a = agent("p1", "w1", 17, None);
        let plain = lay(AgentCard {
            agent: &a,
            approval: false,
        });
        assert_eq!(plain.flow.fills.len(), 1);
        let focused = AgentInfo {
            focused: true,
            ..a.clone()
        };
        let c = lay(AgentCard {
            agent: &focused,
            approval: false,
        });
        let bar = c.flow.fills[1];
        assert_eq!(bar.rect.x, c.rect.x);
        // Its heading ends in the chevron; another's does not.
        assert_eq!(
            c.flow.rows[0].text,
            format!("Claude \u{b7} handlr \u{b7} 17-hr {CHEVRON}")
        );
        assert!(!plain.flow.rows[0].text.ends_with(CHEVRON));
        assert_eq!(bar.rect.w, AgentCard::BAR * 2.0);
        assert_eq!(bar.rect.h, c.rect.h);
        // A tab without a label shows its number.
        let numbered = AgentInfo {
            tab_label: None,
            ..a
        };
        let card = AgentCard {
            agent: &numbered,
            approval: false,
        };
        assert_eq!(card.heading(), "Claude \u{b7} handlr \u{b7} tab 17");
    }
}
