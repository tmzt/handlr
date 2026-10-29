//! A plugin's modal (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.7,
//! item 1): centred over the screen as the settings' remote details are,
//! in their style (the card colour, 14 pt corners, opening and closing at
//! once), with the plugin's name as its title and an X at its top right;
//! under them the device half's content, which scrolls when it is taller
//! than the modal. A tap outside the card, the X, or Back closes it.
//!
//! The device half draws its own content, so the modal is drawn here and
//! not as a libcontrols `ModalDialog`, whose content is a router of
//! hosted controls (CHANGES.md, plan 20261028_04's steps 5-7).

use std::time::Duration;

use libcontrols::{DrawContext, Rect};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use super::{Outgoing, Plugins};
use crate::agent::{PAD, fill, line_h};
use crate::controls::scroll::{Scroll, Touched};
use crate::theme::theme;
use crate::ui::Layout;

/// The card's widest, its corner, its padding and its title row, in
/// points.
const MAX_W: f32 = 560.0;
const RADIUS: f32 = 14.0;
const INSET: f32 = 20.0;
const HEADER: f32 = 56.0;
const TITLE_PT: f32 = 18.0;
/// The X's target and its strokes' half length, in points.
const CLOSE: f32 = 44.0;
const CROSS: f32 = 7.0;
/// The shade over the screen under it.
const DIM: [f32; 4] = [0.0, 0.0, 0.0, 0.45];

/// Where the modal's parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModalFrame {
    pub card: Rect,
    /// The title's top left.
    pub title: [f32; 2],
    pub close: Rect,
    /// The content's window.
    pub content: Rect,
}

/// The modal's frame on `layout` for content `content_h` tall.
pub fn frame(layout: &Layout, content_h: f32) -> ModalFrame {
    let s = if layout.scale > 0.0 {
        layout.scale
    } else {
        1.0
    };
    let viewport = Rect::new(
        0.0,
        layout.top,
        layout.w,
        (layout.h - layout.top - layout.bottom).max(0.0),
    );
    let w = (viewport.w - 2.0 * PAD * s).min(MAX_W * s).max(0.0);
    let inset = INSET * s;
    let header = HEADER * s;
    let max_h = (viewport.h - 4.0 * PAD * s).max(header + inset);
    let h = (header + content_h + inset).min(max_h);
    let [cx, cy] = viewport.center();
    let card = Rect::new(cx - w * 0.5, cy - h * 0.5, w, h);
    let close = Rect::new(
        card.x + card.w - CLOSE * s - 6.0 * s,
        card.y + (header - CLOSE * s) * 0.5,
        CLOSE * s,
        CLOSE * s,
    );
    ModalFrame {
        card,
        title: [
            card.x + inset,
            card.y + (header - line_h(TITLE_PT * s)) * 0.5,
        ],
        close,
        content: Rect::new(
            card.x + inset,
            card.y + header,
            (w - 2.0 * inset).max(0.0),
            (h - header - inset).max(0.0),
        ),
    }
}

/// Where a touch on the modal started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Content,
    Close,
    Outside,
    Card,
}

impl ModalFrame {
    /// The part of the modal at `(x, y)`.
    pub(crate) fn target(&self, x: f32, y: f32) -> Target {
        if self.close.contains([x, y]) {
            Target::Close
        } else if self.content.contains([x, y]) {
            Target::Content
        } else if self.card.contains([x, y]) {
            Target::Card
        } else {
            Target::Outside
        }
    }

    /// Whether a touch that started on `target` and ended at `(x, y)`
    /// closes the modal: on the X, or outside the card.
    pub(crate) fn closes(&self, target: Target, x: f32, y: f32) -> bool {
        match target {
            Target::Close => self.close.contains([x, y]),
            Target::Outside => !self.card.contains([x, y]),
            _ => false,
        }
    }
}

/// The modal's shade over the screen of `layout`, its card, `title` and
/// X, in frame `f` at scale `s`; the content is the caller's.
pub fn draw_shell(
    f: &ModalFrame,
    title: &str,
    layout: &Layout,
    s: f32,
    ctx: &DrawContext,
    list: &mut DrawList,
) {
    fill(list, Rect::new(0.0, 0.0, layout.w, layout.h), DIM, 0.0);
    fill(list, f.card, theme().card, RADIUS * s);
    ctx.draw_text(list, title, f.title, TITLE_PT * s, theme().fg);
    let [cx, cy] = f.close.center();
    let h = CROSS * s;
    let t = 2.0 * s;
    list.push_line([cx - h, cy - h], [cx + h, cy + h], t, theme().dim);
    list.push_line([cx - h, cy + h], [cx + h, cy - h], t, theme().dim);
}

/// The open modal's scroll and touch.
#[derive(Debug, Default)]
pub struct ModalView {
    scroll: Scroll,
    touch: Option<(u64, Target)>,
}

impl ModalView {
    /// A newly opened modal starts at its top.
    pub fn reset(&mut self) {
        *self = ModalView::default();
    }

    fn frame(&mut self, plugins: &Plugins, layout: &Layout) -> ModalFrame {
        let s = if layout.scale > 0.0 {
            layout.scale
        } else {
            1.0
        };
        let probe = frame(layout, 0.0);
        let content_h = plugins.modal_height(probe.content.w, s);
        let f = frame(layout, content_h);
        self.scroll.set_extent(content_h, f.content.h);
        f
    }

    /// A touch while the modal is open: it takes every touch. Returns the
    /// calls a tap in the content asked for.
    pub fn touch(
        &mut self,
        plugins: &mut Plugins,
        layout: &Layout,
        (id, phase, x, y): (u64, TouchPhase, f32, f32),
        now: Duration,
    ) -> Vec<Outgoing> {
        let s = if layout.scale > 0.0 {
            layout.scale
        } else {
            1.0
        };
        let f = self.frame(plugins, layout);
        if phase == TouchPhase::Started {
            self.touch = Some((id, f.target(x, y)));
        }
        let Some((held, target)) = self.touch else {
            return Vec::new();
        };
        if held != id {
            return Vec::new();
        }
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.touch = None;
        }
        match target {
            Target::Content => match self.scroll.touch(id, phase, x, y, now, s) {
                Touched::Tap => {
                    let local = (x - f.content.x, y - f.content.y + self.scroll.offset());
                    plugins.tap_modal(local.0, local.1)
                }
                _ => Vec::new(),
            },
            Target::Close | Target::Outside if phase == TouchPhase::Ended => {
                if f.closes(target, x, y) {
                    plugins.close_modal();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Carries a fling on. Returns whether the content moved.
    pub fn tick(&mut self, dt: Duration, s: f32) -> bool {
        self.scroll.tick(dt, s)
    }

    /// Draws the shade, the card, its title and X, and the content.
    pub fn draw(
        &mut self,
        plugins: &Plugins,
        layout: &Layout,
        ctx: &DrawContext,
        list: &mut DrawList,
    ) {
        let Some((_, name)) = plugins.modal() else {
            return;
        };
        let name = name.to_string();
        let s = if layout.scale > 0.0 {
            layout.scale
        } else {
            1.0
        };
        let f = self.frame(plugins, layout);
        draw_shell(&f, &name, layout, s, ctx, list);
        let content_h = plugins.modal_height(f.content.w, s);
        list.push_clip(f.content.pos(), f.content.size());
        plugins.draw_modal(
            Rect::new(
                f.content.x,
                f.content.y - self.scroll.offset(),
                f.content.w,
                content_h,
            ),
            s,
            ctx,
            list,
        );
        list.push_clip_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: Layout = Layout {
        w: 1080.0,
        h: 2340.0,
        scale: 2.0,
        top: 80.0,
        bottom: 60.0,
    };

    #[test]
    fn the_card_is_centred_and_no_taller_than_the_screen_allows() {
        let small = frame(&PHONE, 100.0);
        assert!((small.card.center()[0] - 540.0).abs() < 0.01);
        assert_eq!(small.card.h, (HEADER + INSET) * 2.0 + 100.0);
        assert!(small.content.h >= 100.0 - 0.01);
        let tall = frame(&PHONE, 10_000.0);
        assert!(tall.card.y >= PHONE.top);
        assert!(tall.card.y + tall.card.h <= PHONE.h - PHONE.bottom);
        assert!(tall.close.x + tall.close.w <= tall.card.x + tall.card.w);
        assert!(tall.card.w <= MAX_W * 2.0);
    }
}
