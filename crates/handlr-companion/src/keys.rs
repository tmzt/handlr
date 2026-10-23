//! Key storage (plan 20261022_04, decision 5).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use libhandlr_proto::link::{self, StaticKey, StaticKeypair};
use serde::{Deserialize, Serialize};

/// Where a client keeps its static key and the server it is paired with.
///
/// The static key is a [`StaticKey`], which does its own Diffie-Hellman, so
/// a store backed by a platform key store (the Android Keystore, a
/// Keychain) never hands out the private key.
pub trait KeyStore: Send {
    /// This client's static key, if it has one.
    fn static_key(&mut self) -> Result<Option<Arc<dyn StaticKey>>, String>;
    /// Makes and stores a new static key, replacing any there was.
    fn generate_static_key(&mut self) -> Result<Arc<dyn StaticKey>, String>;
    /// The paired server, if any.
    fn server(&mut self) -> Result<Option<ServerRecord>, String>;
    fn set_server(&mut self, server: ServerRecord) -> Result<(), String>;
}

/// The server a client is paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRecord {
    pub name: String,
    pub public_key: [u8; 32],
    pub addresses: Vec<String>,
    pub session_id: Option<String>,
}

/// `server.toml`'s layout: the public key in base64.
#[derive(Serialize, Deserialize)]
struct ServerFile {
    name: String,
    public_key: String,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

/// Keys in files: the static key, base64, in `<dir>/key` with mode 0600,
/// and the server record in `<dir>/server.toml`. A key file that anyone
/// but its owner has any permission on is refused.
#[derive(Debug, Clone)]
pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FileKeyStore { dir: dir.into() }
    }

    /// `~/.config/handlr-companion/`.
    pub fn default_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config").join("handlr-companion")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("key")
    }

    fn server_path(&self) -> PathBuf {
        self.dir.join("server.toml")
    }

    /// Writes `contents` to `path` with mode 0600, through a temporary file
    /// renamed into place, creating the directory with mode 0700.
    fn write_private(&self, path: &Path, contents: &[u8]) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700));
        let tmp = path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(contents)?;
            file.sync_all()?;
            fs::rename(&tmp, path)
        };
        write().map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Reads a file, or `None` when it does not exist.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn decode_key(text: &str, what: &str) -> Result<[u8; 32], String> {
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|e| format!("{what} is not base64: {e}"))?;
    bytes
        .try_into()
        .map_err(|b: Vec<u8>| format!("{what} is {} bytes, not 32", b.len()))
}

impl FileKeyStore {
    /// The private key in `<dir>/key`, if there is one.
    fn read_key(&self) -> Result<Option<[u8; 32]>, String> {
        let path = self.key_path();
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "{} has mode {mode:04o}; only its owner may have access (chmod 600)",
                path.display()
            ));
        }
        match read_optional(&path)? {
            Some(text) => decode_key(&text, &path.display().to_string()).map(Some),
            None => Ok(None),
        }
    }
}

impl KeyStore for FileKeyStore {
    fn static_key(&mut self) -> Result<Option<Arc<dyn StaticKey>>, String> {
        let Some(private) = self.read_key()? else {
            return Ok(None);
        };
        let keypair = StaticKeypair::from_private(private).map_err(|e| e.to_string())?;
        Ok(Some(Arc::new(keypair)))
    }

    fn generate_static_key(&mut self) -> Result<Arc<dyn StaticKey>, String> {
        let keypair = link::generate_keypair().map_err(|e| e.to_string())?;
        let text = format!("{}\n", STANDARD.encode(keypair.private));
        self.write_private(&self.key_path(), text.as_bytes())?;
        Ok(Arc::new(keypair))
    }

    fn server(&mut self) -> Result<Option<ServerRecord>, String> {
        let path = self.server_path();
        let Some(text) = read_optional(&path)? else {
            return Ok(None);
        };
        let file: ServerFile =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Some(ServerRecord {
            public_key: decode_key(&file.public_key, "the server's public key")?,
            name: file.name,
            addresses: file.addresses,
            session_id: file.session_id,
        }))
    }

    fn set_server(&mut self, server: ServerRecord) -> Result<(), String> {
        let file = ServerFile {
            name: server.name,
            public_key: STANDARD.encode(server.public_key),
            addresses: server.addresses,
            session_id: server.session_id,
        };
        let text = toml::to_string(&file).map_err(|e| e.to_string())?;
        self.write_private(&self.server_path(), text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, FileKeyStore) {
        let dir = tempfile::tempdir().unwrap();
        let keys = FileKeyStore::new(dir.path().join("keys"));
        (dir, keys)
    }

    #[test]
    fn nothing_stored_reads_as_none() {
        let (_dir, mut keys) = store();
        assert!(keys.static_key().unwrap().is_none());
        assert_eq!(keys.server().unwrap(), None);
    }

    #[test]
    fn the_key_round_trips_at_mode_0600() {
        let (_dir, mut keys) = store();
        let key = keys.generate_static_key().unwrap();
        let mode = fs::metadata(keys.key_path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let stored = keys.static_key().unwrap().expect("the key is stored");
        assert_eq!(stored.public(), key.public());
    }

    #[test]
    fn a_key_file_at_0644_is_refused() {
        let (_dir, mut keys) = store();
        keys.generate_static_key().unwrap();
        fs::set_permissions(keys.key_path(), fs::Permissions::from_mode(0o644)).unwrap();
        let err = keys.static_key().err().expect("refused");
        assert!(err.contains("0644"), "{err}");
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused() {
        let (_dir, mut keys) = store();
        keys.write_private(&keys.key_path(), STANDARD.encode([1u8; 16]).as_bytes())
            .unwrap();
        let err = keys.static_key().err().expect("refused");
        assert!(err.contains("16 bytes"), "{err}");
    }

    #[test]
    fn the_server_record_round_trips() {
        let (_dir, mut keys) = store();
        let record = ServerRecord {
            name: "m4".to_string(),
            public_key: [9u8; 32],
            addresses: vec!["m4.tail.ts.net:7347".into(), "127.0.0.1:7347".into()],
            session_id: Some("companion:tty".into()),
        };
        keys.set_server(record.clone()).unwrap();
        assert_eq!(keys.server().unwrap(), Some(record));
    }
}
