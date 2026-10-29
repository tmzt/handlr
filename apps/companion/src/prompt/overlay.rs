//! A prompt card expanded (plan 20261027_03, decisions 2 and 4), built from
//! `TurnOverlay` (plan 20261023_04, the 18:07 EDT amendment, 3): a sheet
//! that slides up from the bottom, about two thirds of the screen high,
//! with a grab bar at its top, the card's header, its history scrolling,
//! and its input area pinned at its bottom. What is above it is dimmed.
//!
//! It opens and closes only on the user's choice: the card's up arrow
//! opens it ([`super::Prompts::expand`]); a tap on the grab bar, a swipe
//! down on it (or on a history with nothing above to go back to), and
//! Back close it. The slide is short ([`PromptOverlay::SLIDE`]) and driven
//! by `tick`; while it moves, the sheet is drawn where it has got to.
//!
//! Under its input area, while it is typed, the card's row of the + and
//! the keyboard toggle ([`InputRow`], plan 20261028_06, decision 8).

use std::time::Duration;

use libcontrols::{DrawContext, DrawControl, Rect, TextArea};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use super::PromptKind;
use super::history::History;
use super::input_row::{InputRow, InputTools, ROW_GAP};
use crate::agent::AgentHit;
use crate::agent::{PAD, columns, fill, line_h, one_line, wrap_hard};
use crate::controls::scroll::Touched;
use crate::theme::theme;

/// What a touch on the sheet did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayTouch {
    Nothing,
    /// The history scrolled.
    Scrolled,
    /// The grab bar was tapped, or the sheet swiped down: collapse it.
    Collapse,
    /// A tap on the input area.
    Text,
    /// A tap on the input row's + or toggle.
    Control(AgentHit),
}

/// A finger on the sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Swipe {
    id: u64,
    start_y: f32,
    /// Started on the grab bar: the history does not see it.
    handle: bool,
    /// Started on the input area.
    text: bool,
    /// Started on the input row's control.
    control: Option<AgentHit>,
    /// The history had nothing above to scroll back to when it started.
    at_top: bool,
}

/// The overlay: open or not, how far it has slid, and its parts.
pub struct PromptOverlay {
    open: bool,
    /// How far it has slid up: 0 closed, 1 open.
    shown: f32,
    /// Where the sheet is when open.
    area: Rect,
    scale: f32,
    swipe: Option<Swipe>,
    header: String,
    colour: [f32; 4],
    text_area: Option<TextArea>,
    /// The history's window, when open.
    history: Rect,
    /// The typed card's kind and typing's tools, for the row; and the row
    /// as placed (where it opens to).
    row_for: Option<(PromptKind, InputTools)>,
    row: Option<InputRow>,
}

impl Default for PromptOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptOverlay {
    /// How long the slide takes, each way.
    pub const SLIDE: Duration = Duration::from_millis(150);
    /// The share of the screen's height the sheet takes, at most.
    pub const SHARE: f32 = 2.0 / 3.0;
    /// The grab bar's band at the sheet's top, the header's line under it,
    /// and the drag down that closes it, in points.
    pub const HANDLE_H: f32 = 28.0;
    pub const HEADER_H: f32 = 24.0;
    pub const CLOSE_DRAG: f32 = 48.0;
    /// The input area's padding, its type size, and the lines it holds at
    /// least and grows to.
    pub const INPUT_PAD: f32 = 10.0;
    const INPUT_PT: f32 = 12.0;
    const INPUT_LINES: usize = 3;
    const INPUT_MOST: usize = 6;

    pub fn new() -> Self {
        PromptOverlay {
            open: false,
            shown: 0.0,
            area: Rect::default(),
            scale: 1.0,
            swipe: None,
            header: String::new(),
            colour: [0.0; 4],
            text_area: None,
            row_for: None,
            row: None,
            history: Rect::default(),
        }
    }

    /// Where the sheet is when open, at scale `s`.
    pub fn set_area(&mut self, area: Rect, s: f32) {
        self.area = area;
        self.scale = s;
        self.place();
    }

    /// The expanded card's header, its colour, and its input: the text,
    /// the placeholder, and whether it is typed; while typed, `row` is the
    /// card's kind and typing's tools for the row under it.
    pub fn set_input(
        &mut self,
        header: &str,
        colour: [f32; 4],
        text: &str,
        placeholder: &str,
        keyboard: bool,
        row: Option<(PromptKind, InputTools)>,
    ) {
        self.row_for = row.filter(|_| keyboard);
        self.header = header.to_string();
        self.colour = colour;
        let cursor = self
            .text_area
            .as_ref()
            .filter(|ta| ta.text() == text)
            .map(TextArea::cursor);
        let s = self.scale;
        let ta = TextArea::new(text, Rect::default())
            .with_padding([10.0 * s, 8.0 * s])
            .with_font_size(Self::INPUT_PT * s)
            .with_text_color(theme().text)
            .with_bg(theme().input)
            .with_corner_radius(8.0 * s)
            .with_placeholder(placeholder)
            .with_active(keyboard);
        ta.set_cursor(cursor.unwrap_or(text.len()));
        self.text_area = Some(ta);
        self.place();
    }

    /// The input area's height for its text: 3 lines of 12 pt, growing to
    /// 6.
    fn input_h(&self) -> f32 {
        let s = self.scale;
        let px = Self::INPUT_PT * s;
        let pad = Self::INPUT_PAD * s;
        let w = self.area.w - 2.0 * pad - 20.0 * s;
        let text = self
            .text_area
            .as_ref()
            .map(|ta| ta.text())
            .unwrap_or_default();
        let lines = wrap_hard(&text, columns(w, px))
            .len()
            .clamp(Self::INPUT_LINES, Self::INPUT_MOST);
        lines as f32 * line_h(px) + 16.0 * s
    }

    /// The history's window and the input area, in the area.
    fn place(&mut self) {
        let s = self.scale;
        let a = self.area;
        let pad = Self::INPUT_PAD * s;
        let head = (Self::HANDLE_H + Self::HEADER_H) * s;
        let input_h = self.input_h();
        let row_h = self
            .row_for
            .map_or(0.0, |(kind, tools)| tools.grows(kind, s));
        let history_h = (a.h - head - input_h - row_h - 2.0 * pad).max(0.0);
        self.history = Rect::new(a.x, a.y + head, a.w, history_h);
        let r = Rect::new(
            a.x + pad,
            a.y + head + history_h + pad,
            a.w - 2.0 * pad,
            input_h,
        );
        if let Some(ta) = &mut self.text_area {
            ta.rect = r;
        }
        self.row = self.row_for.and_then(|(kind, tools)| {
            InputRow::lay_out(kind, r.x, r.y + r.h + ROW_GAP * s, r.w, s, &tools)
        });
    }

    /// The input row's control at `(x, y)`, where it is now.
    pub fn row_hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        self.row.as_ref()?.hit(x, y, self.drop())
    }

    /// Where the input row's `hit` is now.
    pub fn row_rect(&self, hit: AgentHit) -> Option<Rect> {
        self.row.as_ref()?.rect_of(hit, self.drop())
    }

    /// The history's window when open.
    pub fn history_rect(&self) -> Rect {
        self.history
    }

    /// The input area where it is now.
    pub fn text_rect(&self) -> Option<Rect> {
        let drop = self.drop();
        self.text_area
            .as_ref()
            .map(|ta| Rect::new(ta.rect.x, ta.rect.y + drop, ta.rect.w, ta.rect.h))
    }

    pub fn input_text(&self) -> String {
        self.text_area
            .as_ref()
            .map(|ta| ta.text())
            .unwrap_or_default()
    }

    /// Puts the input's cursor at `(x, y)`, where it is now.
    pub fn place_cursor(&self, x: f32, y: f32) {
        if let Some(ta) = &self.text_area {
            ta.set_cursor_from_pos([x, y - self.drop()]);
        }
    }

    /// The grab bar's band, where it is now.
    pub fn handle_rect(&self) -> Rect {
        let r = self.rect();
        Rect::new(r.x, r.y, r.w, Self::HANDLE_H * self.scale)
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

    /// Open, or opening.
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

    /// A touch that started on the sheet. A tap on the grab bar, or a drag
    /// down on it or on a history with nothing above to go back to,
    /// collapses it; a tap on the input area is the input's; anything else
    /// is the history's.
    #[allow(clippy::too_many_arguments)]
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
        history: &mut History,
    ) -> OverlayTouch {
        if phase == TouchPhase::Started {
            let text = self.text_rect().is_some_and(|r| r.contains([x, y]));
            if text {
                self.place_cursor(x, y);
            }
            self.swipe = Some(Swipe {
                id,
                start_y: y,
                handle: self.handle_rect().contains([x, y]),
                text,
                control: self.row_hit(x, y),
                at_top: history.scroll().offset() <= 0.0,
            });
        }
        let Some(swipe) = self.swipe.filter(|w| w.id == id) else {
            return OverlayTouch::Nothing;
        };
        let ended = phase == TouchPhase::Ended;
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.swipe = None;
        }
        if swipe.text {
            return if ended {
                OverlayTouch::Text
            } else {
                OverlayTouch::Nothing
            };
        }
        if let Some(hit) = swipe.control {
            return if ended && self.row_hit(x, y) == Some(hit) {
                OverlayTouch::Control(hit)
            } else {
                OverlayTouch::Nothing
            };
        }
        let down = y - swipe.start_y > Self::CLOSE_DRAG * self.scale;
        if swipe.handle {
            return if ended {
                // A tap, or a swipe down: either collapses it.
                self.close();
                OverlayTouch::Collapse
            } else {
                OverlayTouch::Nothing
            };
        }
        let touched = history.touch(id, phase, x, y, now);
        if ended && down && swipe.at_top && history.scroll().offset() <= 0.0 {
            self.close();
            return OverlayTouch::Collapse;
        }
        match touched {
            Touched::Scrolled => OverlayTouch::Scrolled,
            _ => OverlayTouch::Nothing,
        }
    }

    /// Carries the slide on. Returns whether it moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let step = dt.as_secs_f32() / Self::SLIDE.as_secs_f32();
        let before = self.shown;
        self.shown = if self.open {
            (self.shown + step).min(1.0)
        } else {
            (self.shown - step).max(0.0)
        };
        self.shown != before
    }

    /// Draws the shade over everything above the sheet, `width` wide, then
    /// the sheet, its grab bar, the header, `history` and the input area.
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext, width: f32, history: &History) {
        if !self.is_shown() {
            return;
        }
        let s = self.scale;
        let r = self.rect();
        let drop = self.drop();
        let mut shade = theme().overlay;
        shade[3] *= self.shown;
        fill(list, Rect::new(0.0, 0.0, width, r.y), shade, 0.0);
        fill(list, r, theme().header, 16.0 * s);
        // The grab bar, a filled pill.
        let (hw, hh) = (40.0 * s, 4.0 * s);
        let bar = Rect::new(
            r.x + (r.w - hw) * 0.5,
            r.y + (Self::HANDLE_H * s - hh) * 0.5,
            hw,
            hh,
        );
        fill(list, bar, theme().band, hh * 0.5);
        let px = 13.0 * s;
        let pad = PAD * s;
        let header = one_line(&self.header, columns(r.w - 2.0 * pad, px));
        let y = r.y + Self::HANDLE_H * s + (Self::HEADER_H * s - line_h(px)) * 0.5;
        ctx.draw_text(list, &header, [r.x + pad, y], px, self.colour);
        history.draw_moved(list, ctx, drop);
        if let Some(ta) = &self.text_area {
            let moved = Rect::new(ta.rect.x, ta.rect.y + drop, ta.rect.w, ta.rect.h);
            fill(list, moved, theme().input, 8.0 * s);
            let mut ta = ta.clone();
            ta.rect = moved;
            ta.draw(list, ctx);
        }
        if let Some(row) = &self.row {
            row.draw(list, drop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flankr_companion::Speaker;

    /// Two thirds of the phone, above the bottom row.
    const AREA: Rect = Rect {
        x: 0.0,
        y: 780.0,
        w: 1080.0,
        h: 1376.0,
    };

    const MS: Duration = Duration::from_millis(16);

    fn overlay(turns: usize) -> (PromptOverlay, History) {
        let mut o = PromptOverlay::new();
        o.set_area(AREA, 2.0);
        o.set_input(
            "\u{2726} Gemini",
            theme().accent,
            "",
            "Ask Gemini\u{2026}",
            false,
            None,
        );
        let mut h = History::new();
        h.set_viewport(o.history_rect(), 2.0);
        for i in 0..turns {
            h.push(Speaker::User, format!("question {i}"));
            h.push(Speaker::Gemini, format!("Answer {i}, at some length."));
        }
        (o, h)
    }

    fn settle(o: &mut PromptOverlay) {
        o.tick(PromptOverlay::SLIDE);
    }

    /// Drags from `from` to `to` at `x` 540, then lifts after a pause.
    fn drag(o: &mut PromptOverlay, h: &mut History, from: f32, to: f32) -> OverlayTouch {
        o.touch(1, TouchPhase::Started, 540.0, from, Duration::ZERO, h);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += MS;
            let y = from + (to - from) * i as f32 / 10.0;
            o.touch(1, TouchPhase::Moved, 540.0, y, now, h);
        }
        o.touch(
            1,
            TouchPhase::Ended,
            540.0,
            to,
            now + Duration::from_millis(300),
            h,
        )
    }

    #[test]
    fn it_slides_up_and_down_in_the_slide_time() {
        let (mut o, _) = overlay(1);
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
    fn the_grab_bar_collapses_it_and_the_history_scrolls() {
        // A tap on the grab bar.
        let (mut o, mut h) = overlay(30);
        o.open();
        settle(&mut o);
        assert_eq!(
            drag(&mut o, &mut h, AREA.y + 10.0, AREA.y + 10.0),
            OverlayTouch::Collapse
        );
        assert!(!o.is_open());
        // A swipe down on it, whatever the history shows.
        o.open();
        settle(&mut o);
        assert!(h.scroll().max() > 0.0);
        assert_eq!(
            drag(&mut o, &mut h, AREA.y + 10.0, AREA.y + 300.0),
            OverlayTouch::Collapse
        );
        // On the history, scrolled to the newest: down scrolls back.
        o.open();
        settle(&mut o);
        let at = h.scroll().offset();
        drag(&mut o, &mut h, AREA.y + 200.0, AREA.y + 500.0);
        assert!(o.is_open());
        assert!((at - h.scroll().offset() - 300.0).abs() < 1.0);
        // On a history that fits: down collapses it.
        let (mut o, mut h) = overlay(1);
        o.open();
        settle(&mut o);
        assert_eq!(
            drag(&mut o, &mut h, AREA.y + 200.0, AREA.y + 500.0),
            OverlayTouch::Collapse
        );
        // A short drag, or one up, leaves it open.
        o.open();
        settle(&mut o);
        drag(&mut o, &mut h, AREA.y + 200.0, AREA.y + 220.0);
        drag(&mut o, &mut h, AREA.y + 500.0, AREA.y + 200.0);
        assert!(o.is_open());
    }

    #[test]
    fn the_input_is_at_the_bottom_three_lines_and_grows() {
        let (mut o, mut h) = overlay(3);
        o.open();
        settle(&mut o);
        let r = o.text_rect().expect("placed");
        assert!(r.y > AREA.y + (PromptOverlay::HANDLE_H + PromptOverlay::HEADER_H) * 2.0);
        assert!(r.y + r.h <= AREA.y + AREA.h);
        assert!(r.h >= 3.0 * line_h(24.0));
        assert!(o.history_rect().y + o.history_rect().h <= r.y);
        assert_eq!(
            drag(&mut o, &mut h, r.y + 10.0, r.y + 10.0),
            OverlayTouch::Text
        );
        assert!(o.is_open());
        o.set_input("x", theme().accent, "1\n2\n3\n4\n5", "", true, None);
        let taller = o.text_rect().unwrap();
        assert!(taller.h > r.h);
        assert_eq!(o.input_text(), "1\n2\n3\n4\n5");
    }

    #[test]
    fn typed_the_row_is_under_the_input_and_its_toggle_is_tapped() {
        let (mut o, mut h) = overlay(3);
        o.open();
        settle(&mut o);
        let tools = InputTools {
            mobile: true,
            images: false,
            keyboard_up: true,
            physical: false,
        };
        let before = o.text_rect().unwrap();
        o.set_input(
            "\u{2726} Gemini",
            theme().accent,
            "",
            "Ask Gemini\u{2026}",
            true,
            Some((PromptKind::GeneralAgent, tools)),
        );
        let r = o.text_rect().unwrap();
        assert!(
            (before.y - r.y - 48.0 * 2.0).abs() < 0.01,
            "lifted by 48 pt"
        );
        let ime = o
            .row_rect(AgentHit::PromptIme(PromptKind::GeneralAgent))
            .expect("the toggle");
        assert!((ime.y - (r.y + r.h + 4.0 * 2.0)).abs() < 0.01);
        assert!(ime.y + ime.h <= o.rect().y + o.rect().h);
        assert!(
            o.row_rect(AgentHit::PromptAttach(PromptKind::GeneralAgent))
                .is_some()
        );
        let (x, y) = (ime.x + 5.0, ime.y + 5.0);
        o.touch(1, TouchPhase::Started, x, y, Duration::ZERO, &mut h);
        assert_eq!(
            o.touch(1, TouchPhase::Ended, x, y, MS, &mut h),
            OverlayTouch::Control(AgentHit::PromptIme(PromptKind::GeneralAgent))
        );
    }

    #[test]
    fn it_is_drawn_only_while_shown() {
        let (mut o, h) = overlay(3);
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        o.draw(&mut list, &ctx, 1080.0, &h);
        assert!(list.instances.is_empty());
        o.open();
        settle(&mut o);
        o.draw(&mut list, &ctx, 1080.0, &h);
        assert!(!list.instances.is_empty());
    }
}
