#[path = "build/copy_gen.rs"]
mod copy_gen;

use std::path::{Path, PathBuf};

/// The `specialUse` reason in the manifest, for Play review: the same text
/// as `notify::SERVICE_REASON` (a test in src/notify.rs holds them to it).
const SERVICE_REASON: &str = "Keeps the encrypted link to the user's own handlr daemon open \
     while the app is in the background, so that coding agents' questions, approvals and \
     finished turns reach the phone.";

fn main() {
    settings_copy();
    // Writes target/aarch64-linux-android/<profile>/gradle/companion/, the
    // whole Gradle project that scripts/android.sh packages.
    libmorph_build::gradle_project("dev.tmzt.handlr")
        .label("handlr")
        // `handlr://<host>:<port>/pair#<pairing string>` links open the app
        // and pair it (libmorph-build 0.2.2).
        .url_scheme("handlr")
        .lib_name("companion")
        // libmorph asks for it after init once the manifest declares it.
        .permission("android.permission.RECORD_AUDIO")
        // The link to handlr, over Tailscale or the LAN.
        .permission("android.permission.INTERNET")
        // The launcher icon: the magenta circuit dog (icon/make_icon.py) on
        // its navy, made into every mipmap by libmorph-build 0.2.5.
        .icon(
            libmorph_build::gradle::Icon::png("icon/artwork-1024.png")
                .background_color([0x0F, 0x0F, 0x26])
                .monochrome_png("icon/artwork-1024.png"),
        )
        // *Stay connected in the background*: libmorph 0.4.0's foreground
        // service, `specialUse`, with POST_NOTIFICATIONS and WAKE_LOCK
        // (src/notify.rs, whose SERVICE_REASON is the same text).
        .foreground_service(&[libmorph_build::gradle::ServiceType::SpecialUse(
            SERVICE_REASON,
        )])
        // *Battery optimization*'s Allow/Deny dialog (libmorph 0.4.0's
        // `battery::request_ignore_optimizations`); not a Play build.
        .request_ignore_battery_optimizations()
        .generate()
        .expect("libmorph-build: generating the Gradle project");
}

/// `SETTINGS_COPY.md` into `$OUT_DIR/settings_copy.rs`, checked against the
/// keys `src/` uses (plan 20261024_04, decision 6). libmorph-build prints
/// `rerun-if-changed` lines of its own, which turns off cargo's "any change
/// in the package" rule, so the copy, the sources it is checked against, and
/// the rest of what this script reads are named here.
fn settings_copy() {
    for path in [
        "SETTINGS_COPY.md",
        "src",
        "build.rs",
        "build",
        "Cargo.toml",
        "icon",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let md = std::fs::read_to_string(dir.join("SETTINGS_COPY.md"))
        .expect("SETTINGS_COPY.md: reading it");
    let mut sources = Vec::new();
    rust_files(&dir.join("src"), &mut sources);
    let used = copy_gen::used_keys(&sources);
    let code = match copy_gen::generate(&md, &used) {
        Ok(code) => code,
        Err(e) => panic!("SETTINGS_COPY.md: {e}"),
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    std::fs::write(out.join("settings_copy.rs"), code).expect("writing settings_copy.rs");
}

/// The text of every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push(text);
        }
    }
}
