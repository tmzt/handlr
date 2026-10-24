//! The Battery section's source (decision 3): Android's battery
//! optimization and the vendor's battery page, as libmobile 0.2.3 has
//! them (`state`, `open_settings`, `request_ignore_optimizations`,
//! `open_vendor_battery`), reached through libmorph's `service::battery`
//! wrapper (libmorph plan 20261023_01, step 5).
//!
//! That wrapper is not released yet, so the screen asks a [`Battery`], and
//! the app gives it [`Unreleased`], which knows nothing: the section then
//! says "Battery settings need an app update" (step 5). Once the wrapper is
//! released, a [`Battery`] on it takes `Unreleased`'s place.

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
        self.manufacturer.eq_ignore_ascii_case("samsung")
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
    /// The state, or `None` when it cannot be known (the wrapper is not
    /// released).
    fn state(&self) -> Option<BatteryState>;
    /// Android's battery optimization page: the request to be exempt when
    /// not yet `exempt`, else the settings page.
    fn open_optimization(&self, exempt: bool);
    /// The vendor's battery page.
    fn open_vendor(&self) -> VendorPage;
}

/// Until libmorph's battery wrapper is released: nothing is known, and
/// nothing opens.
pub struct Unreleased;

impl Battery for Unreleased {
    fn state(&self) -> Option<BatteryState> {
        None
    }

    fn open_optimization(&self, _exempt: bool) {
        log::info!("battery: libmorph's battery wrapper is not released yet");
    }

    fn open_vendor(&self) -> VendorPage {
        VendorPage::None
    }
}
