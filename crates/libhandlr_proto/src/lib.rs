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

pub mod common;
pub mod envelope;

pub use envelope::{ErrorObject, Id, Incoming, Notification, Request, Response};
