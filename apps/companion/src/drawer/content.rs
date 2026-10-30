//! What the drawer's sheet holds (plan 20261028_03, 3.1 and 3.11): the
//! remote cards, 4 across and up to 2 rows, with the **+** after the last
//! when fewer than 8 are paired, then the remote in use's sessions in a
//! libcontrols `ScrollPane`, one [`SessionRow`] each, in `agents.list`
//! order. Built from the client's events into the drawer's content router,
//! in the sheet's own coordinates.
//!
//! [`DrawerContent::build`] makes the nodes again when what they show
//! changes (a list, the remotes, a pending approval, the view open);
//! between lists [`DrawerContent::refresh`] judges each row's activity
//! again at the time given, so a row stops spinning [`RECENT`] after its
//! last write, and says when the sheet must be drawn again (the spinner's
//! frame included). The list's `ScrollState` is kept across rebuilds.
//!
//! [`RECENT`]: flankr_companion::activity::RECENT

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use flankr_companion::activity::{self, Activity, Signals};
use flankr_companion::{MAX_REMOTES, RemoteId, RemoteInfo};
use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseEvent, NodeId, Rect, Router, ScrollAxis, ScrollPane, ScrollState,
};
use libflankr_proto::common::AgentInfo;
use libmsdf::drawlist::DrawList;

use super::remote_card::{CARD_H, RemoteCard, Slot};
use super::session_row::{COLUMN_W, ROW_H, SessionRow};
use super::{DrawerHit, cut_right, spinner};
use crate::agent::line_h;
use crate::settings::copy;
use crate::theme::theme;

/// The sheet's padding, the gap between cards and below the grid, in
/// points; the cards across.
pub const PAD: f32 = 12.0;
pub const CARD_GAP: f32 = 8.0;
pub const COLUMNS: usize = 4;
const NOTE_PT: f32 = 14.0;

/// The list's one row while it has no sessions to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Its connection failed; the next try in this many seconds.
    Unreachable(u32),
    /// Connected, and no sessions.
    NoSessions,
}

/// The card slots' rects for `n` remotes on a sheet `w` wide, from `top`,
/// at scale `s`: 4 across with 8 pt gaps, filling the first row then the
/// second, the **+** after the last when fewer than 8.
pub fn grid(w: f32, top: f32, n: usize, s: f32) -> Vec<Rect> {
    let slots = n.min(MAX_REMOTES) + usize::from(n < MAX_REMOTES);
    let pad = PAD * s;
    let gap = CARD_GAP * s;
    let cw = (w - 2.0 * pad - (COLUMNS as f32 - 1.0) * gap) / COLUMNS as f32;
    let ch = CARD_H * s;
    (0..slots)
        .map(|i| {
            let (row, col) = (i / COLUMNS, i % COLUMNS);
            Rect::new(
                pad + col as f32 * (cw + gap),
                top + row as f32 * (ch + gap),
                cw,
                ch,
            )
        })
        .collect()
}

/// The note's one row, dim, in the list.
#[derive(Clone, Debug)]
pub struct NoteRow {
    pub rect: Rect,
    pub text: String,
    pub scale: f32,
}

impl DrawControl for NoteRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = NOTE_PT * s;
        let x = self.rect.x + 4.0 * s;
        let room = self.rect.w - 8.0 * s;
        let text = cut_right(ctx, &self.text, px, room);
        let y = self.rect.y + (self.rect.h - line_h(px)) * 0.5;
        ctx.draw_text(list, &text, [x, y], px, theme().dim);
    }
}

/// It takes no taps.
impl HostedControl for NoteRow {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, _pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        None
    }

    fn handle_mouse(&self, _event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        None
    }

    fn handle_keyboard(
        &self,
        _event: &KeyboardEvent,
        _ctx: &EventContext,
    ) -> Option<ControlAction> {
        None
    }
}

/// The sheet's controls, as data and as built.
#[derive(Debug)]
pub struct DrawerContent {
    remotes: Vec<RemoteInfo>,
    /// Background remotes with a question or an approval waiting.
    waiting: HashSet<RemoteId>,
    agents: Vec<AgentInfo>,
    /// A list has been given (kept or the daemon's), since the last
    /// [`DrawerContent::unload`].
    listed: bool,
    /// When the list came, on the app's clock.
    received_at: Duration,
    pending: Vec<String>,
    open_pane: Option<String>,
    /// The next try, while the remote in use cannot be reached.
    retry_in: Option<u32>,
    /// The sheet's size, the safe area's top and bottom, the scale.
    size: [f32; 2],
    insets: [f32; 2],
    scale: f32,
    scroll: Arc<ScrollState>,
    /// As built: the cards' rects and slots, the list's viewport, its pane
    /// and its rows' nodes.
    cards: Vec<(Rect, Slot, NodeId)>,
    list: Rect,
    pane: Option<NodeId>,
    rows: Vec<(NodeId, String)>,
    /// The card held, and the hold's progress.
    press: Option<(usize, f32)>,
    /// The spinner's frame last drawn, while any row spins.
    frame: Option<usize>,
    /// Something it shows changed: built again before it is drawn.
    dirty: bool,
}

impl Default for DrawerContent {
    fn default() -> Self {
        DrawerContent {
            remotes: Vec::new(),
            waiting: HashSet::new(),
            agents: Vec::new(),
            listed: false,
            received_at: Duration::ZERO,
            pending: Vec::new(),
            open_pane: None,
            retry_in: None,
            size: [0.0; 2],
            insets: [0.0; 2],
            scale: 1.0,
            scroll: ScrollState::new(),
            cards: Vec::new(),
            list: Rect::new(0.0, 0.0, 0.0, 0.0),
            pane: None,
            rows: Vec::new(),
            press: None,
            frame: None,
            dirty: true,
        }
    }
}

impl DrawerContent {
    pub fn set_remotes(&mut self, remotes: Vec<RemoteInfo>) {
        if remotes != self.remotes {
            self.remotes = remotes;
            self.dirty = true;
        }
    }

    pub fn remotes(&self) -> &[RemoteInfo] {
        &self.remotes
    }

    /// The remote in use, if one is.
    pub fn active(&self) -> Option<&RemoteInfo> {
        self.remotes.iter().find(|r| r.active)
    }

    /// The background remotes whose cards show a dot.
    pub fn waiting(&self) -> &HashSet<RemoteId> {
        &self.waiting
    }

    pub fn set_waiting(&mut self, waiting: HashSet<RemoteId>) {
        if waiting != self.waiting {
            self.waiting = waiting;
            self.dirty = true;
        }
    }

    /// A list of the remote in use's agents, received at `now`.
    pub fn set_agents(&mut self, agents: Vec<AgentInfo>, now: Duration) {
        self.agents = agents;
        self.listed = true;
        self.received_at = now;
        self.dirty = true;
    }

    /// No list: the rows go and no note shows until one is given (a switch
    /// to a remote with none kept).
    pub fn unload(&mut self) {
        self.agents.clear();
        self.listed = false;
        self.dirty = true;
    }

    pub fn agents(&self) -> &[AgentInfo] {
        &self.agents
    }

    pub fn set_pending(&mut self, pending: Vec<String>) {
        if pending != self.pending {
            self.pending = pending;
            self.dirty = true;
        }
    }

    /// The pane whose agent view is open under the drawer.
    pub fn set_open_pane(&mut self, pane: Option<String>) {
        if pane != self.open_pane {
            self.open_pane = pane;
            self.dirty = true;
        }
    }

    /// The remote in use's connection failed; the next try in `retry_in`
    /// seconds (`None`: connected, or connecting).
    pub fn set_retry(&mut self, retry_in: Option<u32>) {
        if retry_in != self.retry_in {
            self.retry_in = retry_in;
            self.dirty = true;
        }
    }

    /// The sheet's size and the safe area's insets, at scale `s`.
    pub fn set_frame(&mut self, size: [f32; 2], insets: [f32; 2], s: f32) {
        if size != self.size || insets != self.insets || s != self.scale {
            self.size = size;
            self.insets = insets;
            self.scale = s;
            self.dirty = true;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks it to be built again (a theme change).
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// The list's one note row, if it shows one: none while there are
    /// rows, and no looking-for message (plan 20261028_03, the 2026-10-30
    /// 13:15 EDT amendment): with no rows, a failed connection's next
    /// try, or *No sessions* once a list came empty.
    pub fn note(&self) -> Option<Note> {
        let active = self.active()?;
        if !self.agents.is_empty() {
            return None;
        }
        if !active.online {
            return self.retry_in.map(Note::Unreachable);
        }
        self.listed.then_some(Note::NoSessions)
    }

    /// The note's words.
    pub fn note_text(&self, note: &Note) -> String {
        let name = self
            .active()
            .map(|r| r.display_name.as_str())
            .unwrap_or_default();
        let text = match note {
            Note::Unreachable(secs) => copy::DRAWER_UNREACHABLE
                .fill(None, &[("nickname", name), ("seconds", &secs.to_string())]),
            Note::NoSessions => copy::DRAWER_NO_SESSIONS.fill(None, &[("nickname", name)]),
        };
        text.trim().to_string()
    }

    /// `agent`'s activity at `now`.
    pub fn activity_of(&self, agent: &AgentInfo, now: Duration) -> Activity {
        let approval = self.pending.contains(&agent.pane_id);
        activity::activity(agent, approval, Signals::of(agent, self.received_at), now)
    }

    /// Makes the content router's nodes for what it shows, at `now`.
    pub fn build(&mut self, router: &mut Router, now: Duration) {
        let s = self.scale;
        let [w, h] = self.size;
        let pad = PAD * s;
        *router = Router::new();
        let top = self.insets[0] + pad;
        let rects = grid(w, top, self.remotes.len(), s);
        let mut slots: Vec<Slot> = self
            .remotes
            .iter()
            .take(MAX_REMOTES)
            .cloned()
            .map(Slot::Remote)
            .collect();
        if slots.len() < rects.len() {
            slots.push(Slot::Add);
        }
        self.cards = rects
            .iter()
            .zip(slots)
            .enumerate()
            .map(|(i, (&rect, slot))| {
                let waiting =
                    matches!(&slot, Slot::Remote(r) if !r.active && self.waiting.contains(&r.id));
                let mut card = RemoteCard::new(rect, slot.clone(), waiting, s);
                card.pressed = self.press.filter(|(p, _)| *p == i).map(|(_, t)| t);
                let id = router.add(None, card);
                (rect, slot, id)
            })
            .collect();
        let grid_bottom = rects.last().map_or(top, |r| r.y + r.h);
        let list_top = grid_bottom + pad;
        let list_bottom = (h - self.insets[1] - pad).max(list_top);
        self.list = Rect::new(pad, list_top, w - 2.0 * pad, list_bottom - list_top);
        let pane = router.add(
            None,
            ScrollPane::new(self.list, ScrollAxis::Vertical).with_state(Arc::clone(&self.scroll)),
        );
        self.pane = Some(pane);
        self.rows.clear();
        let row_h = ROW_H * s;
        let mut y = list_top;
        if let Some(note) = self.note() {
            let text = self.note_text(&note);
            router.add(
                Some(pane),
                NoteRow {
                    rect: Rect::new(self.list.x, y, self.list.w, row_h),
                    text,
                    scale: s,
                },
            );
            y += row_h;
        }
        // The rows show online or not: a kept list shows at once (the
        // 13:15 EDT amendment, item 2).
        for agent in &self.agents {
            let approval = self.pending.contains(&agent.pane_id);
            let open = self.open_pane.as_deref() == Some(agent.pane_id.as_str());
            let mut row = SessionRow::new(agent, approval, self.activity_of(agent, now), open, s);
            row.rect = Rect::new(self.list.x, y, self.list.w, row_h);
            let id = router.add(Some(pane), row);
            self.rows.push((id, agent.pane_id.clone()));
            y += row_h;
        }
        if let Some(p) = router.get::<ScrollPane>(pane) {
            p.set_content_size(Some([self.list.w, y - list_top]));
        }
        self.frame = self.spinning(router).then(|| spinner::frame_at(now));
        self.dirty = false;
    }

    /// Whether any row spins.
    fn spinning(&self, router: &Router) -> bool {
        self.rows.iter().any(|(id, _)| {
            router
                .get::<SessionRow>(*id)
                .is_some_and(|r| r.activity == Activity::Processing)
        })
    }

    /// Judges each row again at `now`, in place. Whether the sheet must be
    /// drawn again: a row's activity or age changed, or the spinner's
    /// frame moved on.
    pub fn refresh(&mut self, router: &mut Router, now: Duration) -> bool {
        let mut changed = false;
        for (id, pane) in &self.rows {
            let Some(agent) = self.agents.iter().find(|a| &a.pane_id == pane) else {
                continue;
            };
            let activity = self.activity_of(agent, now);
            if let Some(row) = router.get_mut::<SessionRow>(*id) {
                if row.indicator() != super::session_row::indicator(activity) {
                    changed = true;
                }
                row.activity = activity;
            }
        }
        let frame = self.spinning(router).then(|| spinner::frame_at(now));
        if frame != self.frame {
            self.frame = frame;
            changed = true;
        }
        changed
    }

    /// The card slot at `pos`, in sheet coordinates.
    pub fn card_at(&self, pos: [f32; 2]) -> Option<usize> {
        self.cards.iter().position(|(r, ..)| r.contains(pos))
    }

    /// What a tap on card `i` is.
    pub fn card_hit(&self, i: usize) -> Option<DrawerHit> {
        match &self.cards.get(i)?.1 {
            Slot::Remote(r) => Some(DrawerHit::Remote(r.id)),
            Slot::Add => Some(DrawerHit::AddRemote),
        }
    }

    /// Where card `i` is, in sheet coordinates.
    pub fn card_rect(&self, i: usize) -> Option<Rect> {
        self.cards.get(i).map(|(r, ..)| *r)
    }

    /// The cards' slots, in order.
    pub fn slots(&self) -> Vec<Slot> {
        self.cards.iter().map(|(_, s, _)| s.clone()).collect()
    }

    /// The card held and the hold's progress, shown on it in place.
    pub fn set_press(&mut self, router: &mut Router, press: Option<(usize, f32)>) -> bool {
        if press == self.press {
            return false;
        }
        self.press = press;
        for (i, (_, _, id)) in self.cards.iter().enumerate() {
            if let Some(card) = router.get_mut::<RemoteCard>(*id) {
                card.pressed = press.filter(|(p, _)| *p == i).map(|(_, t)| t);
            }
        }
        true
    }

    /// The session a node is the row of.
    pub fn row_hit(&self, node: NodeId) -> Option<DrawerHit> {
        self.rows
            .iter()
            .find(|(id, _)| *id == node)
            .map(|(_, pane)| DrawerHit::Session(pane.clone()))
    }

    /// The rows' panes, in order.
    pub fn row_panes(&self) -> Vec<String> {
        self.rows.iter().map(|(_, p)| p.clone()).collect()
    }

    /// Where row `i` is now, in sheet coordinates (scrolled).
    pub fn row_rect(&self, router: &Router, i: usize) -> Option<Rect> {
        let (id, _) = self.rows.get(i)?;
        let row = router.get::<SessionRow>(*id)?;
        let dy = self.scroll.offset()[1];
        Some(Rect::new(
            row.rect.x,
            row.rect.y - dy,
            row.rect.w,
            row.rect.h,
        ))
    }

    /// The list's window, in sheet coordinates.
    pub fn list_rect(&self) -> Rect {
        self.list
    }

    /// The spinner's glyph cells of the rows that spin, in sheet
    /// coordinates, and their colours: drawn over the rows, in the
    /// terminal face, clipped to [`DrawerContent::list_rect`].
    pub fn spinners(&self, router: &Router) -> Vec<(Rect, [f32; 4])> {
        let s = self.scale;
        (0..self.rows.len())
            .filter_map(|i| {
                let (id, _) = &self.rows[i];
                let row = router.get::<SessionRow>(*id)?;
                (row.activity == Activity::Processing).then_some(())?;
                let r = self.row_rect(router, i)?;
                let col = Rect::new(r.x + r.w - COLUMN_W * s, r.y, COLUMN_W * s, r.h);
                (col.y + col.h > self.list.y && col.y < self.list.y + self.list.h)
                    .then_some((col, row.colour))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawer::remote_card::tests::info;
    use crate::drawer::session_row::tests::agent;
    use libflankr_proto::common::Turn;

    /// The S23 Ultra's width in points, at scale 1.
    const W: f32 = 384.0;

    fn remotes(n: usize) -> Vec<RemoteInfo> {
        (0..n)
            .map(|i| {
                let mut r = info(&format!("m{i}"));
                r.id = RemoteId([i as u8; 8]);
                r.active = i == 0;
                r.online = true;
                r
            })
            .collect()
    }

    #[test]
    fn the_grid_has_the_plus_in_the_slot_after_the_last_card_and_none_at_8() {
        // The sheet on a 384 pt screen: 0.85 of it.
        let sheet = W * 0.85;
        for (n, slots, rows) in [(0, 1, 1), (3, 4, 1), (4, 5, 2), (7, 8, 2), (8, 8, 2)] {
            let mut c = DrawerContent::default();
            c.set_remotes(remotes(n));
            c.set_frame([sheet, 800.0], [40.0, 20.0], 1.0);
            let mut router = Router::new();
            c.build(&mut router, Duration::ZERO);
            let s = c.slots();
            assert_eq!(s.len(), slots, "{n} remotes");
            assert_eq!(s.last() == Some(&Slot::Add), n < 8, "{n} remotes");
            assert_eq!(
                s.iter().filter(|x| **x == Slot::Add).count(),
                usize::from(n < 8)
            );
            let ys: HashSet<i32> = (0..slots)
                .map(|i| c.card_rect(i).unwrap().y as i32)
                .collect();
            assert_eq!(ys.len(), rows, "{n} remotes: rows");
            // 4 across, 8 pt apart, 12 pt in, below the top inset.
            let first = c.card_rect(0).unwrap();
            assert_eq!((first.x, first.y), (12.0, 52.0));
            assert!((first.w - (sheet - 24.0 - 24.0) / 4.0).abs() < 0.01);
            assert_eq!(first.h, 52.0);
            if slots > 4 {
                let fifth = c.card_rect(4).unwrap();
                assert_eq!((fifth.x, fifth.y), (12.0, 52.0 + 60.0));
            }
            // The list, 12 pt under the grid.
            let last = c.card_rect(slots - 1).unwrap();
            assert_eq!(c.list_rect().y, last.y + last.h + 12.0);
        }
    }

    #[test]
    fn a_tap_on_a_card_is_its_remote_and_on_the_plus_add() {
        let mut c = DrawerContent::default();
        c.set_remotes(remotes(2));
        c.set_frame([326.0, 800.0], [0.0, 0.0], 1.0);
        let mut router = Router::new();
        c.build(&mut router, Duration::ZERO);
        let at = |i| c.card_rect(i).unwrap().center();
        assert_eq!(c.card_at(at(1)), Some(1));
        assert_eq!(c.card_hit(1), Some(DrawerHit::Remote(RemoteId([1; 8]))));
        assert_eq!(c.card_hit(2), Some(DrawerHit::AddRemote));
        assert_eq!(c.card_at([2.0, 2.0]), None);
    }

    #[test]
    fn a_kept_list_shows_its_rows_while_offline_and_unload_shows_nothing() {
        crate::theme::testing::use_theme("dark_gray");
        let mut c = DrawerContent::default();
        let mut rs = remotes(1);
        rs[0].online = false;
        c.set_remotes(rs);
        c.set_frame([326.0, 800.0], [0.0, 0.0], 1.0);
        c.set_retry(Some(4));
        c.set_agents(vec![agent("p1", Some(1), None)], Duration::ZERO);
        let mut router = Router::new();
        c.build(&mut router, Duration::ZERO);
        assert_eq!(c.row_panes(), ["p1"]);
        assert_eq!(c.note(), None, "rows, not the retry");
        c.unload();
        c.build(&mut router, Duration::ZERO);
        assert!(c.row_panes().is_empty());
        assert_eq!(c.note(), Some(Note::Unreachable(4)));
    }

    #[test]
    fn the_rows_are_the_list_in_order_and_the_note_shows_without_one() {
        crate::theme::testing::use_theme("dark_gray");
        let mut c = DrawerContent::default();
        let mut rs = remotes(2);
        rs[0].online = false;
        c.set_remotes(rs.clone());
        c.set_frame([326.0, 800.0], [0.0, 0.0], 1.0);
        let mut router = Router::new();
        c.build(&mut router, Duration::ZERO);
        // Connecting: no looking-for message.
        assert_eq!(c.note(), None);
        c.set_retry(Some(4));
        assert_eq!(
            c.note_text(&c.note().unwrap()),
            "Can't reach m0 · retrying in 4 s"
        );
        rs[0].online = true;
        c.set_remotes(rs);
        c.set_retry(None);
        // Online with no list yet: nothing; an empty list: No sessions.
        assert_eq!(c.note(), None);
        c.set_agents(Vec::new(), Duration::ZERO);
        assert_eq!(c.note(), Some(Note::NoSessions));
        assert_eq!(c.note_text(&Note::NoSessions), "No sessions on m0");
        c.set_agents(
            vec![agent("p2", Some(2), None), agent("p1", Some(1), None)],
            Duration::ZERO,
        );
        c.set_open_pane(Some("p1".into()));
        c.build(&mut router, Duration::ZERO);
        assert_eq!(c.note(), None);
        assert_eq!(c.row_panes(), ["p2", "p1"], "agents.list order");
        let r0 = c.row_rect(&router, 0).unwrap();
        let r1 = c.row_rect(&router, 1).unwrap();
        assert_eq!(r1.y, r0.y + 44.0);
        assert_eq!(r0.y, c.list_rect().y);
        let open = |i: usize| router.get::<SessionRow>(c.rows[i].0).unwrap().open;
        assert!(!open(0) && open(1), "the view open under it");
        assert_eq!(
            c.row_hit(c.rows[1].0),
            Some(DrawerHit::Session("p1".into()))
        );
    }

    #[test]
    fn a_row_stops_spinning_recent_after_its_last_write_between_lists() {
        crate::theme::testing::use_theme("dark_gray");
        let mut c = DrawerContent::default();
        c.set_remotes(remotes(1));
        c.set_frame([326.0, 800.0], [0.0, 0.0], 1.0);
        let mut a = agent(
            "p1",
            Some(1),
            Some(Turn::Stopped {
                line: "done".into(),
            }),
        );
        a.idle_ms = Some(2_000);
        let t0 = Duration::from_secs(100);
        c.set_agents(vec![a], t0);
        let mut router = Router::new();
        c.build(&mut router, t0);
        assert_eq!(c.spinners(&router).len(), 1, "written 2 s ago");
        // The next frame of the spinner: drawn again.
        assert!(c.refresh(&mut router, t0 + Duration::from_millis(80)));
        assert!(!c.refresh(&mut router, t0 + Duration::from_millis(100)));
        // 10 s after the write: idle, "now".
        assert!(c.refresh(&mut router, t0 + Duration::from_secs(9)));
        assert!(c.spinners(&router).is_empty());
        let row = router.get::<SessionRow>(c.rows[0].0).unwrap();
        assert_eq!(
            row.indicator(),
            crate::drawer::session_row::Indicator::Age("now".into())
        );
        // Each spinning row shows the one clock's frame.
        assert!(!c.refresh(&mut router, t0 + Duration::from_secs(10)));
    }
}
