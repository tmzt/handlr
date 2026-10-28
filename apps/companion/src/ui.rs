//! The pairing screen: a libcontrols `Router` that keeps its nodes for the
//! life of the app, as voicexn's does (libvoicexn v0.2.0,
//! `examples/voicexn/src/ui.rs`). A change of text, colour or size edits
//! the nodes; the draw list is rebuilt only when something changed.
//!
//! From the top: the title and two status lines. From the bottom: the
//! pairing field and its Pair button, and above them the pairing area (the
//! six pairing words and one line, or their Details) and any pairing note.
//! While a pairing waits for the owner, Details and Cancel take the
//! field's place (plan 20261028_01, the 14:42 EDT amendment). It shows
//! while the phone is not paired and not connected, or a pairing waits,
//! or is in its **add** mode, opened by the **+** in Settings'
//! Remotes (plan 20261028_01, decision 9): titled *Pair another remote*,
//! with a "<" at the top left back to Settings. Otherwise the overview is
//! the home screen (plan 20261023_04), and the conversation, the
//! confirmation and the dictation cards, and their buttons, are there. The
//! lines those cards show are built here.

use flankr_companion::{Dictation, DictationPhase, PromptMode};
use libcontrols::{Button, DrawContext, Edit, Label, NodeFlags, NodeId, Rect, Router, TextAlign};
use libflankr_proto::common::FocusInfo;
use libmsdf::drawlist::DrawList;

use crate::agent::fill;
use crate::prompt::PromptInput;
use crate::settings::copy;
use crate::theme::{theme, with_alpha};

/// Lines of the pairing area (the words, or the Details with the device
/// code, and the notes).
const CODE_LINES: usize = 20;
/// The add mode's back glyph: ASCII, as the pairing screen's face has no
/// "‹".
const BACK_GLYPH: &str = "<";
/// Lines under the placeholder's label (the add mode's what to do).
const DETAIL_LINES: usize = 6;
/// The reticle (plan 20261028_02, decision 2): corner marks of this
/// thickness and length, in points, framing a centred square of this share
/// of the area's shorter side, in the text colour at these opacities.
const MARK_PT: f32 = 3.0;
const MARK_LEN_PT: f32 = 28.0;
const RETICLE_SHARE: f32 = 0.62;
const MARK_ALPHA_LIVE: f32 = 0.8;
const MARK_ALPHA_PLACEHOLDER: f32 = 0.4;
/// The title's band over the camera's image: the background at 70 %.
const TITLE_BAND_ALPHA: f32 = 0.7;

/// The camera's area as last laid out, in surface pixels, and what is
/// drawn over it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraDraw {
    area: Rect,
    /// The title's band at the top of the area.
    title_band: Rect,
    /// The reticle's square.
    reticle: Rect,
    live: bool,
    /// The camera glyph over the placeholder's label: its centre.
    glyph: Option<[f32; 2]>,
    scale: f32,
}

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

/// What the prompt card shows of a dictation (plan 20261027_03): where it
/// stands, then any warning (no agent focused, the focus moved), then the
/// text. The target's "Message for …" line is the card's header instead,
/// and the general agent's "✦ Gemini" while it collects.
pub fn prompt_input(d: &Dictation) -> PromptInput {
    let lines = dictation_lines(d);
    let collecting = d.phase == DictationPhase::Collecting && d.submit_in.is_none();
    let status = lines
        .first()
        .filter(|_| !(d.general_agent && collecting))
        .map(|l| (l.text.clone(), l.color));
    let mut notes = Vec::new();
    let untargeted = !d.general_agent && d.target.as_ref().is_none_or(|t| t.agent.is_none());
    if untargeted && let Some(l) = lines.get(1) {
        notes.push((l.text.clone(), l.color));
    }
    if d.focus_moved.is_some()
        && let Some(l) = lines.get(2)
    {
        notes.push((l.text.clone(), l.color));
    }
    PromptInput {
        status,
        notes,
        text: d.text.clone(),
        mode: d.mode,
    }
}

/// Splits `text` into lines of at most `max` characters at spaces, preserving newlines.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() || (para.is_empty() && !lines.is_empty()) {
            lines.push(line);
        }
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
            CardState::Waiting => ("Approve?".to_string(), theme().muted),
            CardState::Answering { approve: true } => {
                ("Approving\u{2026}".to_string(), theme().muted)
            }
            CardState::Answering { approve: false } => {
                ("Denying\u{2026}".to_string(), theme().muted)
            }
            CardState::Refused(why) => (format!("Not answered: {why}"), theme().warn),
            CardState::Done(outcome) => (outcome_label(outcome), theme().spoken),
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
                theme().text,
                4.0,
            ),
            line(
                format!("{words}#{}", self.confirm_id),
                14.0,
                theme().muted,
                4.0,
            ),
        ]
    }
}

/// Where a focus is, as the dictation card and the focus line show it:
/// "Claude · libmorph · tab -ad".
pub fn focus_words(focus: &FocusInfo) -> String {
    let mut parts = vec![focus.agent.clone().unwrap_or_else(|| focus.name.clone())];
    if let Some(project) = focus.project.as_deref().filter(|p| !p.is_empty()) {
        parts.push(project.to_string());
    }
    let tab = focus
        .tab_label
        .clone()
        .filter(|l| !l.is_empty())
        .or_else(|| focus.tab_number.map(|n| n.to_string()));
    if let Some(tab) = tab {
        parts.push(format!("tab {tab}"));
    }
    parts.join(" \u{b7} ")
}

/// The dictation card's lines, unwrapped (plan 20261023_02, decisions 3
/// and 4, and the countdown of the 01:57 EDT amendment): where it stands,
/// the target, a moved focus, then the text.
pub fn dictation_lines(d: &Dictation) -> Vec<Line> {
    let line = |text: String, size: f32, color, gap| Line {
        text,
        size,
        color,
        gap,
    };
    let (state, color) = if d.general_agent {
        match (d.phase, d.submit_in) {
            (DictationPhase::Collecting, Some(n)) => {
                (format!("Submitting in {n} s\u{2026}"), theme().warn)
            }
            (DictationPhase::Collecting, None) => ("\u{2726} Gemini".to_string(), theme().accent),
            (DictationPhase::Submitting, _) => ("Submitting\u{2026}".to_string(), theme().muted),
            (DictationPhase::Confirming, _) => {
                ("Waiting for your approval".to_string(), theme().muted)
            }
        }
    } else {
        match (d.phase, d.submit_in) {
            (DictationPhase::Collecting, Some(n)) => {
                (format!("Submitting in {n} s\u{2026}"), theme().warn)
            }
            (DictationPhase::Collecting, None) => ("Taking a message".to_string(), theme().muted),
            (DictationPhase::Submitting, _) => ("Submitting\u{2026}".to_string(), theme().muted),
            (DictationPhase::Confirming, _) => {
                ("Waiting for your approval".to_string(), theme().muted)
            }
        }
    };
    let target = if d.general_agent {
        ("General agent chat".to_string(), theme().text)
    } else {
        match &d.target {
            Some(t) if t.agent.is_some() => {
                (format!("Message for {}", focus_words(t)), theme().text)
            }
            _ => (
                "No agent focused \u{2014} say \u{2018}focus \u{2026}\u{2019}".to_string(),
                theme().warn,
            ),
        }
    };
    let mut lines = vec![
        line(state, 15.0, color, 28.0),
        line(target.0, 18.0, target.1, 4.0),
    ];
    if let Some(moved) = &d.focus_moved {
        lines.push(line(
            format!("focus moved to {moved}"),
            15.0,
            theme().warn,
            2.0,
        ));
    }
    let size = match d.mode {
        PromptMode::Voice => 20.0,
        PromptMode::Keyboard => 12.0,
    };
    if d.text.is_empty() {
        let placeholder = match (d.general_agent, d.mode) {
            (true, PromptMode::Voice) => "(ask Gemini)",
            (true, PromptMode::Keyboard) => "Ask Gemini\u{2026}",
            (false, PromptMode::Voice) => "(say your message)",
            (false, PromptMode::Keyboard) => "Type your message",
        };
        lines.push(line(placeholder.to_string(), size, theme().muted, 8.0));
    } else {
        lines.push(line(d.text.clone(), size, theme().text, 8.0));
    }
    lines
}

/// What the screen shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// The connection and the audio, one line each.
    pub status: Vec<String>,
    /// The pairing area above the field: the words or Details, and notes.
    pub code: Vec<Line>,
    /// The pairing field's text; the screen shows its tail.
    pub field: String,
    /// The field has the keyboard.
    pub field_focused: bool,
    /// A pairing waits for the owner: the field and Pair are hidden, and
    /// Details and Cancel show in their place. The pairing area shows above
    /// them.
    pub waiting: bool,
    /// The pairing's Details are open: the Details button reads "Hide
    /// details".
    pub details: bool,
    /// The add mode (**+** in Settings' Remotes): its title, and a "<"
    /// back to Settings.
    pub adding: bool,
    /// The camera's window (plan 20261028_02), where there is a camera and
    /// no pairing waits: the viewfinder area, and `code` is the line under
    /// it. `None`: today's paste screen.
    pub camera: Option<CameraView>,
}

/// What the pairing window shows of the camera (plan 20261028_02,
/// decisions 1-4, 12-14).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraView {
    /// The camera runs: the viewfinder under the area, the corner marks
    /// bright, no placeholder.
    pub live: bool,
    /// The placeholder's label (*Tap to scan*, *Tap to scan again*), over
    /// its camera glyph; `None` while the camera runs or a scanned code is
    /// being paired.
    pub prompt: Option<String>,
    /// Under the label in the placeholder: the add mode's what to do.
    pub detail: Option<String>,
    /// The zoom ratio at the top right of the area, after a pinch.
    pub zoom: Option<String>,
    /// **Paste a link** was tapped: the field and Pair in the bottom row.
    pub paste: bool,
    /// **Open Settings** in the bottom row (a refusal only Settings can
    /// undo).
    pub settings: bool,
}

/// A control a touch landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Field,
    Pair,
    /// Cancel, while a pairing waits.
    Cancel,
    /// Details, while a pairing waits: opens or closes them.
    Details,
    /// The add mode's "<": back to Settings.
    Back,
    /// The camera's area: the placeholder's Tap to scan.
    Area,
    /// **Paste a link**: the field instead of the camera.
    PasteLink,
    /// **Open Settings**, after a refusal only Settings can undo.
    OpenSettings,
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
    /// The add mode's "<" target, in points.
    const BACK_W: f32 = 56.0;
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
    back: NodeId,
    status: [NodeId; 2],
    code: Vec<NodeId>,
    field: NodeId,
    pair: NodeId,
    cancel: NodeId,
    details: NodeId,
    /// The camera's window: the placeholder's label and the lines under
    /// it, the zoom ratio, **Paste a link** and **Open Settings**.
    prompt: NodeId,
    detail: Vec<NodeId>,
    zoom: NodeId,
    paste_link: NodeId,
    open_settings: NodeId,
    /// The camera's area and what is drawn in it, as last laid out.
    camera: Option<CameraDraw>,
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
                .with_text_color(theme().text)
                .with_font_size(18.0)
        };
        let title = router.add(None, label("flankr", 32.0, theme().text, TextAlign::Center));
        let back = router.add(None, label(BACK_GLYPH, 28.0, theme().text, TextAlign::Left));
        let status = [
            router.add(None, label("", 15.0, theme().muted, TextAlign::Center)),
            router.add(None, label("", 15.0, theme().muted, TextAlign::Center)),
        ];
        let code = (0..CODE_LINES)
            .map(|_| router.add(None, label("", 20.0, theme().code, TextAlign::Left)))
            .collect();
        let field = router.add(
            None,
            Edit::new("", Rect::new(0.0, 0.0, 1.0, 1.0))
                .with_bg(theme().input)
                .with_border(theme().band, 1.0)
                .with_align(TextAlign::Left)
                .with_font_size(Layout::FIELD_SIZE),
        );
        let pair = router.add(None, button("Pair", theme().message));
        let cancel = router.add(None, button(copy::CANCEL.text, theme().back));
        let details = router.add(None, button(copy::PAIRING_DETAILS.text, theme().back));
        let prompt = router.add(None, label("", 20.0, theme().text, TextAlign::Center));
        let detail = (0..DETAIL_LINES)
            .map(|_| router.add(None, label("", 15.0, theme().muted, TextAlign::Center)))
            .collect();
        let zoom = router.add(None, label("", 18.0, theme().text, TextAlign::Right));
        let paste_link = router.add(None, button(copy::PAIRING_PASTE_LINK.text, theme().back));
        let open_settings = router.add(
            None,
            button(copy::PAIRING_OPEN_SETTINGS.text, theme().message),
        );
        Self {
            router,
            title,
            back,
            status,
            code,
            field,
            pair,
            cancel,
            details,
            prompt,
            detail,
            zoom,
            paste_link,
            open_settings,
            camera: None,
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
        // The add mode's title (smaller, to fit beside the "<").
        let (title, title_pt) = if view.adding {
            (copy::REMOTE_ADD.label(), 24.0)
        } else {
            ("flankr", 32.0)
        };
        let title_y = y;
        place(
            &mut self.router,
            &mut y,
            self.title,
            Some(title),
            (title_pt, theme().text),
            0.0,
        );
        {
            let back_w = Layout::BACK_W * s;
            let label = self.router.get_mut::<Label>(self.back).expect("the back");
            label.font_size = Some(28.0 * s);
            label.color = Some(theme().text);
            label.rect = Rect::new(pad, title_y, back_w, title_pt * s * 1.3);
            self.router
                .set_flags(self.back, NodeFlags::HIDDEN, !view.adding);
            if view.adding {
                let h = (y - layout.top).max(44.0 * s);
                self.hits
                    .push((Hit::Back, Rect::new(0.0, layout.top, back_w + pad, h)));
            }
        }
        // The camera's window, unless a pairing waits (its six words are
        // today's quiet screen).
        let camera = view.camera.clone().filter(|_| !view.waiting);
        // The title's band over the camera's image, to just below it.
        let title_bottom = y + 8.0 * s;
        // The status lines say "not paired", which the camera's window
        // already says: not shown with it.
        for (i, &id) in self.status.iter().enumerate() {
            let text = view.status.get(i).map_or("", String::as_str);
            if camera.is_some() {
                self.router
                    .get_mut::<Label>(id)
                    .expect("a label node")
                    .set_text("");
                continue;
            }
            place(
                &mut self.router,
                &mut y,
                id,
                Some(text),
                (15.0, theme().muted),
                0.0,
            );
        }

        // The bottom: the field and Pair, or Details and Cancel while a
        // pairing waits, or with the camera, Paste a link (and Open
        // Settings, and Cancel in the add mode).
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
                (PLACEHOLDER.to_string(), theme().muted)
            } else {
                // The tail of the entered text.
                let room = layout
                    .columns_in(field_w - 8.0 * s, Layout::FIELD_SIZE * 1.3)
                    .saturating_sub(1);
                let chars: Vec<char> = view.field.chars().collect();
                let tail: String = chars[chars.len().saturating_sub(room)..].iter().collect();
                (tail, theme().text)
            };
            edit.set_text(&text);
            edit.state.set_cursor(text.len());
            edit.text_color = Some(color);
            edit.font_size = Some(Layout::FIELD_SIZE * s);
            edit.corner_radius = Some(8.0 * s);
            edit.rect = field_rect;
            edit.state.set_active(view.field_focused && !view.waiting);
        }
        // The field and Pair: today's screen, or the camera's window after
        // Paste a link.
        let field_shown = !view.waiting && camera.as_ref().is_none_or(|c| c.paste);
        self.router
            .set_flags(self.field, NodeFlags::HIDDEN, !field_shown);
        self.place_button(self.pair, pair_rect, s, field_shown);
        // The camera's row: its buttons side by side.
        let mut row: Vec<(Hit, NodeId)> = Vec::new();
        if let Some(c) = camera.as_ref().filter(|c| !c.paste) {
            row.push((Hit::PasteLink, self.paste_link));
            if c.settings {
                row.push((Hit::OpenSettings, self.open_settings));
            }
            if view.adding {
                row.push((Hit::Cancel, self.cancel));
            }
        }
        let n = row.len().max(1) as f32;
        let each = (layout.w - 2.0 * pad - 12.0 * s * (n - 1.0)) / n;
        for (i, &(hit, id)) in row.iter().enumerate() {
            let rect = Rect::new(pad + i as f32 * (each + 12.0 * s), row_y, each, row_h);
            self.place_button(id, rect, s, true);
            self.hits.push((hit, rect));
        }
        for id in [self.paste_link, self.open_settings] {
            if !row.iter().any(|&(_, r)| r == id) {
                self.router.set_flags(id, NodeFlags::HIDDEN, true);
            }
        }
        let half = (layout.w - 2.0 * pad - 12.0 * s) / 2.0;
        let details_rect = Rect::new(pad, row_y, half, row_h);
        let cancel_rect = Rect::new(pad + half + 12.0 * s, row_y, half, row_h);
        {
            let label = if view.details {
                copy::PAIRING_DETAILS_HIDE.text
            } else {
                copy::PAIRING_DETAILS.text
            };
            let button = self
                .router
                .get_mut::<Button>(self.details)
                .expect("the details");
            button.text = label.to_string();
        }
        self.place_button(self.details, details_rect, s, view.waiting);
        let cancel_in_row = row.iter().any(|&(hit, _)| hit == Hit::Cancel);
        if !cancel_in_row {
            self.place_button(self.cancel, cancel_rect, s, view.waiting);
        }
        if view.waiting {
            self.hits.push((Hit::Details, details_rect));
            self.hits.push((Hit::Cancel, cancel_rect));
        } else if field_shown {
            self.hits.push((Hit::Field, field_rect));
            self.hits.push((Hit::Pair, pair_rect));
        }

        // Above them, the pairing area, as many lines as fit.
        let height = |line: &Line| (line.gap + line.size * 1.35) * s;
        let code_lines = &view.code[..view.code.len().min(CODE_LINES)];
        let code_h: f32 = code_lines.iter().map(height).sum();
        let code_top = (row_y - 12.0 * s - code_h).max(y);
        let mut code_y = code_top;
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

        self.lay_out_camera(layout, camera.as_ref(), code_top, title_bottom);
        self.shown = Some((view, layout));
        self.dirty = true;
    }

    /// The camera's area, from the top safe area to the line under it,
    /// full width (decision 1), with the placeholder's label and lines, the
    /// zoom ratio, and what [`Screen::draw`] draws over it; or nothing
    /// without the camera's window.
    fn lay_out_camera(
        &mut self,
        layout: Layout,
        camera: Option<&CameraView>,
        code_top: f32,
        title_bottom: f32,
    ) {
        let s = layout.scale;
        let pad = Layout::PAD * s;
        let mut prompt = None;
        let mut detail: Vec<String> = Vec::new();
        let mut zoom = None;
        self.camera = camera.map(|c| {
            let top = layout.top;
            let bottom = (code_top - 12.0 * s).max(title_bottom + 48.0 * s);
            let area = Rect::new(0.0, top, layout.w, bottom - top);
            let side = RETICLE_SHARE * area.w.min(area.h - (title_bottom - top));
            let (cx, cy) = (area.w / 2.0, (title_bottom + bottom) / 2.0);
            let reticle = Rect::new(cx - side / 2.0, cy - side / 2.0, side, side);
            self.hits.push((Hit::Area, area));
            let glyph = c.prompt.as_ref().map(|text| {
                prompt = Some((text.clone(), Rect::new(0.0, cy, layout.w, 20.0 * s * 1.3)));
                if let Some(d) = &c.detail {
                    detail = wrap(d, layout.columns_in(side, 15.0));
                }
                [cx, cy - 36.0 * s]
            });
            zoom = c.zoom.clone();
            CameraDraw {
                area,
                title_band: Rect::new(0.0, top, layout.w, title_bottom - top),
                reticle,
                live: c.live,
                glyph,
                scale: s,
            }
        });
        let label = |router: &mut Router, id, text: &str, rect: Rect, size: f32| {
            let l = router.get_mut::<Label>(id).expect("a label node");
            l.set_text(text);
            l.font_size = Some(size * s);
            l.rect = rect;
        };
        match prompt {
            Some((text, rect)) => label(&mut self.router, self.prompt, &text, rect, 20.0),
            None => label(&mut self.router, self.prompt, "", Rect::default(), 20.0),
        }
        let mut detail_y = self
            .camera
            .map_or(0.0, |c| c.reticle.y + c.reticle.h / 2.0 + 34.0 * s);
        for (i, &id) in self.detail.clone().iter().enumerate() {
            let text = detail.get(i).map_or("", String::as_str);
            let rect = Rect::new(pad, detail_y, layout.w - 2.0 * pad, 15.0 * s * 1.3);
            label(&mut self.router, id, text, rect, 15.0);
            detail_y += 15.0 * s * 1.35;
        }
        let zoom_rect = Rect::new(
            layout.w - pad - 120.0 * s,
            title_bottom + 8.0 * s,
            120.0 * s,
            18.0 * s * 1.3,
        );
        label(
            &mut self.router,
            self.zoom,
            zoom.as_deref().unwrap_or(""),
            zoom_rect,
            18.0,
        );
    }

    /// The camera's area in surface pixels, while the camera's window
    /// shows: where the viewfinder is drawn.
    pub fn camera_area(&self) -> Option<Rect> {
        self.camera.map(|c| c.area)
    }

    /// The controls shown, as last laid out, with where they are.
    #[cfg(test)]
    pub(crate) fn hits(&self) -> &[(Hit, Rect)] {
        &self.hits
    }

    /// Every text shown, labels and buttons, in node order (tests).
    #[cfg(test)]
    pub(crate) fn shown_texts(&self) -> Vec<String> {
        let mut ids = vec![self.title, self.back];
        ids.extend(self.status);
        ids.extend(&self.code);
        ids.push(self.prompt);
        ids.extend(&self.detail);
        ids.push(self.zoom);
        ids.extend([
            self.field,
            self.pair,
            self.cancel,
            self.details,
            self.paste_link,
            self.open_settings,
        ]);
        ids.into_iter()
            .filter(|&id| !self.router.flags(id).contains(NodeFlags::HIDDEN))
            .filter_map(|id| {
                let text = if let Some(l) = self.router.get::<Label>(id) {
                    l.text().to_string()
                } else if let Some(e) = self.router.get::<Edit>(id) {
                    e.text().to_string()
                } else {
                    self.router.get::<Button>(id)?.text.clone()
                };
                (!text.is_empty()).then_some(text)
            })
            .collect()
    }

    fn place_button(&mut self, id: NodeId, rect: Rect, s: f32, visible: bool) {
        let button = self.router.get_mut::<Button>(id).expect("a button node");
        button.rect = rect;
        button.font_size = Some(18.0 * s);
        button.corner_radius = Some(10.0 * s);
        self.router.set_flags(id, NodeFlags::HIDDEN, !visible);
    }

    /// Marks the screen to be drawn again, as after another screen drew
    /// over the list.
    /// The theme changed: the nodes, which hold their colours, are made
    /// again, and laid out at the next [`Screen::set`].
    pub fn restyle(&mut self) {
        *self = Self::new();
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Rebuilds `list` if the screen changed since the last call. Returns
    /// whether it did.
    pub fn draw(&mut self, list: &mut DrawList, ctx: &DrawContext) -> bool {
        if !self.dirty {
            return false;
        }
        list.clear();
        if let Some(c) = self.camera {
            draw_camera(list, &c);
        }
        self.router.render(list, ctx);
        self.dirty = false;
        true
    }
}

/// What the camera's window draws under its labels and buttons: the
/// placeholder's band and camera glyph (no viewfinder yet), the corner
/// marks, and the title's band over the image. The viewfinder itself is
/// drawn before the list, in its own pass (`App::render`).
fn draw_camera(list: &mut DrawList, c: &CameraDraw) {
    let s = c.scale;
    if !c.live {
        fill(list, c.area, theme().band, 0.0);
    }
    // The four corner marks, L shapes.
    let alpha = if c.live {
        MARK_ALPHA_LIVE
    } else {
        MARK_ALPHA_PLACEHOLDER
    };
    let color = with_alpha(theme().text, alpha * theme().text[3]);
    let (t, l) = (MARK_PT * s, MARK_LEN_PT * s);
    let r = c.reticle;
    for (x, y, dx, dy) in [
        (r.x, r.y, 1.0, 1.0),
        (r.x + r.w, r.y, -1.0, 1.0),
        (r.x, r.y + r.h, 1.0, -1.0),
        (r.x + r.w, r.y + r.h, -1.0, -1.0),
    ] {
        // Each arm from the corner inwards.
        let across = Rect::new(x.min(x + dx * l), y.min(y + dy * t), l, t);
        let down = Rect::new(x.min(x + dx * t), y.min(y + dy * l), t, l);
        fill(list, across, color, 0.0);
        fill(list, down, color, 0.0);
    }
    // The camera glyph: a body, its top, and a lens, from the same boxes.
    if let Some([cx, cy]) = c.glyph {
        let glyph = with_alpha(theme().text, MARK_ALPHA_LIVE * theme().text[3]);
        let (w, h) = (56.0 * s, 38.0 * s);
        fill(
            list,
            Rect::new(cx - 12.0 * s, cy - h / 2.0 - 6.0 * s, 24.0 * s, 8.0 * s),
            glyph,
            2.0 * s,
        );
        fill(
            list,
            Rect::new(cx - w / 2.0, cy - h / 2.0, w, h),
            glyph,
            6.0 * s,
        );
        let lens = 22.0 * s;
        fill(
            list,
            Rect::new(cx - lens / 2.0, cy - lens / 2.0, lens, lens),
            theme().band,
            lens / 2.0,
        );
        let inner = 12.0 * s;
        fill(
            list,
            Rect::new(cx - inner / 2.0, cy - inner / 2.0, inner, inner),
            glyph,
            inner / 2.0,
        );
    }
    if c.live {
        fill(
            list,
            c.title_band,
            with_alpha(theme().bg, TITLE_BAND_ALPHA),
            0.0,
        );
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
            color: theme().text,
            gap: 0.0,
        }
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(wrap("  ", 9), Vec::<String>::new());
        assert_eq!(wrap("unbreakable", 4), ["unbreakable"]);
        assert_eq!(wrap("one\ntwo", 9), ["one", "two"]);
        assert_eq!(wrap("one\n\ntwo", 9), ["one", "", "two"]);
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
            code: vec![line("hello")],
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
    fn the_pairing_row_is_cancel_while_a_pairing_waits() {
        let mut screen = Screen::new();
        screen.set(PHONE, View::default());
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Pair));
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Field));
        assert!(!screen.hits.iter().any(|(h, _)| *h == Hit::Cancel));
        screen.set(
            PHONE,
            View {
                waiting: true,
                code: vec![line("flankrkey1abc")],
                ..View::default()
            },
        );
        assert!(
            !screen
                .hits
                .iter()
                .any(|(h, _)| matches!(h, Hit::Pair | Hit::Field))
        );
        let cancel = screen
            .hits
            .iter()
            .find(|(h, _)| *h == Hit::Cancel)
            .expect("a Cancel")
            .1;
        assert_eq!(
            screen.hit(cancel.x + 1.0, cancel.y + 1.0),
            Some(Hit::Cancel)
        );
        let label = screen.router.get::<Button>(screen.cancel).unwrap();
        assert_eq!(label.text, "Cancel");
        // Details beside it, which reads "Hide details" once open.
        let details = screen
            .hits
            .iter()
            .find(|(h, _)| *h == Hit::Details)
            .expect("a Details")
            .1;
        assert_eq!(
            screen.hit(details.x + 1.0, details.y + 1.0),
            Some(Hit::Details)
        );
        assert!(details.x + details.w < cancel.x, "side by side");
        let text = |s: &Screen| s.router.get::<Button>(s.details).unwrap().text.clone();
        assert_eq!(text(&screen), "Details");
        screen.set(
            PHONE,
            View {
                waiting: true,
                details: true,
                code: vec![line("flankrkey1abc")],
                ..View::default()
            },
        );
        assert_eq!(text(&screen), "Hide details");
    }

    #[test]
    fn the_pair_button_is_a_control() {
        let mut screen = Screen::new();
        screen.set(PHONE, View::default());
        let pair = screen.hits.iter().find(|(h, _)| *h == Hit::Pair).unwrap().1;
        assert_eq!(screen.hit(pair.x + 1.0, pair.y + 1.0), Some(Hit::Pair));
        // The middle of the screen is no control: a tap.
        assert_eq!(screen.hit(540.0, 1200.0), None);
    }

    #[test]
    fn the_add_mode_has_its_title_a_back_and_the_six_words_large() {
        use crate::remotes::{Pairing, WORDS_PT, Waiting};
        use std::time::Instant;
        let now = Instant::now();
        let mut screen = Screen::new();
        screen.set(PHONE, View::default());
        let title = |s: &Screen| s.router.get::<Label>(s.title).unwrap().text().to_string();
        assert_eq!(title(&screen), "flankr");
        assert!(!screen.hits.iter().any(|(h, _)| *h == Hit::Back));
        // The + in Settings: the add mode, with the field and Pair.
        let mut p = Pairing::default();
        p.open_add();
        let view = |p: &Pairing| View {
            code: p.lines(&PHONE, None, now),
            waiting: p.waiting(),
            details: p.details,
            adding: p.adding,
            ..View::default()
        };
        screen.set(PHONE, view(&p));
        assert_eq!(title(&screen), "Pair another remote");
        let back = screen.hits.iter().find(|(h, _)| *h == Hit::Back).unwrap().1;
        assert_eq!(screen.hit(back.x + 1.0, back.y + 1.0), Some(Hit::Back));
        assert!(back.y >= PHONE.top && back.x == 0.0, "top left");
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Pair));
        // The daemon holds the request: its words show, large, and Details
        // and Cancel replace the field.
        p.show(Waiting::new(
            "m4".into(),
            "m4.ts.net:7347".into(),
            Some("abandon ability able about above absent".into()),
            "zoo zone youth wrist wrong yellow".into(),
            Some(60),
            None,
            now,
        ));
        screen.set(PHONE, view(&p));
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Cancel));
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Details));
        assert!(!screen.hits.iter().any(|(h, _)| *h == Hit::Pair));
        let shown = |s: &Screen| -> Vec<(String, f32)> {
            s.code
                .iter()
                .map(|&id| {
                    let l = s.router.get::<Label>(id).unwrap();
                    (l.text().to_string(), l.font_size.unwrap_or(0.0))
                })
                .filter(|(t, _)| !t.is_empty())
                .collect()
        };
        let quiet = shown(&screen);
        let words: Vec<&str> = quiet
            .iter()
            .filter(|(_, size)| *size == WORDS_PT * PHONE.scale)
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(words, ["abandon ability able", "about above absent"]);
        assert!(
            quiet
                .iter()
                .any(|(t, _)| t.starts_with("If flankr pair on m4")),
            "{quiet:?}"
        );
        assert!(!quiet.iter().any(|(t, _)| t.contains("zoo")), "{quiet:?}");
        // Details: the fingerprint and the address, all of it drawn.
        p.toggle_details();
        screen.set(PHONE, view(&p));
        let details = shown(&screen);
        assert!(details.iter().any(|(t, _)| t == "Daemon fingerprint"));
        assert!(details.iter().any(|(t, _)| t == "zoo zone youth"));
        assert!(details.iter().any(|(t, _)| t == "m4 at m4.ts.net:7347"));
        assert!(
            details
                .iter()
                .any(|(t, _)| t == "The link expires in 60 s.")
        );
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

    fn claude() -> FocusInfo {
        FocusInfo {
            pane_id: "w2:p4".into(),
            project: Some("libmorph".into()),
            agent: Some("Claude".into()),
            tab_label: Some("-ad".into()),
            tab_number: Some(4),
            name: "Claude in libmorph".into(),
            spoken: "Claude in libmorph".into(),
        }
    }

    fn dictation(target: Option<FocusInfo>) -> Dictation {
        Dictation {
            active: true,
            mode: PromptMode::Voice,
            text: "Please run the tests.".into(),
            target,
            focus_moved: None,
            phase: DictationPhase::Collecting,
            submit_in: None,
            general_agent: false,
        }
    }

    #[test]
    fn the_dictation_card_shows_the_target_and_the_text() {
        let texts = |d: &Dictation| -> Vec<String> {
            dictation_lines(d).into_iter().map(|l| l.text).collect()
        };
        assert_eq!(
            texts(&dictation(Some(claude()))),
            [
                "Taking a message",
                "Message for Claude \u{b7} libmorph \u{b7} tab -ad",
                "Please run the tests."
            ]
        );
        let none = dictation(None);
        assert_eq!(
            texts(&none)[1],
            "No agent focused \u{2014} say \u{2018}focus \u{2026}\u{2019}"
        );
        let shell = dictation(Some(FocusInfo {
            agent: None,
            ..claude()
        }));
        assert!(texts(&shell)[1].starts_with("No agent focused"));
        let moved = Dictation {
            focus_moved: Some("Claude in keypad".into()),
            submit_in: Some(3),
            ..dictation(Some(claude()))
        };
        let lines = texts(&moved);
        assert_eq!(lines[0], "Submitting in 3 s\u{2026}");
        assert_eq!(lines[2], "focus moved to Claude in keypad");
        let empty = Dictation {
            text: String::new(),
            phase: DictationPhase::Confirming,
            ..dictation(Some(claude()))
        };
        assert_eq!(texts(&empty)[0], "Waiting for your approval");
        assert_eq!(texts(&empty)[2], "(say your message)");
        let kbd_empty = Dictation {
            text: String::new(),
            mode: PromptMode::Keyboard,
            ..dictation(Some(claude()))
        };
        assert_eq!(texts(&kbd_empty)[2], "Type your message");
        let ga = Dictation {
            text: "Hello".into(),
            general_agent: true,
            ..dictation(None)
        };
        let ga_lines = texts(&ga);
        assert_eq!(ga_lines[0], "\u{2726} Gemini");
        assert_eq!(ga_lines[1], "General agent chat");
        assert_eq!(ga_lines[2], "Hello");
        // The prompt card's: the state, warnings only, and the text.
        let p = prompt_input(&moved);
        assert_eq!(p.status.unwrap().0, "Submitting in 3 s\u{2026}");
        let notes: Vec<&str> = p.notes.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(notes, ["focus moved to Claude in keypad"]);
        assert_eq!(p.text, "Please run the tests.");
        let p = prompt_input(&none);
        assert!(p.notes[0].0.starts_with("No agent focused"));
        assert_eq!(p.notes[0].1, theme().warn);
        let p = prompt_input(&ga);
        assert!(p.status.is_none() && p.notes.is_empty());
        assert_eq!(p.text, "Hello");
        // A tab without a label shows its number.
        let numbered = FocusInfo {
            tab_label: None,
            ..claude()
        };
        assert_eq!(
            focus_words(&numbered),
            "Claude \u{b7} libmorph \u{b7} tab 4"
        );
    }

    #[test]
    fn the_field_shows_its_tail() {
        let mut screen = Screen::new();
        let field = format!("flankr://m4:7347/pair#flankr1{}", "x".repeat(300));
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
