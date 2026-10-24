//! The Battery section's source (decision 3): Android's battery
//! optimization and the vendor's battery page, through libmorph 0.4.0's
//! `libmorph::battery` (`state`, `request_ignore_optimizations`,
//! `open_settings`, `open_vendor_battery`).
//!
//! The screen asks a [`Battery`], and the app gives it [`Morph`]. Where the
//! state cannot be known (not Android), the section says "Battery settings
//! need an app update"; it shows on Android only.

/// What the phone says of the app's battery use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatteryState {
    /// Exempt from battery optimization.
    pub ignoring_optimizations: bool,
    /// Battery use is *Restricted*: the app cannot run in the background.
    pub background_restricted: bool,
    /// `Build.MANUFACTURER`, such as `samsung`.
    pub manufacturer: String,
}

impl BatteryState {
    pub fn is_samsung(&self) -> bool {
        self.manufacturer.trim().eq_ignore_ascii_case("samsung")
    }

    /// The vendor row's label, when the phone has a vendor page the app
    /// knows of.
    pub fn vendor_label(&self) -> Option<&'static str> {
        self.is_samsung()
            .then_some(super::copy::SAMSUNG_BATTERY.label())
    }
}

/// How opening the vendor's battery page went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VendorPage {
    Opened,
    /// The phone has no such page: its row is hidden from then on.
    None,
}

/// The battery pages, on Android.
pub trait Battery {
    /// The state, or `None` when it cannot be known (no battery settings
    /// on this host).
    fn state(&self) -> Option<BatteryState>;
    /// Android's battery optimization page: the request to be exempt when
    /// not yet `exempt`, else the settings page.
    fn open_optimization(&self, exempt: bool);
    /// The vendor's battery page.
    fn open_vendor(&self) -> VendorPage;
}

/// libmorph's battery pages: the real state on Android with
/// `mobile-battery`; elsewhere every call is `Unsupported`, so the state is
/// unknown and nothing opens.
pub struct Morph;

impl Battery for Morph {
    fn state(&self) -> Option<BatteryState> {
        match libmorph::battery::state() {
            Ok(s) => Some(BatteryState {
                ignoring_optimizations: s.ignoring_optimizations,
                background_restricted: s.background_restricted,
                manufacturer: s.manufacturer,
            }),
            Err(e) => {
                log::info!("battery: the state: {e}");
                None
            }
        }
    }

    /// Not yet exempt: Android's Allow/Deny dialog
    /// (`request_ignore_optimizations`, declared in build.rs). Exempt: the
    /// list of every app's setting, to turn it back.
    fn open_optimization(&self, exempt: bool) {
        let opened = if exempt {
            libmorph::battery::open_settings().map(drop)
        } else {
            libmorph::battery::request_ignore_optimizations()
        };
        if let Err(e) = opened {
            log::warn!("battery: optimization: {e}");
        }
    }

    fn open_vendor(&self) -> VendorPage {
        vendor_page(libmorph::battery::open_vendor_battery())
    }
}

/// What `open_vendor_battery` answered, for the vendor row: the vendor's
/// page opened, or libmobile fell back to the app's own page because the
/// phone has none, which hides the row from then on. A failure (the app
/// hidden, say) keeps the row, to be tried again.
pub fn vendor_page(
    opened: Result<libmorph::battery::Page, libmorph::battery::BatteryError>,
) -> VendorPage {
    use libmorph::battery::Page;
    match opened {
        Ok(Page::Vendor) => VendorPage::Opened,
        Ok(_) => {
            log::info!("battery: no vendor page; the app's settings opened");
            VendorPage::None
        }
        Err(e) => {
            log::warn!("battery: the vendor's page: {e}");
            VendorPage::Opened
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libmorph::battery::{BatteryError, Page};

    #[test]
    fn the_vendor_row_goes_only_when_the_phone_has_no_vendor_page() {
        assert_eq!(vendor_page(Ok(Page::Vendor)), VendorPage::Opened);
        assert_eq!(vendor_page(Ok(Page::AppDetails)), VendorPage::None);
        assert_eq!(vendor_page(Ok(Page::OptimizationList)), VendorPage::None);
        assert_eq!(
            vendor_page(Err(BatteryError::NotVisible)),
            VendorPage::Opened,
            "a failure keeps the row"
        );
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn off_android_the_state_is_unknown() {
        assert_eq!(Morph.state(), None);
        // Unsupported: logged, and the row stays.
        Morph.open_optimization(false);
        assert_eq!(Morph.open_vendor(), VendorPage::Opened);
    }

    #[test]
    fn samsung_phones_have_the_vendor_row() {
        let state = |m: &str| BatteryState {
            ignoring_optimizations: false,
            background_restricted: false,
            manufacturer: m.into(),
        };
        assert_eq!(
            state(" Samsung ").vendor_label(),
            Some("Samsung battery (app sleeping)")
        );
        assert_eq!(state("Google").vendor_label(), None);
    }
}
