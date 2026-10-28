//! The session drawer (docs/design/20261028_03_SESSION_DRAWER_PLAN.md,
//! steps 6-8, with the 18:48 and 18:51 EDT amendments): a sheet from the
//! left over the overview or an agent view, opened by the ≡ button at the
//! left of their headers ([`Menu`]) and closed by a tap on the scrim, Back,
//! or choosing something in it.
//!
//! It is libcontrols' `DrawerLayout` (3.9): its main router stays empty,
//! as the app's screens draw and route themselves; its content router
//! holds the sheet's controls ([`content::DrawerContent`]). The app gives
//! it touches only while it shows, marks its main view changed whenever
//! the screen under it drew, and draws by its `LayerPlan`: straight to the
//! surface while it is closed, as without a drawer, and only then through
//! its two textures and one composite pass.
//!
//! What a touch in it does ([`AppDrawer::touch`], [`AppDrawer::tick`]):
//!
//! - a **tap** on a remote's card switches to that remote
//!   ([`DrawerHit::Remote`]; a tap on the one in use does nothing, which
//!   the app decides); on the **+**, the pairing window's add mode;
//! - a **2 s press** on any remote's card opens that remote's details
//!   ([`DrawerHit::Details`]); the card shows the press while it is held,
//!   and a press that became a hold is no tap (the owner, 2026-10-28:
//!   "only pressing for 2s should open the details and allow removing
//!   remote from card view");
//! - a tap on a session's row opens its agent view;
//! - a drag in the list scrolls it; a tap on the scrim closes the drawer.

pub mod content;
pub mod remote_card;
pub mod session_row;
pub mod spinner;

use std::time::Duration;

use flankr_companion::RemoteId;
use libcontrols::{
    DrawContext, DrawerEdge, DrawerLayout, DrawerOptions, DrawerPhase, DrawerRoute, DrawerSize,
    DrawerTheme, InputEvent, MouseButton, MouseEvent, Rect,
};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use self::content::DrawerContent;
use self::remote_card::Slot;

/// How long a card is held to open its remote's details.
pub const HOLD: Duration = Duration::from_secs(2);
/// How far a finger moves, in points, before a press on a card is no tap.
const SLOP: f32 = 10.0;

/// What a touch in the drawer chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawerHit {
    /// A tap on a remote's card.
    Remote(RemoteId),
    /// A tap on the **+**.
    AddRemote,
    /// A 2 s press on a remote's card.
    Details(RemoteId),
    /// A tap on a session's row: its pane.
    Session(String),
}

/// What [`AppDrawer::touch`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawerTouch {
    /// Nothing yet: a press going on, a scroll.
    Nothing,
    /// Something was chosen.
    Hit(DrawerHit),
    /// A tap on the scrim: the drawer closes.
    Close,
}

/// `text` cut at its right with "…" to fit `room` at `px`.
pub fn cut_right(ctx: &DrawContext, text: &str, px: f32, room: f32) -> String {
    if ctx.text_advance(text, px) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for n in (0..chars.len()).rev() {
        let mut t: String = chars[..n].iter().collect::<String>().trim_end().to_string();
        t.push('\u{2026}');
        if ctx.text_advance(&t, px) <= room || n == 0 {
            return t;
        }
    }
    "\u{2026}".to_string()
}

/// `text` cut at its left with "…" to fit `room` at `px`: an address keeps
/// its distinguishing end (`…1.20`).
pub fn cut_left(ctx: &DrawContext, text: &str, px: f32, room: f32) -> String {
    if ctx.text_advance(text, px) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for n in 1..=chars.len() {
        let mut t = String::from('\u{2026}');
        t.extend(&chars[n..]);
        if ctx.text_advance(&t, px) <= room {
            return t;
        }
    }
    "\u{2026}".to_string()
}

/// The theme's drawer values at scale `s` (3.9 item 37): the size, corner
/// and shadow in the app's pixels, and an opaque sheet in the theme's
/// background (the default 85% would show the screen through it). The
/// rest is `DrawerTheme::default()`'s.
pub fn drawer_theme(s: f32) -> DrawerTheme {
    let mut t = DrawerTheme::default();
    t.size = DrawerSize::Share {
        share: 0.85,
        max: 360.0 * s,
    };
    t.corner_radius = 16.0 * s;
    t.shadow = (16.0 * s, t.shadow.1);
    let mut bg = crate::theme::theme().bg;
    bg[3] = 1.0;
    t.sheet_color = Some(bg);
    t
}

/// The ≡ button at the left of the overview's and the agent view's header
/// (3.10 item 42; the 18:48 EDT amendment, item 1): a 44 pt target, three
/// 2 pt bars.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Menu {
    pub rect: Rect,
}

impl Menu {
    /// The target's side, in points.
    pub const SIZE: f32 = 44.0;

    /// At `x`, centred down a line from `y`, `line` high, at scale `s`.
    pub fn place(x: f32, y: f32, line: f32, s: f32) -> Self {
        let side = Self::SIZE * s;
        Menu {
            rect: Rect::new(x, y + (line - side) * 0.5, side, side),
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rect.contains([x, y])
    }

    /// The three bars, 18 pt wide, 5 pt apart, in `color`.
    pub fn draw(&self, list: &mut DrawList, s: f32, color: [f32; 4]) {
        let [cx, cy] = self.rect.center();
        let w = 18.0 * s;
        let t = 2.0 * s;
        for i in [-1.0f32, 0.0, 1.0] {
            let y = cy + i * 5.0 * s - t * 0.5;
            crate::agent::fill(list, Rect::new(cx - w * 0.5, y, w, t), color, t * 0.5);
        }
    }
}

/// A finger in the drawer.
#[derive(Clone, Copy, Debug)]
struct Press {
    id: u64,
    /// On a card: its slot, and the press is timed.
    card: Option<usize>,
    start: Duration,
    at: [f32; 2],
    /// It became a hold: the details were opened, and it is no tap.
    held: bool,
    /// It moved past the slop: no tap and no hold.
    moved: bool,
}

/// The app's drawer: its `DrawerLayout`, what the sheet shows, and the
/// finger in it.
pub struct AppDrawer {
    layout: DrawerLayout,
    content: DrawerContent,
    press: Option<Press>,
    scale: f32,
    /// The app last told the client the drawer shows.
    told: bool,
}

impl Default for AppDrawer {
    fn default() -> Self {
        AppDrawer::new()
    }
}

impl AppDrawer {
    pub fn new() -> Self {
        AppDrawer {
            layout: DrawerLayout::new(
                Rect::new(0.0, 0.0, 1.0, 1.0),
                DrawerEdge::Left,
                DrawerOptions::new(),
            ),
            content: DrawerContent::default(),
            press: None,
            scale: 1.0,
            told: false,
        }
    }

    pub fn layout(&self) -> &DrawerLayout {
        &self.layout
    }

    pub fn content(&self) -> &DrawerContent {
        &self.content
    }

    pub fn content_mut(&mut self) -> &mut DrawerContent {
        &mut self.content
    }

    /// Opening, open or closing: touches and Back are the drawer's.
    pub fn is_shown(&self) -> bool {
        self.layout.state().phase() != DrawerPhase::Closed
    }

    /// Any of it is on screen (`t > 0`).
    pub fn is_visible(&self) -> bool {
        self.layout.drawer().is_visible()
    }

    /// The ≡ button: slides in.
    pub fn open(&mut self) {
        self.layout.open();
    }

    /// Back, the scrim, a row: slides out.
    pub fn close(&mut self) {
        self.press = None;
        self.layout.close();
    }

    /// The **+**, Settings, the pairing window: gone at once.
    pub fn close_now(&mut self) {
        self.press = None;
        self.layout.set_open_now(false);
    }

    /// Whether the client is to be told the drawer shows (`Some(on)`):
    /// once each time that changes.
    pub fn take_shown_change(&mut self) -> Option<bool> {
        let shown = self.is_shown();
        (shown != self.told).then(|| {
            self.told = shown;
            shown
        })
    }

    /// The surface's size, the safe area's top and bottom, the scale.
    pub fn set_frame(&mut self, w: f32, h: f32, top: f32, bottom: f32, s: f32) {
        self.scale = s;
        let theme = libcontrols::ControlTheme {
            drawer: drawer_theme(s),
            ..libcontrols::ControlTheme::default()
        };
        self.layout.set_theme(&theme);
        self.layout.set_rect(Rect::new(0.0, 0.0, w, h));
        let size = self.layout.drawer().sheet_open().size();
        self.content.set_frame(size, [top, bottom], s);
    }

    /// The screen under the drawer drew: its layer is drawn again.
    pub fn mark_main(&self) {
        self.layout.mark_main();
    }

    /// Moves the drawer on by `dt`, builds the content again if what it
    /// shows changed, and judges its rows again at `now`. Returns the
    /// details to open when a card has been held for [`HOLD`].
    pub fn tick(&mut self, dt: Duration, now: Duration) -> Option<DrawerHit> {
        self.layout.advance(dt.as_secs_f32());
        if !self.is_shown() {
            self.press = None;
        }
        let mut hit = None;
        let mut press = None;
        if let Some(p) = self.press.as_mut()
            && let Some(card) = p.card
            && !p.moved
            && !p.held
        {
            let held = now.saturating_sub(p.start);
            if held >= HOLD {
                p.held = true;
                if let Some(Slot::Remote(r)) = self.content.slots().get(card) {
                    hit = Some(DrawerHit::Details(r.id));
                }
            } else {
                press = Some((card, held.as_secs_f32() / HOLD.as_secs_f32()));
            }
        }
        let router = self.layout.content_mut();
        let mut changed = false;
        if self.content.is_dirty() {
            self.content.build(router, now);
            changed = true;
        }
        changed |= self.content.set_press(router, press);
        changed |= self.content.refresh(router, now);
        if changed {
            self.layout.drawer().content_mark().mark();
        }
        hit
    }

    /// A touch at `(x, y)` (the app's pixels) at `now`, while the drawer
    /// shows.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
    ) -> DrawerTouch {
        let pos = [x, y];
        let left = MouseButton::Left;
        let sheet = self.layout.drawer().sheet();
        let local = [x - sheet.x, y - sheet.y];
        match phase {
            TouchPhase::Started => {
                if self.press.is_some() {
                    return DrawerTouch::Nothing;
                }
                let card = sheet
                    .contains(pos)
                    .then(|| self.content.card_at(local))
                    .flatten();
                self.press = Some(Press {
                    id,
                    card,
                    start: now,
                    at: pos,
                    held: false,
                    moved: false,
                });
                self.layout
                    .deliver(InputEvent::Mouse(MouseEvent::Down { pos, button: left }));
                DrawerTouch::Nothing
            }
            TouchPhase::Moved => {
                let Some(p) = self.press.as_mut().filter(|p| p.id == id) else {
                    return DrawerTouch::Nothing;
                };
                let slop = SLOP * self.scale;
                if (x - p.at[0]).hypot(y - p.at[1]) > slop {
                    p.moved = true;
                }
                let start = p.at;
                self.layout.deliver(InputEvent::Mouse(MouseEvent::Drag {
                    start_pos: start,
                    current_pos: pos,
                    delta: [0.0, 0.0],
                    button: left,
                }));
                DrawerTouch::Nothing
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(p) = self.press.take().filter(|p| p.id == id) else {
                    return DrawerTouch::Nothing;
                };
                let route = self
                    .layout
                    .deliver(InputEvent::Mouse(MouseEvent::Up { pos, button: left }));
                if phase == TouchPhase::Cancelled {
                    return DrawerTouch::Nothing;
                }
                if route == DrawerRoute::Scrim {
                    return DrawerTouch::Close;
                }
                if let Some(card) = p.card {
                    // A press that became a hold, or moved away, is no tap.
                    let on_it = self
                        .content
                        .card_rect(card)
                        .is_some_and(|r| r.contains(local));
                    return match (p.held || p.moved || !on_it, self.content.card_hit(card)) {
                        (false, Some(hit)) => DrawerTouch::Hit(hit),
                        _ => DrawerTouch::Nothing,
                    };
                }
                match self
                    .layout
                    .deliver(InputEvent::Mouse(MouseEvent::Click { pos, button: left }))
                {
                    DrawerRoute::Drawer(Some(r)) if r.action.is_some() => r
                        .target
                        .and_then(|t| self.content.row_hit(t))
                        .map_or(DrawerTouch::Nothing, DrawerTouch::Hit),
                    _ => DrawerTouch::Nothing,
                }
            }
        }
    }

    /// The sheet, in the app's pixels, as it is now.
    pub fn sheet(&self) -> Rect {
        self.layout.drawer().sheet()
    }

    /// Where card `i` is when the drawer is open, in the app's pixels.
    pub fn card_on_screen(&self, i: usize) -> Option<Rect> {
        let o = self.layout.drawer().sheet_open();
        self.content
            .card_rect(i)
            .map(|r| Rect::new(r.x + o.x, r.y + o.y, r.w, r.h))
    }

    /// Where row `i` is when the drawer is open, in the app's pixels.
    pub fn row_on_screen(&self, i: usize) -> Option<Rect> {
        let o = self.layout.drawer().sheet_open();
        self.content
            .row_rect(self.layout.content(), i)
            .map(|r| Rect::new(r.x + o.x, r.y + o.y, r.w, r.h))
    }

    /// The sheet's drawing, in its own coordinates, for its texture: the
    /// layout's, then the spinners over the rows that spin, in the
    /// terminal face, clipped to the list.
    pub fn draw_sheet(
        &self,
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&crate::agent::Terminal>,
        now: Duration,
    ) {
        self.layout.render_sheet(list, ctx);
        let spinners = self.content.spinners(self.layout.content());
        if spinners.is_empty() {
            return;
        }
        let clip = self.content.list_rect();
        list.push_clip(clip.pos(), clip.size());
        let glyph = spinner::glyph_at(now).to_string();
        let px = session_row::GLYPH_PT * self.scale;
        for (cell, colour) in spinners {
            let row = crate::agent::Row {
                text: glyph.clone(),
                x: 0.0,
                y: 0.0,
                px,
                color: colour,
                mono: true,
            };
            let w = px * 0.6;
            let x = cell.x + (cell.w - w) * 0.5;
            let y = cell.y + (cell.h - crate::agent::term_h(px)) * 0.5;
            crate::agent::draw_row(list, ctx, term, &row, [x, y]);
        }
        list.push_clip_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawer::remote_card::tests::info;
    use crate::drawer::session_row::tests::agent;

    /// The drawer on a phone 1080 × 2340 at scale 2.8 (about 384 pt wide),
    /// open, with `n` remotes (the first in use, online) and two sessions.
    fn opened(n: usize) -> AppDrawer {
        crate::theme::testing::use_theme("dark_gray");
        let mut d = AppDrawer::new();
        d.set_frame(1080.0, 2340.0, 80.0, 40.0, 2.8);
        let remotes = (0..n)
            .map(|i| {
                let mut r = info(&format!("m{i}"));
                r.id = RemoteId([i as u8; 8]);
                r.active = i == 0;
                r.online = true;
                r
            })
            .collect();
        d.content_mut().set_remotes(remotes);
        d.content_mut().set_agents(
            vec![agent("p1", Some(1), None), agent("p2", Some(2), None)],
            Duration::ZERO,
        );
        d.open();
        for _ in 0..30 {
            d.tick(Duration::from_millis(33), Duration::ZERO);
        }
        assert!(d.is_visible());
        assert_eq!(d.layout().state().phase(), DrawerPhase::Open);
        d
    }

    fn tap(d: &mut AppDrawer, r: Rect, now: Duration) -> DrawerTouch {
        let [x, y] = r.center();
        d.touch(1, TouchPhase::Started, x, y, now);
        d.touch(1, TouchPhase::Ended, x, y, now + Duration::from_millis(80))
    }

    #[test]
    fn the_sheet_is_085_of_the_width_up_to_360_pt_and_opaque() {
        let d = opened(2);
        let sheet = d.sheet();
        assert_eq!(
            (sheet.x, sheet.w),
            (0.0, (1080.0f32 * 0.85).min(360.0 * 2.8))
        );
        let t = drawer_theme(2.8);
        assert_eq!(t.sheet_color.unwrap()[3], 1.0);
        assert_eq!(t.corner_radius, 16.0 * 2.8);
    }

    #[test]
    fn a_tap_on_a_card_is_its_remote_a_tap_on_the_plus_add_and_on_a_row_its_session() {
        let mut d = opened(3);
        let now = Duration::from_secs(5);
        let card = d.card_on_screen(1).unwrap();
        assert_eq!(
            tap(&mut d, card, now),
            DrawerTouch::Hit(DrawerHit::Remote(RemoteId([1; 8])))
        );
        // The one in use is a tap too; the app does nothing with it.
        let card = d.card_on_screen(0).unwrap();
        assert_eq!(
            tap(&mut d, card, now),
            DrawerTouch::Hit(DrawerHit::Remote(RemoteId([0; 8])))
        );
        let plus = d.card_on_screen(3).unwrap();
        assert_eq!(
            tap(&mut d, plus, now),
            DrawerTouch::Hit(DrawerHit::AddRemote)
        );
        let row = d.row_on_screen(1).unwrap();
        assert_eq!(
            tap(&mut d, row, now),
            DrawerTouch::Hit(DrawerHit::Session("p2".into()))
        );
    }

    #[test]
    fn a_2_s_press_on_any_card_opens_its_details_and_is_no_tap() {
        for i in [0usize, 1] {
            let mut d = opened(2);
            let t0 = Duration::from_secs(10);
            let [x, y] = d.card_on_screen(i).unwrap().center();
            assert_eq!(
                d.touch(7, TouchPhase::Started, x, y, t0),
                DrawerTouch::Nothing
            );
            // Held: the card shows the press, and how far it has come.
            assert_eq!(
                d.tick(Duration::from_millis(33), t0 + Duration::from_secs(1)),
                None
            );
            let press = d.content().slots().len();
            assert!(press > 0);
            assert_eq!(
                d.tick(Duration::from_millis(33), t0 + HOLD),
                Some(DrawerHit::Details(RemoteId([i as u8; 8]))),
                "card {i}"
            );
            assert_eq!(
                d.tick(Duration::from_millis(33), t0 + HOLD * 2),
                None,
                "once"
            );
            assert_eq!(
                d.touch(7, TouchPhase::Ended, x, y, t0 + HOLD * 2),
                DrawerTouch::Nothing,
                "the hold is no tap"
            );
        }
    }

    #[test]
    fn a_press_released_early_is_a_tap_and_one_moved_away_is_neither() {
        let mut d = opened(2);
        let t0 = Duration::from_secs(10);
        let [x, y] = d.card_on_screen(1).unwrap().center();
        d.touch(1, TouchPhase::Started, x, y, t0);
        assert_eq!(
            d.tick(Duration::from_millis(33), t0 + Duration::from_millis(1900)),
            None
        );
        assert_eq!(
            d.touch(1, TouchPhase::Ended, x, y, t0 + Duration::from_millis(1950)),
            DrawerTouch::Hit(DrawerHit::Remote(RemoteId([1; 8])))
        );
        d.touch(2, TouchPhase::Started, x, y, t0);
        d.touch(2, TouchPhase::Moved, x + 80.0, y, t0);
        assert_eq!(
            d.tick(Duration::from_millis(33), t0 + HOLD),
            None,
            "moved: no hold"
        );
        assert_eq!(
            d.touch(2, TouchPhase::Ended, x + 80.0, y, t0 + HOLD),
            DrawerTouch::Nothing
        );
    }

    #[test]
    fn a_tap_on_the_scrim_closes_it() {
        let mut d = opened(1);
        let r = Rect::new(1000.0, 1200.0, 10.0, 10.0);
        assert_eq!(tap(&mut d, r, Duration::ZERO), DrawerTouch::Close);
    }

    #[test]
    fn the_client_is_told_once_each_time_it_shows_or_hides() {
        let mut d = AppDrawer::new();
        d.set_frame(1080.0, 2340.0, 0.0, 0.0, 2.8);
        assert_eq!(d.take_shown_change(), None);
        d.open();
        assert_eq!(d.take_shown_change(), Some(true));
        assert_eq!(d.take_shown_change(), None);
        d.close_now();
        assert_eq!(d.take_shown_change(), Some(false));
    }

    #[test]
    fn text_is_cut_at_the_right_or_the_left() {
        let ctx = crate::theme::testing::draw_context();
        let w = ctx.text_advance("abc", 20.0);
        assert_eq!(cut_right(&ctx, "abc", 20.0, w), "abc");
        let cut = cut_right(&ctx, "abcdefgh", 20.0, w);
        assert!(cut.ends_with('\u{2026}') && ctx.text_advance(&cut, 20.0) <= w);
        let cut = cut_left(&ctx, "10.0.0.120", 20.0, w * 1.5);
        assert!(cut.starts_with('\u{2026}') && cut.ends_with("20"), "{cut}");
    }
}
