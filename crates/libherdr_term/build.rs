//! Generates `$OUT_DIR/herdr_wire.rs`, `libherdr_term::herdr`, from herdr's
//! own source in the `vendor/herdr` submodule (or `HERDR_SRC`)
//! (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md, decision 2).

#[path = "build/extract.rs"]
mod extract;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=HERDR_SRC");
    println!("cargo:rerun-if-changed=build/extract.rs");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let herdr_src = std::env::var("HERDR_SRC").ok();
    let root = extract::herdr_root(&manifest_dir, herdr_src.as_deref());
    for source in extract::SOURCES {
        println!("cargo:rerun-if-changed={}", root.join(source).display());
    }
    let extracted = match extract::extract(&root) {
        Ok(extracted) => extracted,
        Err(err) => {
            // One readable line, not a backtrace.
            eprintln!("error: {err}");
            println!("cargo:warning={err}");
            std::process::exit(1);
        }
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("herdr_wire.rs");
    std::fs::write(&out, extracted.code).expect("write herdr_wire.rs");
    let _ = extracted.protocol_version;
}
