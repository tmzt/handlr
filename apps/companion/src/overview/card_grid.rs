//! The overview's cards: one card per agent pane, two per row, in tab
//! order (plan 20261023_04, the 17:04 EDT amendment, A2), on the whole
//! screen between the status lines and the bottom row (the 18:07 EDT
//! amendment, 1). Shells have cards too, terminal only (plan 20261022_03,
//! the L3 amendment of 2026-10-28 00:47 EDT). The order is by `(workspace_id, tab_number)` and never changes as
//! turns change: a card that needs input is marked where it is. Cards are
//! equal width, 12 pt apart and from the edges, at least 112 pt high; the
//! two of a row share its height. The grid scrolls on its own. After the
//! agent cards, full width, come the pinned plugin cards
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.7), whose heights
//! the app gives ([`CardGrid::set_extra`]) and whose content it draws.

use std::cmp::Ordering;
use std::time::Duration;

use libcontrols::{DrawContext, Rect};
use libflankr_proto::common::AgentInfo;
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use super::agent_card::{AgentCard, CardLayout};
use super::panel_shader::PanelInstance;
use crate::agent::PAD;
use crate::controls::scroll::{Scroll, Touched};
use crate::theme::theme;

/// A workspace id as it sorts: its number, where it ends in one ("w2"
/// before "w10"), then as written.
fn workspace_key(id: Option<&str>) -> (u64, String) {
    let id = id.unwrap_or("");
    let digits: String = id
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (digits.parse().unwrap_or(u64::MAX), id.to_string())
}

/// The agents of `agents` that have a card, in tab order: by workspace,
/// then tab number (a tab without one last), then pane id. Shells are
/// among them.
pub fn tab_order(agents: &[AgentInfo]) -> Vec<AgentInfo> {
    let mut shown: Vec<AgentInfo> = agents.to_vec();
    shown.sort_by(|a, b| {
        workspace_key(a.workspace_id.as_deref())
            .cmp(&workspace_key(b.workspace_id.as_deref()))
            .then_with(|| match (a.tab_number, b.tab_number) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| a.pane_id.cmp(&b.pane_id))
    });
    shown
}

/// The card of `a`, which needs approval when its pane is in `pending`.
fn card_of<'a>(a: &'a AgentInfo, pending: &[String]) -> AgentCard<'a> {
    AgentCard {
        agent: a,
        approval: pending.contains(&a.pane_id),
    }
}

/// The cards, laid out in their window.
pub struct CardGrid {
    agents: Vec<AgentInfo>,
    pending: Vec<String>,
    /// `agents.list` has answered.
    loaded: bool,
    viewport: Rect,
    scale: f32,
    /// Each card's pane and layout, in content coordinates (y from the
    /// grid's top).
    cards: Vec<(String, CardLayout)>,
    /// The plugin cards' heights, as given, and their rects in content
    /// coordinates.
    extra: Vec<f32>,
    extra_rects: Vec<Rect>,
    height: f32,
    scroll: Scroll,
}

impl Default for CardGrid {
    fn default() -> Self {
        Self::new()
    }
}

impl CardGrid {
    /// The gap between cards and from the edges, in points.
    pub const GAP: f32 = 12.0;

    pub fn new() -> Self {
        CardGrid {
            agents: Vec::new(),
            pending: Vec::new(),
            loaded: false,
            viewport: Rect::default(),
            scale: 1.0,
            cards: Vec::new(),
            extra: Vec::new(),
            extra_rects: Vec::new(),
            height: 0.0,
            scroll: Scroll::new(false),
        }
    }

    /// The agents as listed, and the panes with a confirmation waiting.
    /// Returns whether anything shown changed.
    pub fn set_agents(&mut self, agents: &[AgentInfo], pending: &[String]) -> bool {
        let agents = tab_order(agents);
        if self.loaded && agents == self.agents && pending == self.pending {
            return false;
        }
        self.loaded = true;
        self.agents = agents;
        self.pending = pending.to_vec();
        self.relayout();
        true
    }

    /// The grid's window on the screen, at scale `s`.
    pub fn set_viewport(&mut self, viewport: Rect, s: f32) {
        if viewport == self.viewport && s == self.scale {
            return;
        }
        self.viewport = viewport;
        self.scale = s;
        self.relayout();
    }

    /// The theme changed: the cards are laid out again in its colours.
    pub fn restyle(&mut self) {
        self.relayout();
    }

    fn relayout(&mut self) {
        let s = self.scale;
        let gap = Self::GAP * s;
        let v = self.viewport;
        let w = ((v.w - 3.0 * gap) / 2.0).max(0.0);
        let min_h = AgentCard::MIN_HEIGHT * s;
        let pending = &self.pending;
        let card = |a| card_of(a, pending);
        let mut cards = Vec::new();
        let mut y = 0.0;
        for pair in self.agents.chunks(2) {
            // The taller of the two sets the row's height.
            let row_h = pair
                .iter()
                .map(|a| card(a).lay_out(0.0, 0.0, w, min_h, s).rect.h)
                .fold(min_h, f32::max);
            for (i, a) in pair.iter().enumerate() {
                let x = v.x + gap + i as f32 * (w + gap);
                cards.push((a.pane_id.clone(), card(a).lay_out(x, y, w, row_h, s)));
            }
            y += row_h + gap;
        }
        // The plugin cards, full width, below the line shown for none.
        if self.extra.is_empty() {
            self.extra_rects.clear();
        } else {
            if self.agents.is_empty() {
                y += 40.0 * s;
            }
            let full = (v.w - 2.0 * gap).max(0.0);
            self.extra_rects = self
                .extra
                .iter()
                .map(|&h| {
                    let r = Rect::new(v.x + gap, y, full, h);
                    y += h + gap;
                    r
                })
                .collect();
        }
        self.height = y;
        self.cards = cards;
        self.scroll.set_extent(self.height, v.h);
    }

    /// The plugin cards' heights, in order: laid out after the agents'.
    pub fn set_extra(&mut self, heights: Vec<f32>) -> bool {
        if heights == self.extra {
            return false;
        }
        self.extra = heights;
        self.relayout();
        true
    }

    /// Plugin card `i`'s rect on the screen.
    pub fn extra_rect(&self, i: usize) -> Option<Rect> {
        let r = self.extra_rects.get(i)?;
        Some(Rect::new(r.x, r.y + self.dy(), r.w, r.h))
    }

    /// The plugin cards in the window, with their indexes.
    pub fn extra_shown(&self) -> Vec<(usize, Rect)> {
        let v = self.viewport;
        (0..self.extra_rects.len())
            .filter_map(|i| self.extra_rect(i).map(|r| (i, r)))
            .filter(|(_, r)| r.y + r.h >= v.y && r.y <= v.y + v.h)
            .collect()
    }

    /// The plugin card at `(x, y)` on the screen, inside the window.
    pub fn extra_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.viewport.contains([x, y]) {
            return None;
        }
        self.extra_shown()
            .into_iter()
            .find(|(_, r)| r.contains([x, y]))
            .map(|(i, _)| i)
    }

    /// The panes with a card, in their order.
    pub fn panes(&self) -> Vec<&str> {
        self.cards.iter().map(|(p, _)| p.as_str()).collect()
    }

    /// Each card as laid out, in content coordinates.
    pub fn cards(&self) -> &[(String, CardLayout)] {
        &self.cards
    }

    /// The content's offset from the window's top: what is scrolled away.
    fn dy(&self) -> f32 {
        self.viewport.y - self.scroll.offset()
    }

    /// Where the card of `pane_id` is on the screen.
    pub fn card_rect(&self, pane_id: &str) -> Option<Rect> {
        let (_, c) = self.cards.iter().find(|(p, _)| p == pane_id)?;
        Some(Rect::new(
            c.rect.x,
            c.rect.y + self.dy(),
            c.rect.w,
            c.rect.h,
        ))
    }

    /// The pane whose card is at `(x, y)` on the screen, inside the window.
    pub fn pane_at(&self, x: f32, y: f32) -> Option<&str> {
        if !self.viewport.contains([x, y]) {
            return None;
        }
        let dy = self.dy();
        self.cards
            .iter()
            .find(|(_, c)| Rect::new(c.rect.x, c.rect.y + dy, c.rect.w, c.rect.h).contains([x, y]))
            .map(|(p, _)| p.as_str())
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn scroll(&self) -> &Scroll {
        &self.scroll
    }

    /// The line shown in place of cards: before the first list, and when
    /// it has none.
    pub fn empty_line(&self) -> Option<&'static str> {
        // No looking-for line before a list (plan 20261028_03, the
        // 2026-10-30 13:15 EDT amendment, item 3).
        if !self.loaded {
            None
        } else if self.agents.is_empty() {
            Some("No agents in herdr.")
        } else {
            None
        }
    }

    /// No list: no cards and no line until one is given.
    pub fn unload(&mut self) {
        self.loaded = false;
        self.agents.clear();
        self.pending.clear();
        self.relayout();
    }

    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, now: Duration) -> Touched {
        self.scroll.touch(id, phase, x, y, now, self.scale)
    }

    pub fn holds(&self, id: u64) -> bool {
        self.scroll.holds(id)
    }

    /// Carries a fling on. Returns whether the grid moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        self.scroll.tick(dt, self.scale)
    }

    /// The panel instances for the custom GPU panel shader.
    pub fn panel_instances(&self) -> Vec<PanelInstance> {
        let dy = self.dy();
        let v = self.viewport;
        let s = self.scale;
        let mut out = Vec::new();
        for (_, c) in &self.cards {
            let top = c.rect.y + dy;
            if top + c.rect.h >= v.y && top <= v.y + v.h {
                let border_col = if let Some((col, _)) = c.outline {
                    col
                } else {
                    theme().band
                };
                let border_w = if c.outline.is_some() {
                    2.0 * s
                } else {
                    1.0 * s
                };
                out.push(PanelInstance {
                    rect: [c.rect.x, c.rect.y + dy, c.rect.w, c.rect.h],
                    color: theme().card,
                    border_color: border_col,
                    clip: [v.x, v.y, v.w, v.h],
                    params: [AgentCard::RADIUS * s, border_w, 1.0, 0.0],
                });
            }
        }
        out
    }

    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext, shader_panels: bool) {
        let v = self.viewport;
        let s = self.scale;
        list.push_clip(v.pos(), v.size());
        if let Some(line) = self.empty_line() {
            ctx.draw_text(
                list,
                line,
                [v.x + PAD * s, v.y + 8.0 * s],
                15.0 * s,
                theme().dim,
            );
        }
        let dy = self.dy();
        for (_, c) in &self.cards {
            let top = c.rect.y + dy;
            if top + c.rect.h >= v.y && top <= v.y + v.h {
                c.draw(list, ctx, dy, s, shader_panels);
            }
        }
        list.push_clip_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::agent_card::tests::{agent, asking};
    use libflankr_proto::common::Turn;

    /// A window on the phone (1080 × 2340 at scale 2.0).
    const HALF: Rect = Rect {
        x: 0.0,
        y: 300.0,
        w: 1080.0,
        h: 800.0,
    };

    fn grid(agents: &[AgentInfo]) -> CardGrid {
        let mut g = CardGrid::new();
        g.set_viewport(HALF, 2.0);
        g.set_agents(agents, &[]);
        g
    }

    fn shell(pane: &str) -> AgentInfo {
        AgentInfo {
            agent: "shell".to_string(),
            ..agent(pane, "w1", 1, None)
        }
    }

    #[test]
    fn cards_are_in_tab_order_with_shells_among_them() {
        let agents = [
            agent("p9", "w10", 1, None),
            agent("p3", "w2", 5, None),
            shell("p4"),
            agent("p1", "w2", 17, None),
            agent("p2", "w2", 5, None),
        ];
        let g = grid(&agents);
        // w1 before w2 before w10; within a tab, by pane. The shell has
        // its card in its place (plan 20261022_03, the 2026-10-28 00:47
        // EDT amendment).
        assert_eq!(g.panes(), ["p4", "p2", "p3", "p1", "p9"]);
    }

    #[test]
    fn the_order_never_changes_as_turns_change() {
        let mut agents = vec![
            agent("p1", "w1", 1, None),
            agent("p2", "w1", 2, None),
            agent("p3", "w1", 3, None),
        ];
        let mut g = grid(&agents);
        let before: Vec<String> = g.panes().into_iter().map(String::from).collect();
        // The last asks, the first stops, and the list comes in another
        // order with an approval pending: no card moves.
        agents[2].turn = asking("Push it?");
        agents[0].turn = Some(Turn::Stopped {
            line: "Done.".to_string(),
        });
        agents.reverse();
        assert!(g.set_agents(&agents, &["p2".to_string()]));
        assert_eq!(g.panes(), before);
        // The marks are on the cards where they are.
        let outline = |pane: &str| {
            g.cards()
                .iter()
                .find(|(p, _)| p == pane)
                .unwrap()
                .1
                .outline
                .map(|(c, _)| c)
        };
        use crate::agent::accent;
        use flankr_companion::CardKind;
        assert_eq!(outline("p3"), Some(accent(CardKind::Question)));
        assert_eq!(outline("p2"), Some(accent(CardKind::Approval)));
        assert_eq!(outline("p1"), None);
        // The same again: nothing changed.
        assert!(!g.set_agents(&agents, &["p2".to_string()]));
    }

    #[test]
    fn two_cards_a_row_equal_width_and_twelve_points_apart() {
        let agents = [
            agent(
                "p1",
                "w1",
                1,
                asking(
                    "A question long enough to need three lines on a card this narrow, and more?",
                ),
            ),
            agent("p2", "w1", 2, None),
            agent("p3", "w1", 3, None),
        ];
        let g = grid(&agents);
        let r = |p| g.card_rect(p).unwrap();
        let (a, b, c) = (r("p1"), r("p2"), r("p3"));
        let gap = CardGrid::GAP * 2.0;
        assert_eq!(a.x, gap);
        assert_eq!(a.w, b.w);
        assert!((b.x - (a.x + a.w) - gap).abs() < 0.01);
        assert!((1080.0 - (b.x + b.w) - gap).abs() < 0.01);
        // A row shares the taller card's height; the next row is under it.
        assert_eq!(a.y, b.y);
        assert_eq!(a.h, b.h);
        assert!(a.h > AgentCard::MIN_HEIGHT * 2.0);
        assert!((c.y - (a.y + a.h) - gap).abs() < 0.01);
        assert_eq!(c.x, a.x);
        assert_eq!(c.h, AgentCard::MIN_HEIGHT * 2.0);
    }

    #[test]
    fn a_tap_finds_its_card_and_the_grid_scrolls() {
        let agents: Vec<AgentInfo> = (1..=12)
            .map(|t| agent(&format!("p{t}"), "w1", t, None))
            .collect();
        let mut g = grid(&agents);
        let r = g.card_rect("p2").unwrap();
        assert_eq!(g.pane_at(r.x + 5.0, r.y + 5.0), Some("p2"));
        // Between cards, and outside the window, is no card.
        assert_eq!(g.pane_at(r.x - 2.0, r.y + 5.0), None);
        assert_eq!(g.pane_at(r.x + 5.0, HALF.y - 1.0), None);
        // Six rows do not fit 800 px: it scrolls.
        assert!(g.scroll().max() > 0.0);
        g.touch(1, TouchPhase::Started, 540.0, 1000.0, Duration::ZERO);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += Duration::from_millis(16);
            g.touch(1, TouchPhase::Moved, 540.0, 1000.0 - 30.0 * i as f32, now);
        }
        g.touch(
            1,
            TouchPhase::Ended,
            540.0,
            700.0,
            now + Duration::from_millis(300),
        );
        assert!((g.scroll().offset() - 300.0).abs() < 1.0);
        // The first row has gone above the window; the third is a tap away
        // where it now is.
        let moved = g.card_rect("p2").unwrap();
        assert!((r.y - moved.y - 300.0).abs() < 1.0);
        assert_eq!(g.pane_at(moved.x + 5.0, moved.y + 5.0), None);
        let third = g.card_rect("p6").unwrap();
        assert!(HALF.contains([third.x + 5.0, third.y + 5.0]));
        assert_eq!(g.pane_at(third.x + 5.0, third.y + 5.0), Some("p6"));
    }

    /// Enough cards for about three windows of HALF: a drag to the middle
    /// has to stay in the middle.
    fn three_screens() -> Vec<AgentInfo> {
        (1..=20)
            .map(|t| agent(&format!("p{t}"), "w1", t, None))
            .collect()
    }

    /// Drags up by `by` px from `y`: quickly, then slowly to a stop that
    /// is still moving at release, the moves arriving in pairs 0.1 ms
    /// apart, as the S23 delivers them. Ends the touch and runs the fling
    /// out. Returns the time it ended.
    fn drag_in_bursts(g: &mut CardGrid, y: f32, by: f32, t: Duration) -> Duration {
        let ms = Duration::from_millis;
        let burst = Duration::from_micros(100);
        g.touch(1, TouchPhase::Started, 540.0, y, t);
        let mut now = t;
        let mut at = y;
        // Most of the way at 1250 px/s...
        let fast = by - 40.0;
        for _ in 0..10 {
            now += ms(16);
            at -= fast / 20.0;
            g.touch(1, TouchPhase::Moved, 540.0, at, now);
            now += burst;
            at -= fast / 20.0;
            g.touch(1, TouchPhase::Moved, 540.0, at, now);
        }
        // ...then the last 40 px at 200 px/s, the finger slowing to lift.
        for _ in 0..10 {
            now += ms(20);
            at -= 2.0;
            g.touch(1, TouchPhase::Moved, 540.0, at, now);
            now += burst;
            at -= 2.0;
            g.touch(1, TouchPhase::Moved, 540.0, at, now);
        }
        now += ms(5);
        g.touch(1, TouchPhase::Ended, 540.0, at, now);
        let mut frames = 0;
        while g.tick(ms(16)) {
            frames += 1;
            assert!(frames < 10_000, "the fling never stopped");
        }
        now
    }

    #[test]
    fn a_drag_to_the_middle_stays_there() {
        // The owner, 2026-10-30: "dragging on the overview screen snaps
        // back to the beginning or end ... it won't let you navigate to
        // the center tiles".
        let agents = three_screens();
        let mut g = grid(&agents);
        let max = g.scroll().max();
        assert!(max > 2.0 * HALF.h, "about three windows: {max}");
        drag_in_bursts(&mut g, 1000.0, 600.0, Duration::ZERO);
        let at = g.scroll().offset();
        // The slow release carries on a little, not to an end.
        assert!(
            (600.0..800.0).contains(&at),
            "left at {at} of {max} after a 600 px drag"
        );
        // And back down by 300 px: from where it was, not from an end.
        let t = Duration::from_secs(5);
        g.touch(1, TouchPhase::Started, 540.0, 500.0, t);
        let mut now = t;
        for i in 1..=10 {
            now += Duration::from_millis(16);
            g.touch(1, TouchPhase::Moved, 540.0, 500.0 + 30.0 * i as f32, now);
        }
        g.touch(
            1,
            TouchPhase::Ended,
            540.0,
            800.0,
            now + Duration::from_millis(300),
        );
        while g.tick(Duration::from_millis(16)) {}
        assert!((g.scroll().offset() - (at - 300.0)).abs() < 1.0);
    }

    #[test]
    fn the_daemons_relist_leaves_the_scroll_where_it_is() {
        let mut agents = three_screens();
        let mut g = grid(&agents);
        drag_in_bursts(&mut g, 1000.0, 600.0, Duration::ZERO);
        let at = g.scroll().offset();
        // The same list again, every few seconds.
        assert!(!g.set_agents(&agents, &[]));
        assert_eq!(g.scroll().offset(), at);
        // A changed list: a turn asks, another is pending, a card is
        // added at the end.
        agents[3].turn = asking("Push it?");
        agents.push(agent("p21", "w1", 21, None));
        assert!(g.set_agents(&agents, &["p5".to_string()]));
        assert_eq!(g.scroll().offset(), at);
        // A plugin card, and the theme.
        g.set_extra(vec![120.0]);
        g.restyle();
        assert_eq!(g.scroll().offset(), at);
    }

    #[test]
    fn plugin_cards_follow_the_agents_full_width_and_take_taps() {
        let mut g = grid(&[
            agent("p1", "w1", 1, None),
            agent("p2", "w1", 2, None),
            agent("p3", "w1", 3, None),
        ]);
        assert!(g.set_extra(vec![120.0, 80.0]));
        assert!(!g.set_extra(vec![120.0, 80.0]));
        let last = g.card_rect("p3").unwrap();
        let first = g.extra_rect(0).unwrap();
        assert!(first.y > last.y + last.h, "below the agents");
        assert_eq!(first.w, HALF.w - 2.0 * CardGrid::GAP * 2.0);
        let second = g.extra_rect(1).unwrap();
        assert!(second.y >= first.y + first.h);
        let [x, y] = first.center();
        if HALF.contains([x, y]) {
            assert_eq!(g.extra_at(x, y), Some(0));
        }
        assert_eq!(g.pane_at(x, y), None);
    }

    #[test]
    fn the_empty_and_loading_lines() {
        let mut g = CardGrid::new();
        g.set_viewport(HALF, 2.0);
        assert_eq!(g.empty_line(), None, "no looking-for line");
        g.set_agents(&[], &[]);
        assert_eq!(g.empty_line(), Some("No agents in herdr."));
        g.set_agents(&[agent("p2", "w1", 1, None)], &[]);
        assert_eq!(g.empty_line(), None);
        // A shell alone is a card.
        g.set_agents(&[shell("p1")], &[]);
        assert_eq!(g.empty_line(), None);
    }
}
