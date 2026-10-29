//! **All agents…**: the agent menu's full list in a modal
//! (docs/design/20261023_07_SUBAGENTS_PLAN.md, the 2026-10-29 03:11 EDT
//! amendment, item 2).
//!
//! Centred over the screen in the plugins' modal style
//! ([`crate::plugins::modal`]: the shade, the card, its title and ✕):
//! **main** first, the pane's own agent; then a **Running** heading and
//! the running subagents; then a **Finished** heading and the finished
//! ones, newest first within each, as the client lists them; a heading
//! with nothing under it is left out. Each row is a radio mark, the
//! subagent's short name, its description dim beneath (when it is not the
//! name itself), and at the right its whole minutes, `12m`, or
//! "finished". The session viewed has the filled radio ⦿, the others the
//! empty ⊙, drawn as the agent menu's are (libcontrols' radio style: a
//! ring in the text colour with a dot in it). A tap on a row opens it as
//! the menu's items do; a tap outside the card, its ✕ or Back closes the
//! modal. The list scrolls when it is taller than the modal.

use std::time::Duration;

use libcontrols::{DrawContext, Rect};
use libflankr_proto::common::ChildInfo;
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use super::subagent_menu::{FINISHED, MAIN_LABEL, RUNNING, name};
use crate::agent::{columns, fill, line_h};
use crate::controls::scroll::{Scroll, Touched};
use crate::plugins::modal::{ModalFrame, Target, draw_shell, frame};
use crate::settings::copy;
use crate::theme::theme;
use crate::ui::Layout;

/// The type sizes, in points: a name, a description and the state, and a
/// heading.
const NAME_PT: f32 = 16.0;
const SMALL_PT: f32 = 13.0;
/// A row's padding above and below, its least height (a tap target), a
/// heading's height, and the mark column's width, in points.
const ROW_PAD: f32 = 8.0;
const ROW_MIN: f32 = 52.0;
const HEADING_H: f32 = 32.0;
const MARK_W: f32 = 28.0;
/// The gap before the state at the right, in points.
const GAP: f32 = 10.0;
/// The radio marks, as the agent menu's (`crate::menu::Menu`'s options):
/// the ring's outer radius and stroke, the filled dot (⦿) and the empty
/// one (⊙), in points.
const RADIO_R: f32 = 6.0;
const RADIO_T: f32 = 1.5;
const DOT_R: f32 = 3.0;
const EMPTY_DOT_R: f32 = 1.25;

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// **Running** or **Finished**.
    Heading(&'static str),
    /// main (`session: None`) or a subagent.
    Agent {
        session: Option<String>,
        name: String,
        /// Dim beneath the name; empty for none.
        description: String,
        /// At the right: its minutes, "finished", or nothing.
        state: String,
        /// The session viewed: the filled radio.
        viewed: bool,
    },
}

/// A running subagent's whole minutes since it started, whatever they
/// are (the modal shows them all, not only past 5); `None` once finished
/// or while its start is not known.
fn minutes(child: &ChildInfo, now: u64) -> Option<u64> {
    (!child.finished && child.started != 0).then(|| now.saturating_sub(child.started) / 60)
}

/// The full list for `children` (running first, then finished, newest
/// first within each), with the session `viewed` (`None`: main), at
/// `now`.
pub fn lines(children: &[ChildInfo], viewed: Option<&str>, now: u64) -> Vec<Line> {
    let mut out = vec![Line::Agent {
        session: None,
        name: MAIN_LABEL.to_string(),
        description: String::new(),
        state: String::new(),
        viewed: viewed.is_none(),
    }];
    for (heading, finished) in [(RUNNING, false), (FINISHED, true)] {
        let group: Vec<&ChildInfo> = children.iter().filter(|c| c.finished == finished).collect();
        if group.is_empty() {
            continue;
        }
        out.push(Line::Heading(heading));
        for c in group {
            let shown = name(c).to_string();
            let description = c.description.trim();
            let state = if c.finished {
                copy::AGENTS_FINISHED.text.trim().to_string()
            } else {
                minutes(c, now).map_or_else(String::new, |m| {
                    copy::AGENTS_MINUTES
                        .fill(None, &[("minutes", &m.to_string())])
                        .trim()
                        .to_string()
                })
            };
            out.push(Line::Agent {
                session: Some(c.id.clone()),
                description: if description == shown {
                    String::new()
                } else {
                    description.to_string()
                },
                name: shown,
                state,
                viewed: viewed == Some(c.id.as_str()),
            });
        }
    }
    out
}

/// `text` on one line of `w` at `px`, cut with … when longer.
fn cut(text: &str, w: f32, px: f32) -> String {
    let max = columns(w, px);
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// A line where the list lays it out, in the content's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    pub line: Line,
    pub rect: Rect,
}

/// The list laid out `w` wide at scale `s`, and its height.
pub fn lay_out(lines: &[Line], w: f32, s: f32) -> (Vec<Laid>, f32) {
    let mut y = 0.0;
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let h = match line {
            Line::Heading(_) => HEADING_H * s,
            Line::Agent { description, .. } => {
                let text = line_h(NAME_PT * s)
                    + if description.is_empty() {
                        0.0
                    } else {
                        line_h(SMALL_PT * s)
                    };
                (2.0 * ROW_PAD * s + text).max(ROW_MIN * s)
            }
        };
        out.push(Laid {
            line: line.clone(),
            rect: Rect::new(0.0, y, w, h),
        });
        y += h;
    }
    (out, y)
}

/// The open modal's agent rows, each its session (`None`: main) and its
/// rect on the screen, and its ✕ ([`AgentsModal::targets`]).
pub type Targets = (Vec<(Option<String>, Rect)>, Rect);

/// What a touch on the open modal did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentsHit {
    /// Show main (`None`) or this subagent; the modal has closed.
    Open(Option<String>),
    /// The ✕ or a tap outside closed it.
    Closed,
}

/// The modal's state: open or not, its scroll and its touch.
#[derive(Debug, Default)]
pub struct AgentsModal {
    open: bool,
    scroll: Scroll,
    touch: Option<(u64, Target)>,
    dirty: bool,
    /// The list last followed, to draw again when it changes.
    followed: Vec<Line>,
}

fn scale(layout: &Layout) -> f32 {
    if layout.scale > 0.0 {
        layout.scale
    } else {
        1.0
    }
}

impl AgentsModal {
    /// Opens it at the top of its list.
    pub fn open(&mut self) {
        *self = AgentsModal {
            open: true,
            dirty: true,
            ..AgentsModal::default()
        };
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Closes it (Back, or its view gone).
    pub fn close(&mut self) {
        if self.open {
            *self = AgentsModal {
                dirty: true,
                ..AgentsModal::default()
            };
        }
    }

    /// While open, the list as it is now: when it changed (a subagent
    /// started or finished, a minute passed) it draws again.
    pub fn follow(&mut self, lines: Vec<Line>) {
        if self.open && self.followed != lines {
            self.followed = lines;
            self.dirty = true;
        }
    }

    /// Whether it opened, closed, scrolled or its list changed since last
    /// asked.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Its frame on `layout` for `lines`, the list laid out in it, and
    /// the scroll's extent set to them.
    fn frame(&mut self, lines: &[Line], layout: &Layout) -> (ModalFrame, Vec<Laid>) {
        let s = scale(layout);
        let probe = frame(layout, 0.0);
        let (laid, h) = lay_out(lines, probe.content.w, s);
        let f = frame(layout, h);
        self.scroll.set_extent(h, f.content.h);
        (f, laid)
    }

    /// A touch while it is open: it takes every touch. `lines` is the
    /// list as it is now ([`lines`]).
    pub fn touch(
        &mut self,
        lines: &[Line],
        layout: &Layout,
        (id, phase, x, y): (u64, TouchPhase, f32, f32),
        now: Duration,
    ) -> Option<AgentsHit> {
        if !self.open {
            return None;
        }
        let s = scale(layout);
        let (f, laid) = self.frame(lines, layout);
        if phase == TouchPhase::Started {
            self.touch = Some((id, f.target(x, y)));
        }
        let (held, target) = self.touch?;
        if held != id {
            return None;
        }
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.touch = None;
        }
        match target {
            Target::Content => match self.scroll.touch(id, phase, x, y, now, s) {
                Touched::Tap => {
                    let local = [x - f.content.x, y - f.content.y + self.scroll.offset()];
                    let session = laid.into_iter().find_map(|l| match l.line {
                        Line::Agent { session, .. } if l.rect.contains(local) => Some(session),
                        _ => None,
                    })?;
                    self.close();
                    Some(AgentsHit::Open(session))
                }
                Touched::Scrolled => {
                    self.dirty = true;
                    None
                }
                Touched::Nothing => None,
            },
            Target::Close | Target::Outside
                if phase == TouchPhase::Ended && f.closes(target, x, y) =>
            {
                self.close();
                Some(AgentsHit::Closed)
            }
            _ => None,
        }
    }

    /// While open, each agent row's session and where it is on the
    /// screen, as last scrolled, and the ✕ (scripted tests).
    pub fn targets(&self, lines: &[Line], layout: &Layout) -> Option<Targets> {
        if !self.open {
            return None;
        }
        let probe = frame(layout, 0.0);
        let (laid, h) = lay_out(lines, probe.content.w, scale(layout));
        let f = frame(layout, h);
        let top = f.content.y - self.scroll.offset();
        let rows = laid
            .into_iter()
            .filter_map(|l| match l.line {
                Line::Agent { session, .. } => Some((
                    session,
                    Rect::new(f.content.x, top + l.rect.y, l.rect.w, l.rect.h),
                )),
                Line::Heading(_) => None,
            })
            .collect();
        Some((rows, f.close))
    }

    /// Carries a fling on. Returns whether the list moved.
    pub fn tick(&mut self, dt: Duration, s: f32) -> bool {
        self.open && self.scroll.tick(dt, s)
    }

    /// Draws the shade, the card, its title and ✕, and the list.
    pub fn draw(
        &mut self,
        lines: &[Line],
        layout: &Layout,
        ctx: &DrawContext,
        list: &mut DrawList,
    ) {
        if !self.open {
            return;
        }
        let s = scale(layout);
        let (f, laid) = self.frame(lines, layout);
        draw_shell(&f, copy::AGENTS_TITLE.text.trim(), layout, s, ctx, list);
        let c = f.content;
        let top = c.y - self.scroll.offset();
        list.push_clip(c.pos(), c.size());
        for l in &laid {
            let y = top + l.rect.y;
            if y + l.rect.h < c.y || y > c.y + c.h {
                continue;
            }
            match &l.line {
                Line::Heading(text) => {
                    let px = SMALL_PT * s;
                    let ty = y + l.rect.h - line_h(px) - 4.0 * s;
                    ctx.draw_text(list, text, [c.x, ty], px, theme().dim);
                }
                Line::Agent {
                    name,
                    description,
                    state,
                    viewed,
                    ..
                } => {
                    let name_px = NAME_PT * s;
                    let small = SMALL_PT * s;
                    let text_h = line_h(name_px)
                        + if description.is_empty() {
                            0.0
                        } else {
                            line_h(small)
                        };
                    let ty = y + (l.rect.h - text_h) * 0.5;
                    let mark = [c.x + RADIO_R * s, ty + line_h(name_px) * 0.5];
                    radio(list, mark, *viewed, s);
                    let x = c.x + MARK_W * s;
                    let state_w = if state.is_empty() {
                        0.0
                    } else {
                        ctx.text_advance(state, small)
                    };
                    if !state.is_empty() {
                        let sy = ty + (line_h(name_px) - line_h(small)) * 0.5;
                        ctx.draw_text(list, state, [c.x + c.w - state_w, sy], small, theme().dim);
                    }
                    let w = (c.w - MARK_W * s - state_w - GAP * s).max(0.0);
                    ctx.draw_text(list, &cut(name, w, name_px), [x, ty], name_px, theme().text);
                    if !description.is_empty() {
                        let w = (c.w - MARK_W * s).max(0.0);
                        ctx.draw_text(
                            list,
                            &cut(description, w, small),
                            [x, ty + line_h(name_px)],
                            small,
                            theme().dim,
                        );
                    }
                }
            }
        }
        list.push_clip_end();
    }
}

/// A radio mark centred at `at`: a ring in the text colour on the card,
/// with the filled dot (⦿) when `filled`, else the small one (⊙).
fn radio(list: &mut DrawList, [cx, cy]: [f32; 2], filled: bool, s: f32) {
    let disc = |list: &mut DrawList, r: f32, color| {
        fill(list, Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), color, r);
    };
    disc(list, RADIO_R * s, theme().text);
    disc(list, (RADIO_R - RADIO_T) * s, theme().card);
    disc(
        list,
        if filled { DOT_R } else { EMPTY_DOT_R } * s,
        theme().text,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagent_menu::tests::{NOW, child, children, many};

    const PHONE: Layout = Layout {
        w: 1080.0,
        h: 2340.0,
        scale: 2.0,
        top: 80.0,
        bottom: 60.0,
    };

    fn shown(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                Line::Heading(h) => format!("# {h}"),
                Line::Agent {
                    name,
                    description,
                    state,
                    viewed,
                    ..
                } => format!(
                    "{} {name} | {description} | {state}",
                    if *viewed { "(*)" } else { "( )" }
                ),
            })
            .collect()
    }

    /// The modal's rows: main, Running, Finished; short name, the
    /// description beneath, the minutes or "finished"; one filled radio.
    #[test]
    fn the_rows_are_main_then_running_then_finished_with_their_state() {
        let l = lines(&children(), Some("b2"), NOW);
        assert_eq!(
            shown(&l),
            [
                "( ) main |  | ",
                "# Running",
                "( ) overview screen | Build the overview screen plan | 12m",
                "(*) spoken summaries | Build the spoken summaries plan | 3m",
                "# Finished",
                "( ) libtypedmd licences | Build the libtypedmd licences plan | finished",
            ]
        );
        // Every finished one, not only three.
        let all = lines(&many(2, 5), None, NOW);
        assert_eq!(all.len(), 1 + 1 + 2 + 1 + 5);
        assert_eq!(
            all.iter()
                .filter(|l| matches!(l, Line::Agent { viewed: true, .. }))
                .count(),
            1
        );
        // Nothing finished: no Finished heading.
        assert!(!shown(&lines(&many(1, 0), None, NOW)).contains(&"# Finished".to_string()));
        // A name that is its description shows it once; no start, no
        // minutes.
        let mut c = child("x", "", false, 0);
        c.started = 0;
        assert_eq!(
            shown(&lines(&[c], None, NOW))[2],
            "( ) Build the  plan |  | "
        );
    }

    /// Draws `m` and returns the list.
    fn drawn(m: &mut AgentsModal, l: &[Line]) -> DrawList {
        let mut list = DrawList::new();
        m.draw(l, &PHONE, &DrawContext::default(), &mut list);
        list
    }

    #[test]
    fn the_marks_are_rings_with_the_filled_dot_on_the_session_viewed() {
        let l = lines(&children(), Some("a1"), NOW);
        let mut m = AgentsModal::default();
        assert!(drawn(&mut m, &l).instances.is_empty(), "closed: nothing");
        m.open();
        let list = drawn(&mut m, &l);
        // Each mark: the ring, its inside and its dot, in that order.
        let s = PHONE.scale;
        let rings: Vec<usize> = (0..list.instances.len().saturating_sub(2))
            .filter(|&i| {
                let [a, b, c] = [
                    &list.instances[i],
                    &list.instances[i + 1],
                    &list.instances[i + 2],
                ];
                a.size[0] == 2.0 * RADIO_R * s
                    && a.color == theme().text
                    && b.size[0] == 2.0 * (RADIO_R - RADIO_T) * s
                    && b.color == theme().card
                    && c.color == theme().text
            })
            .collect();
        assert_eq!(rings.len(), 4, "main and the three subagents");
        let dots: Vec<f32> = rings
            .iter()
            .map(|&i| list.instances[i + 2].size[0])
            .collect();
        assert_eq!(
            dots,
            [
                2.0 * EMPTY_DOT_R * s,
                2.0 * DOT_R * s,
                2.0 * EMPTY_DOT_R * s,
                2.0 * EMPTY_DOT_R * s
            ],
            "⦿ on a1, ⊙ on the others"
        );
    }

    fn tap(m: &mut AgentsModal, l: &[Line], id: u64, [x, y]: [f32; 2]) -> Option<AgentsHit> {
        let t = Duration::from_millis(id * 1000);
        m.touch(l, &PHONE, (id, TouchPhase::Started, x, y), t);
        m.touch(
            l,
            &PHONE,
            (id, TouchPhase::Ended, x, y),
            t + Duration::from_millis(50),
        )
    }

    /// Where line `i` is on the screen, at the top of the list.
    fn at(m: &mut AgentsModal, l: &[Line], i: usize) -> [f32; 2] {
        let (f, laid) = m.frame(l, &PHONE);
        let r = laid[i].rect;
        [f.content.x + r.x + 20.0, f.content.y + r.y + r.h * 0.5]
    }

    #[test]
    fn a_tap_on_a_row_opens_it_and_closes_the_modal() {
        let l = lines(&children(), None, NOW);
        let mut m = AgentsModal::default();
        assert_eq!(
            tap(&mut m, &l, 1, [540.0, 1000.0]),
            None,
            "closed: takes none"
        );
        m.open();
        // c3, the finished one (main, Running, a1, b2, Finished, c3).
        let p = at(&mut m, &l, 5);
        assert_eq!(
            tap(&mut m, &l, 2, p),
            Some(AgentsHit::Open(Some("c3".into())))
        );
        assert!(!m.is_open());
        // main, the first row.
        m.open();
        let p = at(&mut m, &l, 0);
        assert_eq!(tap(&mut m, &l, 3, p), Some(AgentsHit::Open(None)));
        // A heading opens nothing and keeps it open.
        m.open();
        let p = at(&mut m, &l, 1);
        assert_eq!(tap(&mut m, &l, 4, p), None);
        assert!(m.is_open());
    }

    #[test]
    fn a_tap_outside_or_on_the_x_closes_it_and_inside_the_card_does_not() {
        let l = lines(&children(), None, NOW);
        let mut m = AgentsModal::default();
        m.open();
        assert!(m.take_dirty());
        let (f, _) = m.frame(&l, &PHONE);
        // On the card's title row, not the X: nothing.
        assert_eq!(tap(&mut m, &l, 1, [f.card.x + 40.0, f.card.y + 20.0]), None);
        assert!(m.is_open());
        // The X.
        assert_eq!(
            tap(&mut m, &l, 2, f.close.center()),
            Some(AgentsHit::Closed)
        );
        assert!(!m.is_open());
        assert!(m.take_dirty());
        // Outside the card.
        m.open();
        assert_eq!(
            tap(&mut m, &l, 3, [f.card.x + 10.0, f.card.y - 40.0]),
            Some(AgentsHit::Closed)
        );
        assert!(!m.is_open());
        // A touch that starts outside and ends on the card: nothing.
        m.open();
        let t = Duration::from_secs(9);
        m.touch(
            &l,
            &PHONE,
            (4, TouchPhase::Started, 10.0, f.card.y - 40.0),
            t,
        );
        let [x, y] = f.card.center();
        assert_eq!(m.touch(&l, &PHONE, (4, TouchPhase::Ended, x, y), t), None);
        assert!(m.is_open());
        // Back's close (the app calls it).
        m.close();
        assert!(!m.is_open());
    }

    #[test]
    fn it_draws_again_when_its_list_changes_while_open() {
        let mut m = AgentsModal::default();
        m.follow(lines(&children(), None, NOW));
        assert!(!m.take_dirty(), "closed: not followed");
        m.open();
        assert!(m.take_dirty());
        m.follow(lines(&children(), None, NOW));
        assert!(m.take_dirty(), "the first list");
        m.follow(lines(&children(), None, NOW));
        assert!(!m.take_dirty(), "the same list");
        m.follow(lines(&children(), None, NOW + 60));
        assert!(m.take_dirty(), "a minute on");
    }

    #[test]
    fn a_long_list_fits_the_screen_and_scrolls() {
        let l = lines(&many(4, 40), None, NOW);
        let mut m = AgentsModal::default();
        m.open();
        let (f, laid) = m.frame(&l, &PHONE);
        assert!(f.card.y >= PHONE.top);
        assert!(f.card.y + f.card.h <= PHONE.h - PHONE.bottom);
        assert!(
            laid.last().unwrap().rect.y > f.content.h,
            "taller than the modal"
        );
        assert!(m.scroll.max() > 0.0);
        // Every row is a tap target of at least 44 pt.
        assert!(
            laid.iter()
                .filter(|l| matches!(l.line, Line::Agent { .. }))
                .all(|l| l.rect.h >= 44.0 * PHONE.scale)
        );
    }
}
