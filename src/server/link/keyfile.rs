//! The server's static key, `~/.config/handlr/link_key`
//! (plan 20261022_02, decision 4).
//!
//! The file holds the 32-byte X25519 private key in base64 (standard
//! alphabet, padded) and a newline. `serve` creates it on first start with
//! mode 0600, and refuses to start if it is open to anyone but its owner.

use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use libhandlr_proto::link::{generate_keypair, StaticKeypair, KEY_LEN};

/// Whether a file mode is 0600 or stricter: no access for group or others.
pub fn mode_is_private(mode: u32) -> bool {
    mode & 0o077 == 0
}

/// Load the key at `path`, creating it if it does not exist.
pub fn load_or_create(path: &Path) -> Result<StaticKeypair, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => load(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => create(path),
        Err(e) => Err(format!("link key {}: {e}", path.display())),
    }
}

/// Load an existing key, refusing one that others may read.
pub fn load(path: &Path) -> Result<StaticKeypair, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("link key {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("link key {} is not a regular file", path.display()));
    }
    let mode = meta.permissions().mode() & 0o777;
    if !mode_is_private(mode) {
        return Err(format!(
            "link key {} has mode {mode:04o}; it must be 0600 or stricter (chmod 600 {})",
            path.display(),
            path.display()
        ));
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid {
        return Err(format!(
            "link key {} belongs to uid {}, not to this user (uid {uid})",
            path.display(),
            meta.uid()
        ));
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("link key {}: {e}", path.display()))?;
    let private = decode_key(text.trim())
        .ok_or_else(|| format!("link key {} is not a base64 32-byte key", path.display()))?;
    StaticKeypair::from_private(private).map_err(|e| format!("link key {}: {e}", path.display()))
}

fn create(path: &Path) -> Result<StaticKeypair, String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() && !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let keypair = generate_keypair().map_err(|e| format!("generate the link key: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("create link key {}: {e}", path.display()))?;
    writeln!(file, "{}", STANDARD.encode(keypair.private))
        .map_err(|e| format!("write link key {}: {e}", path.display()))?;
    log::info!("Created the link key {}", path.display());
    Ok(keypair)
}

/// A base64 32-byte key, or `None`.
pub fn decode_key(text: &str) -> Option<[u8; KEY_LEN]> {
    STANDARD.decode(text).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        assert!(mode_is_private(0o600));
        assert!(mode_is_private(0o400));
        assert!(!mode_is_private(0o644));
        assert!(!mode_is_private(0o640));
        assert!(!mode_is_private(0o604));
    }

    #[test]
    fn created_on_first_start_and_loaded_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("link_key");
        let first = load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(decode_key(text.trim()), Some(first.private));
        let second = load_or_create(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_key_at_0644_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link_key");
        load_or_create(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_or_create(&path).unwrap_err();
        assert!(err.contains("0644"), "{err}");
        // 0400 is stricter, and fine.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert!(load_or_create(&path).is_ok());
    }

    #[test]
    fn a_malformed_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link_key");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(b"not a key\n")
            .unwrap();
        assert!(load_or_create(&path).unwrap_err().contains("base64"));
    }
}
