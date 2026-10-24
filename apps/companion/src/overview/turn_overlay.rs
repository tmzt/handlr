//! The Gemini turns over the cards (plan 20261023_04, the 18:07 EDT
//! amendment, 3): a sheet that slides up from the bottom, about two thirds
//! of the screen high, holding the conversation as text ([`TurnLog`]),
//! newest at the bottom and scrolling. The cards above it are dimmed. A
//! tap on them, a swipe down on the sheet, or Android's Back closes it;
//! the screen owns those parts and calls [`TurnOverlay::close`]. It never
//! opens by itself (4).
//!
//! The slide is short ([`TurnOverlay::SLIDE`]) and driven by `tick`. While
//! it opens or closes, the sheet is drawn where it has got to; its log is
//! laid out at the place it opens to, and drawn moved by what is left.

use std::time::Duration;

use libcontrols::{DrawContext, Rect};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use super::turn_log::TurnLog;
use crate::agent::fill;
use crate::controls::scroll::Touched;
use crate::theme::theme;

/// A finger on the sheet, watched for a swipe down.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Swipe {
    id: u64,
    start_y: f32,
    /// Started on the handle: the log does not see it.
    handle: bool,
    /// The log had nothing above to scroll back to when it started.
    at_top: bool,
}

/// The overlay: whether it is open, how far it has slid, and its log.
pub struct TurnOverlay {
    log: TurnLog,
    open: bool,
    /// How far it has slid up: 0 closed, 1 open.
    shown: f32,
    /// Where the sheet is when open.
    area: Rect,
    scale: f32,
    swipe: Option<Swipe>,
}

impl Default for TurnOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnOverlay {
    /// How long the slide takes, each way.
    pub const SLIDE: Duration = Duration::from_millis(150);
    /// The share of the screen's height the sheet takes, at most.
    pub const SHARE: f32 = 2.0 / 3.0;
    /// The handle's band at the sheet's top, and the drag down that closes
    /// it, in points.
    pub const HANDLE_H: f32 = 28.0;
    pub const CLOSE_DRAG: f32 = 48.0;

    pub fn new() -> Self {
        TurnOverlay {
            log: TurnLog::new(),
            open: false,
            shown: 0.0,
            area: Rect::default(),
            scale: 1.0,
            swipe: None,
        }
    }

    pub fn log(&self) -> &TurnLog {
        &self.log
    }

    pub fn log_mut(&mut self) -> &mut TurnLog {
        &mut self.log
    }

    /// Where the sheet is when open, at scale `s`.
    pub fn set_area(&mut self, area: Rect, s: f32) {
        self.area = area;
        self.scale = s;
        let handle = Self::HANDLE_H * s;
        self.log.set_viewport(
            Rect::new(area.x, area.y + handle, area.w, (area.h - handle).max(0.0)),
            s,
        );
    }

    /// Starts it sliding up. Returns whether it was closed.
    pub fn open(&mut self) -> bool {
        let was = self.open;
        self.open = true;
        !was
    }

    /// Starts it sliding down. Returns whether it was open.
    pub fn close(&mut self) -> bool {
        let was = self.open;
        self.open = false;
        self.swipe = None;
        was
    }

    /// Open, or opening: touches above the bottom row are its.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Drawn at all: open, or still sliding down.
    pub fn is_shown(&self) -> bool {
        self.shown > 0.0
    }

    /// How far it has slid up, 0 to 1.
    pub fn shown(&self) -> f32 {
        self.shown
    }

    /// What is left of the slide, in pixels down from where it opens to:
    /// eased, so it slows as it arrives.
    fn drop(&self) -> f32 {
        let t = 1.0 - self.shown;
        self.area.h * t * t * t
    }

    /// The sheet where it is now.
    pub fn rect(&self) -> Rect {
        let a = self.area;
        Rect::new(a.x, a.y + self.drop(), a.w, a.h)
    }

    /// Whether `(x, y)` is on the sheet where it is now.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.is_shown() && self.rect().contains([x, y])
    }

    /// A touch that started on the sheet. A drag down on the handle, or on
    /// a log with nothing above to go back to, closes it; anything else is
    /// the log's.
    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, now: Duration) -> Touched {
        if phase == TouchPhase::Started {
            let handle = y < self.rect().y + Self::HANDLE_H * self.scale;
            self.swipe = Some(Swipe {
                id,
                start_y: y,
                handle,
                at_top: self.log.scroll().offset() <= 0.0,
            });
        }
        let Some(swipe) = self.swipe.filter(|w| w.id == id) else {
            return Touched::Nothing;
        };
        let touched = if swipe.handle {
            match phase {
                TouchPhase::Ended => Touched::Tap,
                _ => Touched::Nothing,
            }
        } else {
            self.log.touch(id, phase, x, y, now)
        };
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.swipe = None;
            let down = y - swipe.start_y > Self::CLOSE_DRAG * self.scale;
            let unmoved = swipe.handle || (swipe.at_top && self.log.scroll().offset() <= 0.0);
            if phase == TouchPhase::Ended && down && unmoved {
                self.close();
                return Touched::Scrolled;
            }
        }
        touched
    }

    /// Carries the slide and the log's fling on. Returns whether anything
    /// moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let step = dt.as_secs_f32() / Self::SLIDE.as_secs_f32();
        let before = self.shown;
        self.shown = if self.open {
            (self.shown + step).min(1.0)
        } else {
            (self.shown - step).max(0.0)
        };
        let scrolled = self.is_shown() && self.log.tick(dt);
        scrolled || self.shown != before
    }

    /// Draws the shade over everything above the sheet, `width` wide, then
    /// the sheet and its log.
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext, width: f32) {
        if !self.is_shown() {
            return;
        }
        let s = self.scale;
        let r = self.rect();
        let mut shade = theme().overlay;
        shade[3] *= self.shown;
        fill(list, Rect::new(0.0, 0.0, width, r.y), shade, 0.0);
        fill(list, r, theme().header, 16.0 * s);
        let (hw, hh) = (40.0 * s, 4.0 * s);
        let handle = Rect::new(
            r.x + (r.w - hw) * 0.5,
            r.y + (Self::HANDLE_H * s - hh) * 0.5,
            hw,
            hh,
        );
        fill(list, handle, theme().band, hh * 0.5);
        self.log.draw_moved(list, ctx, self.drop());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handlr_companion::Speaker;

    /// Two thirds of the phone, above the bottom row.
    const AREA: Rect = Rect {
        x: 0.0,
        y: 780.0,
        w: 1080.0,
        h: 1376.0,
    };

    const MS: Duration = Duration::from_millis(16);

    fn overlay(turns: usize) -> TurnOverlay {
        let mut o = TurnOverlay::new();
        o.set_area(AREA, 2.0);
        for i in 0..turns {
            o.log_mut().push(Speaker::User, format!("question {i}"));
            o.log_mut()
                .push(Speaker::Gemini, format!("Answer {i}, at some length."));
        }
        o
    }

    fn settle(o: &mut TurnOverlay) {
        o.tick(TurnOverlay::SLIDE);
    }

    /// Drags from `from` to `to` on the sheet at `x` 540, then lifts after
    /// a pause.
    fn drag(o: &mut TurnOverlay, from: f32, to: f32) -> Touched {
        o.touch(1, TouchPhase::Started, 540.0, from, Duration::ZERO);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += MS;
            o.touch(
                1,
                TouchPhase::Moved,
                540.0,
                from + (to - from) * i as f32 / 10.0,
                now,
            );
        }
        o.touch(
            1,
            TouchPhase::Ended,
            540.0,
            to,
            now + Duration::from_millis(300),
        )
    }

    #[test]
    fn it_slides_up_and_down_in_the_slide_time() {
        let mut o = overlay(1);
        assert!(!o.is_shown());
        assert!(!o.tick(MS), "closed, nothing moves");
        assert!(o.open());
        assert!(!o.open(), "already open");
        assert!(o.tick(MS));
        let part = o.rect();
        assert!(part.y > AREA.y && part.y < AREA.y + AREA.h);
        settle(&mut o);
        assert_eq!(o.shown(), 1.0);
        assert_eq!(o.rect(), AREA);
        assert!(!o.tick(MS), "open, it stays");
        assert!(o.close());
        assert!(o.is_shown(), "still sliding down");
        settle(&mut o);
        assert!(!o.is_shown());
        assert!(!o.contains(540.0, AREA.y + 10.0));
    }

    #[test]
    fn a_swipe_down_closes_it_and_a_swipe_up_scrolls() {
        // On the handle, whatever the log shows.
        let mut o = overlay(30);
        o.open();
        settle(&mut o);
        assert!(o.log().scroll().max() > 0.0);
        assert_eq!(
            drag(&mut o, AREA.y + 10.0, AREA.y + 300.0),
            Touched::Scrolled
        );
        assert!(!o.is_open());
        // On the log, scrolled to the newest: down scrolls back, not shut.
        o.open();
        settle(&mut o);
        let at = o.log().scroll().offset();
        drag(&mut o, AREA.y + 200.0, AREA.y + 500.0);
        assert!(o.is_open());
        assert!((at - o.log().scroll().offset() - 300.0).abs() < 1.0);
        // On a log that fits: down closes it.
        let mut o = overlay(1);
        o.open();
        settle(&mut o);
        assert_eq!(o.log().scroll().max(), 0.0);
        drag(&mut o, AREA.y + 200.0, AREA.y + 500.0);
        assert!(!o.is_open());
        // A short drag, or one up, leaves it open.
        o.open();
        settle(&mut o);
        drag(&mut o, AREA.y + 200.0, AREA.y + 220.0);
        drag(&mut o, AREA.y + 500.0, AREA.y + 200.0);
        assert!(o.is_open());
    }

    #[test]
    fn it_is_drawn_only_while_shown() {
        let mut o = overlay(3);
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        o.draw(&mut list, &ctx, 1080.0);
        assert!(list.instances.is_empty());
        o.open();
        settle(&mut o);
        o.draw(&mut list, &ctx, 1080.0);
        assert!(!list.instances.is_empty());
    }
}
