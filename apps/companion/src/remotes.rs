//! The app's side of the paired remotes (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md,
//! step 5): the pairing screen's state and lines, the notes a removal or a
//! pairing leaves in Settings' Remotes, and *Delete data*'s removals.
//!
//! - [`Pairing`]: whether the pairing screen is in its **add** mode (the
//!   **+** in Remotes, decision 9), what a waiting pairing shows
//!   ([`Waiting`], from [`flankr_companion::Event::PairingWords`]) and
//!   whether its **Details** are open. Cancel, the add mode's "<" and the
//!   system Back all end it with [`Input::CancelPairing`].
//! - [`Pairing::lines`]: the pairing area above the buttons. Quiet, as the
//!   14:42 EDT amendment has it: the six pairing words, large, and one line
//!   saying to confirm on the Mac. Its Details: the daemon's fingerprint
//!   words, its `host:port`, the time the link has left, and in the
//!   device-code fallback the code to paste into `flankr pair`.
//! - [`delete_all`]: *Delete data* removes every remote
//!   ([`Input::RemoveRemote`]), the one in use last; with none, it asks
//!   for [`Input::Unpair`], which answers [`flankr_companion::Event::Unpaired`]
//!   at once. The data directory goes once `Unpaired` comes.

use std::time::{Duration, Instant};

use flankr_companion::{DeviceCodeFallback, Input, RemoteInfo};

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
/// The device code's size, in points.
const CODE_PT: f32 = 26.0;
/// The pairing words on one line: two lines of three, as `flankr pair`
/// prints six in a row.
const WORDS_PER_LINE: usize = 3;

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
            expires_at: expires_in_secs.map(|s| now + Duration::from_secs(u64::from(s))),
            fallback,
        }
    }
}

/// The pairing screen's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    /// The add mode, opened by the **+** in Remotes, over Settings.
    pub adding: bool,
    /// What a waiting pairing shows.
    pub waiting: Option<Waiting>,
    /// Its Details are open.
    pub details: bool,
}

impl Pairing {
    /// The **+**: the add mode, with nothing pairing yet.
    pub fn open_add(&mut self) {
        self.adding = true;
        self.waiting = None;
        self.details = false;
    }

    /// A pairing waits: its words are shown, and the field gives way to
    /// Details and Cancel. With no words to show (a link without an offer)
    /// Details open at once, as the code there is all there is to do.
    pub fn show(&mut self, waiting: Waiting) {
        if waiting.words.is_none() {
            self.details = true;
        } else if self.waiting.as_ref().map(|w| &w.words) != Some(&waiting.words) {
            self.details = false;
        }
        self.waiting = Some(waiting);
    }

    /// A pairing waits for the owner.
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

    /// The daemon added the key: the words go, and the add mode closes.
    /// The daemon's name, if a pairing was shown, and whether it was the
    /// add mode (so the app returns to Settings).
    pub fn paired(&mut self) -> (Option<String>, bool) {
        let server = self.waiting.take().map(|w| w.server);
        self.details = false;
        (server, std::mem::take(&mut self.adding))
    }

    /// A pairing failed or ended: its words go, and the field shows again.
    pub fn failed(&mut self) {
        self.waiting = None;
        self.details = false;
    }

    /// The pairing area's lines at `now`: in the add mode with nothing
    /// pairing, what to do; while a pairing waits, its words, large, and
    /// one line, or its Details; then the note, if any.
    pub fn lines(
        &self,
        layout: &Layout,
        note: Option<(&str, [f32; 4])>,
        now: Instant,
    ) -> Vec<Line> {
        let mut out = Vec::new();
        match &self.waiting {
            None if self.adding => {
                para(&mut out, layout, copy::REMOTE_ADD.text, theme().muted, 0.0)
            }
            None => {}
            Some(w) if self.details => details(&mut out, layout, w, now),
            Some(w) => {
                let server = [("server", w.server.as_str())];
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

/// `text` wrapped at the notes' size, its first line `gap` below the last.
fn para(out: &mut Vec<Line>, layout: &Layout, text: &str, color: [f32; 4], gap: f32) {
    for (i, t) in wrap(text.trim(), layout.columns(NOTE_PT))
        .into_iter()
        .enumerate()
    {
        out.push(Line {
            text: t,
            size: NOTE_PT,
            color,
            gap: if i == 0 { gap } else { 0.0 },
        });
    }
}

/// Six words in lines of [`WORDS_PER_LINE`] at `size`, each wrapped if
/// still too wide.
fn word_rows(out: &mut Vec<Line>, layout: &Layout, words: &str, size: f32, color: [f32; 4]) {
    let words: Vec<&str> = words.split_whitespace().collect();
    for row in words.chunks(WORDS_PER_LINE) {
        for t in wrap(&row.join(" "), layout.columns(size)) {
            out.push(Line {
                text: t,
                size,
                color,
                gap: 4.0,
            });
        }
    }
}

/// The pairing words, large.
fn words_lines(out: &mut Vec<Line>, layout: &Layout, words: &str) {
    word_rows(out, layout, words, WORDS_PT, theme().text);
}

/// The Details: the daemon's fingerprint words and `host:port`, the time
/// the link has left, then the device-code fallback's code (or, without
/// words, why there are none).
fn details(out: &mut Vec<Line>, layout: &Layout, w: &Waiting, now: Instant) {
    let server = [("server", w.server.as_str())];
    para(
        out,
        layout,
        copy::PAIRING_FINGERPRINT.text,
        theme().muted,
        0.0,
    );
    word_rows(out, layout, &w.fingerprint, FINGERPRINT_PT, theme().text);
    let address = copy::PAIRING_ADDRESS.fill(
        None,
        &[
            ("server", w.server.as_str()),
            ("address", w.address.as_str()),
        ],
    );
    para(out, layout, &address, theme().muted, 12.0);
    if let Some(at) = w.expires_at {
        let left = at.saturating_duration_since(now);
        let text = if left.is_zero() {
            copy::PAIRING_EXPIRED.text.to_string()
        } else {
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            copy::PAIRING_EXPIRES.fill(None, &[("seconds", &secs.to_string())])
        };
        para(out, layout, &text, theme().muted, 4.0);
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

/// The note once the first remote is paired: "Paired with <name>."
pub fn first_paired_note(name: &str) -> String {
    copy::PAIRING_PAIRED
        .fill(None, &[("name", name)])
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flankr_companion::RemoteId;

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

    fn texts(p: &Pairing, now: Instant) -> Vec<String> {
        p.lines(&PHONE, None, now)
            .into_iter()
            .map(|l| l.text)
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
        let rest: Vec<&str> = lines
            .iter()
            .filter(|l| l.size != WORDS_PT)
            .map(|l| l.text.as_str())
            .collect();
        let line = rest[..rest.len() - 1].join(" ");
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
        let all: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(all[0], "Daemon fingerprint");
        assert_eq!(all[1..3], ["zoo zone youth", "wrist wrong yellow"]);
        assert!(all.contains(&"m4 at m4.ts.net:7347"), "{all:?}");
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
        let all: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
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
    fn paired_closes_the_add_mode_and_names_the_daemon() {
        let now = Instant::now();
        let mut p = Pairing::default();
        p.show(held(now));
        p.toggle_details();
        // Paired: the words go; it was not the add mode.
        assert_eq!(p.paired(), (Some("m4".to_string()), false));
        assert!(!p.waiting() && !p.details);
        // In the add mode, pairing closes it.
        p.open_add();
        p.show(held(now));
        assert_eq!(p.paired(), (Some("m4".to_string()), true));
        assert!(!p.adding);
        // A failure: the field again.
        p.show(held(now));
        p.failed();
        assert!(!p.waiting());
        assert_eq!(first_paired_note("m4"), "Paired with m4.");
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
