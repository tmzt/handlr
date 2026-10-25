//! `libflankr_proto` — the wire types of flankr's protocols
//! (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md, decisions 2, 3
//! and 7).
//!
//! Every flankr protocol is JSON-RPC 2.0, one object per line. A connection
//! names the protocol it speaks in its first request, `hello`:
//!
//! * `flankr.harness`: structured orchestration for programs (plan 03);
//! * `flankr.companion`: conversational turns for the phone (plan 02).
//!
//! The methods both share are in [`common`]; the companion's own are in
//! [`companion`]. This crate depends on serde
//! alone, so clients can use it without flankr.
//!
//! The `link` feature adds [`link`], the encrypted transport (Noise over
//! any Tokio byte stream), the pairing string and the device code (plan 02,
//! decisions 1–2, as amended at 23:07 EDT).

pub mod common;
pub mod companion;
pub mod envelope;
#[cfg(feature = "link")]
pub mod link;

pub use envelope::{ErrorObject, Id, Incoming, Notification, Request, Response};
