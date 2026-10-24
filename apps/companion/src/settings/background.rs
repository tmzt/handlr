//! The Background and Notifications sections' source (plan 20261024_02,
//! CHANGES.md 2026-10-24 12:38 EDT): libmorph 0.4.0's foreground service
//! (the app's `notify.rs`) and its `POST_NOTIFICATIONS` request.
//!
//! The screen asks a [`Background`]; the app gives it [`Morph`], which is
//! ready, and hands the request to the app, which has the context it needs
//! ([`turn_on`]). Nothing is asked at start: the permission is asked for
//! only when *Alert when an agent needs you* is turned on. When Android
//! won't ask again (asked before, and no rationale after the denial), the
//! app's settings page opens instead, where its notifications are. The
//! toggle shows the real permission ([`follow`]) when the screen opens and
//! when the app is back on screen.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

/// The permission the alerts and the service's notification need on
/// Android 13 and later.
pub const POST_NOTIFICATIONS: &str = "android.permission.POST_NOTIFICATIONS";

/// The file in the data directory that says `POST_NOTIFICATIONS` has been
/// asked for once: after that, no rationale means Android won't ask again.
pub const ASKED_FILE: &str = "notifications_asked";

/// The service and the notification permission, on Android.
pub trait Background {
    /// The service and the permission request are there.
    fn ready(&self) -> bool;
    /// Asks for `POST_NOTIFICATIONS`, or opens the app's notification
    /// settings when Android won't ask again.
    fn request_notifications(&self);
}

/// libmorph's: ready, and a request sets the flag the app reads on its
/// next frame, where it has the context to ask.
pub struct Morph(pub Rc<Cell<bool>>);

impl Background for Morph {
    fn ready(&self) -> bool {
        true
    }

    fn request_notifications(&self) {
        self.0.set(true);
    }
}

/// What the permission calls need: the context's on Android, a fake in
/// the tests.
pub trait PermissionOs {
    fn granted(&self) -> bool;
    /// After a denial: Android would show its dialog again.
    fn rationale(&self) -> bool;
    /// Asks; the answer comes later (`on_permissions`).
    fn request(&mut self);
    /// The app's settings page, for its notifications.
    fn open_settings(&mut self);
}

/// What turning the alerts on did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnedOn {
    /// Already granted: nothing asked.
    Granted,
    /// Android's dialog is asked for.
    Requested,
    /// Android won't ask again: its settings opened.
    OpenedSettings,
}

/// Turning *Alert when an agent needs you* on, the only place the
/// permission is asked for. `asked_before`: it was asked for once already.
pub fn turn_on(os: &mut dyn PermissionOs, asked_before: bool) -> TurnedOn {
    if os.granted() {
        TurnedOn::Granted
    } else if asked_before && !os.rationale() {
        os.open_settings();
        TurnedOn::OpenedSettings
    } else {
        os.request();
        TurnedOn::Requested
    }
}

/// The alerts toggle as the permission allows: off without it.
pub fn follow(alerts: bool, granted: bool) -> bool {
    alerts && granted
}

/// Whether the permission has been asked for in `dir`'s app.
pub fn asked_before(dir: &Path) -> bool {
    dir.join(ASKED_FILE).exists()
}

/// Records in `dir` that the permission has been asked for.
pub fn mark_asked(dir: &Path) {
    if let Err(e) = std::fs::write(dir.join(ASKED_FILE), "") {
        log::warn!("notifications: recording the request: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Android's answers, and what was done.
    struct Phone {
        granted: bool,
        rationale: bool,
        done: Vec<&'static str>,
    }

    impl PermissionOs for Phone {
        fn granted(&self) -> bool {
            self.granted
        }
        fn rationale(&self) -> bool {
            self.rationale
        }
        fn request(&mut self) {
            self.done.push("request");
        }
        fn open_settings(&mut self) {
            self.done.push("settings");
        }
    }

    fn phone(granted: bool, rationale: bool) -> Phone {
        Phone {
            granted,
            rationale,
            done: Vec::new(),
        }
    }

    #[test]
    fn granted_asks_nothing() {
        let mut p = phone(true, false);
        assert_eq!(turn_on(&mut p, false), TurnedOn::Granted);
        assert_eq!(turn_on(&mut p, true), TurnedOn::Granted);
        assert!(p.done.is_empty());
    }

    #[test]
    fn the_first_time_and_after_one_denial_android_asks() {
        // Never asked: no rationale yet, and Android shows its dialog.
        let mut p = phone(false, false);
        assert_eq!(turn_on(&mut p, false), TurnedOn::Requested);
        // Denied once: Android gives a rationale, and asks again.
        let mut p = phone(false, true);
        assert_eq!(turn_on(&mut p, true), TurnedOn::Requested);
        assert_eq!(p.done, ["request"]);
    }

    #[test]
    fn a_refusal_that_wont_be_asked_again_opens_the_settings() {
        let mut p = phone(false, false);
        assert_eq!(turn_on(&mut p, true), TurnedOn::OpenedSettings);
        assert_eq!(p.done, ["settings"]);
    }

    #[test]
    fn the_toggle_follows_the_real_permission() {
        assert!(follow(true, true));
        assert!(!follow(true, false), "revoked: off");
        assert!(!follow(false, true), "granted does not turn it on");
    }

    #[test]
    fn being_asked_is_kept_in_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!asked_before(dir.path()));
        mark_asked(dir.path());
        assert!(asked_before(dir.path()));
    }

    #[test]
    fn morph_is_ready_and_hands_the_request_on() {
        let flag = Rc::new(Cell::new(false));
        let m = Morph(Rc::clone(&flag));
        assert!(m.ready());
        assert!(!flag.get(), "nothing asked until turned on");
        m.request_notifications();
        assert!(flag.get());
    }
}
