//! The app's side of the paired remotes (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md,
//! step 5): the pairing screen's state and lines, the notes a removal or a
//! pairing leaves in Settings' Remotes, and *Delete data*'s removals.
//!
//! - [`Pairing`]: whether the pairing screen is in its **add** mode (the
//!   **+** in Remotes, decision 9), the link last read ([`LinkRead`]),
//!   what a waiting pairing shows ([`Waiting`], from
//!   [`flankr_companion::Event::PairingWords`]), a finished pairing
//!   ([`Finished`]) and whether **Details** are open. Cancel, the add
//!   mode's "<" and the system Back end a pairing with
//!   [`Input::CancelPairing`].
//! - [`Pairing::lines`]: the pairing modal's lines while a pairing waits
//!   (the owner, 2026-10-28: "if we're using the six words to confirm the
//!   connection, shouldn't we show them in a confirmation modal?"). Quiet,
//!   as the 14:42 EDT amendment has it: the six pairing words, large, and
//!   one line saying to confirm on the Mac. Its Details: the daemon's name
//!   and `host:port`, its fingerprint as six words and as BLAKE2s-256 in
//!   hex, the time the link has left, and in the device-code fallback the
//!   code to paste into `flankr pair`.
//! - A finished pairing (`Paired`, or `RemoteUpdated` for one already
//!   listed) stays on the window until **Done** (the owner, 2026-10-28:
//!   "currently it just closes and looks like it failed").
//! - The lines are Markdown (drawn with `flankr-markdown`), so the copy can
//!   set `flankr pair` in bold; values put into them go through
//!   [`md_escape`].
//! - [`delete_all`]: *Delete data* removes every remote
//!   ([`Input::RemoveRemote`]), the one in use last; with none, it asks
//!   for [`Input::Unpair`], which answers [`flankr_companion::Event::Unpaired`]
//!   at once. The data directory goes once `Unpaired` comes.

use std::time::{Duration, Instant};

use flankr_companion::{DeviceCodeFallback, Input, RemoteId, RemoteInfo};
use libflankr_proto::link;

use crate::settings::copy;
use crate::theme::{Role, theme};
use crate::ui::{Layout, Line, chunk, wrap};

/// The pairing words' size, in points: large, to be read against the
/// Mac's terminal at a glance.
pub const WORDS_PT: f32 = 34.0;
/// The notes' and labels' size, in points.
const NOTE_PT: f32 = 15.0;
/// The fingerprint words' size in Details, in points.
const FINGERPRINT_PT: f32 = 20.0;
/// The hex fingerprint's size, in points, and its groups of four a line.
pub const HEX_PT: f32 = 15.0;
const HEX_GROUPS_PER_LINE: usize = 4;
/// The device code's size, in points.
const CODE_PT: f32 = 26.0;
/// The pairing words on one line: two lines of three, as `flankr pair`
/// prints six in a row.
const WORDS_PER_LINE: usize = 3;

/// `text` with every ASCII punctuation character backslash-escaped, so a
/// value (a name, a host, a reason) put into the pairing screen's Markdown
/// shows as written.
pub fn md_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// BLAKE2s-256 of `key` as 64 hex digits in groups of four.
pub fn fingerprint_hex(key: &[u8; 32]) -> String {
    let digits: String = link::fingerprint(key)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    digits
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The host of `host:port`, without the port (an IPv6 address in brackets
/// is kept whole).
pub fn host_of(address: &str) -> &str {
    match address.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.parse::<u16>().is_ok() => host,
        _ => address,
    }
}

/// What the app read of the pairing link it handed to the client: for
/// "Connecting to <host>…", and for the full fingerprint in Details, which
/// `PairingWords` does not carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRead {
    /// The daemon's name, as the client names it.
    pub server: String,
    /// The link's first `host:port`.
    pub address: String,
    /// The daemon's fingerprint as six words.
    pub words: String,
    /// BLAKE2s-256 of the daemon's key in hex ([`fingerprint_hex`]).
    pub hex: String,
}

impl LinkRead {
    /// The link `text`, if `flankr_companion::scan::read` reads it.
    pub fn of(text: &str) -> Option<Self> {
        let p = flankr_companion::scan::read(text).ok()?;
        let key = p.payload.server_key;
        Some(LinkRead {
            server: p.server,
            address: p.payload.addresses.first()?.clone(),
            words: link::fingerprint_words_text(&key),
            hex: fingerprint_hex(&key),
        })
    }
}

/// What a waiting pairing shows ([`flankr_companion::Event::PairingWords`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    /// The daemon's name, from the pairing link.
    pub server: String,
    /// The daemon's `host:port`.
    pub address: String,
    /// The six pairing words; `None` for a link without an offer.
    pub words: Option<String>,
    /// The daemon's fingerprint as six words.
    pub fingerprint: String,
    /// The whole fingerprint in hex, from the link the app read, when its
    /// six words are `fingerprint` (the event does not carry the key).
    pub fingerprint_hex: Option<String>,
    /// When the link's offer expires; `None` in the fallback.
    pub expires_at: Option<Instant>,
    /// The device-code fallback, if this is it.
    pub fallback: Option<DeviceCodeFallback>,
}

impl Waiting {
    /// From the event's fields, received at `now`.
    pub fn new(
        server: String,
        address: String,
        words: Option<String>,
        fingerprint: String,
        expires_in_secs: Option<u16>,
        fallback: Option<DeviceCodeFallback>,
        now: Instant,
    ) -> Self {
        Waiting {
            server,
            address,
            words,
            fingerprint,
            fingerprint_hex: None,
            expires_at: expires_in_secs.map(|s| now + Duration::from_secs(u64::from(s))),
            fallback,
        }
    }
}

/// A pairing that ended well, shown until **Done**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    /// The daemon's name.
    pub server: String,
    /// The `host:port` the phone reached (or the remote's first).
    pub address: Option<String>,
    /// The remote's short fingerprint, `3f9a 12c4`, from its row.
    pub label: Option<String>,
    /// The daemon's fingerprint as six words, and in hex.
    pub words: Option<String>,
    pub hex: Option<String>,
    /// It was already paired (`RemoteUpdated`).
    pub already: bool,
    /// Its addresses changed: its row's first address is not the old one.
    pub updated: bool,
    /// The remote, once known (`RemoteUpdated` names it; `Paired`'s is
    /// found by name in the next `Remotes`).
    pub id: Option<RemoteId>,
    /// Its row's first address before the update.
    old_host: Option<String>,
}

impl Finished {
    /// "Paired with m4", or "Already paired with m4".
    pub fn title(&self) -> String {
        let key = if self.already {
            copy::PAIRING_ALREADY
        } else {
            copy::PAIRING_PAIRED
        };
        key.fill(None, &[("name", &self.server)]).trim().to_string()
    }

    /// The dim lines under the title, Markdown: `host:port` and the short
    /// fingerprint, then whether the addresses were updated.
    pub fn summary(&self) -> String {
        let mut out = match (&self.address, &self.label) {
            (Some(address), Some(label)) => copy::PAIRING_REACHED
                .fill(
                    None,
                    &[
                        ("address", &md_escape(address)),
                        ("fingerprint", &md_escape(label)),
                    ],
                )
                .trim()
                .to_string(),
            (Some(address), None) => md_escape(address),
            _ => String::new(),
        };
        if self.updated {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(copy::PAIRING_ADDRESSES_UPDATED.text.trim());
        }
        out
    }

    /// Its Details: the daemon's name and address, and the full
    /// fingerprint.
    pub fn details(&self, layout: &Layout) -> Vec<Line> {
        let mut out = Vec::new();
        identity(
            &mut out,
            layout,
            &self.server,
            self.address.as_deref(),
            self.words.as_deref(),
            self.hex.as_deref(),
        );
        out
    }
}

/// The pairing screen's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    /// The add mode, opened by the **+** in Remotes, over Settings.
    pub adding: bool,
    /// The link last handed to the client.
    pub link: Option<LinkRead>,
    /// What a waiting pairing shows.
    pub waiting: Option<Waiting>,
    /// A pairing that ended well, until **Done**.
    pub finished: Option<Finished>,
    /// Details are open (the modal's, or the finished pairing's).
    pub details: bool,
}

impl Pairing {
    /// The **+**: the add mode, with nothing pairing yet.
    pub fn open_add(&mut self) {
        *self = Pairing {
            adding: true,
            ..Pairing::default()
        };
    }

    /// A link was handed to the client: what was read of it.
    pub fn read(&mut self, link: Option<LinkRead>) {
        self.link = link;
        self.finished = None;
        self.details = false;
    }

    /// A pairing waits: its words are shown in the modal, with Details and
    /// Cancel. With no words to show (a link without an offer) Details open
    /// at once, as the code there is all there is to do.
    pub fn show(&mut self, mut waiting: Waiting) {
        if waiting.words.is_none() {
            self.details = true;
        } else if self.waiting.as_ref().map(|w| &w.words) != Some(&waiting.words) {
            self.details = false;
        }
        if waiting.fingerprint_hex.is_none() {
            waiting.fingerprint_hex = self
                .link
                .as_ref()
                .filter(|l| l.words == waiting.fingerprint)
                .map(|l| l.hex.clone());
        }
        self.finished = None;
        self.waiting = Some(waiting);
    }

    /// A pairing waits for the owner: the modal shows.
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }

    /// The Details button: opens or closes them.
    pub fn toggle_details(&mut self) {
        self.details = !self.details;
    }

    /// Cancel, the add mode's "<", or the system Back: the pairing in
    /// progress ends (its connection closed and its new key deleted by the
    /// client) and the add mode closes, back to Settings. The input to
    /// send.
    pub fn cancel(&mut self) -> Input {
        *self = Pairing::default();
        Input::CancelPairing
    }

    /// The daemon added the key: the modal closes and the window shows the
    /// finished pairing until **Done**. The daemon's name, if known.
    pub fn paired(&mut self, rows: &[RemoteInfo]) -> Option<String> {
        let waiting = self.waiting.take();
        let link = self.link.take();
        let server = waiting
            .as_ref()
            .map(|w| w.server.clone())
            .or_else(|| link.as_ref().map(|l| l.server.clone()));
        let mut finished = Finished {
            server: server.clone().unwrap_or_default(),
            address: waiting
                .as_ref()
                .map(|w| w.address.clone())
                .or_else(|| link.as_ref().map(|l| l.address.clone())),
            label: None,
            words: waiting
                .as_ref()
                .map(|w| w.fingerprint.clone())
                .or_else(|| link.as_ref().map(|l| l.words.clone())),
            hex: waiting
                .as_ref()
                .and_then(|w| w.fingerprint_hex.clone())
                .or_else(|| link.as_ref().map(|l| l.hex.clone())),
            already: false,
            updated: false,
            id: None,
            old_host: None,
        };
        fill_from_rows(&mut finished, rows);
        self.details = false;
        self.finished = Some(finished);
        server
    }

    /// A pairing named a remote already listed, and its addresses were
    /// checked: the window shows it until **Done**. `rows` are the rows
    /// before the update.
    pub fn remote_updated(&mut self, id: RemoteId, name: &str, rows: &[RemoteInfo]) {
        let link = self.link.take();
        self.waiting = None;
        self.details = false;
        let row = rows.iter().find(|r| r.id == id);
        self.finished = Some(Finished {
            server: name.to_string(),
            address: row.map(|r| r.host.clone()),
            label: row.map(|r| r.fingerprint.clone()),
            words: link.as_ref().map(|l| l.words.clone()),
            hex: link.map(|l| l.hex),
            already: true,
            updated: false,
            id: Some(id),
            old_host: row.map(|r| r.host.clone()),
        });
    }

    /// The remotes changed: a finished pairing takes its row's short
    /// fingerprint, and an updated one whether its address changed.
    pub fn remotes_changed(&mut self, rows: &[RemoteInfo]) {
        if let Some(f) = &mut self.finished {
            fill_from_rows(f, rows);
        }
    }

    /// **Done**: the finished pairing goes. Whether it was the add mode
    /// (back to Settings) rather than the first run (on to the overview).
    pub fn done(&mut self) -> bool {
        let adding = self.adding;
        *self = Pairing::default();
        adding
    }

    /// A pairing failed or ended: the modal closes, and the window shows
    /// why.
    pub fn failed(&mut self) {
        self.waiting = None;
        self.link = None;
        self.details = false;
    }

    /// The pairing area's lines at `now`, Markdown: in the add mode with
    /// nothing pairing, what to do; while a pairing waits, the modal's: its
    /// words, large, and one line, or its Details; then the note, if any.
    pub fn lines(
        &self,
        layout: &Layout,
        note: Option<(&str, [f32; 4])>,
        now: Instant,
    ) -> Vec<Line> {
        let mut out = Vec::new();
        match &self.waiting {
            None if self.adding => para(
                &mut out,
                layout,
                copy::REMOTE_ADD_PASTE.text,
                theme().muted,
                0.0,
            ),
            None => {}
            Some(w) if self.details => details(&mut out, layout, w, now),
            Some(w) => {
                let escaped = md_escape(&w.server);
                let server = [("server", escaped.as_str())];
                let line = match (&w.words, &w.fallback) {
                    (Some(words), None) => {
                        words_lines(&mut out, layout, words);
                        copy::PAIRING_CONFIRM.fill(None, &server)
                    }
                    (Some(words), Some(_)) => {
                        words_lines(&mut out, layout, words);
                        copy::PAIRING_FALLBACK.fill(None, &server)
                    }
                    (None, _) => copy::PAIRING_FALLBACK_NO_WORDS.fill(None, &server),
                };
                para(&mut out, layout, &line, theme().muted, 16.0);
            }
        }
        if let Some((note, color)) = note {
            para(&mut out, layout, note, color, 8.0);
        }
        out
    }
}

/// `f`'s row, by id or else by name: its short fingerprint, its id, and
/// for an update whether its first address changed.
fn fill_from_rows(f: &mut Finished, rows: &[RemoteInfo]) {
    let row = match f.id {
        Some(id) => rows.iter().find(|r| r.id == id),
        None => rows.iter().find(|r| r.name == f.server),
    };
    let Some(row) = row else {
        return;
    };
    f.id = Some(row.id);
    f.label = Some(row.fingerprint.clone());
    if f.already {
        if f.old_host.as_deref().is_some_and(|old| old != row.host) {
            f.updated = true;
        }
        f.address = Some(row.host.clone());
    } else if f.address.is_none() {
        f.address = Some(row.host.clone());
    }
}

/// `text` (Markdown, one paragraph) at the notes' size, its first line
/// `gap` below the last. The screen wraps it to its width.
pub(crate) fn para(out: &mut Vec<Line>, _layout: &Layout, text: &str, color: [f32; 4], gap: f32) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    out.push(Line {
        text: text.to_string(),
        size: NOTE_PT,
        color,
        gap,
    });
}

/// Six words in lines of [`WORDS_PER_LINE`] at `size`, each wrapped if
/// still too wide.
fn word_rows(out: &mut Vec<Line>, layout: &Layout, words: &str, size: f32, color: [f32; 4]) {
    let words: Vec<&str> = words.split_whitespace().collect();
    for row in words.chunks(WORDS_PER_LINE) {
        for t in wrap(&row.join(" "), layout.columns(size)) {
            out.push(Line {
                text: md_escape(&t),
                size,
                color,
                gap: 4.0,
            });
        }
    }
}

/// The pairing words, large: two rows of three, at [`WORDS_PT`] or as
/// large as fits the longer row on one line (Roboto's lower case is about
/// 0.56 em a letter).
fn words_lines(out: &mut Vec<Line>, layout: &Layout, words: &str) {
    let list: Vec<&str> = words.split_whitespace().collect();
    let longest = list
        .chunks(WORDS_PER_LINE)
        .map(|row| row.join(" ").chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    let width_pt = layout.w / layout.scale.max(0.1) - 2.0 * Layout::PAD;
    let size = WORDS_PT.min((width_pt / (longest as f32 * 0.56)).floor());
    word_rows(out, layout, words, size, theme().text);
}

/// The daemon's name and `host:port`, then its fingerprint: six words, and
/// BLAKE2s-256 in hex in the terminal face, four groups a line.
fn identity(
    out: &mut Vec<Line>,
    layout: &Layout,
    server: &str,
    address: Option<&str>,
    words: Option<&str>,
    hex: Option<&str>,
) {
    if let Some(address) = address {
        let line = copy::PAIRING_ADDRESS.fill(
            None,
            &[
                ("server", &md_escape(server)),
                ("address", &md_escape(address)),
            ],
        );
        para(out, layout, &line, theme().text, 0.0);
    }
    if let Some(words) = words {
        para(
            out,
            layout,
            copy::PAIRING_FINGERPRINT.text,
            theme().muted,
            12.0,
        );
        word_rows(out, layout, words, FINGERPRINT_PT, theme().text);
    }
    // TODO(flankr-companion): `PairingWords` carries no daemon key, so the
    // hex comes from the link this app read and handed over.
    if let Some(hex) = hex {
        para(
            out,
            layout,
            copy::PAIRING_FINGERPRINT_FULL.text,
            theme().muted,
            12.0,
        );
        let groups: Vec<&str> = hex.split_whitespace().collect();
        for row in groups.chunks(HEX_GROUPS_PER_LINE) {
            out.push(Line {
                text: format!("`{}`", row.join(" ")),
                size: HEX_PT,
                color: theme().code,
                gap: 4.0,
            });
        }
    }
}

/// The modal's Details: the daemon's name and `host:port`, its full
/// fingerprint, the time the link has left, then the device-code
/// fallback's code (or, without words, why there are none).
fn details(out: &mut Vec<Line>, layout: &Layout, w: &Waiting, now: Instant) {
    let escaped = md_escape(&w.server);
    let server = [("server", escaped.as_str())];
    identity(
        out,
        layout,
        &w.server,
        Some(&w.address),
        Some(&w.fingerprint),
        w.fingerprint_hex.as_deref(),
    );
    if let Some(at) = w.expires_at {
        let left = at.saturating_duration_since(now);
        let text = if left.is_zero() {
            copy::PAIRING_EXPIRED.text.to_string()
        } else {
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            copy::PAIRING_EXPIRES.fill(None, &[("seconds", &secs.to_string())])
        };
        para(out, layout, &text, theme().muted, 12.0);
    }
    if w.words.is_none() {
        para(
            out,
            layout,
            copy::PAIRING_NO_WORDS.text,
            theme().muted,
            12.0,
        );
    }
    if let Some(fallback) = &w.fallback {
        if fallback.replace {
            para(
                out,
                layout,
                &copy::PAIRING_REPLACE.fill(None, &server),
                theme().get(Role::Warn),
                12.0,
            );
        }
        para(
            out,
            layout,
            &copy::PAIRING_PASTE.fill(None, &server),
            theme().muted,
            12.0,
        );
        // Base58 is mostly capitals and wide letters: wider than the half
        // an em `columns` assumes.
        for part in chunk(&fallback.code, layout.columns(CODE_PT * 1.3)) {
            out.push(Line {
                text: part,
                size: CODE_PT,
                color: theme().code,
                gap: 4.0,
            });
        }
    }
}
/// *Delete data*'s inputs: [`Input::RemoveRemote`] for every remote, the
/// one in use last (the client asks the others by their own short
/// connections meanwhile, and the last removal gives `Unpaired`); with
/// none paired, [`Input::Unpair`], which gives `Unpaired` at once.
pub fn delete_all(remotes: &[RemoteInfo]) -> Vec<Input> {
    if remotes.is_empty() {
        return vec![Input::Unpair];
    }
    let (active, others): (Vec<_>, Vec<_>) = remotes.iter().partition(|r| r.active);
    others
        .into_iter()
        .chain(active)
        .map(|r| Input::RemoveRemote(r.id))
        .collect()
}

/// The note while `name` is being removed.
pub fn removing_note(name: &str) -> String {
    copy::REMOTE_REMOVING.fill(None, &[("name", name)])
}

/// The note once `name` is removed: whether its daemon answered
/// `link.unpair`, and, if not, the `auth.toml` entry left there for
/// `device`.
pub fn removed_note(name: &str, daemon: bool, device: &str) -> String {
    if daemon {
        copy::REMOTE_REMOVED.fill(None, &[("name", name)])
    } else {
        copy::REMOTE_REMOVED_HERE.fill(None, &[("name", name), ("device", device)])
    }
}

/// The note once `name` is paired beside the remote in use.
pub fn paired_note(name: &str) -> String {
    copy::REMOTE_PAIRED.fill(None, &[("name", name)])
}

/// The note when a pairing named a remote already in the list.
pub fn already_note(name: &str) -> String {
    copy::REMOTE_ALREADY.fill(None, &[("name", name)])
}

/// Whether `text` is a pairing link, `flankr://<host>:<port>/pair#…`:
/// the test an opened link and a scanned QR code both pass before they
/// are acted on (plan 20261028_02, decision 8). Only the part before the
/// fragment (or a query) is looked at, as the fragment is the pairing
/// string.
pub fn is_pairing_link(text: &str) -> bool {
    let before_fragment = text.trim().split(['#', '?']).next().unwrap_or("");
    before_fragment
        .strip_prefix("flankr://")
        .is_some_and(|rest| rest.ends_with("/pair"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flankr_companion::RemoteId;

    #[test]
    fn a_pairing_link_is_flankr_host_port_pair() {
        assert!(is_pairing_link("flankr://m4.ts.net:7347/pair#flankr1abc"));
        assert!(is_pairing_link(" flankr://127.0.0.1:7347/pair#x \n"));
        assert!(!is_pairing_link("https://example.com"));
        assert!(!is_pairing_link("flankr1abc"));
        assert!(!is_pairing_link("flankr://m4:7347/other#flankr1abc"));
        assert!(!is_pairing_link("flankr://m4:7347/x#/pair"));
        assert!(!is_pairing_link(""));
    }

    const PHONE: Layout = Layout {
        w: 1080.0,
        h: 2340.0,
        scale: 2.0,
        top: 80.0,
        bottom: 40.0,
    };

    const WORDS: &str = "abandon ability able about above absent";
    const FINGERPRINT: &str = "zoo zone youth wrist wrong yellow";

    fn remote(n: u8, active: bool) -> RemoteInfo {
        RemoteInfo {
            id: RemoteId([n; 8]),
            name: format!("m{n}"),
            host: format!("m{n}.ts.net:7347"),
            fingerprint: format!("{n}{n}{n}{n} 0000"),
            paired_at: Some(1_793_188_800 + u64::from(n)),
            active,
            online: active,
        }
    }

    /// A request the daemon holds, received at `now`, with 60 s left.
    fn held(now: Instant) -> Waiting {
        Waiting::new(
            "m4".into(),
            "m4.ts.net:7347".into(),
            Some(WORDS.into()),
            FINGERPRINT.into(),
            Some(60),
            None,
            now,
        )
    }

    fn code() -> String {
        "flankrkey1".to_string() + &"Ab3".repeat(20)
    }

    /// The device-code fallback, with or without words.
    fn fallback(words: bool, replace: bool, now: Instant) -> Waiting {
        Waiting::new(
            "m4".into(),
            "m4.ts.net:7347".into(),
            words.then(|| WORDS.to_string()),
            FINGERPRINT.into(),
            None,
            Some(DeviceCodeFallback {
                code: code(),
                replace,
            }),
            now,
        )
    }

    /// A line's Markdown as it reads: no `**`, no escapes, one space
    /// between words.
    fn plain(md: &str) -> String {
        let mut out = String::new();
        let mut chars = md.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek().is_some_and(char::is_ascii_punctuation) => {
                    out.extend(chars.next())
                }
                '*' if chars.peek() == Some(&'*') => {
                    chars.next();
                }
                c => out.push(c),
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn texts(p: &Pairing, now: Instant) -> Vec<String> {
        p.lines(&PHONE, None, now)
            .into_iter()
            .map(|l| plain(&l.text))
            .collect()
    }

    #[test]
    fn the_quiet_screen_is_the_six_words_large_and_one_line() {
        let now = Instant::now();
        let mut p = Pairing::default();
        assert!(p.lines(&PHONE, None, now).is_empty(), "nothing to say yet");
        p.open_add();
        assert!(p.adding && !p.waiting());
        let hint = texts(&p, now).join(" ");
        assert!(hint.starts_with("Run flankr pair on the machine"), "{hint}");

        p.show(held(now));
        assert!(p.waiting() && !p.details);
        let lines = p.lines(&PHONE, Some(("pairing…", theme().muted)), now);
        let large: Vec<&str> = lines
            .iter()
            .filter(|l| l.size == WORDS_PT)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(large, ["abandon ability able", "about above absent"]);
        assert!(lines.iter().all(|l| l.size <= WORDS_PT));
        let rest: Vec<String> = lines
            .iter()
            .filter(|l| l.size != WORDS_PT)
            .map(|l| plain(&l.text))
            .collect();
        let line = rest[..rest.len() - 1].join(" ");
        assert!(
            lines.iter().any(|l| l.text.contains("**flankr pair**")),
            "the command in bold"
        );
        assert_eq!(
            line,
            "If flankr pair on m4 shows these six words, type yes there."
        );
        assert_eq!(*rest.last().unwrap(), "pairing…", "the note last");
        // Nothing else: no fingerprint, address, countdown or code.
        let all = texts(&p, now).join(" ");
        for hidden in [
            FINGERPRINT,
            "m4.ts.net:7347",
            "expires",
            "flankrkey1",
            "Daemon",
        ] {
            assert!(!all.contains(hidden), "{hidden} in {all}");
        }

        // Cancel ends it, back to Settings.
        assert_eq!(p.cancel(), Input::CancelPairing);
        assert_eq!(p, Pairing::default());
    }

    #[test]
    fn details_show_the_fingerprint_address_and_time_left() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.show(held(now));
        p.toggle_details();
        let lines = p.lines(&PHONE, None, now + Duration::from_millis(15_500));
        let all: Vec<String> = lines.iter().map(|l| plain(&l.text)).collect();
        let all: Vec<&str> = all.iter().map(String::as_str).collect();
        // The hostname first, then the fingerprint (no hex: no link read).
        assert_eq!(all[0], "m4 at m4.ts.net:7347");
        assert_eq!(all[1], "Daemon fingerprint");
        assert_eq!(all[2..4], ["zoo zone youth", "wrist wrong yellow"]);
        assert!(!all.contains(&"BLAKE2s-256 fingerprint"), "{all:?}");
        assert!(all.contains(&"The link expires in 45 s."), "{all:?}");
        // The pairing words are not the fingerprint: not repeated here, and
        // no device code without the fallback.
        assert!(!all.join(" ").contains("abandon"));
        assert!(!all.join(" ").contains("flankrkey1"));
        assert!(lines.iter().all(|l| l.size < WORDS_PT));
        let later = texts(&p, now + Duration::from_secs(61)).join(" ");
        assert!(later.contains("The link has expired"), "{later}");
        // Closed again: the quiet screen.
        p.toggle_details();
        assert!(texts(&p, now)[0].starts_with("abandon"));
    }

    #[test]
    fn the_fallback_keeps_its_code_in_details() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.show(fallback(true, false, now));
        assert!(!p.details, "the words come first");
        let quiet = texts(&p, now);
        assert_eq!(quiet[0], "abandon ability able");
        let line = quiet[2..].join(" ");
        assert!(
            line.starts_with("m4 couldn't be asked directly: paste this phone's code from Details"),
            "{line}"
        );
        assert!(!line.contains("flankrkey1"));

        p.toggle_details();
        let lines = p.lines(&PHONE, None, now);
        let all: Vec<String> = lines.iter().map(|l| plain(&l.text)).collect();
        let all: Vec<&str> = all.iter().map(String::as_str).collect();
        assert!(all.contains(&"Paste this code into flankr pair on m4:"));
        let pasted: String = lines
            .iter()
            .filter(|l| l.size == CODE_PT)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(pasted, code());
        // No countdown in the fallback.
        assert!(!all.join(" ").contains("expire"));
    }

    #[test]
    fn a_link_without_an_offer_opens_details_and_says_there_are_no_words() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.show(fallback(false, false, now));
        assert!(p.details, "the code is all there is to do");
        let all = texts(&p, now).join(" ");
        assert!(all.contains("no pairing words to compare"), "{all}");
        assert!(all.contains("flankrkey1"));
        // Closed: one line pointing at the code.
        p.toggle_details();
        let quiet = texts(&p, now).join(" ");
        assert!(
            quiet.starts_with("m4 couldn't be asked directly: paste this phone's code below"),
            "{quiet}"
        );
    }

    #[test]
    fn a_lost_key_says_flankr_pair_replace_is_needed() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.show(fallback(true, false, now));
        p.toggle_details();
        assert!(!texts(&p, now).join(" ").contains("--replace"));

        p.show(fallback(true, true, now));
        let all = texts(&p, now);
        let at = all
            .iter()
            .position(|t| t.starts_with("Paste this code"))
            .expect("the paste line");
        let warning = all
            .iter()
            .position(|t| t.starts_with("This phone's key for m4 was lost"))
            .expect("the warning");
        assert!(warning < at);
        assert!(all[warning..at].join(" ").contains("flankr pair --replace"));
    }

    #[test]
    fn paired_stays_until_done_with_the_address_and_fingerprint() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.show(held(now));
        p.toggle_details();
        // Paired: the modal closes, and the finished pairing shows.
        assert_eq!(p.paired(&[]), Some("m4".to_string()));
        assert!(!p.waiting() && !p.details);
        let f = p.finished.clone().expect("the finished pairing");
        assert_eq!(f.title(), "Paired with m4");
        assert_eq!(f.summary(), "m4\\.ts\\.net\\:7347", "no row yet");
        // Its row comes: the short fingerprint.
        let mut row = remote(4, true);
        row.name = "m4".into();
        p.remotes_changed(&[remote(1, false), row.clone()]);
        let f = p.finished.clone().unwrap();
        assert_eq!(f.id, Some(row.id));
        assert_eq!(
            f.summary(),
            "m4\\.ts\\.net\\:7347 · fingerprint 4444 0000",
            "the address reached, then the label"
        );
        assert!(!f.updated);
        // Done: on to the overview, it was not the add mode.
        assert!(!p.done());
        assert_eq!(p, Pairing::default());
        // In the add mode, Done returns to Settings.
        p.open_add();
        p.show(held(now));
        p.paired(&[row]);
        assert!(p.adding, "kept until Done");
        assert!(p.done());
        assert!(!p.adding && p.finished.is_none());
        // A failure: the modal closes, and nothing is finished.
        p.show(held(now));
        p.failed();
        assert!(!p.waiting() && p.finished.is_none());
    }

    #[test]
    fn already_paired_says_so_and_whether_the_addresses_changed() {
        let mut p = Pairing::default();
        p.open_add();
        let old = remote(4, true);
        p.remote_updated(old.id, "m4", std::slice::from_ref(&old));
        let f = p.finished.clone().unwrap();
        assert_eq!(f.title(), "Already paired with m4");
        assert_eq!(f.summary(), "m4\\.ts\\.net\\:7347 · fingerprint 4444 0000");
        // The same rows again: nothing changed.
        p.remotes_changed(std::slice::from_ref(&old));
        assert!(!p.finished.as_ref().unwrap().updated);
        // New addresses.
        let mut moved = old.clone();
        moved.host = "10.0.0.4:7347".into();
        p.remotes_changed(&[moved]);
        let f = p.finished.clone().unwrap();
        assert!(f.updated);
        assert_eq!(
            f.summary(),
            "10\\.0\\.0\\.4\\:7347 · fingerprint 4444 0000\n\nIts addresses were updated."
        );
    }

    #[test]
    fn details_show_the_full_fingerprint_from_the_link_read() {
        let now = Instant::now();
        let key = [7u8; 32];
        let url = libflankr_proto::link::PairingPayload {
            server_key: key,
            addresses: vec!["127.0.0.1:7347".into()],
            offer: Some([1; libflankr_proto::link::OFFER_LEN]),
        }
        .url()
        .unwrap();
        let read = LinkRead::of(&url).expect("a pairing link");
        assert_eq!(read.address, "127.0.0.1:7347");
        assert_eq!(host_of(&read.address), "127.0.0.1");
        let hex = fingerprint_hex(&key);
        assert_eq!(hex.len(), 64 + 15, "64 digits in 16 groups");
        let mut p = Pairing::default();
        p.read(Some(read.clone()));
        // The event's fingerprint words match the link's: the hex is shown.
        p.show(Waiting::new(
            read.server.clone(),
            "127.0.0.1:7347".into(),
            Some(WORDS.into()),
            read.words.clone(),
            Some(60),
            None,
            now,
        ));
        p.toggle_details();
        let all = texts(&p, now);
        assert_eq!(all[0], "127.0.0.1 at 127.0.0.1:7347", "{all:?}");
        let groups: Vec<&str> = hex.split(' ').collect();
        let first = format!("`{}`", groups[..4].join(" "));
        assert!(all.contains(&first), "{all:?}");
        assert!(all.contains(&"BLAKE2s-256 fingerprint".to_string()));
        // Words that are not the link's: no hex.
        p.show(held(now));
        assert_eq!(p.waiting.as_ref().unwrap().fingerprint_hex, None);
    }

    #[test]
    fn delete_data_removes_every_remote_the_one_in_use_last() {
        let remotes = [remote(1, false), remote(2, true), remote(3, false)];
        assert_eq!(
            delete_all(&remotes),
            [
                Input::RemoveRemote(RemoteId([1; 8])),
                Input::RemoveRemote(RemoteId([3; 8])),
                Input::RemoveRemote(RemoteId([2; 8])),
            ]
        );
        // None paired: Unpair, which answers Unpaired at once.
        assert_eq!(delete_all(&[]), [Input::Unpair]);
    }

    #[test]
    fn the_notes_say_whether_the_daemon_was_told() {
        let told = removed_note("m4", true, "s23");
        assert!(
            told.contains("Removed m4") && told.contains("`link.unpair`"),
            "{told}"
        );
        let here = removed_note("m4", false, "s23");
        assert!(
            here.contains("on this phone only") && here.contains("`s23`"),
            "{here}"
        );
        assert!(here.contains("`auth.toml`"));
        assert!(removing_note("m4").starts_with("Removing m4"));
        assert!(already_note("m4").contains("addresses are updated"));
        assert!(paired_note("mini").starts_with("Paired with mini."));
    }
}
