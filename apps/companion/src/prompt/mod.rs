//! One prompt card for the general agent and dictated messages
//! (docs/design/20261027_03_UNIFIED_PROMPT_CARD_PLAN.md).
//!
//! [`PromptCard`] is the one control, in two modes ([`PromptKind`]): the
//! general agent ("✦ Gemini", in the Gemini accent) and a message to a
//! session ("Message to <agent>", in that session's colour). It is pinned
//! above the controls by default, expanded on the user's choice into an
//! overlay sheet ([`PromptOverlay`]) holding its history ([`History`]),
//! and, for the general agent, hidden to two one-line bars ([`Bars`])
//! while a message card is open (decisions 1 to 5).
//!
//! [`Prompts`] is what each screen owns: its Gemini card, its message card
//! and their overlay, laid out, touched and drawn together, one card open
//! at a time. Nothing is borrowed across screens (decision 1).

pub mod bars;
pub mod card;
pub mod history;
pub mod overlay;

use std::time::Duration;

use flankr_companion::PromptMode;
use libcontrols::{DrawContext, Rect};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::AgentHit;
use crate::settings;

pub use bars::Bars;
pub use card::{PromptCard, Reply};
pub use history::{History, Sent};
pub use overlay::{OverlayTouch, PromptOverlay};

/// A prompt card's mode (decision 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromptKind {
    /// Speaking or typing to Gemini.
    GeneralAgent,
    /// Dictating or typing a message to a coding agent's session.
    Message,
}

/// A prompt card's state (decision 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CardState {
    /// Above the controls: the header, the latest exchange, and the input
    /// area while there is input.
    #[default]
    Pinned,
    /// The overlay sheet, with the history and the input area.
    Expanded,
    /// *General agent* only: the card goes, its two bars stay.
    Hidden,
}

/// What is being said or typed into a card, as the client's dictation
/// gives it (`ui::prompt_input`).
#[derive(Clone, Debug, PartialEq)]
pub struct PromptInput {
    /// Where it stands ("Taking a message", "Submitting in 3 s…"), and its
    /// colour; none for the general agent while collecting.
    pub status: Option<(String, [f32; 4])>,
    /// Warnings under it: no agent focused, the focus moved.
    pub notes: Vec<(String, [f32; 4])>,
    /// The text so far.
    pub text: String,
    pub mode: PromptMode,
}

impl Default for PromptInput {
    fn default() -> Self {
        PromptInput {
            status: None,
            notes: Vec::new(),
            text: String::new(),
            mode: PromptMode::Voice,
        }
    }
}

impl PromptInput {
    /// `text` being said.
    pub fn said(text: &str) -> Self {
        PromptInput {
            text: text.to_string(),
            mode: PromptMode::Voice,
            ..Default::default()
        }
    }

    /// `text` being typed.
    pub fn typed(text: &str) -> Self {
        PromptInput {
            text: text.to_string(),
            mode: PromptMode::Keyboard,
            ..Default::default()
        }
    }

    pub fn keyboard(&self) -> bool {
        self.mode == PromptMode::Keyboard
    }
}

/// What a touch on the prompts did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptTouched {
    /// Theirs, and nothing to tell: a scroll, or a touch still going on.
    Held,
    /// A tap or a swipe that the screen is to act on.
    Hit(AgentHit),
}

/// Where a touch on the prompts started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// The overlay's sheet.
    Sheet,
    /// Above the open sheet: a tap collapses it.
    Shade,
    /// A pinned card, or the bars.
    Pinned,
}

/// A screen's prompt cards: Gemini's and a message's, and the overlay the
/// expanded one is drawn in.
pub struct Prompts {
    gemini: PromptCard,
    message: PromptCard,
    overlay: PromptOverlay,
    /// The Gemini card shown over an open message, from a tap on its bars.
    peek: bool,
    touch: Option<(u64, Target)>,
    scale: f32,
}

impl Default for Prompts {
    fn default() -> Self {
        Self::new()
    }
}

impl Prompts {
    /// The gap between the cards stacked above the controls, in points.
    pub const GAP: f32 = 8.0;

    pub fn new() -> Self {
        Prompts {
            gemini: PromptCard::new(PromptKind::GeneralAgent),
            message: PromptCard::new(PromptKind::Message),
            overlay: PromptOverlay::new(),
            peek: false,
            touch: None,
            scale: 1.0,
        }
    }

    pub fn gemini(&self) -> &PromptCard {
        &self.gemini
    }

    pub fn gemini_mut(&mut self) -> &mut PromptCard {
        &mut self.gemini
    }

    pub fn message(&self) -> &PromptCard {
        &self.message
    }

    pub fn message_mut(&mut self) -> &mut PromptCard {
        &mut self.message
    }

    pub fn card(&self, kind: PromptKind) -> &PromptCard {
        match kind {
            PromptKind::GeneralAgent => &self.gemini,
            PromptKind::Message => &self.message,
        }
    }

    pub fn card_mut(&mut self, kind: PromptKind) -> &mut PromptCard {
        match kind {
            PromptKind::GeneralAgent => &mut self.gemini,
            PromptKind::Message => &mut self.message,
        }
    }

    pub fn overlay(&self) -> &PromptOverlay {
        &self.overlay
    }

    /// Whether a message card is open.
    pub fn message_open(&self) -> bool {
        self.message.has_content()
    }

    /// Whether Gemini's card can show at all: the general agent on and
    /// offered by the daemon (plan 20261027_02, decision 9).
    fn gemini_on() -> bool {
        settings::general_agent_on()
    }

    /// The card shown expanded, if one is: the overlay holds it.
    pub fn expanded(&self) -> Option<PromptKind> {
        if self.message_open() && !self.peek {
            return self.message.is_expanded().then_some(PromptKind::Message);
        }
        (Self::gemini_on() && self.gemini.has_content() && self.gemini.is_expanded())
            .then_some(PromptKind::GeneralAgent)
    }

    /// `kind`'s state as shown now.
    pub fn state(&self, kind: PromptKind) -> CardState {
        match kind {
            PromptKind::GeneralAgent if self.message_open() && !self.peek => CardState::Hidden,
            _ if self.card(kind).is_expanded() => CardState::Expanded,
            _ => CardState::Pinned,
        }
    }

    /// Expands `kind`'s card into the overlay (the up arrow).
    pub fn expand(&mut self, kind: PromptKind) {
        self.card_mut(kind).set_expanded(true);
        self.sync_overlay();
    }

    /// Collapses the expanded card to pinned (the grab bar, a swipe down,
    /// Back). Returns whether one was expanded.
    pub fn collapse(&mut self) -> bool {
        let Some(kind) = self.expanded() else {
            return false;
        };
        self.card_mut(kind).set_expanded(false);
        if kind == PromptKind::GeneralAgent {
            self.peek = false;
        }
        self.sync_overlay();
        true
    }

    /// A tap on Gemini's bars: its card opens again, over the message, in
    /// the overlay (the message has no hidden state to go to).
    pub fn open_from_bars(&mut self) {
        self.peek = true;
        self.gemini.set_expanded(true);
        self.sync_overlay();
    }

    /// The centre button's 2 s hold (decision 7): Gemini's card in its
    /// pinned state.
    pub fn hold_open(&mut self) {
        self.gemini.open_pinned();
        self.peek = false;
        self.sync_overlay();
    }

    /// The system Back: collapses the expanded card. Returns whether one
    /// was.
    pub fn back(&mut self) -> bool {
        self.collapse()
    }

    /// The overlay follows the expanded card: open while one is.
    fn sync_overlay(&mut self) {
        if !self.message_open() {
            self.peek = false;
        }
        if self.expanded().is_some() {
            self.overlay.open();
        } else {
            self.overlay.close();
        }
    }

    /// Lays the pinned cards and the bars out above `bottom`, `x` to `x +
    /// w`, and the overlay's sheet from two thirds up the `screen_h`-high
    /// screen (but not above `top`) down to above them. Returns the top of
    /// what was placed: `bottom` with nothing pinned.
    pub fn lay_out(&mut self, x: f32, w: f32, bottom: f32, top: f32, screen_h: f32, s: f32) -> f32 {
        self.scale = s;
        self.sync_overlay();
        let gap = Self::GAP * s;
        let mut above = bottom;
        let gemini_on = Self::gemini_on();
        let message = self.message_open() && !self.peek;
        // The message card, pinned, nearest the controls.
        if message && !self.message.is_expanded() {
            let h = self.message.lay_out(x, w, above, s);
            above -= h + gap;
        } else {
            self.message.clear_layout();
        }
        // Gemini's: its bars while a message is open, else its card.
        let gemini_pinned = gemini_on && self.gemini.has_content() && !self.gemini.is_expanded();
        if gemini_on && message && self.gemini.has_content() {
            self.gemini.clear_layout();
            let h = self.gemini.lay_out_bars(x, w, above, s);
            if h > 0.0 {
                above -= h + gap;
            }
        } else if gemini_pinned {
            let h = self.gemini.lay_out(x, w, above, s);
            above -= h + gap;
        } else {
            self.gemini.clear_layout();
        }
        let sheet_top = (screen_h * (1.0 - PromptOverlay::SHARE))
            .max(top)
            .min(above);
        self.overlay
            .set_area(Rect::new(0.0, sheet_top, x * 2.0 + w, above - sheet_top), s);
        if let Some(kind) = self.expanded() {
            let card = self.card_mut(kind);
            let (header, colour) = card.header();
            let placeholder = card.placeholder();
            let text = card.input_text();
            let keyboard = card.input().is_some_and(PromptInput::keyboard);
            self.overlay
                .set_input(&header, colour, &text, &placeholder, keyboard);
            let area = self.overlay.history_rect();
            self.card_mut(kind).history_mut().set_viewport(area, s);
        }
        above
    }

    /// The control at `(x, y)` on a pinned card, the bars or the open
    /// sheet, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        if self.overlay.is_open() {
            if let Some(kind) = self.expanded()
                && self.overlay.text_rect().is_some_and(|r| r.contains([x, y]))
            {
                self.overlay.place_cursor(x, y);
                return Some(AgentHit::PromptText(kind));
            }
            return None;
        }
        self.message
            .hit(x, y)
            .or_else(|| self.gemini.hit(x, y))
            .or_else(|| {
                self.gemini
                    .bars()
                    .contains(x, y)
                    .then_some(AgentHit::PromptBars)
            })
    }

    /// Where `hit` is, if it shows.
    pub fn rect_of(&self, hit: AgentHit) -> Option<Rect> {
        match hit {
            AgentHit::PromptText(kind) if self.expanded() == Some(kind) => self.overlay.text_rect(),
            AgentHit::PromptCollapse if self.overlay.is_open() => Some(self.overlay.handle_rect()),
            AgentHit::PromptBars => self.gemini.bars().rect(),
            _ => self
                .message
                .rect_of(hit)
                .or_else(|| self.gemini.rect_of(hit)),
        }
    }

    /// Whether a touch starting at `(x, y)` is the prompts': on the open
    /// sheet or above it, or on a pinned card or the bars.
    pub fn claims(&self, x: f32, y: f32) -> bool {
        if self.overlay.is_open() {
            return self.overlay.contains(x, y) || y < self.overlay.rect().y;
        }
        self.message.contains(x, y) || self.gemini.contains(x, y)
    }

    /// A touch at `now`. `None`: not the prompts' (it started elsewhere);
    /// the screen goes on with it. The arrow, the grab bar, a swipe down
    /// on it, a tap above the sheet and the bars are acted on here and
    /// also returned, for the app to remember the card's state.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Option<PromptTouched> {
        if phase == TouchPhase::Started {
            let target = if self.overlay.is_open() && self.overlay.contains(x, y) {
                Target::Sheet
            } else if self.overlay.is_open() && y < self.overlay.rect().y {
                Target::Shade
            } else if !self.overlay.is_open()
                && (self.message.contains(x, y) || self.gemini.contains(x, y))
            {
                Target::Pinned
            } else {
                return None;
            };
            self.touch = Some((id, target));
        }
        let (held, target) = self.touch?;
        if held != id {
            return None;
        }
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.touch = None;
        }
        let ended = phase == TouchPhase::Ended;
        let hit = match target {
            Target::Sheet => {
                let Some(kind) = self.expanded() else {
                    return Some(PromptTouched::Held);
                };
                let history = match kind {
                    PromptKind::GeneralAgent => self.gemini.history_mut(),
                    PromptKind::Message => self.message.history_mut(),
                };
                match self.overlay.touch(id, phase, x, y, now, history) {
                    OverlayTouch::Collapse => {
                        self.collapse();
                        Some(AgentHit::PromptCollapse)
                    }
                    OverlayTouch::Text => Some(AgentHit::PromptText(kind)),
                    OverlayTouch::Nothing | OverlayTouch::Scrolled => None,
                }
            }
            Target::Shade if ended => {
                self.collapse();
                Some(AgentHit::PromptCollapse)
            }
            Target::Shade => None,
            Target::Pinned if ended => match self.hit(x, y) {
                Some(AgentHit::PromptExpand(kind)) => {
                    self.expand(kind);
                    Some(AgentHit::PromptExpand(kind))
                }
                Some(AgentHit::PromptBars) => {
                    self.open_from_bars();
                    Some(AgentHit::PromptBars)
                }
                other => other,
            },
            Target::Pinned => None,
        };
        Some(match hit {
            Some(hit) => PromptTouched::Hit(hit),
            None => PromptTouched::Held,
        })
    }

    /// Carries the overlay's slide and the history's fling on. Returns
    /// whether anything moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let slid = self.overlay.tick(dt);
        let scrolled = match self.expanded() {
            Some(kind) if self.overlay.is_shown() => self.card_mut(kind).history_mut().tick(dt),
            _ => false,
        };
        slid || scrolled
    }

    /// The theme changed: the histories are laid out again in its colours.
    pub fn restyle(&mut self) {
        self.gemini.history_mut().restyle();
        self.message.history_mut().restyle();
    }

    /// Draws the pinned cards and the bars. Their buttons (a reply's
    /// controls) are the screen's to draw, from [`PromptCard::buttons`].
    pub fn draw_pinned(&self, list: &mut DrawList, ctx: &DrawContext) {
        self.message.draw(list, ctx);
        self.gemini.draw(list, ctx);
    }

    /// Draws the overlay, `width` wide, if it shows: the shade over what is
    /// above it, the sheet, and the expanded card's history and input.
    pub fn draw_overlay(&self, list: &mut DrawList, ctx: &DrawContext, width: f32) {
        if !self.overlay.is_shown() {
            return;
        }
        // While it slides down, the card it held is drawn in it still.
        let kind = self.expanded().unwrap_or(if self.message_open() {
            PromptKind::Message
        } else {
            PromptKind::GeneralAgent
        });
        self.overlay
            .draw(list, ctx, width, self.card(kind).history());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flankr_companion::Speaker;

    /// The phone at scale 2: the controls' top at 2200.
    const W: f32 = 1080.0;
    const H: f32 = 2340.0;
    const BOTTOM: f32 = 2150.0;
    const TOP: f32 = 300.0;
    const MS: Duration = Duration::from_millis(16);

    fn prompts() -> Prompts {
        let mut p = Prompts::new();
        p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0);
        p
    }

    fn settle(p: &mut Prompts) {
        p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0);
        p.tick(PromptOverlay::SLIDE);
        p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0);
    }

    fn tap(p: &mut Prompts, x: f32, y: f32) -> Option<PromptTouched> {
        p.touch(1, TouchPhase::Started, x, y, Duration::ZERO)?;
        p.touch(1, TouchPhase::Ended, x, y, MS)
    }

    fn talk(p: &mut Prompts, turns: usize) {
        for i in 0..turns {
            p.gemini_mut()
                .push_turn(Speaker::User, &format!("question {i}"));
            p.gemini_mut()
                .push_turn(Speaker::Gemini, &format!("Answer {i}, at some length."));
        }
    }

    #[test]
    fn each_mode_in_each_state() {
        let mut p = prompts();
        // Nothing yet: nothing pinned, no overlay.
        assert_eq!(p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0), BOTTOM);
        assert!(!p.overlay().is_open());
        // Gemini speaks: its card, pinned.
        talk(&mut p, 2);
        let top = p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0);
        assert!(top < BOTTOM);
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Pinned);
        let card = p.gemini().rect();
        assert!(card.y + card.h <= BOTTOM + 0.01 && card.y >= top - 0.01);
        assert!(
            p.rect_of(AgentHit::PromptExpand(PromptKind::GeneralAgent))
                .is_some()
        );
        // Expanded: the sheet holds it, nothing pinned.
        p.expand(PromptKind::GeneralAgent);
        settle(&mut p);
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Expanded);
        assert!(p.overlay().is_open());
        assert_eq!(p.expanded(), Some(PromptKind::GeneralAgent));
        let sheet = p.overlay().rect();
        assert!((sheet.y - H / 3.0).abs() < 1.0);
        assert!((sheet.y + sheet.h - BOTTOM).abs() < 0.01);
        // A message opens: Gemini's card goes to its bars, the overlay
        // closes, and the message is pinned.
        p.message_mut()
            .set_input(Some(PromptInput::said("please go on")));
        settle(&mut p);
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Hidden);
        assert_eq!(p.state(PromptKind::Message), CardState::Pinned);
        assert!(!p.overlay().is_shown());
        let message = p.message().rect();
        let bars = p.gemini().bars().rect().expect("the two bars");
        assert!(bars.y + bars.h <= message.y);
        assert_eq!(p.gemini().bars().lines().len(), 2);
        // The message expanded: the sheet ends above the bars.
        p.expand(PromptKind::Message);
        settle(&mut p);
        assert_eq!(p.expanded(), Some(PromptKind::Message));
        let bars = p.gemini().bars().rect().expect("still the bars");
        assert!(
            (bars.y + bars.h - BOTTOM).abs() < 0.01,
            "where the card was"
        );
        assert!(p.overlay().rect().y + p.overlay().rect().h <= bars.y + 0.01);
        // Sent or cancelled: Gemini's card comes back as it was, expanded.
        p.message_mut().set_input(None);
        settle(&mut p);
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Expanded);
        assert_eq!(p.expanded(), Some(PromptKind::GeneralAgent));
        // The message card remembers it was expanded.
        assert!(p.message().is_expanded());
    }

    #[test]
    fn the_arrow_expands_and_the_grab_bar_collapses() {
        let mut p = prompts();
        talk(&mut p, 3);
        settle(&mut p);
        let arrow = p
            .rect_of(AgentHit::PromptExpand(PromptKind::GeneralAgent))
            .unwrap();
        // A tap elsewhere on the card does not expand it.
        let card = p.gemini().rect();
        assert_eq!(
            tap(&mut p, card.x + 40.0, card.y + card.h - 10.0),
            Some(PromptTouched::Held)
        );
        assert!(!p.overlay().is_open());
        assert_eq!(
            tap(&mut p, arrow.x + arrow.w * 0.5, arrow.y + arrow.h * 0.5),
            Some(PromptTouched::Hit(AgentHit::PromptExpand(
                PromptKind::GeneralAgent
            )))
        );
        settle(&mut p);
        assert!(p.overlay().is_open());
        // The grab bar collapses it.
        let bar = p.rect_of(AgentHit::PromptCollapse).unwrap();
        assert_eq!(
            tap(&mut p, bar.x + bar.w * 0.5, bar.y + bar.h * 0.5),
            Some(PromptTouched::Hit(AgentHit::PromptCollapse))
        );
        settle(&mut p);
        assert!(!p.overlay().is_shown());
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Pinned);
        // A swipe down on the bar, and Back, also collapse it.
        p.expand(PromptKind::GeneralAgent);
        settle(&mut p);
        let bar = p.rect_of(AgentHit::PromptCollapse).unwrap();
        let y0 = bar.y + bar.h * 0.5;
        p.touch(1, TouchPhase::Started, 540.0, y0, Duration::ZERO);
        p.touch(1, TouchPhase::Moved, 540.0, y0 + 150.0, MS);
        assert_eq!(
            p.touch(1, TouchPhase::Ended, 540.0, y0 + 300.0, MS * 2),
            Some(PromptTouched::Hit(AgentHit::PromptCollapse))
        );
        assert!(!p.overlay().is_open());
        p.expand(PromptKind::GeneralAgent);
        assert!(p.back());
        assert!(!p.back(), "Back with nothing expanded is not theirs");
        // Nothing expands on its own: Gemini speaking leaves it pinned.
        talk(&mut p, 2);
        settle(&mut p);
        assert!(!p.overlay().is_open());
    }

    #[test]
    fn the_history_is_geminis_conversation_or_only_the_sent_messages() {
        let mut p = prompts();
        talk(&mut p, 2);
        p.expand(PromptKind::GeneralAgent);
        settle(&mut p);
        let texts: Vec<String> = p
            .gemini()
            .history()
            .rows()
            .iter()
            .map(|r| r.text.clone())
            .collect();
        assert_eq!(texts[0], "> question 0");
        assert!(texts.iter().any(|t| t.starts_with("< Answer 1")));
        p.collapse();
        // The message's: only what was sent, with its time; no replies.
        p.message_mut()
            .set_input(Some(PromptInput::typed("and push")));
        p.message_mut().push_sent(Sent {
            text: "run the tests".into(),
            time: "09:41".into(),
        });
        p.expand(PromptKind::Message);
        settle(&mut p);
        let texts: Vec<&str> = p
            .message()
            .history()
            .rows()
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(texts, ["09:41", "> run the tests"]);
        // Its input is at the sheet's bottom, a tap away.
        let text = p.overlay().text_rect().unwrap();
        let sheet = p.overlay().rect();
        assert!(text.y + text.h <= sheet.y + sheet.h + 0.01);
        assert_eq!(p.overlay().input_text(), "and push");
        assert_eq!(
            tap(&mut p, text.x + 10.0, text.y + 10.0),
            Some(PromptTouched::Hit(AgentHit::PromptText(
                PromptKind::Message
            )))
        );
    }

    #[test]
    fn hidden_shows_the_two_bars_and_a_tap_brings_gemini_back() {
        let mut p = prompts();
        talk(&mut p, 1);
        p.gemini_mut().set_partial(Some("and co".into()));
        p.message_mut().set_input(Some(PromptInput::said("hello")));
        settle(&mut p);
        let lines = p.gemini().bars().lines();
        assert_eq!(lines[0], "> and co\u{2026}");
        assert_eq!(lines[1], "< Answer 0, at some length.");
        // Listening with nothing heard yet.
        p.gemini_mut().set_partial(None);
        p.gemini_mut().set_listening(true);
        settle(&mut p);
        assert_eq!(p.gemini().bars().lines()[0], "Listening\u{2026}");
        let bars = p.gemini().bars().rect().unwrap();
        assert_eq!(
            tap(&mut p, bars.x + 20.0, bars.y + 5.0),
            Some(PromptTouched::Hit(AgentHit::PromptBars))
        );
        settle(&mut p);
        assert_eq!(p.expanded(), Some(PromptKind::GeneralAgent));
        assert!(p.message().rect().h == 0.0, "one card at a time");
        // Collapsed: back to the bars over the message.
        assert!(p.collapse());
        settle(&mut p);
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Hidden);
        assert!(p.message().rect().h > 0.0);
    }

    #[test]
    fn the_hold_opens_geminis_card_pinned() {
        let mut p = prompts();
        // Nothing said yet: the hold still shows the card.
        p.hold_open();
        settle(&mut p);
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Pinned);
        assert!(p.gemini().rect().h > 0.0);
        assert!(!p.overlay().is_open());
        // Left expanded: the hold brings it back to pinned.
        p.expand(PromptKind::GeneralAgent);
        settle(&mut p);
        p.hold_open();
        settle(&mut p);
        assert!(!p.overlay().is_open());
        assert_eq!(p.state(PromptKind::GeneralAgent), CardState::Pinned);
    }

    #[test]
    fn without_the_general_agent_geminis_card_and_bars_are_hidden() {
        use flankr_companion::wire::LlmInfo;
        let mut p = prompts();
        talk(&mut p, 1);
        p.expand(PromptKind::GeneralAgent);
        settings::set_llm(LlmInfo {
            general_agent: false,
            summaries: true,
            reason: None,
        });
        assert_eq!(p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0), BOTTOM);
        assert!(!p.overlay().is_open());
        p.message_mut().set_input(Some(PromptInput::said("hi")));
        p.lay_out(24.0, W - 48.0, BOTTOM, TOP, H, 2.0);
        assert!(p.gemini().bars().rect().is_none());
        settings::set_llm(LlmInfo::available());
    }
}
