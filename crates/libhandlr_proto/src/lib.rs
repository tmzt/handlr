//! `libhandlr_proto` — the wire types of handlr's protocols
//! (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md, decisions 2, 3
//! and 7).
//!
//! Every handlr protocol is JSON-RPC 2.0, one object per line. A connection
//! names the protocol it speaks in its first request, `hello`:
//!
//! * `handlr.harness`: structured orchestration for programs (plan 03);
//! * `handlr.companion`: conversational turns for the phone (plan 02).
//!
//! The methods both share are in [`common`]. This crate depends on serde
//! alone, so clients can use it without handlr.
//!
//! The `link` feature adds [`link`], the encrypted transport (Noise over
//! any Tokio byte stream) and the pairing string (plan 02, decisions 1–2).

pub mod common;
pub mod envelope;
#[cfg(feature = "link")]
pub mod link;

pub use envelope::{ErrorObject, Id, Incoming, Notification, Request, Response};
