//! The app's side of the paired remotes (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md,
//! step 5): the pairing screen's state and lines, the notes a removal or a
//! pairing leaves in Settings' Remotes, and *Delete data*'s removals.
//!
//! - [`Pairing`]: whether the pairing screen is in its **add** mode (the
//!   **+** in Remotes, decision 9) and the device code shown while a
//!   pairing waits, with its six-digit **confirmation code** (the 13:03
//!   EDT amendment, item 7). Cancel, the add mode's "<" and the system
//!   Back all end it with [`Input::CancelPairing`].
//! - [`Pairing::lines`]: the pairing area above the field: the device
//!   code to paste into `flankr pair`, the confirmation code, large, with
//!   what to check it against, and any note.
//! - [`delete_all`]: *Delete data* removes every remote
//!   ([`Input::RemoveRemote`]), the one in use last; with none, it asks
//!   for [`Input::Unpair`], which answers [`flankr_companion::Event::Unpaired`]
//!   at once. The data directory goes once `Unpaired` comes.

use flankr_companion::{Input, RemoteInfo};

use crate::settings::copy;
use crate::theme::{Role, theme};
use crate::ui::{Layout, Line, chunk, wrap};

/// The confirmation code's size, in points: large, to be read against the
/// Mac's terminal at a glance.
pub const CONFIRMATION_PT: f32 = 48.0;
/// The notes' and labels' size, in points.
const NOTE_PT: f32 = 15.0;
/// The device code's size, in points.
const CODE_PT: f32 = 26.0;

/// A device code the client gave ([`flankr_companion::Event::PairingCode`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    /// `flankrkey1…`, to paste into `flankr pair`.
    pub code: String,
    /// The daemon's name, from the pairing URL.
    pub server: String,
    /// Six digits from the daemon's static key, as `flankr pair` prints.
    pub confirmation: String,
    /// The remote is listed and this phone's key for it was lost: the
    /// daemon adds this code only with `flankr pair --replace`.
    pub replace: bool,
}

/// The pairing screen's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    /// The add mode, opened by the **+** in Remotes, over Settings.
    pub adding: bool,
    /// The device code, while a pairing waits for it to be added.
    pub code: Option<Code>,
}

impl Pairing {
    /// The **+**: the add mode, with nothing pairing yet.
    pub fn open_add(&mut self) {
        self.adding = true;
        self.code = None;
    }

    /// A pairing began: its code is shown, and the field gives way to
    /// Cancel.
    pub fn show(&mut self, code: Code) {
        self.code = Some(code);
    }

    /// A pairing waits for its code to be added.
    pub fn waiting(&self) -> bool {
        self.code.is_some()
    }

    /// Cancel, the add mode's "<", or the system Back: the pairing in
    /// progress ends (its new key deleted by the client) and the add mode
    /// closes, back to Settings. The input to send.
    pub fn cancel(&mut self) -> Input {
        self.adding = false;
        self.code = None;
        Input::CancelPairing
    }

    /// The daemon added the code: the code goes, and the add mode closes.
    /// The daemon's name, if a code was shown, and whether it was the add
    /// mode (so the app returns to Settings).
    pub fn paired(&mut self) -> (Option<String>, bool) {
        let server = self.code.take().map(|c| c.server);
        (server, std::mem::take(&mut self.adding))
    }

    /// A pairing failed or ended: its code goes, and the field shows again.
    pub fn failed(&mut self) {
        self.code = None;
    }

    /// The pairing area's lines: in the add mode with nothing pairing,
    /// what to do; while a pairing waits, the device code, then the
    /// confirmation code, large, and what to check it against; then the
    /// note, if any.
    pub fn lines(&self, layout: &Layout, note: Option<(&str, [f32; 4])>) -> Vec<Line> {
        let line = |text: String, size: f32, color, gap| Line {
            text,
            size,
            color,
            gap,
        };
        let mut out = Vec::new();
        let para = |out: &mut Vec<Line>, text: &str, color, gap: f32| {
            for (i, t) in wrap(text.trim(), layout.columns(NOTE_PT))
                .into_iter()
                .enumerate()
            {
                out.push(line(t, NOTE_PT, color, if i == 0 { gap } else { 0.0 }));
            }
        };
        match &self.code {
            None if self.adding => para(&mut out, copy::REMOTE_ADD.text, theme().muted, 0.0),
            None => {}
            Some(c) => {
                let server = [("server", c.server.as_str())];
                if c.replace {
                    para(
                        &mut out,
                        &copy::PAIRING_REPLACE.fill(None, &server),
                        theme().get(Role::Warn),
                        0.0,
                    );
                }
                para(
                    &mut out,
                    &copy::PAIRING_PASTE.fill(None, &server),
                    theme().muted,
                    if c.replace { 8.0 } else { 0.0 },
                );
                // Base58 is mostly capitals and wide letters: wider than the
                // half an em `columns` assumes.
                for part in chunk(&c.code, layout.columns(CODE_PT * 1.3)) {
                    out.push(line(part, CODE_PT, theme().code, 4.0));
                }
                out.push(line(
                    copy::PAIRING_CONFIRMATION.text.trim().to_string(),
                    NOTE_PT,
                    theme().muted,
                    16.0,
                ));
                out.push(line(
                    c.confirmation.clone(),
                    CONFIRMATION_PT,
                    theme().text,
                    0.0,
                ));
                para(
                    &mut out,
                    &copy::PAIRING_CONFIRMATION_CHECK.fill(None, &server),
                    theme().muted,
                    4.0,
                );
            }
        }
        if let Some((note, color)) = note {
            para(&mut out, note, color, 8.0);
        }
        out
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

    fn code() -> Code {
        Code {
            code: "flankrkey1".to_string() + &"Ab3".repeat(20),
            server: "m4".into(),
            confirmation: "543268".into(),
            replace: false,
        }
    }

    #[test]
    fn a_lost_key_says_flankr_pair_replace_is_needed() {
        let mut p = Pairing::default();
        p.show(code());
        let plain: Vec<String> = p.lines(&PHONE, None).into_iter().map(|l| l.text).collect();
        assert!(!plain.join(" ").contains("--replace"), "{plain:?}");

        p.show(Code {
            replace: true,
            ..code()
        });
        let texts: Vec<String> = p.lines(&PHONE, None).into_iter().map(|l| l.text).collect();
        let at = texts
            .iter()
            .position(|t| t.starts_with("Paste this code"))
            .expect("the paste line");
        let before = texts[..at].join(" ");
        assert!(
            before.starts_with("This phone's key for m4 was lost")
                && before.contains("flankr pair --replace"),
            "{before}"
        );
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
    fn the_add_mode_shows_the_confirmation_code_large_and_cancel_ends_it() {
        let mut p = Pairing::default();
        assert!(p.lines(&PHONE, None).is_empty(), "nothing to say yet");
        p.open_add();
        assert!(p.adding && !p.waiting());
        let hint: String = p
            .lines(&PHONE, None)
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(hint.starts_with("Run flankr pair on the machine"), "{hint}");

        p.show(code());
        assert!(p.waiting());
        let lines = p.lines(&PHONE, Some(("pairing…", theme().muted)));
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts[0], "Paste this code into flankr pair on m4:");
        let at = texts.iter().position(|t| *t == "543268").expect("the code");
        assert_eq!(texts[at - 1], "Confirmation code");
        assert_eq!(lines[at].size, CONFIRMATION_PT, "large");
        assert!(lines.iter().all(|l| l.size <= CONFIRMATION_PT));
        let check = texts[at + 1..].join(" ");
        assert!(
            check.starts_with(
                "Check that it matches the confirmation code flankr pair printed on m4."
            ),
            "{check}"
        );
        assert_eq!(*texts.last().unwrap(), "pairing…", "the note last");
        // The device code, whole, across its lines.
        let pasted: String = lines
            .iter()
            .filter(|l| l.size == CODE_PT)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(pasted, code().code);

        // Cancel ends it, back to Settings.
        assert_eq!(p.cancel(), Input::CancelPairing);
        assert_eq!(p, Pairing::default());
    }

    #[test]
    fn the_first_pairing_shows_the_confirmation_code_too() {
        let mut p = Pairing::default();
        p.show(code());
        let texts: Vec<String> = p.lines(&PHONE, None).into_iter().map(|l| l.text).collect();
        assert!(texts.iter().any(|t| t == "543268"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Confirmation code"));
        // Paired: the code goes; it was not the add mode.
        assert_eq!(p.paired(), (Some("m4".to_string()), false));
        assert!(!p.waiting());
        // In the add mode, pairing closes it.
        p.open_add();
        p.show(code());
        assert_eq!(p.paired(), (Some("m4".to_string()), true));
        assert!(!p.adding);
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
