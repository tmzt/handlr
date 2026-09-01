//! Adds the rpath entries the sherpa-onnx prebuilt dylibs need at load time.
//!
//! `sherpa-onnx-sys`'s build script emits `cargo:rustc-link-arg=-Wl,-rpath,...`
//! for exactly this, but cargo does not propagate `rustc-link-arg` from a
//! *dependency's* build script to a downstream binary — only `link-search` and
//! `link-lib` cross that boundary. The binary therefore links cleanly against
//! `@rpath/libsherpa-onnx-c-api.dylib` and then dies at startup with
//!
//! ```text
//! dyld: Library not loaded: @rpath/libsherpa-onnx-c-api.dylib
//!   Reason: no LC_RPATH's found
//! ```
//!
//! Emitting the same rpath here, from the package that owns the binary, is what
//! actually reaches the linker.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // Not needed off-macOS: ELF targets resolve these through the search path
    // and $ORIGIN handling the sys crate already emits for its own artifacts.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // The prebuilt dylibs are copied next to the binary in the target dir.
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path");

    // ...and also live in the sys crate's download cache, which is where they
    // sit when the copy beside the binary has not happened yet.
    if let Some(lib_dir) = prebuilt_lib_dir() {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{}", lib_dir.display());
    }
}

/// Locate `<target-dir>/sherpa-onnx-prebuilt/<release>/lib`, the directory
/// `sherpa-onnx-sys` unpacks its download into. The release subdirectory is
/// version- and platform-stamped, so it is discovered rather than hardcoded.
fn prebuilt_lib_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    let target_dir = match std::env::var("CARGO_TARGET_DIR") {
        Ok(explicit) => PathBuf::from(explicit),
        // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out
        Err(_) => out_dir.ancestors().nth(4)?.to_path_buf(),
    };

    let root = target_dir.join("sherpa-onnx-prebuilt");
    let entries = std::fs::read_dir(&root).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("lib"))
        .find(|lib: &PathBuf| lib.is_dir())
        .filter(|lib| Path::new(lib).exists())
}
