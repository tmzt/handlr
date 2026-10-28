//! Print the herdr types `build.rs` extracted, for review when the
//! `vendor/herdr` pin moves (plan 20261027_08, decision 2):
//!
//! ```text
//! cargo run -p libherdr_term --example dump_herdr_types
//! ```

fn main() {
    print!("{}", libherdr_term::HERDR_WIRE_RS);
    eprintln!(
        "herdr PROTOCOL_VERSION {}, endpoint generation {}",
        libherdr_term::HERDR_PROTOCOL_VERSION,
        libherdr_term::ENDPOINT_PROTOCOL_GENERATION
    );
}
