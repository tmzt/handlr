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
use std::sync::Arc;

use libcontrols::{Button, DrawContext, Edit, Label, NodeFlags, NodeId, Rect, Router, TextAlign};
use libflankr_proto::common::FocusInfo;
use libmsdf::drawlist::DrawList;

use crate::agent::fill;
use crate::markdown_view::{Faces, MarkdownView};
use crate::prompt::PromptInput;
use crate::settings::copy;
use crate::theme::{theme, with_alpha};

/// The add mode's back glyph: ASCII, as the pairing screen's face has no
/// "‹".
const BACK_GLYPH: &str = "<";
/// The check glyph's disc, in points.
const CHECK_PT: f32 = 44.0;
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
    /// The corner marks are drawn (the camera's window, not a check).
    marks: bool,
    /// The glyph over the label, and its centre.
    glyph: Option<(Glyph, [f32; 2])>,
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
    /// The pairing area above the bottom row, Markdown, one paragraph a
    /// line; while a pairing waits, the modal's lines instead.
    pub code: Vec<Line>,
    /// The pairing field's text; the screen shows its tail.
    pub field: String,
    /// The field has the keyboard.
    pub field_focused: bool,
    /// A pairing waits for the owner: its words, `code`, show in a modal
    /// titled *Confirm on your Mac*, with Details and Cancel (the owner,
    /// 2026-10-28). Nothing under it takes a tap.
    pub waiting: bool,
    /// Details are open (the modal's or a finished pairing's): the Details
    /// button reads "Hide details".
    pub details: bool,
    /// The add mode (**+** in Settings' Remotes): its title, and a "<"
    /// back to Settings.
    pub adding: bool,
    /// The area at the top (plan 20261028_02): the camera's window where
    /// there is a camera, and the read and finished states everywhere;
    /// `code` is under it. `None`: the paste screen.
    pub camera: Option<CameraView>,
}

/// What the area at the top shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Glyph {
    /// The camera's window: the camera glyph over the placeholder, the
    /// corner marks, and a tap on the area starts the scan.
    #[default]
    Camera,
    /// A check, drawn from primitives as the camera glyph is: a code was
    /// read, or the pairing is done. No marks, and the area takes no tap.
    Check,
}

/// The bottom row under the area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Row {
    /// **Paste a link**, **Open Settings** after a refusal, and Cancel in
    /// the add mode; or the field and Pair after Paste a link.
    #[default]
    Scan,
    /// Cancel alone: a scanned code is being paired.
    Cancel,
    /// Details and **Done**: the pairing is done.
    Done,
}

/// What the pairing window shows of the camera (plan 20261028_02,
/// decisions 1-4, 12-14), and of a read code and a finished pairing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraView {
    /// The camera runs: the viewfinder under the area, the corner marks
    /// bright, no placeholder.
    pub live: bool,
    /// The glyph over the label.
    pub glyph: Glyph,
    /// The label under the glyph (*Tap to scan*, *Code read*, *Paired
    /// with m4*); `None` while the camera runs.
    pub prompt: Option<String>,
    /// Under the label, Markdown, centred: the add mode's what to do,
    /// "Connecting to m4…", a finished pairing's address.
    pub detail: Option<String>,
    /// The zoom ratio at the top right of the area, after a pinch.
    pub zoom: Option<String>,
    /// **Paste a link** was tapped: the field and Pair in the bottom row.
    pub paste: bool,
    /// **Open Settings** in the bottom row (a refusal only Settings can
    /// undo).
    pub settings: bool,
    /// The bottom row.
    pub row: Row,
}

/// A control a touch landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Field,
    Pair,
    /// Cancel: in the pairing modal, while a scanned code is paired, or in
    /// the add mode's camera row.
    Cancel,
    /// Details, in the pairing modal or on a finished pairing: opens or
    /// closes them.
    Details,
    /// The add mode's "<": back to Settings.
    Back,
    /// The camera's area: the placeholder's Tap to scan.
    Area,
    /// **Paste a link**: the field instead of the camera.
    PasteLink,
    /// **Open Settings**, after a refusal only Settings can undo.
    OpenSettings,
    /// **Done**, on a finished pairing.
    Done,
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
    pub(crate) const PAD: f32 = 20.0;
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

/// The pairing modal, as the settings' confirming modal is drawn
/// (`settings/confirm.rs`): a card at most this wide, in points, with this
/// padding, corner radius and title size, centred over the screen dimmed
/// by black at this opacity; opened and closed at once, never faded.
const MODAL_MAX_W: f32 = 560.0;
const MODAL_MARGIN: f32 = 12.0;
const MODAL_PAD: f32 = 24.0;
const MODAL_RADIUS: f32 = 14.0;
const MODAL_TITLE_PT: f32 = 22.0;
const MODAL_DIM: [f32; 4] = [0.0, 0.0, 0.0, 0.45];

/// The modal's card width and its content's width, in pixels.
fn modal_widths(layout: &Layout) -> (f32, f32) {
    let s = layout.scale;
    let w = (layout.w - 2.0 * MODAL_MARGIN * s).min(MODAL_MAX_W * s);
    (w, w - 2.0 * MODAL_PAD * s)
}

/// The layout the modal's lines are wrapped for: as wide as its content.
pub fn modal_layout(layout: &Layout) -> Layout {
    let (_, inner) = modal_widths(layout);
    Layout {
        w: inner + 2.0 * Layout::PAD * layout.scale,
        ..*layout
    }
}

/// The placeholder in the empty pairing field.
const PLACEHOLDER: &str = "paste a pairing URL";

/// The modal's card and the screen it dims, as last laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ModalDraw {
    screen: Rect,
    card: Rect,
    radius: f32,
}

/// The persistent screen.
pub struct Screen {
    router: Router,
    title: NodeId,
    back: NodeId,
    status: [NodeId; 2],
    /// The pairing area's Markdown, one node a line, made at each change.
    code: Vec<NodeId>,
    field: NodeId,
    pair: NodeId,
    cancel: NodeId,
    details: NodeId,
    done: NodeId,
    /// The camera's window: the placeholder's label and the Markdown under
    /// it, the zoom ratio, **Paste a link** and **Open Settings**.
    prompt: NodeId,
    detail: Option<NodeId>,
    zoom: NodeId,
    paste_link: NodeId,
    open_settings: NodeId,
    /// The pairing modal: drawn over the rest, from its own router.
    modal: Router,
    modal_title: NodeId,
    modal_lines: Vec<NodeId>,
    modal_details: NodeId,
    modal_cancel: NodeId,
    modal_draw: Option<ModalDraw>,
    /// The camera's area and what is drawn in it, as last laid out.
    camera: Option<CameraDraw>,
    /// The faces the Markdown is drawn in (`None` in unit tests: the
    /// context's font).
    faces: Option<Arc<Faces>>,
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
        let done = router.add(None, button(copy::PAIRING_DONE.text, theme().message));
        let prompt = router.add(None, label("", 20.0, theme().text, TextAlign::Center));
        let zoom = router.add(None, label("", 18.0, theme().text, TextAlign::Right));
        let paste_link = router.add(None, button(copy::PAIRING_PASTE_LINK.text, theme().back));
        let open_settings = router.add(
            None,
            button(copy::PAIRING_OPEN_SETTINGS.text, theme().message),
        );
        let mut modal = Router::new();
        let modal_title = modal.add(
            None,
            label(
                copy::PAIRING_CONFIRM_TITLE.text,
                MODAL_TITLE_PT,
                theme().text,
                TextAlign::Left,
            ),
        );
        let modal_details = modal.add(None, button(copy::PAIRING_DETAILS.text, theme().back));
        let modal_cancel = modal.add(None, button(copy::CANCEL.text, theme().back));
        for id in [modal_title, modal_details, modal_cancel] {
            modal.set_flags(id, NodeFlags::HIDDEN, true);
        }
        Self {
            router,
            title,
            back,
            status,
            code: Vec::new(),
            field,
            pair,
            cancel,
            details,
            done,
            prompt,
            detail: None,
            zoom,
            paste_link,
            open_settings,
            modal,
            modal_title,
            modal_lines: Vec::new(),
            modal_details,
            modal_cancel,
            modal_draw: None,
            camera: None,
            faces: None,
            hits: Vec::new(),
            shown: None,
            dirty: true,
        }
    }

    /// The faces the pairing area's Markdown is drawn in.
    pub fn with_faces(mut self, faces: Option<Arc<Faces>>) -> Self {
        self.faces = faces;
        self.shown = None;
        self
    }

    /// The control at `(x, y)`, in surface pixels, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(_, rect)| rect.contains([x, y]))
            .map(|&(hit, _)| hit)
    }

    /// `line` as Markdown, `width` wide at the layout's scale, at `(x, 0)`.
    fn markdown(&self, line: &Line, x: f32, width: f32, s: f32) -> MarkdownView {
        MarkdownView::new(
            &line.text,
            x,
            0.0,
            width,
            line.size * s,
            line.color,
            self.faces.clone(),
        )
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
        // The area, unless a pairing waits (its modal is over the window).
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

        // The bottom: the field and Pair, or with the area its row:
        // Paste a link (and Open Settings, and Cancel in the add mode),
        // Cancel while a scanned code is paired, or Details and Done.
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
        // The field and Pair: the paste screen, or the camera's window
        // after Paste a link.
        let field_shown = !view.waiting
            && camera
                .as_ref()
                .is_none_or(|c| c.paste && c.row == Row::Scan);
        self.router
            .set_flags(self.field, NodeFlags::HIDDEN, !field_shown);
        self.place_button(self.pair, pair_rect, s, field_shown);
        if field_shown {
            self.hits.push((Hit::Field, field_rect));
            self.hits.push((Hit::Pair, pair_rect));
        }
        // The area's row: its buttons side by side.
        let mut row: Vec<(Hit, NodeId)> = Vec::new();
        if let Some(c) = camera.as_ref() {
            match c.row {
                Row::Scan if !c.paste => {
                    row.push((Hit::PasteLink, self.paste_link));
                    if c.settings {
                        row.push((Hit::OpenSettings, self.open_settings));
                    }
                    if view.adding {
                        row.push((Hit::Cancel, self.cancel));
                    }
                }
                Row::Scan => {}
                Row::Cancel => row.push((Hit::Cancel, self.cancel)),
                Row::Done => {
                    row.push((Hit::Details, self.details));
                    row.push((Hit::Done, self.done));
                }
            }
        }
        set_details_label(&mut self.router, self.details, view.details);
        let n = row.len().max(1) as f32;
        let each = (layout.w - 2.0 * pad - 12.0 * s * (n - 1.0)) / n;
        for (i, &(hit, id)) in row.iter().enumerate() {
            let rect = Rect::new(pad + i as f32 * (each + 12.0 * s), row_y, each, row_h);
            self.place_button(id, rect, s, true);
            self.hits.push((hit, rect));
        }
        for id in [
            self.paste_link,
            self.open_settings,
            self.cancel,
            self.details,
            self.done,
        ] {
            if !row.iter().any(|&(_, r)| r == id) {
                self.router.set_flags(id, NodeFlags::HIDDEN, true);
            }
        }

        // Above them, the pairing area, as many lines as fit; under the
        // modal, nothing.
        for id in std::mem::take(&mut self.code) {
            self.router.remove(id);
        }
        let width = layout.w - 2.0 * pad;
        let mut views: Vec<(f32, MarkdownView)> = Vec::new();
        if !view.waiting {
            for line in &view.code {
                let md = self.markdown(line, pad, width, s);
                views.push((line.gap * s, md));
            }
        }
        let code_h: f32 = views.iter().map(|(gap, md)| gap + md.height()).sum();
        let code_top = (row_y - 12.0 * s - code_h).max(y);
        let mut code_y = code_top;
        for (gap, mut md) in views {
            if code_y + gap + md.height() > row_y {
                break;
            }
            code_y += gap;
            md.move_to(pad, code_y);
            code_y += md.height();
            self.code.push(self.router.add(None, md));
        }

        self.lay_out_camera(layout, camera.as_ref(), code_top, title_bottom);
        self.lay_out_modal(layout, &view);
        self.shown = Some((view, layout));
        self.dirty = true;
    }

    /// The pairing modal while a pairing waits: its title, its lines
    /// (`view.code`) and Details and Cancel, centred over the screen. Its
    /// controls are then the only ones.
    fn lay_out_modal(&mut self, layout: Layout, view: &View) {
        for id in std::mem::take(&mut self.modal_lines) {
            self.modal.remove(id);
        }
        let open = view.waiting;
        for id in [self.modal_title, self.modal_details, self.modal_cancel] {
            self.modal.set_flags(id, NodeFlags::HIDDEN, !open);
        }
        if !open {
            self.modal_draw = None;
            return;
        }
        self.hits.clear();
        let s = layout.scale;
        let (w, inner) = modal_widths(&layout);
        let pad = MODAL_PAD * s;
        let gap = 12.0 * s;
        let button_h = Layout::ROW_H * s;
        let title_h = MODAL_TITLE_PT * s * 1.35;
        let room = layout.h - layout.top - layout.bottom - 2.0 * MODAL_MARGIN * s;
        let fixed = pad + title_h + gap + 2.0 * gap + button_h + pad;
        // The lines that fit, in order.
        let mut lines: Vec<(f32, MarkdownView)> = Vec::new();
        let mut lines_h = 0.0;
        for line in &view.code {
            let md = self.markdown(line, 0.0, inner, s);
            let h = line.gap * s + md.height();
            if fixed + lines_h + h > room {
                break;
            }
            lines_h += h;
            lines.push((line.gap * s, md));
        }
        let h = fixed + lines_h;
        let viewport_mid = layout.top + (layout.h - layout.top - layout.bottom) / 2.0;
        let card = Rect::new((layout.w - w) / 2.0, viewport_mid - h / 2.0, w, h);
        let x = card.x + pad;
        let mut y = card.y + pad;
        {
            let title = self
                .modal
                .get_mut::<Label>(self.modal_title)
                .expect("the modal's title");
            title.font_size = Some(MODAL_TITLE_PT * s);
            title.color = Some(theme().text);
            title.rect = Rect::new(x, y, inner, MODAL_TITLE_PT * s * 1.3);
        }
        y += title_h + gap;
        for (line_gap, mut md) in lines {
            y += line_gap;
            md.move_to(x, y);
            y += md.height();
            self.modal_lines.push(self.modal.add(None, md));
        }
        let by = card.y + h - pad - button_h;
        let bw = (inner - gap) / 2.0;
        let details = Rect::new(x, by, bw, button_h);
        let cancel = Rect::new(x + bw + gap, by, bw, button_h);
        set_details_label(&mut self.modal, self.modal_details, view.details);
        for (id, rect) in [(self.modal_details, details), (self.modal_cancel, cancel)] {
            let button = self.modal.get_mut::<Button>(id).expect("a button node");
            button.rect = rect;
            button.font_size = Some(18.0 * s);
            button.corner_radius = Some(10.0 * s);
        }
        self.hits.push((Hit::Details, details));
        self.hits.push((Hit::Cancel, cancel));
        self.modal_draw = Some(ModalDraw {
            screen: Rect::new(0.0, 0.0, layout.w, layout.h),
            card,
            radius: MODAL_RADIUS * s,
        });
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
        let mut detail: Option<(String, f32, f32)> = None;
        let mut zoom = None;
        self.camera = camera.map(|c| {
            let top = layout.top;
            let bottom = (code_top - 12.0 * s).max(title_bottom + 48.0 * s);
            let area = Rect::new(0.0, top, layout.w, bottom - top);
            let side = RETICLE_SHARE * area.w.min(area.h - (title_bottom - top));
            let (cx, cy) = (area.w / 2.0, (title_bottom + bottom) / 2.0);
            let reticle = Rect::new(cx - side / 2.0, cy - side / 2.0, side, side);
            if c.glyph == Glyph::Camera {
                self.hits.push((Hit::Area, area));
            }
            let glyph = c.prompt.as_ref().map(|text| {
                prompt = Some((text.clone(), Rect::new(0.0, cy, layout.w, 20.0 * s * 1.3)));
                if let Some(d) = &c.detail {
                    // A finished pairing's lines may be wider than the
                    // reticle: the screen's width, less its margins.
                    let width = match c.glyph {
                        Glyph::Camera => side,
                        Glyph::Check => layout.w - 2.0 * pad,
                    };
                    detail = Some((d.clone(), width, cy + 34.0 * s));
                }
                [cx, cy - 36.0 * s]
            });
            zoom = c.zoom.clone();
            CameraDraw {
                area,
                title_band: Rect::new(0.0, top, layout.w, title_bottom - top),
                reticle,
                live: c.live,
                marks: c.glyph == Glyph::Camera,
                glyph: glyph.map(|at| (c.glyph, at)),
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
        if let Some(id) = self.detail.take() {
            self.router.remove(id);
        }
        if let Some((text, width, y)) = detail {
            let mut md = MarkdownView::new(
                &text,
                (layout.w - width) / 2.0,
                y,
                width,
                15.0 * s,
                theme().muted,
                self.faces.clone(),
            );
            md.center();
            self.detail = Some(self.router.add(None, md));
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
        self.camera.filter(|c| c.marks).map(|c| c.area)
    }

    /// The controls shown, as last laid out, with where they are.
    #[cfg(test)]
    pub(crate) fn hits(&self) -> &[(Hit, Rect)] {
        &self.hits
    }

    /// Every text shown, labels, Markdown and buttons, the window's then
    /// the modal's, in node order (tests).
    #[cfg(test)]
    pub(crate) fn shown_texts(&self) -> Vec<String> {
        let mut ids = vec![self.title, self.back];
        ids.extend(self.status);
        ids.extend(&self.code);
        ids.push(self.prompt);
        ids.extend(self.detail);
        ids.push(self.zoom);
        ids.extend([
            self.field,
            self.pair,
            self.cancel,
            self.details,
            self.done,
            self.paste_link,
            self.open_settings,
        ]);
        let mut out = texts_of(&self.router, &ids);
        let mut modal = vec![self.modal_title];
        modal.extend(&self.modal_lines);
        modal.extend([self.modal_details, self.modal_cancel]);
        out.extend(texts_of(&self.modal, &modal));
        out
    }

    fn place_button(&mut self, id: NodeId, rect: Rect, s: f32, visible: bool) {
        let button = self.router.get_mut::<Button>(id).expect("a button node");
        button.rect = rect;
        button.font_size = Some(18.0 * s);
        button.corner_radius = Some(10.0 * s);
        self.router.set_flags(id, NodeFlags::HIDDEN, !visible);
    }

    /// The theme changed: the nodes, which hold their colours, are made
    /// again, and laid out at the next [`Screen::set`].
    pub fn restyle(&mut self) {
        let faces = self.faces.take();
        *self = Self::new().with_faces(faces);
    }

    /// Marks the screen to be drawn again, as after another screen drew
    /// over the list.
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
        if let Some(m) = self.modal_draw {
            fill(list, m.screen, MODAL_DIM, 0.0);
            fill(list, m.card, theme().card, m.radius);
            self.modal.render(list, ctx);
        }
        self.dirty = false;
        true
    }
}

/// The Details button's label: "Hide details" while they are open.
fn set_details_label(router: &mut Router, id: NodeId, open: bool) {
    let label = if open {
        copy::PAIRING_DETAILS_HIDE.text
    } else {
        copy::PAIRING_DETAILS.text
    };
    router.get_mut::<Button>(id).expect("the details").text = label.to_string();
}

/// The texts of the nodes `ids` of `router` that are shown (tests).
#[cfg(test)]
fn texts_of(router: &Router, ids: &[NodeId]) -> Vec<String> {
    ids.iter()
        .filter(|&&id| !router.flags(id).contains(NodeFlags::HIDDEN))
        .filter_map(|&id| {
            let text = if let Some(l) = router.get::<Label>(id) {
                l.text().to_string()
            } else if let Some(e) = router.get::<Edit>(id) {
                e.text().to_string()
            } else if let Some(md) = router.get::<MarkdownView>(id) {
                // Its wrapped lines as one text.
                md.text().replace('\n', " ")
            } else {
                router.get::<Button>(id)?.text.clone()
            };
            (!text.is_empty()).then_some(text)
        })
        .collect()
}

/// What the area draws under its labels and buttons: the placeholder's
/// band and its glyph (no viewfinder yet), the corner marks, and the
/// title's band over the image. The viewfinder itself is drawn before the
/// list, in its own pass (`App::render`).
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
    let corners = [
        (r.x, r.y, 1.0, 1.0),
        (r.x + r.w, r.y, -1.0, 1.0),
        (r.x, r.y + r.h, 1.0, -1.0),
        (r.x + r.w, r.y + r.h, -1.0, -1.0),
    ];
    for (x, y, dx, dy) in corners.into_iter().filter(|_| c.marks) {
        // Each arm from the corner inwards.
        let across = Rect::new(x.min(x + dx * l), y.min(y + dy * t), l, t);
        let down = Rect::new(x.min(x + dx * t), y.min(y + dy * l), t, l);
        fill(list, across, color, 0.0);
        fill(list, down, color, 0.0);
    }
    let glyph = with_alpha(theme().text, MARK_ALPHA_LIVE * theme().text[3]);
    match c.glyph {
        // The camera glyph: a body, its top, and a lens, from the same
        // boxes.
        Some((Glyph::Camera, [cx, cy])) => {
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
        // The check: a disc, and two strokes through it in the band's
        // colour.
        Some((Glyph::Check, [cx, cy])) => {
            let d = CHECK_PT * s;
            fill(
                list,
                Rect::new(cx - d / 2.0, cy - d / 2.0, d, d),
                glyph,
                d / 2.0,
            );
            let at = |x: f32, y: f32| [cx + x * d, cy + y * d];
            let thick = 0.09 * d;
            list.push_line(at(-0.24, 0.0), at(-0.07, 0.17), thick, theme().band);
            list.push_line(at(-0.07, 0.17), at(0.25, -0.16), thick, theme().band);
        }
        None => {}
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
    fn a_waiting_pairing_is_a_modal_with_details_and_cancel() {
        let mut screen = Screen::new();
        screen.set(PHONE, View::default());
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Pair));
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Field));
        assert!(!screen.hits.iter().any(|(h, _)| *h == Hit::Cancel));
        assert!(screen.modal_draw.is_none());
        screen.set(
            PHONE,
            View {
                waiting: true,
                adding: true,
                code: vec![line("flankrkey1abc")],
                ..View::default()
            },
        );
        // The modal's Details and Cancel are the only controls: not the
        // field, Pair or the add mode's "<" under it.
        let hits: Vec<Hit> = screen.hits.iter().map(|&(h, _)| h).collect();
        assert_eq!(hits, [Hit::Details, Hit::Cancel]);
        let m = screen.modal_draw.expect("the modal");
        let cancel = screen.hits[1].1;
        assert!(m.card.contains([cancel.x + 1.0, cancel.y + 1.0]), "in it");
        assert_eq!(
            screen.hit(cancel.x + 1.0, cancel.y + 1.0),
            Some(Hit::Cancel)
        );
        // A tap off the card is nothing: a stray touch keeps the pairing.
        assert_eq!(screen.hit(m.card.x - 2.0, m.card.y - 2.0), None);
        let texts = screen.shown_texts();
        assert!(
            texts.contains(&"Confirm on your Mac".to_string()),
            "{texts:?}"
        );
        assert!(texts.contains(&"flankrkey1abc".to_string()), "{texts:?}");
        let details = screen.hits[0].1;
        assert!(details.x + details.w < cancel.x, "side by side");
        let text = |s: &Screen| s.modal.get::<Button>(s.modal_details).unwrap().text.clone();
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
        // It closes at once.
        screen.set(PHONE, View::default());
        assert!(screen.modal_draw.is_none());
        assert!(
            !screen
                .shown_texts()
                .contains(&"Confirm on your Mac".to_string())
        );
    }

    #[test]
    fn a_finished_pairing_shows_a_check_details_and_done() {
        let mut screen = Screen::new();
        let view = |details: bool| View {
            adding: true,
            details,
            code: if details {
                vec![line("m4 at m4.ts.net:7347")]
            } else {
                Vec::new()
            },
            camera: Some(CameraView {
                glyph: Glyph::Check,
                prompt: Some("Paired with m4".into()),
                detail: Some("m4.ts.net:7347 · fingerprint 3f9a 12c4".into()),
                row: Row::Done,
                ..CameraView::default()
            }),
            ..View::default()
        };
        screen.set(PHONE, view(false));
        let hits: Vec<Hit> = screen.hits.iter().map(|&(h, _)| h).collect();
        assert_eq!(hits, [Hit::Back, Hit::Details, Hit::Done], "no Area");
        let texts = screen.shown_texts();
        for t in ["Paired with m4", "Done", "Details"] {
            assert!(texts.contains(&t.to_string()), "{t} in {texts:?}");
        }
        assert!(texts.iter().any(|t| t.contains("fingerprint 3f9a 12c4")));
        assert!(screen.camera_area().is_none(), "no viewfinder");
        let c = screen.camera.unwrap();
        assert!(!c.marks && matches!(c.glyph, Some((Glyph::Check, _))));
        // The check is drawn: a disc and two strokes.
        let mut list = DrawList::new();
        screen.draw(&mut list, &DrawContext::default());
        let lines = list
            .instances
            .iter()
            .filter(|i| matches!(i.kind, libmsdf::drawlist::SdfKind::Line { .. }))
            .count();
        assert_eq!(lines, 2);
        screen.set(PHONE, view(true));
        assert!(
            screen
                .shown_texts()
                .contains(&"m4 at m4.ts.net:7347".to_string())
        );
        let b = screen.router.get::<Button>(screen.details).unwrap();
        assert_eq!(b.text, "Hide details");
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
            code: p.lines(&modal_layout(&PHONE), None, now),
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
        // The daemon holds the request: its words show, large, in the
        // modal, with Details and Cancel.
        p.show(Waiting::new(
            "m4".into(),
            "m4.ts.net:7347".into(),
            Some("abandon ability able about above absent".into()),
            "zoo zone youth wrist wrong yellow".into(),
            &"3f9a12c4".repeat(8),
            Some(60),
            None,
            now,
        ));
        screen.set(PHONE, view(&p));
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Cancel));
        assert!(screen.hits.iter().any(|(h, _)| *h == Hit::Details));
        assert!(!screen.hits.iter().any(|(h, _)| *h == Hit::Pair));
        let shown = |s: &Screen| -> Vec<(String, f32)> {
            s.modal_lines
                .iter()
                .map(|&id| {
                    let md = s.modal.get::<MarkdownView>(id).unwrap();
                    (md.text(), md.px())
                })
                .collect()
        };
        let quiet = shown(&screen);
        let words: Vec<&str> = quiet
            .iter()
            .filter(|(_, px)| *px == WORDS_PT * PHONE.scale)
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(words, ["abandon ability able", "about above absent"]);
        let all = quiet
            .iter()
            .map(|(t, _)| t.replace('\n', " "))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            all.contains("If flankr pair on m4 shows these six words"),
            "{quiet:?}"
        );
        assert!(!all.contains("zoo"), "{quiet:?}");
        // Inside the card.
        let card = screen.modal_draw.unwrap().card;
        for &id in &screen.modal_lines {
            let r = screen.modal.get::<MarkdownView>(id).unwrap().rect();
            assert!(r.x >= card.x && r.x + r.w <= card.x + card.w, "{r:?}");
        }
        // Details: the hostname and the fingerprint, all of it drawn.
        p.toggle_details();
        screen.set(PHONE, view(&p));
        let details: Vec<String> = shown(&screen).into_iter().map(|(t, _)| t).collect();
        assert!(
            details.iter().any(|t| t == "Daemon fingerprint"),
            "{details:?}"
        );
        assert!(details.iter().any(|t| t == "zoo zone youth"));
        assert!(details.iter().any(|t| t == "m4 at m4.ts.net:7347"));
        assert!(details.iter().any(|t| t == "The link expires in 60 s."));
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
