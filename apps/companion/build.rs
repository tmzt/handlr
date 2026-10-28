#[path = "build/copy_gen.rs"]
mod copy_gen;
// The app includes it too (src/theme.rs), using what this script does not.
#[allow(dead_code)]
#[path = "build/themes_check.rs"]
mod themes_check;

use std::path::{Path, PathBuf};

/// The `specialUse` reason in the manifest, for Play review: the same text
/// as `notify::SERVICE_REASON` (a test in src/notify.rs holds them to it).
const SERVICE_REASON: &str = "Keeps the encrypted link to the user's own flankr daemon open \
     while the app is in the background, so that coding agents' questions, approvals and \
     finished turns reach the phone.";

fn main() {
    copy();
    themes();
    // Writes target/aarch64-linux-android/<profile>/gradle/companion/, the
    // whole Gradle project that scripts/android.sh packages.
    libmorph_build::gradle_project("dev.flankr.companion")
        .label("flankr")
        // `flankr://<host>:<port>/pair#<pairing string>` links open the app
        // and pair it (libmorph-build 0.2.2).
        .url_scheme("flankr")
        .lib_name("companion")
        // libmorph asks for it after init once the manifest declares it.
        .permission("android.permission.RECORD_AUDIO")
        // The link to flankr, over Tailscale or the LAN.
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

/// Every `.md` file in `copy/` (`SETTINGS_COPY.md`, `PAIRING_COPY.md`), in
/// name order, into one `$OUT_DIR/copy.rs`, checked against the keys `src/`
/// uses (plan 20261024_04, decision 6). libmorph-build prints
/// `rerun-if-changed` lines of its own, which turns off cargo's "any change
/// in the package" rule, so the copy, the sources it is checked against, and
/// the rest of what this script reads are named here.
fn copy() {
    for path in [
        "copy",
        "themes.toml",
        "src",
        "build.rs",
        "build",
        "Cargo.toml",
        "icon",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir.join("copy"))
        .expect("copy/: reading it")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            let text = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("copy/{name}: reading it: {e}"));
            (format!("copy/{name}"), text)
        })
        .collect();
    let mut sources = Vec::new();
    rust_files(&dir.join("src"), &mut sources);
    let used = copy_gen::used_keys(&sources);
    let code = match copy_gen::generate_files(&files, &used) {
        Ok(code) => code,
        Err(e) => panic!("copy: {e}"),
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    std::fs::write(out.join("copy.rs"), code).expect("writing copy.rs");
}

/// `themes.toml`, checked as the app parses it (plan 20261024_05, the
/// 15:33 EDT amendment): a bad file fails the build. Its
/// `rerun-if-changed` line is with the others, in [`copy`].
fn themes() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let text = std::fs::read_to_string(dir.join("themes.toml")).expect("themes.toml: reading it");
    if let Err(e) = themes_check::check(&text) {
        panic!("themes.toml: {e}");
    }
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
