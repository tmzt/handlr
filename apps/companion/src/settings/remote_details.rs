//! A remote's details modal (plan 20261024_02, the 2026-10-28 17:46, 17:56
//! and 17:57 EDT amendments): a tap on a Remotes row, anywhere but its X,
//! opens it, and so does a 2 s press on the remote's card in the drawer
//! (plan 20261028_03, step 8). In the settings style, as the confirm
//! modals: libcontrols' `ModalDialog`, centred on an opaque card, opened
//! and closed at once.
//!
//! From the top:
//!
//! - the remote's shown name ([`RemoteInfo::display_name`]), bold, and its
//!   status: *in use*, *in use · offline*, or *not in use*;
//! - **Nickname**: libcontrols' `EditableLabel` (plan 20261028_03, the
//!   18:48 EDT amendment, item 10): the nickname, or the default name dim
//!   when it has none, with a pencil; a tap on it or its pencil turns it
//!   into a field with a check and an X. The X empties the draft, which is
//!   the default; the check keeps it. libcontrols has no IME hook, so the
//!   app shows the soft keyboard while it is edited and gives it the
//!   keyboard's text ([`EditableLabelState::set_draft`]);
//! - **Hostname**: the daemon's name and its first `host:port`, the address
//!   in the mono face;
//! - when it was paired (date and time, in the phone's time zone), or
//!   *Paired before this list*;
//! - **Fingerprint**, collapsed: a row ([`Disclosure`]) that expands to the
//!   six fingerprint words and the 64-hex BLAKE2s-256 fingerprint in groups
//!   of four, in the mono face;
//! - **Close**, and **Remove** in red, which opens the X's confirm modal.
//!
//! The nickname and the Fingerprint row are not close controls: the screen
//! finds a tap on them by their rects ([`DetailsModal::field`],
//! [`DetailsModal::fingerprint`]), reads what the label did from its
//! state's events, and builds the modal again.

use std::sync::Arc;

use flankr_companion::RemoteInfo;
use libcontrols::{
    Button, ControlAction, ControlPart, DrawContext, DrawControl, EditableLabel,
    EditableLabelOptions, EditableLabelState, EventContext, HostedControl, KeyboardEvent,
    ModalDialog, MouseButton, MouseEvent, NodeId, PanelFrame, Rect,
};
use libmsdf::drawlist::DrawList;

use super::copy;
use super::remote_row::paired_date_time;
use crate::agent::{PAD, line_h};
use crate::markdown_view::{Faces, MarkdownView};
use crate::remotes::md_escape;
use crate::theme::theme;

/// The title's, a line's, a section label's and the field's sizes, in
/// points.
const TITLE_PT: f32 = 20.0;
const TEXT_PT: f32 = 15.0;
const LABEL_PT: f32 = 13.0;
const FIELD_PT: f32 = 17.0;
/// The field's and the Fingerprint row's height, the buttons', and the
/// space between parts, in points.
const FIELD_H: f32 = 44.0;
const BUTTON_H: f32 = 48.0;
const GAP: f32 = 12.0;
/// The nickname's pencil, check and X: each a 44 pt target, the glyph 16
/// pt.
const ICON_W: f32 = 44.0;
const ICON_PT: f32 = 16.0;
/// The hex fingerprint's groups of four on one line.
const HEX_GROUPS_PER_LINE: usize = 4;

/// What the modal shows of a remote, besides its row.
#[derive(Clone, Debug)]
pub struct DetailsState {
    /// The nickname as typed so far (the stored one when opened).
    pub draft: String,
    /// The soft keyboard types into the nickname.
    pub editing: bool,
    /// The Fingerprint section is expanded.
    pub fingerprint_open: bool,
    /// The nickname's `EditableLabel` state, kept across the modal's
    /// rebuilds: its committed text is the stored nickname, its draft
    /// [`DetailsState::draft`] while it is edited.
    pub label: Arc<EditableLabelState>,
}

impl DetailsState {
    /// The details of a remote whose nickname is `nickname` (`""`: none).
    pub fn new(nickname: &str) -> Self {
        DetailsState {
            draft: nickname.to_string(),
            editing: false,
            fingerprint_open: false,
            label: Arc::new(EditableLabelState::new(nickname)),
        }
    }
}

impl Default for DetailsState {
    fn default() -> Self {
        DetailsState::new("")
    }
}

/// The status words: *in use*, *in use · offline*, or *not in use*.
pub fn status(info: &RemoteInfo) -> &'static str {
    match (info.active, info.online) {
        (true, true) => copy::REMOTE_IN_USE.text,
        (true, false) => copy::REMOTE_IN_USE_OFFLINE.text,
        (false, _) => copy::REMOTE_NOT_IN_USE.text,
    }
}

/// The name shown without a nickname: the row's name when it has none,
/// else the base hostname.
pub fn default_name(info: &RemoteInfo) -> String {
    if info.nickname.is_none() {
        info.display_name.clone()
    } else {
        flankr_companion::base_hostname(&info.host)
    }
}

/// The 64 hex digits in groups of four, [`HEX_GROUPS_PER_LINE`] a line.
pub fn hex_lines(hex: &str) -> Vec<String> {
    let groups: Vec<String> = hex
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    groups
        .chunks(HEX_GROUPS_PER_LINE)
        .map(|row| row.join(" "))
        .collect()
}

/// The Markdown of the expanded Fingerprint section: the six words, then
/// the hex, each under its label, the hex in the mono face.
pub fn fingerprint_markdown(info: &RemoteInfo) -> String {
    let mut out = vec![
        copy::REMOTE_DETAILS_FINGERPRINT_WORDS
            .text
            .trim()
            .to_string(),
        md_escape(&info.fingerprint_words),
        copy::REMOTE_DETAILS_FINGERPRINT_HEX.text.trim().to_string(),
    ];
    out.extend(
        hex_lines(&info.fingerprint_hex)
            .into_iter()
            .map(|l| format!("`{l}`")),
    );
    out.join("\n\n")
}

/// The nickname's `EditableLabel` in `rect`, over `state`, at scale `s`:
/// the pencil, check and X as 44 pt targets, the X shown while the draft
/// is not empty, a tap on the text editing it too, in the theme's
/// colours.
pub fn nickname_label(
    rect: Rect,
    state: &DetailsState,
    placeholder: &str,
    s: f32,
) -> EditableLabel {
    let options = EditableLabelOptions::new()
        .button_width(ICON_W * s)
        .icon_size(ICON_PT * s)
        .icon_thickness(1.8 * s)
        .gap(4.0 * s)
        .pencil_color(theme().dim)
        .confirm_color(theme().accent)
        .clear_color(theme().dim)
        .placeholder_color(theme().placeholder)
        .field_bg(theme().input)
        .field_border(theme().band)
        .font_size(FIELD_PT * s)
        .text_color(theme().text)
        .placeholder(placeholder)
        .clear(true)
        .label_edits(true);
    EditableLabel::new("", rect)
        .with_state(Arc::clone(&state.label))
        .with_options(options)
}

/// The Fingerprint row: its label and a chevron, right when collapsed and
/// down when expanded.
#[derive(Clone, Debug, PartialEq)]
pub struct Disclosure {
    pub rect: Rect,
    pub label: String,
    pub open: bool,
    pub scale: f32,
}

impl DrawControl for Disclosure {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = TEXT_PT * s;
        let y = self.rect.y + (self.rect.h - line_h(px)) * 0.5;
        ctx.draw_text(list, &self.label, [self.rect.x, y], px, theme().fg);
        let cx = self.rect.x + self.rect.w - 12.0 * s;
        let cy = self.rect.y + self.rect.h * 0.5;
        let h = 5.0 * s;
        let t = 1.8 * s;
        let c = theme().dim;
        if self.open {
            list.push_line([cx - h, cy - h * 0.5], [cx, cy + h * 0.5], t, c);
            list.push_line([cx, cy + h * 0.5], [cx + h, cy - h * 0.5], t, c);
        } else {
            list.push_line([cx - h * 0.5, cy - h], [cx + h * 0.5, cy], t, c);
            list.push_line([cx + h * 0.5, cy], [cx - h * 0.5, cy + h], t, c);
        }
    }
}

/// It takes a tap anywhere on it as its body; the screen acts on it.
impl HostedControl for Disclosure {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } if self.rect.contains(*pos) => Some(ControlAction::Activated {
                part: ControlPart::Body,
            }),
            _ => None,
        }
    }

    fn handle_keyboard(
        &self,
        _event: &KeyboardEvent,
        _ctx: &EventContext,
    ) -> Option<ControlAction> {
        None
    }
}

/// The modal, and where its controls are.
pub struct DetailsModal {
    pub dialog: ModalDialog,
    pub close: NodeId,
    pub remove: NodeId,
    /// The nickname and the Fingerprint row, on the screen.
    pub field: Rect,
    pub fingerprint: Rect,
    /// The first Markdown view (the title), for links.
    pub text: NodeId,
}

impl DetailsModal {
    /// `info`'s details over `viewport`, at scale `s`, as `state` has them.
    pub fn new(
        viewport: Rect,
        info: &RemoteInfo,
        state: &DetailsState,
        s: f32,
        faces: Option<Arc<Faces>>,
    ) -> Self {
        let pad = 2.0 * PAD * s;
        let w = (viewport.w - 2.0 * PAD * s).min(560.0 * s);
        let inner_w = w - 2.0 * pad;
        let view = |md: &str, pt: f32, color| {
            MarkdownView::new(md, 0.0, 0.0, inner_w, pt * s, color, faces.clone())
        };
        let title = view(
            &format!("**{}**", md_escape(&info.display_name)),
            TITLE_PT,
            theme().fg,
        );
        let state_line = view(status(info), TEXT_PT, theme().dim);
        let nickname_label_md = view(copy::REMOTE_DETAILS_NICKNAME.text, LABEL_PT, theme().dim);
        let host_label = view(copy::REMOTE_DETAILS_HOSTNAME.text, LABEL_PT, theme().dim);
        let host = view(
            &copy::REMOTE_DETAILS_HOST.fill(
                None,
                &[("name", &md_escape(&info.name)), ("address", &info.host)],
            ),
            TEXT_PT,
            theme().text,
        );
        let paired = match info.paired_at {
            Some(at) => {
                let (date, time) = paired_date_time(at);
                copy::REMOTE_DETAILS_PAIRED.fill(None, &[("date", &date), ("time", &time)])
            }
            None => copy::REMOTE_DETAILS_PAIRED_UNDATED.text.to_string(),
        };
        let paired = view(&paired, TEXT_PT, theme().text);
        let print = state
            .fingerprint_open
            .then(|| view(&fingerprint_markdown(info), TEXT_PT, theme().text));

        let small = 4.0 * s;
        let gap = GAP * s;
        let field_h = FIELD_H * s;
        let mut h = pad + title.height() + small + state_line.height();
        h += gap + nickname_label_md.height() + small + field_h;
        h += gap + host_label.height() + small + host.height();
        h += gap + paired.height();
        h += gap + field_h;
        if let Some(p) = &print {
            h += small + p.height();
        }
        h += 2.0 * gap + BUTTON_H * s + pad;
        let h = h.min(viewport.h - 2.0 * PAD * s);

        let mut dialog = ModalDialog::centered(viewport, [w, h], pad).with_frame(PanelFrame {
            bg_color: Some(theme().card),
            border_color: None,
            border_thickness: 0.0,
            corner_radius: Some(14.0 * s),
        });
        let x = viewport.center()[0] - w * 0.5 + pad;
        let top = viewport.center()[1] - h * 0.5;
        let content = dialog.content_mut();
        let mut y = top + pad;
        let mut place = |mut v: MarkdownView, y: &mut f32, after: f32| {
            v.move_to(x, *y);
            *y += v.height() + after;
            content.add(None, v)
        };
        let text = place(title, &mut y, small);
        place(state_line, &mut y, gap);
        place(nickname_label_md, &mut y, small);
        let field = Rect::new(x, y, inner_w, field_h);
        y += field_h + gap;
        place(host_label, &mut y, small);
        place(host, &mut y, gap);
        place(paired, &mut y, gap);
        let fingerprint = Rect::new(x, y, inner_w, field_h);
        y += field_h + small;
        if let Some(p) = print {
            place(p, &mut y, 0.0);
        }
        let placeholder = copy::REMOTE_DETAILS_DEFAULT
            .fill(None, &[("name", &default_name(info))])
            .trim()
            .to_string();
        content.add(None, nickname_label(field, state, &placeholder, s));
        content.add(
            None,
            Disclosure {
                rect: fingerprint,
                label: copy::REMOTE_DETAILS_FINGERPRINT.text.to_string(),
                open: state.fingerprint_open,
                scale: s,
            },
        );
        let by = top + h - pad - BUTTON_H * s;
        let bw = (inner_w - gap) / 2.0;
        let button = |label: &str, bx: f32, bg| {
            Button::new(label, Rect::new(bx, by, bw, BUTTON_H * s))
                .with_bg(bg)
                .with_text_color(theme().fg)
                .with_font_size(17.0 * s)
                .with_corner_radius(10.0 * s)
        };
        let close = content.add(
            None,
            button(copy::REMOTE_DETAILS_CLOSE.text, x, theme().back),
        );
        let remove = content.add(
            None,
            button(copy::REMOVE_REMOTE_ACTION.text, x + bw + gap, theme().deny),
        );
        let dialog = dialog.with_close_control(close).with_close_control(remove);
        DetailsModal {
            dialog,
            close,
            remove,
            field,
            fingerprint,
            text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hex_is_four_lines_of_four_groups() {
        let hex: String = (0..64)
            .map(|i| char::from(b"0123456789abcdef"[i % 16]))
            .collect();
        let lines = hex_lines(&hex);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "0123 4567 89ab cdef");
        assert!(lines.iter().all(|l| l.split(' ').count() == 4));
    }

    #[test]
    fn the_nickname_is_an_editable_label_with_a_clear() {
        let state = DetailsState::new("Office");
        let label = nickname_label(
            Rect::new(0.0, 0.0, 300.0, 88.0),
            &state,
            "m2 (default)",
            2.0,
        );
        assert_eq!(label.text(), "Office");
        assert!(!label.is_editing());
        assert!(label.options.clear && label.options.label_edits);
        // Edited, then cleared by the X: the placeholder, the default.
        assert!(state.label.begin_edit());
        assert!(label.clear_shown());
        assert!(state.label.clear());
        assert!(label.shows_placeholder());
        assert_eq!(label.shown_text(), "m2 (default)");
        assert!(!label.clear_shown(), "no X on an empty draft");
        assert_eq!(state.label.confirm().as_deref(), Some(""));
    }
}
