//! Shared by the tests: herdr's checkout, and an in-process fake herdr
//! server that speaks the extracted types (`src/fake.rs`, which the
//! `fake-server` feature also exports as `libherdr_term::fake`). Nothing
//! here touches a real herdr socket.

#![allow(dead_code)]

use libherdr_term::herdr;

#[path = "../../src/fake.rs"]
mod fake;

pub use fake::*;
