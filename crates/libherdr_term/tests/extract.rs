//! build.rs's extractor on herdr's source and on broken copies of it
//! (plan 20261027_08, step 2: "the build fails cleanly with `HERDR_SRC`
//! pointing at a copy where one kept type names `crate::foo`"). The
//! extractor runs here as a library, the same module build.rs includes,
//! rather than by spawning cargo on the copy: the result is the same error
//! text build.rs prints, without a second build of the crate.

mod common;

#[path = "../build/extract.rs"]
mod extract;

use std::path::Path;

fn copy_sources(to: &Path) {
    let root = common::herdr_root();
    for rel in extract::SOURCES {
        let dest = to.join(rel);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(root.join(rel), dest).unwrap();
    }
}

#[test]
fn herdrs_source_extracts() {
    let out = extract::extract(&common::herdr_root()).expect("extract");
    assert_eq!(out.protocol_version, 22);
    for name in [
        "ClientMessage",
        "ServerMessage",
        "WindowsKeyRecord",
        "AgentStatus",
        "ToastHerdrPosition",
        "write_message",
    ] {
        assert!(out.items.contains(name), "{name}");
    }
    // Nothing herdr-internal and no test code survives (doc comments may
    // still mention ratatui).
    let code: String = out
        .code
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for gone in [
        "crate::",
        "ratatui",
        "crossterm",
        "schemars",
        "bincode::Decode",
        "mod tests",
        "#[cfg",
        // cfg(any(windows, test)) items.
        "enum ClientInputEvent",
        "enum ClientKeySource",
    ] {
        assert!(!code.contains(gone), "{gone}");
    }
    // And it is what the crate compiled.
    assert_eq!(out.code, libherdr_term::HERDR_WIRE_RS);
}

#[test]
fn a_kept_type_naming_an_unmapped_crate_path_fails_with_the_fix() {
    let dir = tempfile::tempdir().unwrap();
    copy_sources(dir.path());
    let wire = dir.path().join(extract::WIRE);
    let text = std::fs::read_to_string(&wire).unwrap();
    let broken = text.replacen(
        "pub struct ClientSurfaceSize {\n    pub cols: u16,",
        "pub struct ClientSurfaceSize {\n    pub foo: crate::foo::Foo,\n    pub cols: u16,",
        1,
    );
    assert_ne!(broken, text, "the copy was not changed");
    std::fs::write(&wire, broken).unwrap();

    let err = extract::extract(dir.path()).unwrap_err().to_string();
    assert!(err.contains("`ClientSurfaceSize`"), "{err}");
    assert!(err.contains("crate::foo::Foo"), "{err}");
    assert!(err.contains("map it in"), "{err}");
}

#[test]
fn a_missing_listed_item_fails_with_the_fix() {
    let dir = tempfile::tempdir().unwrap();
    copy_sources(dir.path());
    let model = dir.path().join(extract::INPUT_MODEL);
    let text = std::fs::read_to_string(&model).unwrap();
    std::fs::write(
        &model,
        text.replace("pub struct WindowsKeyRecord", "pub struct RenamedRecord"),
    )
    .unwrap();

    let err = extract::extract(dir.path()).unwrap_err().to_string();
    assert!(err.contains("`WindowsKeyRecord`"), "{err}");
    assert!(err.contains("map it in"), "{err}");
}

#[test]
fn a_missing_submodule_says_how_to_fetch_it() {
    let dir = tempfile::tempdir().unwrap();
    let err = extract::extract(dir.path()).unwrap_err().to_string();
    assert!(
        err.contains("git submodule update --init vendor/herdr"),
        "{err}"
    );
    assert!(err.contains("HERDR_SRC"), "{err}");
}
