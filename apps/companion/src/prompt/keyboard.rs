//! The one keyboard route to the prompt cards (plan 20261027_03, decision
//! 6, step 3), in place of the agent view's and the overlay's own: a
//! single focused prompt (the screen and the card's mode), and one place
//! that decides whether the soft keyboard is up ([`KeyboardRoute::sync`]),
//! so the app calls `start_text_input` and `stop_text_input` once each.
//!
//! The L3 amendment of 2026-10-25 23:34 EDT carries over: a tap on the text
//! area focuses it and shows the soft keyboard; dismissing the keyboard
//! leaves the card and its text, and a tap shows it again; a physical
//! keyboard types directly and keeps the soft keyboard hidden.

use libmorph::Key;

use super::PromptKind;

/// The screen a prompt card is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptScreen {
    Overview,
    Agent,
}

/// What the keyboard types into: a prompt card (its screen and its
/// card's mode), or the agent view's armed terminal (plan 20261027_07,
/// decision 7: keys go to the pane, not into a card).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusedPrompt {
    Card {
        screen: PromptScreen,
        kind: PromptKind,
    },
    Terminal,
    /// A one-line field outside the prompt cards: a remote's nickname in
    /// its details (plan 20261024_02, the 2026-10-28 17:56 EDT amendment).
    Field,
}

/// What is typed into, as the app sees it each frame: a typed prompt of a
/// card's mode, or the armed terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typing {
    Prompt(PromptKind),
    Terminal,
    /// A one-line field ([`FocusedPrompt::Field`]).
    Field,
}

impl Typing {
    fn focus(self, screen: PromptScreen) -> FocusedPrompt {
        match self {
            Typing::Prompt(kind) => FocusedPrompt::Card { screen, kind },
            Typing::Terminal => FocusedPrompt::Terminal,
            Typing::Field => FocusedPrompt::Field,
        }
    }
}

/// What the app is to do with the soft keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ime {
    /// Show it, editing this text.
    Start(String),
    /// Hide it.
    Stop,
    /// Leave it as it is.
    Keep,
}

/// The keyboard's state for the prompt cards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardRoute {
    focused: Option<FocusedPrompt>,
    /// The user dismissed the soft keyboard: hidden until a tap.
    dismissed: bool,
    /// A physical keyboard types: the soft keyboard stays hidden.
    physical: bool,
    /// A tap asked for the soft keyboard again.
    reshow: bool,
    /// The typed prompt (or the terminal) open at the last sync, if any.
    open: Option<Typing>,
}

impl KeyboardRoute {
    /// A route with a physical keyboard already in use (the desktop).
    pub fn new(physical: bool) -> Self {
        KeyboardRoute {
            physical,
            ..Default::default()
        }
    }

    pub fn focused(&self) -> Option<FocusedPrompt> {
        self.focused
    }

    pub fn dismissed(&self) -> bool {
        self.dismissed
    }

    pub fn physical(&self) -> bool {
        self.physical
    }

    /// A tap on a card's text area, or a typed prompt being started: it is
    /// focused, and the soft keyboard is shown, again if it was dismissed.
    pub fn focus(&mut self, prompt: FocusedPrompt) {
        self.focused = Some(prompt);
        self.dismissed = false;
        self.reshow = true;
    }

    /// The soft keyboard was dismissed: the card and its text stay.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
        self.reshow = false;
    }

    /// The prompt was sent or cancelled.
    pub fn close(&mut self) {
        self.focused = None;
        self.dismissed = false;
        self.reshow = false;
    }

    /// The screen shown changed: the focus follows to its card, which
    /// shows the same prompt.
    pub fn shown(&mut self, screen: PromptScreen) {
        if let Some(FocusedPrompt::Card { screen: s, .. }) = &mut self.focused {
            *s = screen;
        }
    }

    /// A physical key typed into the terminal: the soft keyboard stays
    /// hidden from now on, as for a card.
    pub fn typed_physically(&mut self) {
        self.physical = true;
    }

    /// A physical key while `text` is being typed: the new text, and the
    /// soft keyboard stays hidden from now on. `None` for a key that does
    /// not edit.
    pub fn key(&mut self, text: &str, key: Key) -> Option<String> {
        let mut text = text.to_string();
        match key {
            Key::Character('\u{8}' | '\u{7f}') => {
                text.pop();
            }
            Key::Character('\r' | '\n') => text.push('\n'),
            Key::Character(c) if !c.is_control() => text.push(c),
            _ => return None,
        }
        self.physical = true;
        Some(text)
    }

    /// The one decision, each frame: `typing` is the typed prompt open (its
    /// card's mode and text) or the armed terminal (the soft keyboard's
    /// text for it), `active` whether the soft keyboard is up, and
    /// `screen` the screen shown. A typed prompt with nothing focused is
    /// focused on it.
    pub fn sync(
        &mut self,
        typing: Option<(Typing, &str)>,
        active: bool,
        screen: PromptScreen,
    ) -> Ime {
        let Some((kind, text)) = typing else {
            self.close();
            self.open = None;
            return if active { Ime::Stop } else { Ime::Keep };
        };
        // A typed prompt just opened (or another in its place): focused.
        if self.open != Some(kind) {
            self.open = Some(kind);
            self.focused = Some(kind.focus(screen));
            self.dismissed = false;
            self.reshow = true;
        }
        self.shown(screen);
        let wanted = self.focused.is_some() && !self.dismissed && !self.physical;
        if wanted && (!active || self.reshow) {
            self.reshow = false;
            return Ime::Start(text.to_string());
        }
        self.reshow = false;
        if !wanted && active {
            return Ime::Stop;
        }
        Ime::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERVIEW: PromptScreen = PromptScreen::Overview;

    #[test]
    fn typing_into_either_mode_shows_the_keyboard_once() {
        for kind in [PromptKind::Message, PromptKind::GeneralAgent] {
            let mut k = KeyboardRoute::new(false);
            assert_eq!(k.sync(None, false, OVERVIEW), Ime::Keep);
            // A typed prompt opens: focused, and the keyboard shows.
            assert_eq!(
                k.sync(Some((Typing::Prompt(kind), "")), false, OVERVIEW),
                Ime::Start(String::new())
            );
            assert_eq!(
                k.focused(),
                Some(FocusedPrompt::Card {
                    screen: OVERVIEW,
                    kind
                })
            );
            // Up: nothing more.
            assert_eq!(
                k.sync(Some((Typing::Prompt(kind), "hel")), true, OVERVIEW),
                Ime::Keep
            );
            // Sent: it goes at once, and does not come back while the
            // prompt is still on its way out.
            k.close();
            assert_eq!(
                k.sync(Some((Typing::Prompt(kind), "hel")), true, OVERVIEW),
                Ime::Stop
            );
            assert_eq!(
                k.sync(Some((Typing::Prompt(kind), "hel")), false, OVERVIEW),
                Ime::Keep
            );
            assert_eq!(k.sync(None, false, OVERVIEW), Ime::Keep);
            assert_eq!(k.focused(), None);
            // Opened again: shown again.
            assert_eq!(
                k.sync(Some((Typing::Prompt(kind), "")), false, OVERVIEW),
                Ime::Start(String::new())
            );
            assert_eq!(k.sync(None, true, OVERVIEW), Ime::Stop);
        }
    }

    #[test]
    fn dismissed_it_stays_hidden_until_a_tap() {
        let mut k = KeyboardRoute::new(false);
        let typing = Some((Typing::Prompt(PromptKind::Message), "draft"));
        k.sync(typing, false, OVERVIEW);
        k.dismiss();
        // The card and its text stay; the keyboard does not come back.
        assert_eq!(k.sync(typing, false, OVERVIEW), Ime::Keep);
        assert_eq!(k.sync(typing, false, OVERVIEW), Ime::Keep);
        // A tap on the text area shows it again, with the text.
        k.focus(FocusedPrompt::Card {
            screen: OVERVIEW,
            kind: PromptKind::Message,
        });
        assert_eq!(k.sync(typing, false, OVERVIEW), Ime::Start("draft".into()));
        // A tap while it is up asks for it again (its cursor moved).
        k.focus(FocusedPrompt::Card {
            screen: OVERVIEW,
            kind: PromptKind::Message,
        });
        assert_eq!(k.sync(typing, true, OVERVIEW), Ime::Start("draft".into()));
        assert_eq!(k.sync(typing, true, OVERVIEW), Ime::Keep);
    }

    #[test]
    fn a_physical_keyboard_types_without_the_soft_keyboard() {
        let mut k = KeyboardRoute::new(false);
        let typing = Some((Typing::Prompt(PromptKind::GeneralAgent), "hi"));
        k.sync(typing, false, OVERVIEW);
        assert_eq!(k.key("hi", Key::Character('!')), Some("hi!".into()));
        assert!(k.physical());
        assert_eq!(
            k.sync(
                Some((Typing::Prompt(PromptKind::GeneralAgent), "hi!")),
                true,
                OVERVIEW
            ),
            Ime::Stop
        );
        assert_eq!(k.key("hi!", Key::Character('\u{8}')), Some("hi".into()));
        assert_eq!(k.key("hi", Key::Character('\r')), Some("hi\n".into()));
        assert_eq!(k.key("hi", Key::Back), None);
        // A tap does not bring the soft keyboard over it.
        k.focus(FocusedPrompt::Card {
            screen: OVERVIEW,
            kind: PromptKind::GeneralAgent,
        });
        assert_eq!(k.sync(typing, false, OVERVIEW), Ime::Keep);
        // The desktop starts with one.
        let mut desk = KeyboardRoute::new(true);
        assert_eq!(desk.sync(typing, false, OVERVIEW), Ime::Keep);
    }

    #[test]
    fn the_focus_follows_the_screen_shown_and_the_mode_typed() {
        let mut k = KeyboardRoute::new(false);
        k.sync(
            Some((Typing::Prompt(PromptKind::Message), "")),
            false,
            OVERVIEW,
        );
        k.sync(
            Some((Typing::Prompt(PromptKind::Message), "")),
            true,
            PromptScreen::Agent,
        );
        assert!(matches!(
            k.focused(),
            Some(FocusedPrompt::Card {
                screen: PromptScreen::Agent,
                ..
            })
        ));
        // Gemini's typed prompt in its place: focused on it, shown.
        assert_eq!(
            k.sync(
                Some((Typing::Prompt(PromptKind::GeneralAgent), "")),
                true,
                PromptScreen::Agent
            ),
            Ime::Start(String::new())
        );
        assert!(matches!(
            k.focused(),
            Some(FocusedPrompt::Card {
                kind: PromptKind::GeneralAgent,
                ..
            })
        ));
    }

    #[test]
    fn the_armed_terminal_takes_the_keyboard_and_gives_it_back() {
        let mut k = KeyboardRoute::new(false);
        // The keyboard glyph while armed: the terminal is focused, and the
        // soft keyboard shows on an empty text.
        k.focus(FocusedPrompt::Terminal);
        let typing = Some((Typing::Terminal, ""));
        assert_eq!(
            k.sync(typing, false, PromptScreen::Agent),
            Ime::Start(String::new())
        );
        assert_eq!(k.focused(), Some(FocusedPrompt::Terminal));
        assert_eq!(
            k.sync(Some((Typing::Terminal, "ls")), true, PromptScreen::Agent),
            Ime::Keep
        );
        // The arm runs out: the keyboard goes.
        assert_eq!(k.sync(None, true, PromptScreen::Agent), Ime::Stop);
        assert_eq!(k.focused(), None);
        // A physical key into the terminal keeps the soft keyboard hidden.
        k.typed_physically();
        k.focus(FocusedPrompt::Terminal);
        assert_eq!(
            k.sync(Some((Typing::Terminal, "")), false, PromptScreen::Agent),
            Ime::Keep
        );
    }
}
