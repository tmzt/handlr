//! The phone's vendor, for the copy's vendor additions (CHANGES.md,
//! 2026-10-24 13:02 and 13:08 EDT): libmorph 0.4.1's `ctx.build_info()`, a
//! `BuildInfo { vendor, make, model }` from `Build.MANUFACTURER`,
//! `Build.BRAND` and `Build.MODEL` on Android, and `None` elsewhere. Only
//! the vendor is used; without it only the copy's base text shows.

use libmorph::MorphContext;

/// What the settings use of libmorph's `BuildInfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// Lower-cased, as the copy's subkeys are: `samsung`, `google`.
    pub vendor: String,
}

impl BuildInfo {
    /// The vendor as `Build.MANUFACTURER` gives it, trimmed and lower-cased.
    pub fn from_vendor(vendor: &str) -> Self {
        BuildInfo {
            vendor: vendor.trim().to_ascii_lowercase(),
        }
    }
}

/// The phone's build info: `None` on the desktop, the web and in scripted
/// runs, and when the vendor is empty.
pub fn build_info(ctx: &MorphContext) -> Option<BuildInfo> {
    ctx.build_info()
        .map(|b| BuildInfo::from_vendor(&b.vendor))
        .filter(|b| !b.vendor.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::copy;

    #[test]
    fn the_vendor_is_lower_cased_as_the_subkeys_are() {
        assert_eq!(BuildInfo::from_vendor("samsung").vendor, "samsung");
        assert_eq!(BuildInfo::from_vendor(" Google ").vendor, "google");
        let samsung = BuildInfo::from_vendor("Samsung");
        assert!(
            copy::BACKGROUND
                .body(Some(&samsung.vendor))
                .contains("Freecess")
        );
    }
}
