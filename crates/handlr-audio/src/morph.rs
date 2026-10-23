//! libmorph adapters (feature `morph`): the microphone's end as a libmorph
//! `Sink`, and the speaker's as a `Source`.
//!
//! Copied from libvoicexn's `src/morph.rs` at v0.2.0
//! (`32b8032082997a34190b458f79716d998a0d850a`), so this crate names its
//! own libmorph revision instead of the one libvoicexn's `morph` feature
//! pins. The source wraps [`Playback`] rather than libvoicexn's `Consumer`,
//! for the session's flush.

use libmorph::{Captured, Sink, Source};
use libvoicexn::capture::Mic;

use crate::Playback;

/// The libmorph `Sink` that feeds `mic`.
pub(crate) fn sink(mut mic: Mic) -> Sink {
    Box::new(move |captured: &Captured<'_>| mic.push(captured.samples))
}

/// The libmorph `Source` that plays the ring.
pub(crate) fn source(mut playback: Playback) -> Source {
    Box::new(move |out: &mut [f32]| playback.fill(out))
}
