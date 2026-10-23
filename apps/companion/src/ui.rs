//! The screen: a libcontrols `Router` that keeps its nodes for the life of
//! the app, as voicexn's does (libvoicexn v0.2.0, `examples/voicexn/src/ui.rs`).
//! A change of text, colour or size edits the nodes; the draw list is
//! rebuilt only when something changed.
//!
//! From the top: the title, two status lines, and the conversation, with
//! the Approve and Deny buttons under it while a confirmation waits. From
//! the bottom: the pairing field and its Pair button, and above them the
//! device code and any pairing note.

use libcontrols::{Button, DrawContext, Edit, Label, NodeFlags, NodeId, Rect, Router, TextAlign};
use libmsdf::drawlist::DrawList;

/// Lines of conversation the screen can show.
const LINES: usize = 16;
/// Lines of the pairing area (the code and its notes).
const CODE_LINES: usize = 8;

pub const TEXT: [f32; 4] = [0.92, 0.93, 0.95, 1.0];
pub const MUTED: [f32; 4] = [0.60, 0.62, 0.68, 1.0];
pub const SPOKEN: [f32; 4] = [0.55, 0.78, 1.0, 1.0];
pub const WARN: [f32; 4] = [1.0, 0.62, 0.45, 1.0];
pub const CODE: [f32; 4] = [1.0, 0.86, 0.45, 1.0];

const APPROVE_BG: [f32; 4] = [0.16, 0.45, 0.25, 1.0];
const DENY_BG: [f32; 4] = [0.55, 0.18, 0.18, 1.0];
const PAIR_BG: [f32; 4] = [0.20, 0.32, 0.55, 1.0];

/// One line of text, before layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// In points; scaled by the display's scale factor.
    pub size: f32,
    pub color: [f32; 4],
    /// Space before the line, in points.
    pub gap: f32,
}

/// Splits `text` into lines of at most `max` characters at spaces.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Splits `text`, which has no spaces (a device code), into lines of at
/// most `max` characters.
pub fn chunk(text: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(max.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

/// Where a confirmation's card is (plan 20261023_01, section 4).
#[derive(Clone, Debug, PartialEq)]
pub enum CardState {
    /// Waiting for the user: Approve and Deny are shown.
    Waiting,
    /// Answered; the reply is on its way.
    Answering { approve: bool },
    /// The daemon refused the answer.
    Refused(String),
    /// Resolved, with plan 01's outcome (`sent`, `denied` …).
    Done(String),
}

/// A confirmation, as its card shows it: the command, the agent and its
/// project, and the handle as words with the id. The token is never shown
/// and the handle never spoken.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub confirm_id: u64,
    pub command: String,
    /// The agent's shown name.
    pub agent: String,
    pub project: Option<String>,
    pub handle: Option<String>,
    pub state: CardState,
}

/// "Sent", "Denied", "Expired", "Cancelled", "Failed": an outcome as the
/// card shows it.
pub fn outcome_label(outcome: &str) -> String {
    let mut chars = outcome.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Ended".to_string(),
    }
}

impl Card {
    /// Whether Approve and Deny are shown.
    pub fn asks(&self) -> bool {
        self.state == CardState::Waiting
    }

    /// The agent, with its project unless its shown name already says it.
    pub fn target(&self) -> String {
        match &self.project {
            Some(p) if !p.is_empty() && !self.agent.to_lowercase().contains(&p.to_lowercase()) => {
                format!("{} in {p}", self.agent)
            }
            _ => self.agent.clone(),
        }
    }

    /// The card's lines, unwrapped: its state, the command and the agent,
    /// then the handle's words and the id in smaller type.
    pub fn lines(&self) -> Vec<Line> {
        let line = |text: String, size: f32, color, gap| Line {
            text,
            size,
            color,
            gap,
        };
        let (state, color) = match &self.state {
            CardState::Waiting => ("Approve?".to_string(), MUTED),
            CardState::Answering { approve: true } => ("Approving\u{2026}".to_string(), MUTED),
            CardState::Answering { approve: false } => ("Denying\u{2026}".to_string(), MUTED),
            CardState::Refused(why) => (format!("Not answered: {why}"), WARN),
            CardState::Done(outcome) => (outcome_label(outcome), SPOKEN),
        };
        let words = self
            .handle
            .as_deref()
            .map(|h| format!("{} \u{b7} ", h.replace('-', " ")))
            .unwrap_or_default();
        vec![
            line(state, 15.0, color, 28.0),
            line(
                format!("\"{}\" to {}", self.command, self.target()),
                22.0,
                TEXT,
                4.0,
            ),
            line(format!("{words}#{}", self.confirm_id), 14.0, MUTED, 4.0),
        ]
    }
}

/// What the screen shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// The connection and the audio, one line each.
    pub status: Vec<String>,
    /// The conversation, and the confirmation's text.
    pub body: Vec<Line>,
    /// Show Approve and Deny under the body.
    pub confirm: bool,
    /// The pairing area above the field: the device code and notes.
    pub code: Vec<Line>,
    /// The pairing field's text; the screen shows its tail.
    pub field: String,
    /// The field has the keyboard.
    pub field_focused: bool,
}

/// A control a touch landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Approve,
    Deny,
    Field,
    Pair,
}

/// Surface size, scale factor and safe area top and bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub scale: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Layout {
    const PAD: f32 = 20.0;
    /// The Pair button's width and the row's height, in points.
    const PAIR_W: f32 = 96.0;
    const ROW_H: f32 = 48.0;
    const FIELD_SIZE: f32 = 16.0;

    /// How many characters of `size` points fit on a line: roughly half an
    /// em each in Roboto.
    pub fn columns(&self, size: f32) -> usize {
        self.columns_in(self.w - 2.0 * Self::PAD * self.scale, size)
    }

    fn columns_in(&self, width: f32, size: f32) -> usize {
        (width / (size * self.scale * 0.5)).max(8.0) as usize
    }
}

/// The placeholder in the empty pairing field.
const PLACEHOLDER: &str = "paste a pairing URL";

/// The persistent screen.
pub struct Screen {
    router: Router,
    title: NodeId,
    status: [NodeId; 2],
    lines: Vec<NodeId>,
    code: Vec<NodeId>,
    field: NodeId,
    pair: NodeId,
    approve: NodeId,
    deny: NodeId,
    /// Where each visible control is, as last laid out.
    hits: Vec<(Hit, Rect)>,
    /// What the screen shows, as last laid out.
    shown: Option<(View, Layout)>,
    dirty: bool,
}

impl Screen {
    pub fn new() -> Self {
        let mut router = Router::new();
        let label = |text: &str, size: f32, color, align| {
            Label::new(text, Rect::new(0.0, 0.0, 1.0, 1.0))
                .with_font_size(size)
                .with_color(color)
                .with_align(align)
        };
        let button = |text: &str, bg| {
            Button::new(text, Rect::new(0.0, 0.0, 1.0, 1.0))
                .with_bg(bg)
                .with_text_color(TEXT)
                .with_font_size(18.0)
        };
        let title = router.add(None, label("handlr", 32.0, TEXT, TextAlign::Center));
        let status = [
            router.add(None, label("", 15.0, MUTED, TextAlign::Center)),
            router.add(None, label("", 15.0, MUTED, TextAlign::Center)),
        ];
        let lines = (0..LINES)
            .map(|_| router.add(None, label("", 20.0, TEXT, TextAlign::Left)))
            .collect();
        let approve = router.add(None, button("Approve", APPROVE_BG));
        let deny = router.add(None, button("Deny", DENY_BG));
        let code = (0..CODE_LINES)
            .map(|_| router.add(None, label("", 20.0, CODE, TextAlign::Left)))
            .collect();
        let field = router.add(
            None,
            Edit::new("", Rect::new(0.0, 0.0, 1.0, 1.0))
                .with_align(TextAlign::Left)
                .with_font_size(Layout::FIELD_SIZE),
        );
        let pair = router.add(None, button("Pair", PAIR_BG));
        Self {
            router,
            title,
            status,
            lines,
            code,
            field,
            pair,
            approve,
            deny,
            hits: Vec::new(),
            shown: None,
            dirty: true,
        }
    }

    /// The control at `(x, y)`, in surface pixels, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(_, rect)| rect.contains([x, y]))
            .map(|&(hit, _)| hit)
    }

    /// Sets what the screen shows; nodes change only if it differs.
    pub fn set(&mut self, layout: Layout, view: View) {
        if self.shown.as_ref() == Some(&(view.clone(), layout)) {
            return;
        }
        let s = layout.scale;
        let pad = Layout::PAD * s;
        self.hits.clear();

        // The top: title and status.
        let mut y = layout.top + 24.0 * s;
        let place = |router: &mut Router,
                     y: &mut f32,
                     id,
                     text: Option<&str>,
                     line: (f32, [f32; 4]),
                     x: f32| {
            let (size, color) = line;
            let label = router.get_mut::<Label>(id).expect("a label node");
            if let Some(text) = text {
                label.set_text(text);
            }
            label.font_size = Some(size * s);
            label.color = Some(color);
            label.rect = Rect::new(x, *y, layout.w - 2.0 * x, size * s * 1.3);
            *y += size * s * 1.35;
        };
        place(
            &mut self.router,
            &mut y,
            self.title,
            None,
            (32.0, TEXT),
            0.0,
        );
        for (i, &id) in self.status.iter().enumerate() {
            let text = view.status.get(i).map_or("", String::as_str);
            place(&mut self.router, &mut y, id, Some(text), (15.0, MUTED), 0.0);
        }

        // The bottom: the field and Pair.
        let bottom = layout.h - layout.bottom - 24.0 * s;
        let row_h = Layout::ROW_H * s;
        let row_y = bottom - row_h;
        let pair_w = Layout::PAIR_W * s;
        let field_w = layout.w - 2.0 * pad - pair_w - 12.0 * s;
        let field_rect = Rect::new(pad, row_y, field_w, row_h);
        let pair_rect = Rect::new(pad + field_w + 12.0 * s, row_y, pair_w, row_h);
        {
            let edit = self.router.get_mut::<Edit>(self.field).expect("the field");
            let (text, color) = if view.field.is_empty() && !view.field_focused {
                (PLACEHOLDER.to_string(), MUTED)
            } else {
                // The tail, with a caret while focused.
                let room = layout
                    .columns_in(field_w - 8.0 * s, Layout::FIELD_SIZE * 1.3)
                    .saturating_sub(1);
                let chars: Vec<char> = view.field.chars().collect();
                let tail: String = chars[chars.len().saturating_sub(room)..].iter().collect();
                let caret = if view.field_focused { "|" } else { "" };
                (format!("{tail}{caret}"), TEXT)
            };
            edit.set_text(text);
            edit.text_color = Some(color);
            edit.font_size = Some(Layout::FIELD_SIZE * s);
            edit.corner_radius = Some(8.0 * s);
            edit.rect = field_rect;
            edit.state.set_active(view.field_focused);
        }
        self.place_button(self.pair, pair_rect, s, true);
        self.hits.push((Hit::Field, field_rect));
        self.hits.push((Hit::Pair, pair_rect));

        // Above them, the pairing area, as many lines as fit.
        let height = |line: &Line| (line.gap + line.size * 1.35) * s;
        let code_lines = &view.code[..view.code.len().min(CODE_LINES)];
        let code_h: f32 = code_lines.iter().map(height).sum();
        let mut code_y = (row_y - 12.0 * s - code_h).max(y);
        let code_top = code_y;
        let mut code_lines = code_lines.iter();
        for &id in &self.code {
            match code_lines.next() {
                Some(line) if code_y + height(line) <= row_y => {
                    code_y += line.gap * s;
                    let line_style = (line.size, line.color);
                    place(
                        &mut self.router,
                        &mut code_y,
                        id,
                        Some(&line.text),
                        line_style,
                        pad,
                    );
                }
                _ => self
                    .router
                    .get_mut::<Label>(id)
                    .expect("a label node")
                    .set_text(""),
            }
        }

        // The conversation, then the buttons while confirming.
        let buttons_h = if view.confirm { (52.0 + 16.0) * s } else { 0.0 };
        let limit = code_top - 16.0 * s - buttons_h;
        let mut body_lines = view.body.iter();
        for &id in &self.lines {
            match body_lines.next() {
                Some(line) if y + height(line) <= limit => {
                    y += line.gap * s;
                    let line_style = (line.size, line.color);
                    place(
                        &mut self.router,
                        &mut y,
                        id,
                        Some(&line.text),
                        line_style,
                        pad,
                    );
                }
                _ => self
                    .router
                    .get_mut::<Label>(id)
                    .expect("a label node")
                    .set_text(""),
            }
        }
        if view.confirm {
            let w = (layout.w - 2.0 * pad - 16.0 * s) / 2.0;
            let top = y + 16.0 * s;
            let approve = Rect::new(pad, top, w, 52.0 * s);
            let deny = Rect::new(pad + w + 16.0 * s, top, w, 52.0 * s);
            self.place_button(self.approve, approve, s, true);
            self.place_button(self.deny, deny, s, true);
            self.hits.insert(0, (Hit::Approve, approve));
            self.hits.insert(1, (Hit::Deny, deny));
        } else {
            self.place_button(self.approve, Rect::default(), s, false);
            self.place_button(self.deny, Rect::default(), s, false);
        }

        self.shown = Some((view, layout));
        self.dirty = true;
    }

    fn place_button(&mut self, id: NodeId, rect: Rect, s: f32, visible: bool) {
        let button = self.router.get_mut::<Button>(id).expect("a button node");
        button.rect = rect;
        button.font_size = Some(18.0 * s);
        button.corner_radius = Some(10.0 * s);
        self.router.set_flags(id, NodeFlags::HIDDEN, !visible);
    }

    /// Rebuilds `list` if the screen changed since the last call. Returns
    /// whether it did.
    pub fn draw(&mut self, list: &mut DrawList, ctx: &DrawContext) -> bool {
        if !self.dirty {
            return false;
        }
        list.clear();
        self.router.render(list, ctx);
        self.dirty = false;
        true
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
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
        bottom: 40.0,
    };

    fn line(text: &str) -> Line {
        Line {
            text: text.into(),
            size: 20.0,
            color: TEXT,
            gap: 0.0,
        }
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(wrap("  ", 9), Vec::<String>::new());
        assert_eq!(wrap("unbreakable", 4), ["unbreakable"]);
    }

    #[test]
    fn chunk_splits_codes() {
        assert_eq!(chunk("abcdefg", 3), ["abc", "def", "g"]);
        assert_eq!(chunk("", 3), Vec::<String>::new());
    }

    #[test]
    fn the_list_is_rebuilt_only_on_change() {
        let mut screen = Screen::new();
        let view = View {
            status: vec!["ready".into()],
            body: vec![line("hello")],
            ..View::default()
        };
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        screen.set(PHONE, view.clone());
        assert!(screen.draw(&mut list, &ctx));
        assert!(!screen.draw(&mut list, &ctx));
        screen.set(PHONE, view.clone());
        assert!(
            !screen.draw(&mut list, &ctx),
            "the same content is no change"
        );
        screen.set(
            PHONE,
            View {
                status: vec!["listening".into()],
                ..view
            },
        );
        assert!(screen.draw(&mut list, &ctx));
        let status = screen.router.get::<Label>(screen.status[0]).unwrap();
        assert_eq!(status.text(), "listening");
    }

    #[test]
    fn approve_and_deny_are_there_only_while_confirming() {
        let mut screen = Screen::new();
        screen.set(PHONE, View::default());
        let pair = screen.hits.iter().find(|(h, _)| *h == Hit::Pair).unwrap().1;
        assert_eq!(screen.hit(pair.x + 1.0, pair.y + 1.0), Some(Hit::Pair));
        assert!(!screen.hits.iter().any(|(h, _)| *h == Hit::Approve));
        // The middle of the screen is no control: a tap.
        assert_eq!(screen.hit(540.0, 1200.0), None);

        screen.set(
            PHONE,
            View {
                body: vec![line("Approve sending \"continue\" to claude?")],
                confirm: true,
                ..View::default()
            },
        );
        let approve = screen
            .hits
            .iter()
            .find(|(h, _)| *h == Hit::Approve)
            .unwrap()
            .1;
        let deny = screen.hits.iter().find(|(h, _)| *h == Hit::Deny).unwrap().1;
        assert_eq!(
            screen.hit(approve.x + 1.0, approve.y + 1.0),
            Some(Hit::Approve)
        );
        assert_eq!(screen.hit(deny.x + 1.0, deny.y + 1.0), Some(Hit::Deny));
        assert!(approve.y + approve.h < pair.y, "above the pairing row");
    }

    fn card(state: CardState) -> Card {
        Card {
            confirm_id: 12,
            command: "continue".into(),
            agent: "Claude".into(),
            project: Some("highbay".into()),
            handle: Some("send-continue-claude-highbay".into()),
            state,
        }
    }

    #[test]
    fn a_card_shows_the_command_agent_project_and_handle_words() {
        let c = card(CardState::Waiting);
        let texts: Vec<String> = c.lines().into_iter().map(|l| l.text).collect();
        assert_eq!(
            texts,
            [
                "Approve?",
                "\"continue\" to Claude in highbay",
                "send continue claude highbay \u{b7} #12"
            ]
        );
        assert!(c.asks());
        // A shown name that already names the project does not repeat it.
        let named = Card {
            agent: "Claude in highbay, tab -mb".into(),
            ..card(CardState::Waiting)
        };
        assert_eq!(named.target(), "Claude in highbay, tab -mb");
        let bare = Card {
            handle: None,
            project: None,
            ..card(CardState::Waiting)
        };
        assert_eq!(bare.lines()[2].text, "#12");
    }

    #[test]
    fn a_card_shows_the_answer_then_the_outcome() {
        let first = |state| card(state).lines()[0].text.clone();
        assert_eq!(
            first(CardState::Answering { approve: true }),
            "Approving\u{2026}"
        );
        assert_eq!(
            first(CardState::Answering { approve: false }),
            "Denying\u{2026}"
        );
        for (outcome, shown) in [
            ("sent", "Sent"),
            ("denied", "Denied"),
            ("expired", "Expired"),
            ("cancelled", "Cancelled"),
        ] {
            assert_eq!(first(CardState::Done(outcome.into())), shown);
        }
        let refused = first(CardState::Refused(
            "wrong nonce; 4 attempt(s) left (-32008)".into(),
        ));
        assert!(refused.contains("4 attempt(s) left"), "{refused}");
        assert!(!card(CardState::Done("sent".into())).asks());
        assert!(!card(CardState::Answering { approve: true }).asks());
    }

    #[test]
    fn the_field_shows_its_tail() {
        let mut screen = Screen::new();
        let field = format!("handlr://m4:7347/pair#handlr1{}", "x".repeat(300));
        screen.set(
            PHONE,
            View {
                field: field.clone(),
                ..View::default()
            },
        );
        let edit = screen.router.get::<Edit>(screen.field).unwrap();
        let shown = edit.text();
        assert!(shown.len() < field.len());
        assert!(field.ends_with(&shown));
    }
}
