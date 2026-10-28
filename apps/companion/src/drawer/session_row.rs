//! A session's row in the drawer (plan 20261028_03, 3.5; the 18:48 EDT
//! amendment, item 3): 44 pt high, the sheet's inner width.
//!
//! - At its left, a 4 pt bar in the session's colour, the row's height
//!   less 8 pt, the same colour as the overview card's band;
//! - then one 14 pt line, cut with "…": the **tab** in the foreground
//!   colour, `" · "`, and the **summary** ([`summary`]) in its colour;
//! - at its right, a 20 pt column for the activity ([`Indicator`]): the
//!   Braille spinner while processing (drawn by the drawer over the rows,
//!   from the terminal face), a dot while it needs input or is broken, and
//!   the dim idle age;
//! - the row whose agent view is open under the drawer has the card
//!   colour behind it.
//!
//! A tap on it is its body ([`ControlAction::Activated`]): the drawer
//! opens its agent view.

use std::time::Duration;

use flankr_companion::activity::{Activity, NeedsInput, short_age};
use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libflankr_proto::common::{AgentInfo, Turn};
use libmsdf::drawlist::DrawList;

use super::cut_right;
use crate::agent::{fill, line_h};
use crate::overview::agent_card::{AgentCard, NeedsInput as CardNeeds, shown_agent};
use crate::settings::copy;
use crate::theme::theme;

/// The row's height, its bar's width and radius, the text's size, the gap
/// after the bar and before the indicator column, the column's width, the
/// glyph's size, a dot's diameter and the age's size, in points.
pub const ROW_H: f32 = 44.0;
pub const BAR_W: f32 = 4.0;
const BAR_INSET: f32 = 4.0;
const BAR_R: f32 = 2.0;
pub const TEXT_PT: f32 = 14.0;
const AFTER_BAR: f32 = 12.0;
const BEFORE_COLUMN: f32 = 8.0;
pub const COLUMN_W: f32 = 20.0;
pub const GLYPH_PT: f32 = 14.0;
const DOT: f32 = 8.0;
const AGE_PT: f32 = 11.0;

/// The row's tab: its tab label (`17-fl`), else `tab <n>`, else the
/// agent's shown name.
pub fn tab(agent: &AgentInfo) -> String {
    agent
        .tab_label
        .clone()
        .filter(|l| !l.is_empty())
        .or_else(|| agent.tab_number.map(|n| format!("tab {n}")))
        .unwrap_or_else(|| shown_agent(&agent.agent))
}

/// The first line of `text`, trimmed.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// The row's summary and its colour, by the first rule that applies (3.5
/// item 17): an approval pending; the question asked; the latest
/// finished card's spoken summary; the turn's line; a shell's folder;
/// herdr's status word.
pub fn summary(agent: &AgentInfo, approval: bool) -> (String, [f32; 4]) {
    if approval {
        return (
            copy::DRAWER_SESSION_APPROVAL.text.to_string(),
            CardNeeds::Approval.colour(),
        );
    }
    let card = AgentCard { agent, approval };
    if let Some(Turn::Asking { question }) = &agent.turn {
        return (
            first_line(question).to_string(),
            CardNeeds::Question.colour(),
        );
    }
    if let Some(s) = agent
        .summary
        .as_deref()
        .map(first_line)
        .filter(|s| !s.is_empty())
    {
        return (s.to_string(), theme().dim);
    }
    match &agent.turn {
        Some(Turn::Stopped { line } | Turn::Working { line }) if !first_line(line).is_empty() => {
            return (first_line(line).to_string(), theme().dim);
        }
        _ => {}
    }
    if card.terminal_only()
        && let Some(folder) = agent
            .cwd
            .as_deref()
            .map(|c| c.trim_end_matches('/'))
            .and_then(|c| c.rsplit('/').next())
            .filter(|f| !f.is_empty())
    {
        return (folder.to_string(), theme().dim);
    }
    (agent.status.clone(), theme().dim)
}

/// What the indicator column shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Indicator {
    /// The Braille spinner, in the session's colour.
    Spinner,
    /// A filled dot in this colour.
    Dot([f32; 4]),
    /// The idle age, dim: `now`, `4m`, `2h`, `1d`.
    Age(String),
    /// Nothing: idle with no time known.
    None,
}

/// The column for `activity` (3.5 item 18).
pub fn indicator(activity: Activity) -> Indicator {
    match activity {
        Activity::Processing => Indicator::Spinner,
        Activity::NeedsInput(NeedsInput::Approval) => Indicator::Dot(CardNeeds::Approval.colour()),
        Activity::NeedsInput(NeedsInput::Question) => Indicator::Dot(CardNeeds::Question.colour()),
        Activity::Broken => Indicator::Dot(theme().red),
        Activity::Idle { since: Some(age) } => Indicator::Age(age_text(age)),
        Activity::Idle { since: None } => Indicator::None,
    }
}

/// An idle row's age: `now` under a minute, else `1m` … `1d` ….
pub fn age_text(age: Duration) -> String {
    short_age(age).unwrap_or_else(|| copy::DRAWER_SESSION_IDLE_NOW.text.to_string())
}

/// One session's row.
#[derive(Clone, Debug)]
pub struct SessionRow {
    pub rect: Rect,
    pub pane_id: String,
    pub tab: String,
    pub summary: String,
    pub summary_color: [f32; 4],
    /// The session's colour: the bar's and the spinner's.
    pub colour: [f32; 4],
    pub activity: Activity,
    /// Its agent view is open under the drawer.
    pub open: bool,
    pub scale: f32,
}

impl SessionRow {
    /// `agent`'s row, `approval` pending or not, judged `activity`.
    pub fn new(agent: &AgentInfo, approval: bool, activity: Activity, open: bool, s: f32) -> Self {
        let (summary, summary_color) = summary(agent, approval);
        SessionRow {
            rect: Rect::new(0.0, 0.0, 1.0, ROW_H * s),
            pane_id: agent.pane_id.clone(),
            tab: tab(agent),
            summary,
            summary_color,
            colour: theme().session(flankr_companion::cross::colour_of(agent)),
            activity,
            open,
            scale: s,
        }
    }

    /// The bar, in the row's coordinates.
    pub fn bar(&self) -> Rect {
        let s = self.scale;
        Rect::new(
            self.rect.x,
            self.rect.y + BAR_INSET * s,
            BAR_W * s,
            self.rect.h - 2.0 * BAR_INSET * s,
        )
    }

    /// The indicator column.
    pub fn column(&self) -> Rect {
        let s = self.scale;
        Rect::new(
            self.rect.x + self.rect.w - COLUMN_W * s,
            self.rect.y,
            COLUMN_W * s,
            self.rect.h,
        )
    }

    /// Where the line starts and how wide it may be.
    pub fn text_span(&self) -> (f32, f32) {
        let s = self.scale;
        let x = self.rect.x + BAR_W * s + AFTER_BAR * s;
        let end = self.column().x - BEFORE_COLUMN * s;
        (x, (end - x).max(0.0))
    }

    /// What the column shows.
    pub fn indicator(&self) -> Indicator {
        indicator(self.activity)
    }
}

impl DrawControl for SessionRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        if self.open {
            fill(list, self.rect, theme().card, 8.0 * s);
        }
        fill(list, self.bar(), self.colour, BAR_R * s);
        let px = TEXT_PT * s;
        let (x, room) = self.text_span();
        let y = self.rect.y + (self.rect.h - line_h(px)) * 0.5;
        let tab = cut_right(ctx, &self.tab, px, room);
        ctx.draw_text(list, &tab, [x, y], px, theme().fg);
        let used = ctx.text_advance(&tab, px);
        let rest = room - used;
        let sep = " \u{b7} ";
        if !self.summary.is_empty() && rest > ctx.text_advance(sep, px) + px {
            let line = cut_right(ctx, &format!("{sep}{}", self.summary), px, rest);
            ctx.draw_text(list, &line, [x + used, y], px, self.summary_color);
        }
        let col = self.column();
        match self.indicator() {
            // Drawn over the rows by the drawer, in the terminal face.
            Indicator::Spinner | Indicator::None => {}
            Indicator::Dot(c) => {
                let d = DOT * s;
                let [cx, cy] = col.center();
                fill(
                    list,
                    Rect::new(cx - d * 0.5, cy - d * 0.5, d, d),
                    c,
                    d * 0.5,
                );
            }
            Indicator::Age(text) => {
                let apx = AGE_PT * s;
                let w = ctx.text_advance(&text, apx);
                let ay = self.rect.y + (self.rect.h - line_h(apx)) * 0.5;
                // Right-aligned in the column; it may reach left of it.
                ctx.draw_text(list, &text, [col.x + col.w - w, ay], apx, theme().dim);
            }
        }
    }
}

impl HostedControl for SessionRow {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } if self.rect.contains(*pos) => Some(ControlAction::Activated {
                part: ControlPart::Body,
            }),
            _ => None,
        }
    }

    fn handle_keyboard(
        &self,
        _event: &KeyboardEvent,
        _ctx: &EventContext,
    ) -> Option<ControlAction> {
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A Claude Code pane in tab `tab`.
    pub(crate) fn agent(pane: &str, tab: Option<u32>, turn: Option<Turn>) -> AgentInfo {
        AgentInfo {
            pane_id: pane.to_string(),
            name: Some("Claude".to_string()),
            spoken: None,
            agent: "claude".to_string(),
            project: Some("flankr".to_string()),
            cwd: Some("/Users/me/src/flankr".to_string()),
            workspace_id: Some("w1".to_string()),
            tab_id: None,
            tab_label: tab.map(|t| format!("{t}-fl")),
            tab_number: tab,
            status: "working".to_string(),
            focused: false,
            session: None,
            turn,
            idle_ms: None,
            subagents_running: None,
            summary: None,
        }
    }

    #[test]
    fn the_tab_is_the_label_then_the_number_then_the_agent() {
        let mut a = agent("p1", Some(17), None);
        assert_eq!(tab(&a), "17-fl");
        a.tab_label = None;
        assert_eq!(tab(&a), "tab 17");
        a.tab_number = None;
        assert_eq!(tab(&a), "Claude");
    }

    #[test]
    fn each_summary_rule_in_its_order() {
        crate::theme::testing::use_theme("dark_gray");
        let stopped = Some(Turn::Stopped {
            line: "\n  Done: 3 files\nmore".into(),
        });
        let mut a = agent("p1", Some(1), stopped);
        a.summary = Some("Fixed the build.\nAnd more.".into());
        // 1. An approval pending, in amber.
        assert_eq!(
            summary(&a, true),
            ("Approval needed".to_string(), CardNeeds::Approval.colour())
        );
        // 3. The spoken summary's first line.
        assert_eq!(summary(&a, false).0, "Fixed the build.");
        // 4. The turn's line.
        a.summary = None;
        assert_eq!(summary(&a, false), ("Done: 3 files".into(), theme().dim));
        a.turn = Some(Turn::Working {
            line: "Running tests".into(),
        });
        assert_eq!(summary(&a, false).0, "Running tests");
        // 2. A question, in periwinkle, before the summary.
        a.summary = Some("old".into());
        a.turn = Some(Turn::Asking {
            question: "Which one?\n1. A".into(),
        });
        assert_eq!(
            summary(&a, false),
            ("Which one?".into(), CardNeeds::Question.colour())
        );
        // 5. A shell: its folder.
        let mut shell = agent("p2", None, None);
        shell.agent = "shell".into();
        shell.cwd = Some("/Users/me/src/htop-dir/".into());
        assert_eq!(summary(&shell, false).0, "htop-dir");
        // 6. Otherwise herdr's status.
        let mut bare = agent("p3", None, None);
        bare.status = "idle".into();
        assert_eq!(summary(&bare, false), ("idle".into(), theme().dim));
    }

    #[test]
    fn each_indicator() {
        crate::theme::testing::use_theme("dark_gray");
        assert_eq!(indicator(Activity::Processing), Indicator::Spinner);
        assert_eq!(
            indicator(Activity::NeedsInput(NeedsInput::Approval)),
            Indicator::Dot(CardNeeds::Approval.colour())
        );
        assert_eq!(
            indicator(Activity::NeedsInput(NeedsInput::Question)),
            Indicator::Dot(CardNeeds::Question.colour())
        );
        assert_eq!(indicator(Activity::Broken), Indicator::Dot(theme().red));
        let idle = |secs| {
            indicator(Activity::Idle {
                since: Some(Duration::from_secs(secs)),
            })
        };
        assert_eq!(idle(5), Indicator::Age("now".into()));
        assert_eq!(idle(4 * 60 + 5), Indicator::Age("4m".into()));
        assert_eq!(idle(2 * 3600), Indicator::Age("2h".into()));
        assert_eq!(idle(86_400 * 3), Indicator::Age("3d".into()));
        assert_eq!(indicator(Activity::Idle { since: None }), Indicator::None);
    }

    #[test]
    fn the_bar_is_the_overview_bands_colour_4_pt_wide_and_inset() {
        crate::theme::testing::use_theme("dark_gray");
        let a = agent("p1", Some(1), None);
        let mut row = SessionRow::new(&a, false, Activity::Idle { since: None }, false, 2.0);
        row.rect = Rect::new(24.0, 100.0, 600.0, ROW_H * 2.0);
        let band = AgentCard {
            agent: &a,
            approval: false,
        }
        .session_colour();
        assert_eq!(row.colour, band);
        let bar = row.bar();
        assert_eq!(
            (bar.x, bar.y, bar.w, bar.h),
            (24.0, 108.0, 8.0, 88.0 - 16.0)
        );
        let col = row.column();
        assert_eq!((col.x + col.w, col.w), (624.0, 40.0));
        let (x, w) = row.text_span();
        assert_eq!(x, 24.0 + 8.0 + 24.0);
        assert_eq!(x + w, col.x - 16.0);
    }
}
