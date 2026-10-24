//! The Background and Notifications sections' source (plan 20261024_02,
//! CHANGES.md 2026-10-24 12:38 EDT): libmorph 0.4.0's foreground service
//! and its `POST_NOTIFICATIONS` request.
//!
//! libmorph 0.4.0 is not released, so the screen asks a [`Background`],
//! and the app gives it [`Unreleased`]: both toggles are kept in
//! `settings.toml`, and while one is on the section says it needs an app
//! update. Nothing is asked for at start; the permission is asked for only
//! when *Alert when an agent needs you* is turned on.

/// The service and the notification permission, on Android.
pub trait Background {
    /// The service and the permission request are there (libmorph 0.4.0).
    fn ready(&self) -> bool;
    /// Asks for `POST_NOTIFICATIONS`, or opens the app's notification
    /// settings when Android won't ask again.
    fn request_notifications(&self);
}

/// Until libmorph 0.4.0: not ready, and nothing is asked.
pub struct Unreleased;

impl Background for Unreleased {
    fn ready(&self) -> bool {
        false
    }

    fn request_notifications(&self) {
        log::info!("notifications: libmorph 0.4.0's permission request is not released yet");
    }
}
