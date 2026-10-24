//! The phone's vendor, for the copy's vendor additions (CHANGES.md,
//! 2026-10-24 13:02 EDT). It will come from libmorph 0.4.0's
//! `ctx.build_info()`, a `BuildInfo { vendor, make, model }` from
//! `Build.MANUFACTURER`, `Build.BRAND` and `Build.MODEL` on Android, and
//! `None` elsewhere. Only the vendor is used.
//!
//! libmorph 0.4.0 is not wired in yet, so [`build_info`] is `None` and only
//! the copy's base text shows, as the battery rows wait behind
//! `battery::Unreleased`.

/// What the settings use of libmorph's `BuildInfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// Lower-cased, as the copy's subkeys are: `samsung`, `google`.
    pub vendor: String,
}

/// The phone's build info; `None` until libmorph 0.4.0's `ctx.build_info()`.
pub fn build_info() -> Option<BuildInfo> {
    None
}
