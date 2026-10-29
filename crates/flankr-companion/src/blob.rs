//! Binary transfers, the client's half
//! (docs/design/20261028_05_BINARY_FILES_PLAN.md, step 7): images to the
//! daemon (`images`), files and `git` output from it (`files`, `git`),
//! over the link's kind 2, with the transfer state machines of
//! `libflankr_blob`.
//!
//! [`Blobs`] is pure state, as [`State`](crate::client::State) is, but for
//! the preview cache ([`PreviewCache`]), which writes the files it
//! receives to disk so the app can show and share them. The rules it
//! keeps, which the daemon's half sets:
//!
//! - kind 2 goes only to a daemon whose `hello` (or `server.capabilities`)
//!   carries `blob`, and only for a namespace it lists
//!   ([`BlobInfo::serves`]); anything else fails here at once;
//! - an image's CHUNKs go only after its ACCEPT, at most two queued to the
//!   writer at a time, and END only once every CHUNK is written;
//! - an `images` DONE's body is the 8-byte reference, which `parts` and
//!   `terminal.paste` name as 16 lowercase hex digits; a `files` or `git`
//!   DONE's body is empty;
//! - every REQUEST and OFFER names the session (the pane of the agent
//!   view);
//! - a connection that closes ends every transfer: downloads fail, and
//!   uploads that had no DONE start again on the next connection; a DONE's
//!   reference survives it (the daemon keeps it by principal, decision 5.6).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libflankr_blob::{
    ACTIVE_PER_SIDE, BlobMessage, Body, CHUNK_DEFAULT, CHUNK_MAX, Code, DecodeError, FILE_MAX,
    GIT_MAX, IMAGE_MAX, IMAGE_MIN, IMAGES_PER_SEND, Ids, ImageType, NS_FILES, NS_GIT, NS_IMAGES,
    Namespace, Offer, REFERENCE_LEN, Receiving, Sending, Step, TransferId,
};
use libflankr_proto::common::SendPart;
use libflankr_proto::companion::BlobInfo;

use crate::client::{Event, Output};

/// The preview cache's largest total size: 64 MiB (decision 6.4).
pub const PREVIEW_CACHE_MAX: u64 = 64 << 20;

/// The folder under the cache directory the preview cache keeps its files
/// in: the one the libmobile share-and-view plan's provider serves
/// (decision 10.6).
pub const PREVIEW_CACHE_FOLDER: &str = "libmobile-share";

/// One attached image, as the message card's grid shows it (plan
/// 20261028_05, decision 9.2 as the 21:29 EDT amendment's item 1 has it).
/// Given in [`Event::Attachments`] whenever an image is attached, starts
/// uploading, is ready, fails or goes. Added in 0.5.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Its number, 1-4: the `[image N]` token in the text.
    pub n: u8,
    /// The session (pane id) it was uploaded for.
    pub session: String,
    pub mime: String,
    pub size: u64,
    /// The upload's transfer id, for [`Event::TransferProgress`]; 0 while
    /// waiting for a connection.
    pub transfer: u32,
    /// The daemon holds it (DONE): it can be sent.
    pub ready: bool,
    /// Its upload failed, with the code's line (decision 4.6); retry with
    /// [`Input::RetryImage`](crate::Input::RetryImage).
    pub failed: Option<String>,
}

/// Where an image's upload is.
#[derive(Debug)]
enum Upload {
    /// To start once online (attached offline, or its connection closed
    /// before DONE).
    Waiting,
    Sending {
        id: TransferId,
        tx: Sending,
        /// The lengths of the CHUNKs queued to the writer and not yet
        /// written, oldest first: at most two.
        queued: VecDeque<usize>,
        written: u64,
    },
    Ready {
        reference: [u8; REFERENCE_LEN],
    },
    Failed {
        line: String,
    },
}

#[derive(Debug)]
struct Image {
    n: u8,
    session: String,
    mime: String,
    bytes: Arc<Vec<u8>>,
    /// Its token has been in the draft: removing it detaches the image.
    seen: bool,
    up: Upload,
}

/// A file or `git` output coming in.
#[derive(Debug)]
struct Download {
    ns: Namespace,
    rx: Receiving,
    buf: Vec<u8>,
    /// The largest size taken.
    max: u64,
}

/// One piece of a draft: text, or an `[image N]` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Text(&'a str),
    Image(u8),
}

/// `text` split at its `[image N]` tokens, N in 1-4.
pub(crate) fn pieces(text: &str) -> Vec<Piece<'_>> {
    const OPEN: &str = "[image ";
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while let Some(i) = text[at..].find(OPEN).map(|i| at + i) {
        let digit = bytes.get(i + OPEN.len()).copied();
        match digit {
            Some(d @ b'1'..=b'4') if bytes.get(i + OPEN.len() + 1) == Some(&b']') => {
                if start < i {
                    out.push(Piece::Text(&text[start..i]));
                }
                out.push(Piece::Image(d - b'0'));
                start = i + OPEN.len() + 2;
                at = start;
            }
            _ => at = i + 1,
        }
    }
    if start < text.len() {
        out.push(Piece::Text(&text[start..]));
    }
    out
}

/// The token of image `n`.
pub fn token(n: u8) -> String {
    format!("[image {n}]")
}

/// The client's transfers on one daemon's link.
#[derive(Debug)]
pub(crate) struct Blobs {
    /// What the daemon offers, from `hello` or `server.capabilities`.
    info: Option<BlobInfo>,
    /// `hello` has been answered on this connection.
    online: bool,
    ids: Ids,
    /// The driver's clock is a time since some start: this instant plus
    /// it is the `Instant` the state machines read.
    epoch: Instant,
    images: Vec<Image>,
    downloads: HashMap<TransferId, Download>,
    /// Ids answered with CANCEL `unknown_transfer` or `protocol` once.
    answered: HashSet<TransferId>,
    /// Kind-2 messages to send, with the transfer of each CHUNK.
    out: Vec<(Vec<u8>, Option<u32>)>,
    cache: PreviewCache,
}

fn event(e: Event) -> Output {
    Output::Event(e)
}

impl Blobs {
    pub(crate) fn new() -> Blobs {
        Blobs {
            info: None,
            online: false,
            ids: Ids::phone(),
            epoch: Instant::now(),
            images: Vec::new(),
            downloads: HashMap::new(),
            answered: HashSet::new(),
            out: Vec::new(),
            cache: PreviewCache::new(&std::env::temp_dir().join("flankr-companion")),
        }
    }

    pub(crate) fn set_cache(&mut self, cache: PreviewCache) {
        self.cache = cache;
    }

    pub(crate) fn info(&self) -> Option<&BlobInfo> {
        self.info.as_ref().filter(|_| self.online)
    }

    /// Whether the daemon serves `ns` on this connection now.
    pub(crate) fn serves(&self, ns: &str) -> bool {
        self.info().is_some_and(|i| i.serves(ns))
    }

    fn at(&self, now: Duration) -> Instant {
        self.epoch + now
    }

    fn send(&mut self, id: TransferId, ns: &Namespace, body: Body) {
        self.out
            .push((BlobMessage::new(id, ns.clone(), body).encode(), None));
    }

    /// The kind-2 messages to send, in order, with the transfer of each
    /// CHUNK (for the writer's bulk queue).
    pub(crate) fn take(&mut self) -> Vec<(Vec<u8>, Option<u32>)> {
        std::mem::take(&mut self.out)
    }

    /// The attachments, for [`Event::Attachments`].
    pub(crate) fn attachments(&self) -> Vec<Attachment> {
        self.images
            .iter()
            .map(|i| Attachment {
                n: i.n,
                session: i.session.clone(),
                mime: i.mime.clone(),
                size: i.bytes.len() as u64,
                transfer: match &i.up {
                    Upload::Sending { id, .. } => id.0,
                    _ => 0,
                },
                ready: matches!(i.up, Upload::Ready { .. }),
                failed: match &i.up {
                    Upload::Failed { line } => Some(line.clone()),
                    _ => None,
                },
            })
            .collect()
    }

    fn show(&self, out: &mut Vec<Output>) {
        out.push(event(Event::Attachments(self.attachments())));
    }

    fn phone_live(&self) -> usize {
        self.downloads.len()
            + self
                .images
                .iter()
                .filter(|i| matches!(i.up, Upload::Sending { .. }))
                .count()
    }

    // --- The connection ---------------------------------------------------

    /// A new connection: ids count from 1 again (decision 4.3).
    pub(crate) fn connected(&mut self) {
        self.ids = Ids::phone();
        self.online = false;
        self.answered.clear();
        self.out.clear();
    }

    /// `hello` answered, with what it said of `blob`: uploads waiting
    /// start.
    pub(crate) fn hello(&mut self, info: Option<BlobInfo>, now: Duration, out: &mut Vec<Output>) {
        self.info = info;
        self.online = true;
        if self.info.is_some() {
            out.push(event(Event::Blob(self.info.clone())));
        }
        self.start_waiting(now, out);
    }

    /// `server.capabilities` with `blob`.
    pub(crate) fn capabilities(&mut self, info: BlobInfo, now: Duration, out: &mut Vec<Output>) {
        self.info = Some(info);
        out.push(event(Event::Blob(self.info().cloned())));
        self.start_waiting(now, out);
    }

    /// The connection closed: downloads fail, uploads without DONE wait
    /// for the next one.
    pub(crate) fn disconnected(&mut self, out: &mut Vec<Output>) {
        self.online = false;
        self.out.clear();
        let mut ids: Vec<TransferId> = self.downloads.keys().copied().collect();
        ids.sort();
        for id in ids {
            self.downloads.remove(&id);
            out.push(event(Event::TransferFailed {
                id: id.0,
                code: Code::CANCELLED.0,
                line: Some("The connection closed; try again.".to_string()),
            }));
        }
        let mut changed = false;
        for i in &mut self.images {
            if matches!(i.up, Upload::Sending { .. }) {
                i.up = Upload::Waiting;
                changed = true;
            }
        }
        if changed {
            self.show(out);
        }
    }

    fn start_waiting(&mut self, now: Duration, out: &mut Vec<Output>) {
        if !self.serves(NS_IMAGES) {
            return;
        }
        let waiting: Vec<u8> = self
            .images
            .iter()
            .filter(|i| matches!(i.up, Upload::Waiting))
            .map(|i| i.n)
            .collect();
        if waiting.is_empty() {
            return;
        }
        for n in waiting {
            self.start_upload(n, now, out);
        }
        self.show(out);
    }

    // --- Images -----------------------------------------------------------

    /// Attaches an image for `session`'s draft and starts its upload
    /// (decision 9.3). Its number is the lowest free; the app puts its
    /// token in the text.
    pub(crate) fn attach(
        &mut self,
        session: &str,
        bytes: Vec<u8>,
        mime: &str,
        now: Duration,
        out: &mut Vec<Output>,
    ) {
        let refusal = if !self.serves(NS_IMAGES) {
            Some("This daemon can't take images.".to_string())
        } else if self.images.len() >= self.images_max() {
            Some(format!(
                "Only {} images can go in one message.",
                self.images_max()
            ))
        } else if (bytes.len() as u64) < IMAGE_MIN
            || ImageType::sniff(&bytes).is_none_or(|t| Some(t) != ImageType::from_mime(mime))
        {
            Code::BAD_TYPE.line(NS_IMAGES, "")
        } else if bytes.len() as u64 > self.image_max() {
            Code::TOO_LARGE.line(NS_IMAGES, "")
        } else {
            None
        };
        if let Some(why) = refusal {
            out.push(event(Event::Error(why)));
            return;
        }
        let n = (1..=IMAGES_PER_SEND as u8)
            .find(|n| self.images.iter().all(|i| i.n != *n))
            .expect("fewer than four attached");
        self.images.push(Image {
            n,
            session: session.to_string(),
            mime: mime.to_string(),
            bytes: Arc::new(bytes),
            seen: false,
            up: Upload::Waiting,
        });
        self.images.sort_by_key(|i| i.n);
        self.start_upload(n, now, out);
        self.show(out);
    }

    fn images_max(&self) -> usize {
        self.info
            .as_ref()
            .map_or(IMAGES_PER_SEND, |i| i.images_per_send as usize)
            .min(IMAGES_PER_SEND)
    }

    fn image_max(&self) -> u64 {
        self.info
            .as_ref()
            .map_or(IMAGE_MAX, |i| i.image_max)
            .min(IMAGE_MAX)
    }

    /// Starts image `n`'s upload: its OFFER. Busy, it fails at once.
    fn start_upload(&mut self, n: u8, now: Duration, out: &mut Vec<Output>) {
        let at = self.at(now);
        let busy = self.phone_live() >= ACTIVE_PER_SIDE;
        let id = if busy { None } else { self.ids.try_next() };
        let Some(i) = self.images.iter_mut().find(|i| i.n == n) else {
            return;
        };
        let Some(id) = id else {
            let line = Code::BUSY.line(NS_IMAGES, "").unwrap_or_default();
            i.up = Upload::Failed { line };
            return;
        };
        let ext = ImageType::from_mime(&i.mime).map_or("img", |t| t.ext());
        let offer = Offer::for_bytes(&i.session, &format!("image{n}.{ext}"), &i.mime, &i.bytes, 0);
        let size = offer.size;
        i.up = Upload::Sending {
            id,
            tx: Sending::new(&offer, at),
            queued: VecDeque::new(),
            written: 0,
        };
        self.send(id, &Namespace::images(), Body::Offer(offer));
        out.push(event(Event::TransferStarted {
            id: id.0,
            ns: NS_IMAGES.to_string(),
            session: self
                .images
                .iter()
                .find(|i| i.n == n)
                .map(|i| i.session.clone())
                .unwrap_or_default(),
            request: format!("image {n}"),
        }));
        out.push(event(Event::TransferProgress {
            id: id.0,
            done: 0,
            size,
        }));
    }

    /// Image `n` goes: its upload is cancelled.
    pub(crate) fn detach(&mut self, n: u8, out: &mut Vec<Output>) -> bool {
        let Some(at) = self.images.iter().position(|i| i.n == n) else {
            return false;
        };
        let image = self.images.remove(at);
        if let Upload::Sending { id, mut tx, .. } = image.up {
            let body = tx.cancel(Code::CANCELLED, "detached");
            self.send(id, &Namespace::images(), body);
        }
        self.show(out);
        true
    }

    /// Every image goes (the card cleared, or the message sent).
    pub(crate) fn clear(&mut self, out: &mut Vec<Output>) {
        if self.images.is_empty() {
            return;
        }
        for n in self.images.iter().map(|i| i.n).collect::<Vec<_>>() {
            self.detach(n, out);
        }
    }

    /// Image `n`'s failed upload starts again.
    pub(crate) fn retry(&mut self, n: u8, now: Duration, out: &mut Vec<Output>) {
        let Some(i) = self.images.iter_mut().find(|i| i.n == n) else {
            return;
        };
        if !matches!(i.up, Upload::Failed { .. }) {
            return;
        }
        i.up = Upload::Waiting;
        if self.serves(NS_IMAGES) {
            self.start_upload(n, now, out);
        }
        self.show(out);
    }

    /// The draft is now `text`: an image whose token was in it and is
    /// gone is detached (decision 9.2).
    pub(crate) fn text_changed(&mut self, text: &str, out: &mut Vec<Output>) {
        let shown: HashSet<u8> = pieces(text)
            .into_iter()
            .filter_map(|p| match p {
                Piece::Image(n) => Some(n),
                Piece::Text(_) => None,
            })
            .collect();
        let gone: Vec<u8> = self
            .images
            .iter_mut()
            .filter_map(|i| {
                if shown.contains(&i.n) {
                    i.seen = true;
                    None
                } else {
                    i.seen.then_some(i.n)
                }
            })
            .collect();
        for n in gone {
            self.detach(n, out);
        }
    }

    /// Why images cannot be sent yet, if one is uploading or failed: "Uploading
    /// 1 of 2…", or the failed one's line.
    fn not_ready(&self, wanted: &[u8]) -> Option<String> {
        let wanted: Vec<&Image> = self
            .images
            .iter()
            .filter(|i| wanted.contains(&i.n))
            .collect();
        if let Some(line) = wanted.iter().find_map(|i| match &i.up {
            Upload::Failed { line } => Some(line.clone()),
            _ => None,
        }) {
            return Some(line);
        }
        let ready = wanted
            .iter()
            .filter(|i| matches!(i.up, Upload::Ready { .. }))
            .count();
        (ready < wanted.len()).then(|| format!("Uploading {} of {}…", ready + 1, wanted.len()))
    }

    /// The draft `text` for `pane` as `agents.send`'s `parts` (decision
    /// 9.4): split at the tokens of attached images; `None` when it names
    /// none (sent as today). `Err` while one is not uploaded, or was for
    /// another session.
    pub(crate) fn parts(&self, text: &str, pane: &str) -> Option<Result<Vec<SendPart>, String>> {
        let attached = |n: u8| self.images.iter().find(|i| i.n == n);
        let pieces = pieces(text);
        let named: Vec<u8> = pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Image(n) if attached(*n).is_some() => Some(*n),
                _ => None,
            })
            .collect();
        if named.is_empty() {
            return None;
        }
        if let Some(why) = self.not_ready(&named) {
            return Some(Err(why));
        }
        if named
            .iter()
            .any(|n| attached(*n).is_some_and(|i| i.session != pane))
        {
            return Some(Err(
                "Those images were attached for another session.".to_string()
            ));
        }
        let mut parts = Vec::new();
        let mut text_run = String::new();
        let flush = |run: &mut String, parts: &mut Vec<SendPart>| {
            if !run.trim().is_empty() {
                parts.push(SendPart::Text(run.trim().to_string()));
            }
            run.clear();
        };
        for p in pieces {
            match p {
                Piece::Image(n) => match attached(n).map(|i| &i.up) {
                    Some(Upload::Ready { reference }) => {
                        flush(&mut text_run, &mut parts);
                        parts.push(SendPart::Image(libflankr_blob::hex(reference)));
                    }
                    _ => text_run.push_str(&token(n)),
                },
                Piece::Text(t) => text_run.push_str(t),
            }
        }
        flush(&mut text_run, &mut parts);
        Some(Ok(parts))
    }

    /// The references of images `ns` for `pane`, for `terminal.paste`.
    pub(crate) fn references(&self, ns: &[u8], pane: &str) -> Result<Vec<String>, String> {
        if ns.is_empty() {
            return Err("No images to paste.".to_string());
        }
        let mut refs = Vec::new();
        for n in ns {
            let Some(i) = self.images.iter().find(|i| i.n == *n) else {
                return Err(format!("No image {n} is attached."));
            };
            if i.session != pane {
                return Err("That image was attached for another session.".to_string());
            }
            match &i.up {
                Upload::Ready { reference } => refs.push(libflankr_blob::hex(reference)),
                _ => return Err(self.not_ready(ns).unwrap_or_default()),
            }
        }
        Ok(refs)
    }

    /// Images `ns` were used (pasted): they go.
    pub(crate) fn used(&mut self, ns: &[u8], out: &mut Vec<Output>) {
        let before = self.images.len();
        self.images.retain(|i| !ns.contains(&i.n));
        if self.images.len() != before {
            self.show(out);
        }
    }

    /// Queues the next CHUNKs of image `n`'s upload, at most two to the
    /// writer, and END once every one is written.
    fn pump(&mut self, id: TransferId, now: Duration) {
        let at = self.at(now);
        let mut msgs = Vec::new();
        let Some(i) = self
            .images
            .iter_mut()
            .find(|i| matches!(&i.up, Upload::Sending { id: x, .. } if *x == id))
        else {
            return;
        };
        let Upload::Sending { tx, queued, .. } = &mut i.up else {
            return;
        };
        let ns = Namespace::images();
        while queued.len() < 2 {
            let Some((offset, len)) = tx.next_chunk(CHUNK_DEFAULT as usize) else {
                break;
            };
            let o = offset as usize;
            msgs.push((
                BlobMessage::encode_chunk(id, &ns, offset, &i.bytes[o..o + len]),
                Some(id.0),
            ));
            queued.push_back(len);
        }
        if queued.is_empty()
            && let Some(end) = tx.end(at)
        {
            msgs.push((BlobMessage::new(id, ns, end).encode(), None));
        }
        self.out.extend(msgs);
    }

    /// The writer wrote one of `transfer`'s CHUNKs.
    pub(crate) fn written(&mut self, transfer: u32, now: Duration, out: &mut Vec<Output>) {
        let id = TransferId(transfer);
        let Some(i) = self
            .images
            .iter_mut()
            .find(|i| matches!(&i.up, Upload::Sending { id: x, .. } if *x == id))
        else {
            return;
        };
        let size = i.bytes.len() as u64;
        let Upload::Sending {
            queued, written, ..
        } = &mut i.up
        else {
            return;
        };
        if let Some(len) = queued.pop_front() {
            *written += len as u64;
        }
        out.push(event(Event::TransferProgress {
            id: transfer,
            done: *written,
            size,
        }));
        self.pump(id, now);
    }

    fn upload_failed(&mut self, id: TransferId, code: Code, reason: &str, out: &mut Vec<Output>) {
        let line = code.line(NS_IMAGES, reason);
        if let Some(i) = self
            .images
            .iter_mut()
            .find(|i| matches!(&i.up, Upload::Sending { id: x, .. } if *x == id))
        {
            log::info!("image {}: upload {id} ended: {code} ({reason})", i.n);
            i.up = Upload::Failed {
                line: line.clone().unwrap_or_default(),
            };
        }
        out.push(event(Event::TransferFailed {
            id: id.0,
            code: code.0,
            line,
        }));
        self.show(out);
    }

    fn on_upload(&mut self, id: TransferId, body: Body, now: Duration, out: &mut Vec<Output>) {
        let at = self.at(now);
        let Some(i) = self
            .images
            .iter_mut()
            .find(|i| matches!(&i.up, Upload::Sending { id: x, .. } if *x == id))
        else {
            return;
        };
        let Upload::Sending { tx, .. } = &mut i.up else {
            return;
        };
        match tx.on_message(&body, at) {
            Step::Accepted { .. } => self.pump(id, now),
            Step::Done(body) => {
                let Ok(reference) = <[u8; REFERENCE_LEN]>::try_from(body.as_slice()) else {
                    return self.upload_failed(id, Code::PROTOCOL, "a bad DONE body", out);
                };
                let n = i.n;
                log::info!("image {n}: uploaded ({id})");
                i.up = Upload::Ready { reference };
                out.push(event(Event::ImageReady { n }));
                self.show(out);
            }
            Step::Ended { code, reason } => self.upload_failed(id, code, &reason, out),
            Step::Abort(body) => {
                let code = cancel_code(&body);
                self.send(id, &Namespace::images(), body);
                self.upload_failed(id, code, "", out);
            }
            _ => {}
        }
    }

    // --- Files and git ----------------------------------------------------

    /// Asks for `request` in `ns` (`files`: a path; `git`: decision 6.5)
    /// for `session`. Refused at once when the daemon does not serve
    /// `ns`, or too many transfers are live.
    pub(crate) fn request(
        &mut self,
        ns: &str,
        session: &str,
        request: &str,
        now: Duration,
        out: &mut Vec<Output>,
    ) {
        let Some(id) = self.ids.try_next() else {
            out.push(event(Event::Error(
                "Too many transfers on this connection.".to_string(),
            )));
            return;
        };
        out.push(event(Event::TransferStarted {
            id: id.0,
            ns: ns.to_string(),
            session: session.to_string(),
            request: request.to_string(),
        }));
        let code = if !self.serves(ns) {
            Some(if ns == NS_FILES {
                Code::FILES_OFF
            } else {
                Code::UNSUPPORTED
            })
        } else if self.phone_live() >= ACTIVE_PER_SIDE {
            Some(Code::BUSY)
        } else if request.is_empty()
            || request.len() > libflankr_blob::REQUEST_MAX
            || session.is_empty()
            || session.len() > libflankr_blob::SESSION_MAX
        {
            Some(Code::BAD_REQUEST)
        } else {
            None
        };
        let ns_v = match Namespace::new(ns) {
            Ok(ns) => ns,
            Err(_) => return self.refused_here(id, ns, Code::UNSUPPORTED, out),
        };
        if let Some(code) = code {
            return self.refused_here(id, ns, code, out);
        }
        let max = match ns {
            NS_GIT => self.info().map_or(GIT_MAX, |i| i.git_max).min(GIT_MAX),
            _ => self.info().map_or(FILE_MAX, |i| i.file_max).min(FILE_MAX),
        };
        self.send(
            id,
            &ns_v,
            Body::Request {
                session: session.to_string(),
                request: request.to_string(),
                max_size: u32::try_from(max).unwrap_or(u32::MAX),
            },
        );
        self.downloads.insert(
            id,
            Download {
                ns: ns_v,
                rx: Receiving::requested(self.at(now)),
                buf: Vec::new(),
                max,
            },
        );
    }

    fn refused_here(&mut self, id: TransferId, ns: &str, code: Code, out: &mut Vec<Output>) {
        log::info!("transfer {id} ({ns}) refused here: {code}");
        out.push(event(Event::TransferFailed {
            id: id.0,
            code: code.0,
            line: code.line(ns, ""),
        }));
    }

    /// The app cancelled transfer `id`.
    pub(crate) fn cancel(&mut self, id: u32, out: &mut Vec<Output>) {
        let id = TransferId(id);
        if let Some(mut d) = self.downloads.remove(&id) {
            let body = d.rx.cancel(Code::CANCELLED, "cancelled");
            self.send(id, &d.ns, body);
            out.push(event(Event::TransferFailed {
                id: id.0,
                code: Code::CANCELLED.0,
                line: None,
            }));
            return;
        }
        if let Some(n) = self
            .images
            .iter()
            .find(|i| matches!(&i.up, Upload::Sending { id: x, .. } if *x == id))
            .map(|i| i.n)
        {
            self.detach(n, out);
        }
    }

    fn download_failed(
        &mut self,
        id: TransferId,
        ns: &str,
        code: Code,
        reason: &str,
        out: &mut Vec<Output>,
    ) {
        self.downloads.remove(&id);
        log::info!("transfer {id} ({ns}) ended: {code} ({reason})");
        out.push(event(Event::TransferFailed {
            id: id.0,
            code: code.0,
            line: code.line(ns, reason),
        }));
    }

    fn on_download(&mut self, id: TransferId, body: Body, now: Duration, out: &mut Vec<Output>) {
        let at = self.at(now);
        let Some(d) = self.downloads.get_mut(&id) else {
            return;
        };
        let ns = d.ns.clone();
        match d.rx.on_message(&body, at) {
            Step::Offered(offer) => {
                if offer.size > d.max {
                    let refuse = d.rx.refuse(Code::TOO_LARGE, "over the size asked for");
                    self.send(id, &ns, refuse);
                    return self.download_failed(id, ns.as_str(), Code::TOO_LARGE, "", out);
                }
                if let Some(path) = self.cache.find(&offer) {
                    let refuse = d.rx.refuse(Code::HAVE_IT, "cached");
                    self.downloads.remove(&id);
                    self.send(id, &ns, refuse);
                    log::info!("transfer {id} ({ns}): {} from the cache", offer.name);
                    out.push(event(Event::FileReady {
                        id: id.0,
                        path,
                        mime: offer.mime,
                        name: offer.name,
                        size: offer.size,
                    }));
                    return;
                }
                d.buf = Vec::with_capacity(usize::try_from(offer.size).unwrap_or(0));
                if let Some(accept) = d.rx.accept(CHUNK_MAX, at) {
                    self.send(id, &ns, accept);
                }
                out.push(event(Event::TransferProgress {
                    id: id.0,
                    done: 0,
                    size: offer.size,
                }));
            }
            Step::Store => {
                if let Body::Chunk { data, .. } = &body {
                    d.buf.extend_from_slice(data);
                }
                let size = d.rx.offer().map_or(0, |o| o.size);
                out.push(event(Event::TransferProgress {
                    id: id.0,
                    done: d.rx.received(),
                    size,
                }));
            }
            Step::Complete => {
                let Some(mut d) = self.downloads.remove(&id) else {
                    return;
                };
                let offer = d.rx.offer().cloned().expect("an offer before END");
                match self.cache.store(&offer.sha256, &offer.mime, &d.buf) {
                    Ok(path) => {
                        let done = d.rx.done(Vec::new());
                        self.send(id, &ns, done);
                        out.push(event(Event::FileReady {
                            id: id.0,
                            path,
                            mime: offer.mime,
                            name: offer.name,
                            size: offer.size,
                        }));
                    }
                    Err(e) => {
                        log::warn!("the preview cache: {e}");
                        let cancel = d.rx.cancel(Code::IO, "the phone couldn't save it");
                        self.send(id, &ns, cancel);
                        out.push(event(Event::TransferFailed {
                            id: id.0,
                            code: Code::IO.0,
                            line: Some("The phone couldn't save it.".to_string()),
                        }));
                    }
                }
            }
            Step::Ended { code, reason } => {
                self.download_failed(id, ns.as_str(), code, &reason, out)
            }
            Step::Abort(body) => {
                let code = cancel_code(&body);
                self.send(id, &ns, body);
                self.download_failed(id, ns.as_str(), code, "", out);
            }
            _ => {}
        }
    }

    // --- Messages and the clock -------------------------------------------

    /// A kind-2 message from the daemon. `Err` when its header cannot be
    /// read: the connection is closed (decision 4.1).
    pub(crate) fn on_binary(
        &mut self,
        bytes: &[u8],
        now: Duration,
        out: &mut Vec<Output>,
    ) -> Result<(), String> {
        let msg = match BlobMessage::decode(bytes) {
            Ok(m) => m,
            Err(DecodeError::Fatal) => return Err("an unreadable binary message".to_string()),
            Err(DecodeError::Answer(id, ns)) => {
                log::warn!("transfer {id} ({ns}): a message that does not decode");
                if self.downloads.contains_key(&id) {
                    let body = Body::cancel(Code::PROTOCOL, "bad message body");
                    self.send(id, &ns, body);
                    self.download_failed(id, ns.as_str(), Code::PROTOCOL, "", out);
                } else if self.is_upload(id) {
                    self.send(id, &ns, Body::cancel(Code::PROTOCOL, "bad message body"));
                    self.upload_failed(id, Code::PROTOCOL, "", out);
                } else if self.answered.insert(id) {
                    self.send(id, &ns, Body::cancel(Code::PROTOCOL, "bad message body"));
                }
                return Ok(());
            }
        };
        let BlobMessage {
            transfer: id,
            ns,
            body,
        } = msg;
        if let Some(d) = self.downloads.get(&id) {
            if d.ns != ns {
                let mine = d.ns.clone();
                self.send(id, &mine, Body::cancel(Code::PROTOCOL, "wrong namespace"));
                self.download_failed(id, mine.as_str(), Code::PROTOCOL, "", out);
                return Ok(());
            }
            self.on_download(id, body, now, out);
            return Ok(());
        }
        if self.is_upload(id) {
            if ns.as_str() != NS_IMAGES {
                self.send(
                    id,
                    &Namespace::images(),
                    Body::cancel(Code::PROTOCOL, "wrong namespace"),
                );
                self.upload_failed(id, Code::PROTOCOL, "", out);
                return Ok(());
            }
            self.on_upload(id, body, now, out);
            return Ok(());
        }
        // No such live transfer (decision 4.3).
        match body {
            Body::Request { .. } | Body::Offer(_) => {
                // The daemon starts no transfers in this plan.
                let code = if id.started_by_daemon() {
                    Code::UNSUPPORTED
                } else {
                    Code::PROTOCOL
                };
                self.send(id, &ns, Body::refuse(code, "not served by this phone"));
            }
            Body::Chunk { .. } | Body::End { .. } => {
                if self.answered.insert(id) {
                    self.send(
                        id,
                        &ns,
                        Body::cancel(Code::UNKNOWN_TRANSFER, "no such transfer"),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn is_upload(&self, id: TransferId) -> bool {
        self.images
            .iter()
            .any(|i| matches!(&i.up, Upload::Sending { id: x, .. } if *x == id))
    }

    /// The clock: the timeouts of decision 4.4.
    pub(crate) fn tick(&mut self, now: Duration, out: &mut Vec<Output>) {
        let at = self.at(now);
        let mut aborts = Vec::new();
        for i in &mut self.images {
            if let Upload::Sending { id, tx, .. } = &mut i.up
                && let Step::Abort(body) = tx.on_tick(at)
            {
                aborts.push((*id, body, true));
            }
        }
        for (id, d) in &mut self.downloads {
            if let Step::Abort(body) = d.rx.on_tick(at) {
                aborts.push((*id, body, false));
            }
        }
        aborts.sort_by_key(|(id, _, _)| *id);
        for (id, body, upload) in aborts {
            if upload {
                self.send(id, &Namespace::images(), body);
                self.upload_failed(id, Code::TIMEOUT, "", out);
            } else if let Some(ns) = self.downloads.get(&id).map(|d| d.ns.clone()) {
                self.send(id, &ns, body);
                self.download_failed(id, ns.as_str(), Code::TIMEOUT, "", out);
            }
        }
    }
}

/// A CANCEL's or REFUSE's code.
fn cancel_code(body: &Body) -> Code {
    match body {
        Body::Cancel { code, .. } | Body::Refuse { code, .. } => *code,
        _ => Code::PROTOCOL,
    }
}

/// The files fetched over the link, kept for the preview and for sharing
/// (decision 6.4): `<cache dir>/libmobile-share/fl-<hash16>.<ext>`, keyed
/// by SHA-256, at most [`PREVIEW_CACHE_MAX`] in all, the least recently
/// viewed deleted first (a file's modification time is when it was last
/// stored or found). Added in 0.5.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCache {
    dir: PathBuf,
}

impl PreviewCache {
    /// The cache in `cache_dir`'s `libmobile-share/`.
    pub fn new(cache_dir: &Path) -> PreviewCache {
        PreviewCache {
            dir: cache_dir.join(PREVIEW_CACHE_FOLDER),
        }
    }

    /// The folder the files are in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The extension a file of `mime` is kept with.
    pub fn ext(mime: &str) -> &'static str {
        match mime {
            "text/markdown" => "md",
            "text/x-diff" => "diff",
            "text/plain" => "txt",
            m => ImageType::from_mime(m).map_or("bin", ImageType::ext),
        }
    }

    /// The file name for bytes with this SHA-256 and MIME type.
    pub fn name(sha256: &[u8; 32], mime: &str) -> String {
        format!(
            "fl-{}.{}",
            libflankr_blob::hex(&sha256[..8]),
            Self::ext(mime)
        )
    }

    /// The cached copy of what `offer` offers, if this cache holds it
    /// (same size and SHA-256); marked as just viewed.
    pub fn find(&self, offer: &Offer) -> Option<PathBuf> {
        let path = self.dir.join(Self::name(&offer.sha256, &offer.mime));
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() || meta.len() != offer.size {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        if libflankr_blob::sha256(&bytes) != offer.sha256 {
            return None;
        }
        touch(&path);
        Some(path)
    }

    /// Keeps `bytes`, making room first; their path.
    pub fn store(&self, sha256: &[u8; 32], mime: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let name = Self::name(sha256, mime);
        let path = self.dir.join(&name);
        self.evict(bytes.len() as u64, &name);
        let part = self.dir.join(format!(".{name}.part"));
        std::fs::write(&part, bytes)?;
        std::fs::rename(&part, &path)?;
        Ok(path)
    }

    /// Deletes the least recently viewed files until `incoming` more
    /// bytes fit (`keep` aside).
    fn evict(&self, incoming: u64, keep: &str) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("fl-") && name != keep
            })
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                meta.is_file().then(|| {
                    (
                        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                        meta.len(),
                        e.path(),
                    )
                })
            })
            .collect();
        files.sort();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (_, len, path) in files {
            if total + incoming <= PREVIEW_CACHE_MAX {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}

/// Sets `path`'s modification time to now.
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_the_draft() {
        assert_eq!(
            pieces("look [image 1] and [image 2]"),
            vec![
                Piece::Text("look "),
                Piece::Image(1),
                Piece::Text(" and "),
                Piece::Image(2)
            ]
        );
        assert_eq!(pieces("[image 3]"), vec![Piece::Image(3)]);
        assert_eq!(
            pieces("[image 5] [image x] [image 1"),
            vec![Piece::Text("[image 5] [image x] [image 1")]
        );
        assert_eq!(pieces(""), vec![]);
        assert_eq!(
            pieces("[[image 1]]"),
            vec![Piece::Text("["), Piece::Image(1), Piece::Text("]")]
        );
    }

    fn info() -> BlobInfo {
        BlobInfo {
            version: 1,
            namespaces: vec!["images".into(), "files".into()],
            read: vec!["docs/**/*.md".into()],
            image_max: IMAGE_MAX,
            file_max: FILE_MAX,
            git_max: GIT_MAX,
            chunk_max: CHUNK_MAX,
            images_per_send: 4,
        }
    }

    fn png() -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        b.resize(300 << 10, 1);
        b
    }

    /// What the daemon would answer `msg` with, as `Body`.
    fn reply(msg: &[u8], body: Body) -> Vec<u8> {
        let m = BlobMessage::decode(msg).unwrap();
        BlobMessage::new(m.transfer, m.ns, body).encode()
    }

    #[test]
    fn chunks_wait_for_accept_two_at_a_time_then_end() {
        let mut b = Blobs::new();
        let mut out = Vec::new();
        b.connected();
        b.hello(Some(info()), Duration::ZERO, &mut out);
        b.attach("w1:1", png(), "image/png", Duration::ZERO, &mut out);
        let sent = b.take();
        assert_eq!(sent.len(), 1, "the OFFER only");
        let offer = &sent[0].0;
        assert!(matches!(
            BlobMessage::decode(offer).unwrap().body,
            Body::Offer(_)
        ));
        // Not ready: the message cannot go.
        assert_eq!(
            b.parts("[image 1]", "w1:1"),
            Some(Err("Uploading 1 of 1…".to_string()))
        );
        let accept = reply(
            offer,
            Body::Accept {
                chunk_max: CHUNK_MAX,
            },
        );
        b.on_binary(&accept, Duration::ZERO, &mut out).unwrap();
        let sent = b.take();
        // Two CHUNKs of 256 KiB at most, on the bulk queue.
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|(_, t)| *t == Some(1)));
        b.written(1, Duration::ZERO, &mut out);
        let sent = b.take();
        assert!(sent.is_empty(), "300 KiB is two CHUNKs: none left");
        b.written(1, Duration::ZERO, &mut out);
        let sent = b.take();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, None);
        assert_eq!(
            BlobMessage::decode(&sent[0].0).unwrap().body,
            Body::End { size: 300 << 10 }
        );
        let done = reply(
            offer,
            Body::Done {
                body: vec![0x9f; 8],
            },
        );
        out.clear();
        b.on_binary(&done, Duration::ZERO, &mut out).unwrap();
        assert!(out.contains(&Output::Event(Event::ImageReady { n: 1 })));
        assert_eq!(
            b.parts(" [image 1] hi", "w1:1"),
            Some(Ok(vec![
                SendPart::Image("9f9f9f9f9f9f9f9f".into()),
                SendPart::Text("hi".into())
            ]))
        );
        assert_eq!(
            b.parts("[image 1]", "w2:1").unwrap().unwrap_err(),
            "Those images were attached for another session."
        );
        assert_eq!(b.parts("no images", "w1:1"), None);
    }

    #[test]
    fn a_files_offer_over_the_size_asked_for_is_refused() {
        let mut b = Blobs::new();
        let mut out = Vec::new();
        b.connected();
        b.hello(Some(info()), Duration::ZERO, &mut out);
        b.request(NS_FILES, "w1:1", "docs/a.md", Duration::ZERO, &mut out);
        let req = b.take().remove(0).0;
        let mut offer = Offer::for_bytes("w1:1", "docs/a.md", "text/markdown", b"x", 0);
        offer.size = FILE_MAX + 1;
        out.clear();
        b.on_binary(&reply(&req, Body::Offer(offer)), Duration::ZERO, &mut out)
            .unwrap();
        let sent = b.take();
        assert!(matches!(
            BlobMessage::decode(&sent[0].0).unwrap().body,
            Body::Refuse {
                code: Code::TOO_LARGE,
                ..
            }
        ));
        assert!(
            out.iter()
                .any(|o| matches!(o, Output::Event(Event::TransferFailed { code: 4, .. })))
        );
        // A CHUNK for it now is unknown: answered once.
        let chunk = reply(
            &req,
            Body::Chunk {
                offset: 0,
                data: vec![1],
            },
        );
        b.on_binary(&chunk, Duration::ZERO, &mut out).unwrap();
        b.on_binary(&chunk, Duration::ZERO, &mut out).unwrap();
        assert_eq!(b.take().len(), 1);
        // An unreadable header closes the connection.
        assert!(b.on_binary(&[1, 2], Duration::ZERO, &mut out).is_err());
    }

    #[test]
    fn a_silent_daemon_times_out() {
        let mut b = Blobs::new();
        let mut out = Vec::new();
        b.connected();
        b.hello(Some(info()), Duration::ZERO, &mut out);
        b.request(NS_FILES, "w1:1", "docs/a.md", Duration::ZERO, &mut out);
        b.take();
        out.clear();
        b.tick(Duration::from_secs(29), &mut out);
        assert!(out.is_empty());
        b.tick(Duration::from_secs(30), &mut out);
        assert!(
            out.iter()
                .any(|o| matches!(o, Output::Event(Event::TransferFailed { code: 12, .. })))
        );
        assert!(matches!(
            BlobMessage::decode(&b.take()[0].0).unwrap().body,
            Body::Cancel {
                code: Code::TIMEOUT,
                ..
            }
        ));
    }

    #[test]
    fn the_cache_keeps_by_hash_and_evicts_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PreviewCache::new(dir.path());
        let a = b"# a\n".to_vec();
        let sha = libflankr_blob::sha256(&a);
        let path = cache.store(&sha, "text/markdown", &a).unwrap();
        assert_eq!(
            path,
            dir.path()
                .join("libmobile-share")
                .join(format!("fl-{}.md", libflankr_blob::hex(&sha[..8])))
        );
        let offer = Offer::for_bytes("w1:1", "a.md", "text/markdown", &a, 0);
        assert_eq!(cache.find(&offer), Some(path.clone()));
        // Another type is another file.
        let plain = Offer::for_bytes("w1:1", "a.txt", "text/plain", &a, 0);
        assert_eq!(cache.find(&plain), None);
        // A damaged copy is not used.
        std::fs::write(&path, b"# b\n").unwrap();
        assert_eq!(cache.find(&offer), None);

        // Over 64 MiB, the least recently viewed goes first.
        let big = vec![7u8; 40 << 20];
        let old = cache
            .store(&libflankr_blob::sha256(b"old"), "text/plain", &big)
            .unwrap();
        let older = std::time::SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .append(true)
            .open(&old)
            .unwrap()
            .set_modified(older)
            .unwrap();
        let new = cache
            .store(&libflankr_blob::sha256(b"new"), "text/plain", &big)
            .unwrap();
        assert!(!old.exists());
        assert!(new.exists());
    }
}
