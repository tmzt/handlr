//! The built-in preview (docs/design/20261028_05_BINARY_FILES_PLAN.md,
//! decisions 10.1-10.3, as the 23:00 and 23:48 EDT amendments have them):
//! a full-screen modal over the agent view showing a file fetched through
//! `files` or a `git` output through `git`.
//!
//! A 56 pt title bar: ✕ at the left, the file's name (middle-ellipsised)
//! over its folder and size, dim (for `git`, the request over the session's
//! project, and "cut at 8 MB" beside the size of a cut output). The share
//! icon is hidden: the libmobile share-and-view plan it needs is not built.
//! While the transfer runs the body shows "Loading…" and a progress bar; a
//! refusal shows its code's line and **Try again**.
//!
//! The bodies: Markdown (`MarkdownView` on flankr-markdown, at the
//! transcript's text size; over 512 KiB, plain text with a note), plain
//! text and diffs (monospaced at 12 pt, wrapped with a dim ↩, at most
//! 20 000 lines; a diff in the transcript's diff colours), each in a
//! libcontrols `ScrollPane` with its `Scrollbar`, labelled per the 23:00
//! EDT amendment ([`text`]); and images ([`image`], drawn by [`gpu`]),
//! which pan and zoom and have no scrollbar.
//!
//! The app opens it ([`Preview::open`]) with the [`Input`] it gives back,
//! feeds it the client's transfer events ([`Preview::on_event`]), its
//! touches ([`Preview::touch`]) and the clock ([`Preview::tick`]), and
//! draws it last ([`Preview::draw`], then the image with
//! [`Preview::image`]).

pub mod gpu;
pub mod image;
pub mod links;
pub mod text;

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use flankr_companion::{Event, Input};
use libcontrols::{
    BookmarkText, ControlAction, ControlPart, DrawContext, DrawControl, EventContext,
    HostedControl, KeyboardEvent, MouseButton, MouseEvent, NodeId, Rect, Router, ScrollAxis,
    ScrollPane, ScrollState, ScrollbarOptions, ScrollbarVisibility, Section, SectionId,
    SectionedContent,
};
use libflankr_blob::{GIT_CUT_MARKER, NS_FILES, NS_GIT};
use libmorph::TouchPhase;
use libmsdf::drawlist::{AtlasRef, DrawList};
use libmsdf::font::GlyphStyle;

pub use links::{Linker, Target};

use crate::agent::{CELL_EM, PAD, fill, line_h, term_h};
use crate::markdown_view::{Faces, MarkdownView, measure};
use crate::settings::copy;
use crate::theme::theme;
use crate::ui::Layout;

/// The title bar's height, its ✕'s target, in points.
const HEADER: f32 = 56.0;
const CLOSE: f32 = 44.0;
const TITLE_PT: f32 = 16.0;
const SUB_PT: f32 = 12.0;
/// The Markdown body's type, the transcript's; the text body's, 12 pt.
const MARKDOWN_PT: f32 = 15.0;
const TEXT_PT: f32 = 12.0;
const NOTE_PT: f32 = 14.0;
/// Over this, a Markdown file is shown as plain text.
pub const MARKDOWN_MAX: usize = 512 << 10;
/// The longest a double tap's two taps are apart.
const DOUBLE_TAP: Duration = Duration::from_millis(300);

/// What a touch on the preview asks the app for.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewHit {
    /// The ✕: it is closed.
    Closed,
    /// Send this (Try again, or a Markdown link to another `.md` file).
    Send(Input),
    /// Open this `http(s)` link.
    Link(String),
}

/// The body as it stands.
enum Body {
    Loading {
        done: u64,
        size: u64,
    },
    Failed(String),
    /// A note in place of the body (an image not shown here).
    Note(String),
    /// Text or Markdown in the router's pane.
    Scroll,
    /// An image: decoding on a worker, then shown.
    Decoding(mpsc::Receiver<Result<image::Rgba, image::NoImage>>),
    Image(image::Rgba, image::View),
}

/// What is shown in a scroll body.
#[derive(Clone, Debug, PartialEq)]
enum Content {
    Markdown(String),
    Text {
        text: String,
        diff: bool,
        note: Option<&'static str>,
    },
}

/// The open preview.
struct Open {
    target: Target,
    session: String,
    project: Option<String>,
    /// The transfer, once [`Event::TransferStarted`] named it.
    id: Option<u32>,
    title: String,
    sub: String,
    body: Body,
    content: Option<Content>,
    /// The folder of the file shown, for its Markdown's relative links.
    folder: String,
    router: Router,
    pane: Option<NodeId>,
    markdown: Option<NodeId>,
    scroll: Arc<ScrollState>,
    /// The layout the router was built for.
    built: Option<(f32, f32, f32)>,
    /// Its scrollbar is shown always, past three screens.
    always: bool,
    generation: u64,
}

/// The preview: closed, or one file.
#[derive(Default)]
pub struct Preview {
    open: Option<Open>,
    dirty: bool,
    generations: u64,
    /// The touch held: its id, where it began, and whether on the ✕.
    touch: Option<(u64, [f32; 2], bool)>,
    /// A second finger on an image, for the pinch: its id and place.
    second: Option<(u64, [f32; 2])>,
    /// Where each finger of the pinch is now, and the zoom at its start.
    pinch: Option<(f32, f32)>,
    last: [f32; 2],
    last_tap: Option<(Duration, [f32; 2])>,
    faces: Option<Arc<Faces>>,
    /// The app's *Scrollbar* setting.
    visibility: ScrollbarVisibility,
}

impl Preview {
    pub fn new(faces: Option<Arc<Faces>>) -> Preview {
        Preview {
            faces,
            ..Preview::default()
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The scrollbar's visibility, from the app's *Scrollbar* setting.
    pub fn set_visibility(&mut self, v: crate::agent::ScrollbarVisibility) {
        let v = match v {
            crate::agent::ScrollbarVisibility::Disabled => ScrollbarVisibility::None,
            crate::agent::ScrollbarVisibility::Hidden => ScrollbarVisibility::OnScroll,
            crate::agent::ScrollbarVisibility::Always => ScrollbarVisibility::Always,
        };
        if v != self.visibility {
            self.visibility = v;
            if let Some(o) = &mut self.open {
                o.built = None;
            }
        }
    }

    /// Opens `target` of `session` (whose project is `project`): the
    /// request to send for it.
    pub fn open(&mut self, target: Target, session: &str, project: Option<&str>) -> Input {
        self.generations += 1;
        let (title, folder) = match &target {
            Target::File(p) => {
                let (folder, name) = p.rsplit_once('/').unwrap_or(("", p));
                (name.to_string(), folder.to_string())
            }
            Target::Git(r) => (r.clone(), String::new()),
        };
        let sub = match &target {
            Target::File(_) => folder.clone(),
            Target::Git(_) => project.unwrap_or_default().to_string(),
        };
        log::info!("preview: {}", target.url());
        self.open = Some(Open {
            target: target.clone(),
            session: session.to_string(),
            project: project.map(str::to_string),
            id: None,
            title,
            sub,
            body: Body::Loading { done: 0, size: 0 },
            content: None,
            folder,
            router: Router::new(),
            pane: None,
            markdown: None,
            scroll: ScrollState::new(),
            built: None,
            always: false,
            generation: self.generations,
        });
        self.touch = None;
        self.second = None;
        self.pinch = None;
        self.dirty = true;
        request(&target, session)
    }

    /// Closes it.
    pub fn close(&mut self) {
        if self.open.take().is_some() {
            self.dirty = true;
        }
    }

    /// The transfer to cancel when it closes before its file came.
    pub fn pending_transfer(&self) -> Option<u32> {
        let o = self.open.as_ref()?;
        matches!(o.body, Body::Loading { .. })
            .then_some(o.id)
            .flatten()
    }

    /// It changed since last asked.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// A client event: the transfer's start, progress, file or refusal.
    pub fn on_event(&mut self, event: &Event) {
        let Some(o) = self.open.as_mut() else {
            return;
        };
        match event {
            Event::TransferStarted {
                id,
                ns,
                session,
                request: r,
            } if o.id.is_none() && *session == o.session && wants(&o.target, ns, r) => {
                o.id = Some(*id);
            }
            Event::TransferProgress { id, done, size } if o.id == Some(*id) => {
                if let Body::Loading { .. } = o.body {
                    o.body = Body::Loading {
                        done: *done,
                        size: *size,
                    };
                    self.dirty = true;
                }
            }
            Event::TransferFailed { id, line, .. } if o.id == Some(*id) => {
                o.body = Body::Failed(line.clone().unwrap_or_default());
                self.dirty = true;
            }
            Event::FileReady {
                id,
                path,
                mime,
                name,
                size,
            } if o.id == Some(*id) => {
                self.ready(path.clone(), mime, name, *size);
            }
            _ => {}
        }
    }

    /// The file came: its body.
    fn ready(&mut self, path: PathBuf, mime: &str, name: &str, size: u64) {
        let Some(o) = self.open.as_mut() else {
            return;
        };
        self.dirty = true;
        let mut sub = o.sub.clone();
        let size_text = format_size(size);
        sub = if sub.is_empty() {
            size_text
        } else {
            format!("{sub} \u{00B7} {size_text}")
        };
        if let Target::File(_) = o.target {
            let (folder, file) = name.rsplit_once('/').unwrap_or(("", name));
            o.title = file.to_string();
            o.folder = folder.to_string();
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("preview: reading {}: {e}", path.display());
                o.body = Body::Failed(copy::PREVIEW_UNREADABLE.text.to_string());
                return;
            }
        };
        if mime.starts_with("image/") {
            o.sub = sub;
            let (tx, rx) = mpsc::channel();
            let mime = mime.to_string();
            let spawned = std::thread::Builder::new()
                .name("preview decode".into())
                .spawn(move || {
                    let _ = tx.send(image::decode(&bytes, &mime));
                });
            o.body = match spawned {
                Ok(_) => Body::Decoding(rx),
                Err(_) => Body::Note(copy::PREVIEW_IMAGE_DAMAGED.text.to_string()),
            };
            return;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            o.body = Body::Failed(copy::PREVIEW_UNREADABLE.text.to_string());
            return;
        };
        let content = match mime {
            "text/markdown" if text.len() <= MARKDOWN_MAX => Content::Markdown(text),
            "text/markdown" => Content::Text {
                text,
                diff: false,
                note: Some(copy::PREVIEW_MARKDOWN_AS_TEXT.text),
            },
            "text/x-diff" => {
                if text.ends_with(GIT_CUT_MARKER) {
                    sub = format!("{sub} \u{00B7} {}", copy::PREVIEW_GIT_CUT.text);
                }
                Content::Text {
                    text,
                    diff: true,
                    note: None,
                }
            }
            _ => Content::Text {
                text,
                diff: false,
                note: None,
            },
        };
        o.sub = sub;
        o.content = Some(content);
        o.body = Body::Scroll;
        o.built = None;
    }

    /// The clock: a decoded image arrives; the pane's fling and its
    /// scrollbar's fade go on. Whether it must be drawn again.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let Some(o) = self.open.as_mut() else {
            return false;
        };
        if let Body::Decoding(rx) = &o.body
            && let Ok(result) = rx.try_recv()
        {
            o.body = match result {
                Ok(img) => {
                    let view = image::View::new(img.width, img.height);
                    o.built = None;
                    Body::Image(img, view)
                }
                Err(why) => Body::Note(
                    match why {
                        image::NoImage::TooLarge => copy::PREVIEW_IMAGE_TOO_LARGE.text,
                        image::NoImage::Damaged => copy::PREVIEW_IMAGE_DAMAGED.text,
                        image::NoImage::Unsupported if cfg!(target_os = "android") => {
                            copy::PREVIEW_IMAGE_NEWER_APP.text
                        }
                        image::NoImage::Unsupported => copy::PREVIEW_IMAGE_PNG_ONLY.text,
                    }
                    .to_string(),
                ),
            };
            self.dirty = true;
        }
        let moved = matches!(o.body, Body::Scroll) && o.router.advance(dt.as_secs_f32());
        if moved {
            self.dirty = true;
        }
        moved
    }

    /// Where its parts are on `layout`.
    fn frame(layout: &Layout) -> Frame {
        let s = scale(layout);
        let top = layout.top;
        let header = Rect::new(0.0, top, layout.w, HEADER * s);
        let close = Rect::new(
            4.0 * s,
            top + (HEADER - CLOSE) * 0.5 * s,
            CLOSE * s,
            CLOSE * s,
        );
        let body = Rect::new(
            0.0,
            top + HEADER * s,
            layout.w,
            (layout.h - top - HEADER * s - layout.bottom).max(0.0),
        );
        Frame {
            header,
            close,
            body,
            s,
        }
    }

    /// The Try again button's rect, under the refusal's line.
    fn retry_rect(f: &Frame) -> Rect {
        let s = f.s;
        let w = 160.0 * s;
        let h = 44.0 * s;
        let [cx, cy] = f.body.center();
        Rect::new(cx - w * 0.5, cy + 12.0 * s, w, h)
    }

    /// Builds the scroll body's router for `layout`, when it is not built
    /// for it.
    fn build(&mut self, layout: &Layout) {
        let visibility = self.visibility;
        let faces = self.faces.clone();
        let Some(o) = self.open.as_mut() else {
            return;
        };
        let key = (layout.w, layout.h, layout.scale);
        if o.built == Some(key) {
            return;
        }
        o.built = Some(key);
        if let Body::Image(img, view) = &mut o.body {
            *view = image::View::new(img.width, img.height);
            view.clamp(Self::frame(layout).body);
            return;
        }
        let Some(content) = o.content.clone() else {
            return;
        };
        let f = Self::frame(layout);
        let s = f.s;
        let pad = PAD * s;
        let mut router = Router::new();
        let inner_w = (f.body.w - 2.0 * pad).max(1.0);
        let (control_h, labels, always, md) = match &content {
            Content::Markdown(md) => {
                let px = MARKDOWN_PT * s;
                let view = MarkdownView::new(
                    md,
                    f.body.x + pad,
                    f.body.y + pad,
                    inner_w,
                    px,
                    theme().fg,
                    faces.clone(),
                );
                let ys = heading_offsets(md, inner_w, px, faces.as_deref());
                (view.height(), ys, false, Some(view))
            }
            Content::Text { text, diff, note } => {
                let body = TextBody::new(
                    text,
                    *diff,
                    *note,
                    Rect::new(f.body.x + pad, f.body.y + pad, inner_w, 0.0),
                    s,
                    faces.clone(),
                    Arc::clone(&o.scroll),
                    f.body,
                );
                let labels: Vec<(f32, String)> = if *diff {
                    text::diff_labels(text)
                        .into_iter()
                        .filter_map(|l| body.y_of(l.line).map(|y| (y, l.text)))
                        .collect()
                } else {
                    Vec::new()
                };
                let h = body.rect.h;
                let always = text::always_shown(h + 2.0 * pad, f.body.h);
                o.always = always;
                let pane = add_pane(
                    &mut router,
                    f.body,
                    &o.scroll,
                    visibility,
                    always,
                    h + 2.0 * pad,
                    &labels,
                );
                router.add(Some(pane), body);
                o.pane = Some(pane);
                o.markdown = None;
                o.router = router;
                return;
            }
        };
        let content_h = control_h + 2.0 * pad;
        o.always = always;
        let pane = add_pane(
            &mut router,
            f.body,
            &o.scroll,
            visibility,
            always,
            content_h,
            &labels,
        );
        o.markdown = md.map(|v| router.add(Some(pane), v));
        o.pane = Some(pane);
        o.router = router;
    }

    /// A touch while it is open: it takes every touch.
    pub fn touch(
        &mut self,
        layout: &Layout,
        (id, phase, x, y): (u64, TouchPhase, f32, f32),
        now: Duration,
    ) -> Option<PreviewHit> {
        self.build(layout);
        let f = Self::frame(layout);
        let pos = [x, y];
        let image = self
            .open
            .as_ref()
            .is_some_and(|o| matches!(o.body, Body::Image(..)));
        if image && self.image_touch(&f, id, phase, pos, now) {
            return None;
        }
        let left = MouseButton::Left;
        match phase {
            TouchPhase::Started => {
                if self.touch.is_some() {
                    return None;
                }
                let on_close = f.close.contains(pos);
                self.touch = Some((id, pos, on_close));
                if !on_close && f.body.contains(pos) {
                    self.router_mut()?
                        .deliver_mouse(MouseEvent::Down { pos, button: left });
                }
                self.dirty = true;
                None
            }
            TouchPhase::Moved => {
                let (held, start, on_close) = self.touch?;
                if held != id || on_close {
                    return None;
                }
                self.router_mut()?.deliver_mouse(MouseEvent::Drag {
                    start_pos: start,
                    current_pos: pos,
                    delta: [0.0, 0.0],
                    button: left,
                });
                self.dirty = true;
                None
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let (held, start, on_close) = self.touch?;
                if held != id {
                    return None;
                }
                self.touch = None;
                self.dirty = true;
                let ended = phase == TouchPhase::Ended;
                if on_close {
                    if ended && f.close.contains(pos) {
                        self.close();
                        return Some(PreviewHit::Closed);
                    }
                    return None;
                }
                let failed = self
                    .open
                    .as_ref()
                    .is_some_and(|o| matches!(o.body, Body::Failed(_)));
                if failed {
                    let r = Self::retry_rect(&f);
                    if ended && r.contains(pos) && r.contains(start) {
                        return self.retry().map(PreviewHit::Send);
                    }
                    return None;
                }
                let router = self.router_mut()?;
                router.deliver_mouse(MouseEvent::Up { pos, button: left });
                if !ended {
                    return None;
                }
                let response = router.deliver_mouse(MouseEvent::Click { pos, button: left });
                let Some(ControlAction::Activated {
                    part: ControlPart::Custom(i),
                }) = response.action
                else {
                    return None;
                };
                self.markdown_link(i as usize)
            }
        }
    }

    fn router_mut(&mut self) -> Option<&mut Router> {
        self.open
            .as_mut()
            .filter(|o| matches!(o.body, Body::Scroll))
            .map(|o| &mut o.router)
    }

    /// Try again: the same request.
    fn retry(&mut self) -> Option<Input> {
        let o = self.open.as_mut()?;
        o.id = None;
        o.body = Body::Loading { done: 0, size: 0 };
        self.dirty = true;
        Some(request(&o.target, &o.session))
    }

    /// A tap on the Markdown's link `i`: `http(s)` opens; a relative link
    /// to a `.md` file opens another preview; others do nothing.
    fn markdown_link(&mut self, i: usize) -> Option<PreviewHit> {
        let o = self.open.as_ref()?;
        let id = o.markdown?;
        let url = o.router.get::<MarkdownView>(id)?.links().get(i)?.clone();
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(PreviewHit::Link(url));
        }
        let path = resolve(&o.folder, &url)?;
        let (session, project) = (o.session.clone(), o.project.clone());
        Some(PreviewHit::Send(self.open(
            Target::File(path),
            &session,
            project.as_deref(),
        )))
    }

    /// A touch on an image: pan with one finger, pinch with two, a double
    /// tap toggles fit and 1:1. Whether it was the image's.
    fn image_touch(
        &mut self,
        f: &Frame,
        id: u64,
        phase: TouchPhase,
        pos: [f32; 2],
        now: Duration,
    ) -> bool {
        let first = self.touch.map(|t| t.0);
        let Some(Open {
            body: Body::Image(_, view),
            ..
        }) = self.open.as_mut()
        else {
            return false;
        };
        let window = f.body;
        match phase {
            TouchPhase::Started if first.is_some() && first != Some(id) => {
                if self.second.is_none() {
                    self.second = Some((id, pos));
                    let d = dist(self.last, pos);
                    self.pinch = Some((d.max(1.0), view.zoom));
                }
                true
            }
            TouchPhase::Started if !f.close.contains(pos) && window.contains(pos) => {
                self.touch = Some((id, pos, false));
                self.last = pos;
                if let Some((at, p)) = self.last_tap
                    && now.saturating_sub(at) <= DOUBLE_TAP
                    && dist(p, pos) < 40.0 * f.s
                {
                    view.toggle(pos, window);
                    self.last_tap = None;
                } else {
                    self.last_tap = Some((now, pos));
                }
                self.dirty = true;
                true
            }
            TouchPhase::Moved if first == Some(id) || self.second.is_some_and(|s| s.0 == id) => {
                if let (Some((d0, z0)), Some((sid, sp))) = (self.pinch, self.second) {
                    let (a, b) = if sid == id {
                        (self.last, pos)
                    } else {
                        (pos, sp)
                    };
                    if sid == id {
                        self.second = Some((sid, pos));
                    } else {
                        self.last = pos;
                    }
                    let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                    view.zoom_about(z0 * dist(a, b) / d0, mid, window);
                } else {
                    view.pan_by([pos[0] - self.last[0], pos[1] - self.last[1]], window);
                    self.last = pos;
                }
                self.dirty = true;
                true
            }
            TouchPhase::Ended | TouchPhase::Cancelled if self.second.is_some_and(|s| s.0 == id) => {
                self.second = None;
                self.pinch = None;
                true
            }
            TouchPhase::Ended | TouchPhase::Cancelled if first == Some(id) => {
                self.touch = None;
                self.second = None;
                self.pinch = None;
                true
            }
            _ => false,
        }
    }

    /// Draws it over the screen, while open.
    pub fn draw(&mut self, layout: &Layout, ctx: &DrawContext, list: &mut DrawList) {
        self.build(layout);
        let Some(o) = self.open.as_ref() else {
            return;
        };
        let f = Self::frame(layout);
        let s = f.s;
        let t = theme();
        fill(list, Rect::new(0.0, 0.0, layout.w, layout.h), t.bg, 0.0);
        fill(list, f.header, t.header, 0.0);
        // The ✕.
        let [cx, cy] = f.close.center();
        let h = 7.0 * s;
        list.push_line([cx - h, cy - h], [cx + h, cy + h], 2.0 * s, t.text);
        list.push_line([cx - h, cy + h], [cx + h, cy - h], 2.0 * s, t.text);
        let x = f.close.x + f.close.w + 8.0 * s;
        let room = (layout.w - x - PAD * s).max(0.0);
        let title_px = TITLE_PT * s;
        let sub_px = SUB_PT * s;
        let block = line_h(title_px) + line_h(sub_px);
        let y = f.header.y + (f.header.h - block) * 0.5;
        let title = cut_middle(ctx, &o.title, title_px, room);
        ctx.draw_text(list, &title, [x, y], title_px, t.text);
        let sub = cut_middle(ctx, &o.sub, sub_px, room);
        ctx.draw_text(list, &sub, [x, y + line_h(title_px)], sub_px, t.dim);
        let note_px = NOTE_PT * s;
        let centred = |list: &mut DrawList, text: &str, y: f32, color: [f32; 4]| {
            let w = ctx.text_advance(text, note_px);
            ctx.draw_text(
                list,
                text,
                [f.body.center()[0] - w * 0.5, y],
                note_px,
                color,
            );
        };
        let [_, by] = f.body.center();
        match &o.body {
            Body::Loading { done, size } => {
                centred(
                    list,
                    copy::PREVIEW_LOADING.text,
                    by - line_h(note_px) - 8.0 * s,
                    t.dim,
                );
                let w = (f.body.w * 0.6).min(320.0 * s);
                let bar = Rect::new(f.body.center()[0] - w * 0.5, by, w, 4.0 * s);
                fill(list, bar, t.band, 2.0 * s);
                if *size > 0 {
                    let part = (*done as f32 / *size as f32).clamp(0.0, 1.0);
                    fill(
                        list,
                        Rect::new(bar.x, bar.y, bar.w * part, bar.h),
                        t.accent,
                        2.0 * s,
                    );
                }
            }
            Body::Failed(line) => {
                centred(list, line, by - line_h(note_px), t.text);
                let r = Self::retry_rect(&f);
                fill(list, r, t.card, 8.0 * s);
                let label = copy::PREVIEW_TRY_AGAIN.text;
                let w = ctx.text_advance(label, note_px);
                ctx.draw_text(
                    list,
                    label,
                    [
                        r.center()[0] - w * 0.5,
                        r.center()[1] - line_h(note_px) * 0.5,
                    ],
                    note_px,
                    t.accent,
                );
            }
            Body::Note(line) => centred(list, line, by - line_h(note_px) * 0.5, t.dim),
            Body::Decoding(_) => centred(list, copy::PREVIEW_LOADING.text, by, t.dim),
            Body::Scroll => {
                list.push_clip(f.body.pos(), f.body.size());
                o.router.render(list, ctx);
                list.push_clip_end();
            }
            // Drawn by the GPU after the list (`image`).
            Body::Image(..) => {}
        }
    }

    /// The image to draw after the list: its generation, pixels, rect and
    /// the body it is clipped to.
    pub fn image(&mut self, layout: &Layout) -> Option<(u64, &image::Rgba, Rect, Rect)> {
        self.build(layout);
        let f = Self::frame(layout);
        let o = self.open.as_ref()?;
        match &o.body {
            Body::Image(img, view) => Some((o.generation, img, view.rect(f.body), f.body)),
            _ => None,
        }
    }

    /// What the body shows, for tests: `markdown`, `text`, `diff`, an
    /// image's size, a note, a failure or loading.
    #[cfg(test)]
    pub fn shows(&self) -> String {
        match self.open.as_ref().map(|o| (&o.body, &o.content)) {
            None => "closed".into(),
            Some((Body::Scroll, Some(Content::Markdown(_)))) => "markdown".into(),
            Some((Body::Scroll, Some(Content::Text { diff: true, .. }))) => "diff".into(),
            Some((Body::Scroll, _)) => "text".into(),
            Some((Body::Image(img, _), _)) => format!("image {}x{}", img.width, img.height),
            Some((Body::Note(n), _)) => format!("note {n}"),
            Some((Body::Failed(l), _)) => format!("failed {l}"),
            Some((Body::Decoding(_), _)) => "decoding".into(),
            Some((Body::Loading { .. }, _)) => "loading".into(),
        }
    }

    /// The scroll body's scrollbar: its bookmarks' texts and offsets, and
    /// whether it is shown always.
    #[cfg(test)]
    pub fn bookmarks(&mut self, layout: &Layout) -> (Vec<(f32, String)>, bool) {
        self.build(layout);
        let Some(o) = self.open.as_ref() else {
            return (Vec::new(), false);
        };
        let Some(pane) = o.pane.and_then(|p| o.router.get::<ScrollPane>(p)) else {
            return (Vec::new(), false);
        };
        let Some(sections) = pane.sections() else {
            return (Vec::new(), false);
        };
        let marks = sections
            .bookmarks()
            .into_iter()
            .map(|(id, text)| {
                let t = match text {
                    BookmarkText::Text(t) | BookmarkText::Markdown(t) => t,
                    _ => String::new(),
                };
                (sections.offset_of(id).unwrap_or(-1.0), t)
            })
            .collect();
        (marks, o.always)
    }

    /// The scroll body's text rows, for tests.
    #[cfg(test)]
    pub fn rows(&mut self, layout: &Layout) -> Vec<text::TextRow> {
        self.build(layout);
        let Some(o) = self.open.as_ref() else {
            return Vec::new();
        };
        let Some(pane) = o.pane else {
            return Vec::new();
        };
        o.router
            .children(Some(pane))
            .find_map(|id| o.router.get::<TextBody>(id).map(|b| b.rows.clone()))
            .unwrap_or_default()
    }
}

/// The request a target asks for.
fn request(target: &Target, session: &str) -> Input {
    match target {
        Target::File(path) => Input::RequestFile {
            session: session.to_string(),
            path: path.clone(),
        },
        Target::Git(request) => Input::RequestGit {
            session: session.to_string(),
            request: request.clone(),
        },
    }
}

/// Whether a transfer of `ns` asking for `request` is `target`'s.
fn wants(target: &Target, ns: &str, request: &str) -> bool {
    match target {
        Target::File(p) => ns == NS_FILES && p == request,
        Target::Git(r) => ns == NS_GIT && r == request,
    }
}

/// A relative Markdown link to a `.md` file, against `folder`; `None` for
/// any other link. `..` goes up, never above the project's root.
pub fn resolve(folder: &str, link: &str) -> Option<String> {
    let link = link.split('#').next()?;
    if link.contains(':') || link.starts_with('/') || !link.ends_with(".md") {
        return None;
    }
    let mut parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// `bytes` as the subtitle shows them.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1 << 20 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

/// `text` cut in its middle with "…" to fit `room` at `px`.
fn cut_middle(ctx: &DrawContext, text: &str, px: f32, room: f32) -> String {
    if ctx.text_advance(text, px) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = chars.len();
    while keep > 1 {
        keep -= 1;
        let head = keep / 2;
        let tail = keep - head;
        let t: String = chars[..head]
            .iter()
            .chain(std::iter::once(&'\u{2026}'))
            .chain(chars[chars.len() - tail..].iter())
            .collect();
        if ctx.text_advance(&t, px) <= room {
            return t;
        }
    }
    "\u{2026}".to_string()
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn scale(layout: &Layout) -> f32 {
    if layout.scale > 0.0 {
        layout.scale
    } else {
        1.0
    }
}

/// Where the preview's parts are.
struct Frame {
    header: Rect,
    close: Rect,
    body: Rect,
    s: f32,
}

/// The body's pane: a vertical `ScrollPane` with the scrollbar, its
/// sections an index whose bookmarks are `labels` (content offsets).
fn add_pane(
    router: &mut Router,
    window: Rect,
    scroll: &Arc<ScrollState>,
    visibility: ScrollbarVisibility,
    always: bool,
    content_h: f32,
    labels: &[(f32, String)],
) -> NodeId {
    let visibility = if always {
        ScrollbarVisibility::Always
    } else {
        visibility
    };
    let sections = SectionedContent::new();
    // Sections from each label's offset to the next, the first from 0.
    let mut tops: Vec<f32> = vec![0.0];
    tops.extend(labels.iter().map(|(y, _)| *y).filter(|y| *y > 0.0));
    tops.dedup();
    let list: Vec<Section> = tops
        .iter()
        .enumerate()
        .map(|(i, top)| {
            let end = tops.get(i + 1).copied().unwrap_or(content_h);
            Section::new(SectionId(i as u64), (end - top).max(0.0))
        })
        .collect();
    sections.set_sections(list);
    for (y, text) in labels {
        if let Some(i) = tops.iter().position(|t| (*t - y.max(0.0)).abs() < 0.5) {
            sections.add_bookmark(SectionId(i as u64), BookmarkText::Text(text.clone()));
        }
    }
    let pane = ScrollPane::new(window, ScrollAxis::Vertical)
        .with_state(Arc::clone(scroll))
        .with_scrollbar(ScrollbarOptions::new().visibility(visibility))
        .with_sections(sections)
        .with_content_size([window.w, content_h]);
    router.add(None, pane)
}

/// The content offsets of a Markdown file's `#` and `##` headings, laid out
/// `width` wide at `px` as its `MarkdownView` lays it out: each heading's
/// label at the first laid line after the last that reads as its text.
fn heading_offsets(md: &str, width: f32, px: f32, faces: Option<&Faces>) -> Vec<(f32, String)> {
    let labels = text::markdown_labels(md);
    if labels.is_empty() {
        return Vec::new();
    }
    let laid = flankr_markdown::lay_out(md, width, &|t, style| measure(faces, t, style, px));
    let lh = line_h(px);
    let pad = PAD * (px / (MARKDOWN_PT));
    let mut ys = Vec::new();
    let mut y = 0.0;
    for line in &laid.lines {
        y += line.gap_before * lh;
        let text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
        ys.push((y, text));
        y += lh;
    }
    let mut out = Vec::new();
    let mut from = 0;
    for l in labels {
        let want = l.text.trim_end_matches('\u{2026}');
        if let Some(i) = ys[from..]
            .iter()
            .position(|(_, t)| t.trim().starts_with(want.trim()))
        {
            let (ly, _) = &ys[from + i];
            out.push((*ly + pad, l.text));
            from += i + 1;
        }
    }
    out
}

/// A plain text or diff body: its rows, drawn monospaced, those in the
/// pane's window only.
pub struct TextBody {
    rows: Vec<text::TextRow>,
    note: Option<&'static str>,
    cut: bool,
    rect: Rect,
    px: f32,
    row_h: f32,
    faces: Option<Arc<Faces>>,
    scroll: Arc<ScrollState>,
    window: Rect,
}

impl TextBody {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source: &str,
        diff: bool,
        note: Option<&'static str>,
        at: Rect,
        s: f32,
        faces: Option<Arc<Faces>>,
        scroll: Arc<ScrollState>,
        window: Rect,
    ) -> TextBody {
        let px = TEXT_PT * s;
        let cell = faces.as_ref().map_or(CELL_EM, |f| f.code.shaper.cell_em());
        let cols = (at.w / (px * cell)).floor().max(8.0) as usize;
        let (rows, cut) = text::rows(source, cols, diff);
        let row_h = term_h(px);
        let extra = usize::from(note.is_some()) + usize::from(cut);
        let h = (rows.len() + extra) as f32 * row_h;
        TextBody {
            rows,
            note,
            cut,
            rect: Rect::new(at.x, at.y, at.w, h),
            px,
            row_h,
            faces,
            scroll,
            window,
        }
    }

    /// The content offset of source line `line`'s first row, from the
    /// pane's top.
    fn y_of(&self, line: usize) -> Option<f32> {
        let first = self.rows.iter().position(|r| r.line == line)?;
        let extra = usize::from(self.note.is_some());
        Some((first + extra) as f32 * self.row_h + (self.rect.y - self.window.y))
            .map(|y| if line == 0 { 0.0 } else { y })
    }

    fn draw_mono(
        &self,
        list: &mut DrawList,
        ctx: &DrawContext,
        text: &str,
        pos: [f32; 2],
        color: [f32; 4],
    ) {
        match &self.faces {
            Some(f) => {
                let c = &f.code;
                let line = c.shaper.line(text, GlyphStyle::Regular);
                let t = AtlasRef {
                    id: c.text.0,
                    atlas: &c.text.1,
                };
                let sym = AtlasRef {
                    id: c.symbols.0,
                    atlas: &c.symbols.1,
                };
                list.push_terminal_line(&line, t, sym, pos, self.px, f.px_range, color);
            }
            None => {
                ctx.draw_text(list, text, pos, self.px, color);
            }
        }
    }
}

impl DrawControl for TextBody {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let t = theme();
        let offset = self.scroll.offset()[1];
        // The rows in the window, and one either side.
        let top = self.window.y + offset - self.rect.y;
        let mut lines: Vec<(String, [f32; 4], bool)> = Vec::new();
        if let Some(n) = self.note {
            lines.push((n.to_string(), t.dim, false));
        }
        let skip = (top / self.row_h).floor().max(0.0) as usize;
        let shown = (self.window.h / self.row_h).ceil() as usize + 2;
        let note_rows = lines.len();
        let all = note_rows + self.rows.len() + usize::from(self.cut);
        #[allow(clippy::needless_range_loop)]
        for i in skip.saturating_sub(1)..(skip + shown).min(all) {
            let y = self.rect.y + i as f32 * self.row_h;
            let (text, color, wraps) = if let Some(line) = lines.get(i) {
                line.clone()
            } else if let Some(r) = self.rows.get(i - note_rows) {
                (r.text.clone(), r.color, r.wraps)
            } else {
                (copy::PREVIEW_CUT_LINES.text.to_string(), t.dim, false)
            };
            if i < note_rows || i >= note_rows + self.rows.len() {
                ctx.draw_text(
                    list,
                    &text,
                    [self.rect.x, y],
                    NOTE_PT / TEXT_PT * self.px,
                    color,
                );
                continue;
            }
            self.draw_mono(list, ctx, &text, [self.rect.x, y], color);
            if wraps {
                let x = self.rect.x + self.rect.w - self.px * 0.8;
                self.draw_mono(list, ctx, "\u{21A9}", [x, y], t.dim);
            }
        }
    }
}

/// It takes no taps of its own: drags go to its pane.
impl HostedControl for TextBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, _pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        None
    }

    fn handle_mouse(&self, _event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        None
    }

    fn handle_keyboard(
        &self,
        _event: &KeyboardEvent,
        _ctx: &EventContext,
    ) -> Option<ControlAction> {
        None
    }
}

#[cfg(test)]
mod tests;
