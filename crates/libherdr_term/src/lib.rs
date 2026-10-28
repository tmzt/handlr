//! Attaching to the local herdr as one of herdr's own shell clients
//! (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md).
//!
//! * [`herdr`] is herdr's own protocol types, extracted by `build.rs` from
//!   the `vendor/herdr` submodule (decision 2): `ClientMessage`,
//!   `ServerMessage` and every type under them, field for field.
//! * [`Attachment`] speaks herdr's stable endpoint contract, generation 1,
//!   on herdr's client socket (decisions 3, 5, 6 and 7, and the 23:32 EDT
//!   amendment): one blocking `UnixStream`, one reader thread, events
//!   through `std::sync::mpsc`.
//! * [`SurfaceView`] is the tab as herdr last drew it for this client, full
//!   surfaces and patches applied in order; [`SurfaceView::pane`] cuts one
//!   pane out of it.
//! * `From` conversions into `libflankr_term` and from `libflankr_proto`
//!   (decision 8) live in [`convert`].

/// herdr's protocol types, generated from herdr's source at the submodule's
/// pin. herdr is Apache-2.0; see this crate's README.
#[allow(
    dead_code,
    unused_imports,
    clippy::all,
    rustdoc::all,
    missing_docs,
    unreachable_pub
)]
pub mod herdr {
    include!(concat!(env!("OUT_DIR"), "/herdr_wire.rs"));
}

mod attach;
pub mod convert;
mod surface;

pub use attach::{
    AttachError, AttachOptions, Attachment, Event, METHOD_PANE_FOCUS, METHOD_SURFACE_SET,
    Requested, clamp_surface, client_socket_for,
};
pub use convert::{Color32, KeyToken, Modifier16, UnknownMouseKind, key_events, row_from_cells};
pub use surface::{DesktopChange, PaneView, SurfaceView};

/// The text of the generated [`herdr`] module, for review when the pin
/// moves (`cargo run -p libherdr_term --example dump_herdr_types`).
pub const HERDR_WIRE_RS: &str = include_str!(concat!(env!("OUT_DIR"), "/herdr_wire.rs"));

/// herdr's `PROTOCOL_VERSION` at the submodule's pin (22 at `2713feb`),
/// read from herdr's `src/protocol/wire.rs` by `build.rs`. It names the pin
/// in logs; flankr does not speak the direct-terminal protocol it guards.
pub const HERDR_PROTOCOL_VERSION: u32 = herdr::PROTOCOL_VERSION;

/// The endpoint generation flankr speaks (herdr's
/// `ENDPOINT_PROTOCOL_GENERATION`, 1).
pub const ENDPOINT_PROTOCOL_GENERATION: u32 = herdr::ENDPOINT_PROTOCOL_GENERATION;
