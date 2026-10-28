//! Key storage (plan 20261022_04, decision 5), with a list of paired
//! remotes (plan 20261028_01, decisions 1-7 and 13).
//!
//! A **remote** is one paired daemon, identified by its link static public
//! key; its [`RemoteId`] is the first 8 bytes of BLAKE2s-256 over that key.
//! The phone keeps one [`ServerRecord`] per remote in a [`Remotes`] list,
//! written whole, and a separate Noise static key per remote, named by the
//! record's `key_name`: [`LEGACY_KEY_NAME`] for the pairing made before the
//! list, [`RemoteId::key_name`] (`noise.static.<id>`) for every other.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use libflankr_proto::link::{self, REMOTE_ID_LEN, StaticKey, StaticKeypair};
use serde::{Deserialize, Serialize};

/// The key name of the one static key the phone had before the list: the
/// pairing migrated from the `server` record keeps it (decision 5).
pub const LEGACY_KEY_NAME: &str = "noise.static";

/// The most remotes the phone keeps (decision 12).
pub const MAX_REMOTES: usize = 8;

/// Where a client keeps its static keys and the remotes it is paired with
/// (plan 20261028_01, decision 6).
///
/// A static key is a [`StaticKey`], which does its own Diffie-Hellman, so
/// a store backed by a platform key store (the Android Keystore, a
/// Keychain) never hands out the private key. Keys are named; each
/// [`ServerRecord`] names its own (`key_name`).
pub trait KeyStore: Send {
    /// The paired remotes. The first call on a store that still holds the
    /// single `server` record of before migrates it (decision 13).
    fn remotes(&mut self) -> Result<Remotes, String>;
    /// Writes the whole list.
    fn set_remotes(&mut self, remotes: &Remotes) -> Result<(), String>;
    /// The static key named `key_name`, if there is one.
    fn static_key(&mut self, key_name: &str) -> Result<Option<Arc<dyn StaticKey>>, String>;
    /// Makes and stores a new static key named `key_name`, replacing any
    /// there was.
    fn generate_static_key(&mut self, key_name: &str) -> Result<Arc<dyn StaticKey>, String>;
    /// Deletes the static key named `key_name`. Nothing stored is not an
    /// error.
    fn delete_static_key(&mut self, key_name: &str) -> Result<(), String>;
}

/// A remote's id: the first 8 bytes of BLAKE2s-256 over its link static
/// public key ([`libflankr_proto::link::fingerprint`]). Displayed as 16
/// lower-case hex digits; it names the remote's state directory and its
/// key-store entry, and is never shown whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemoteId(pub [u8; REMOTE_ID_LEN]);

impl RemoteId {
    /// The id of the remote whose static public key is `public_key`.
    pub fn of(public_key: &[u8; 32]) -> Self {
        let digest = link::fingerprint(public_key);
        let mut id = [0u8; REMOTE_ID_LEN];
        id.copy_from_slice(&digest[..REMOTE_ID_LEN]);
        RemoteId(id)
    }

    /// The fingerprint shown in the Remotes row: the first 8 hex digits in
    /// two groups, `3f9a 12c4` (as
    /// [`libflankr_proto::link::fingerprint_text`]).
    pub fn fingerprint(&self) -> String {
        let digits = self.to_string();
        format!("{} {}", &digits[..4], &digits[4..8])
    }

    /// The key-store name of a new remote's phone key, `noise.static.<id>`
    /// (decision 5).
    pub fn key_name(&self) -> String {
        format!("{LEGACY_KEY_NAME}.{self}")
    }
}

impl fmt::Display for RemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for RemoteId {
    type Err = String;

    /// 16 lower-case hex digits.
    fn from_str(text: &str) -> Result<Self, String> {
        let bad = || format!("'{text}' is not a remote id (16 lower-case hex digits)");
        if text.len() != REMOTE_ID_LEN * 2
            || !text.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(bad());
        }
        let mut id = [0u8; REMOTE_ID_LEN];
        for (i, b) in id.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
        }
        Ok(RemoteId(id))
    }
}

/// A paired remote (decision 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRecord {
    /// Its name: the host's first DNS label, not unique.
    pub name: String,
    pub public_key: [u8; 32],
    pub addresses: Vec<String>,
    pub session_id: Option<String>,
    /// [`RemoteId::of`] its `public_key`.
    pub id: RemoteId,
    /// When it was paired, Unix seconds; `None` for the pairing migrated
    /// from before the list.
    pub paired_at: Option<u64>,
    /// The key-store name of this remote's phone key.
    pub key_name: String,
    /// Reserved for plan 20261023_08 (2.5 step 3), base64; this crate never
    /// sets it, and keeps it as read.
    pub signing_key: Option<String>,
    /// The owner's name for it, set in its details (plan 20261024_02, the
    /// 2026-10-28 17:56 EDT amendment); `None`: the default
    /// ([`RemoteInfo::display_name`]). Kept as [`nickname`] makes it.
    pub nickname: Option<String>,
}

impl ServerRecord {
    /// A new remote's record: its id from `public_key`, its key named
    /// [`RemoteId::key_name`], no session and no pairing time.
    pub fn new(name: impl Into<String>, public_key: [u8; 32], addresses: Vec<String>) -> Self {
        let id = RemoteId::of(&public_key);
        ServerRecord {
            name: name.into(),
            public_key,
            addresses,
            session_id: None,
            id,
            paired_at: None,
            key_name: id.key_name(),
            signing_key: None,
            nickname: None,
        }
    }

    /// Its first address, `host:port`, or "" with none.
    pub fn host(&self) -> &str {
        self.addresses.first().map_or("", String::as_str)
    }

    /// The name shown when it has no nickname and no other listed remote
    /// shows the same: [`base_hostname`] of its first address, or its
    /// `name` with no address.
    fn default_name(&self) -> String {
        match self.addresses.first() {
            Some(address) => base_hostname(address),
            None => self.name.clone(),
        }
    }
}

/// The longest nickname kept, in characters.
pub const NICKNAME_MAX: usize = 40;

/// A nickname as it is kept: control characters dropped, trimmed, and cut
/// to [`NICKNAME_MAX`] characters; empty is `None` (the default).
pub fn nickname(text: Option<&str>) -> Option<String> {
    let cleaned: String = text?.chars().filter(|c| !c.is_control()).collect();
    let cut: String = cleaned.trim().chars().take(NICKNAME_MAX).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// `host:port` split at the port: the host (an IPv6 address keeps its
/// brackets) and the port, if there is one.
fn split_port(address: &str) -> (&str, Option<&str>) {
    match address.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && (!host.contains(':') || host.ends_with(']')) =>
        {
            (host, Some(port))
        }
        _ => (address, None),
    }
}

/// The base hostname of `host:port` (plan 20261024_02, the 2026-10-28
/// 17:56 EDT amendment): the host's first DNS label, `m4` for
/// `m4.tail1a2b3c.ts.net:7347`; an IP address whole, `127.0.0.1`.
pub fn base_hostname(address: &str) -> String {
    let (host, _) = split_port(address.trim());
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return host.to_string();
    }
    host.split('.').next().unwrap_or(host).to_string()
}

/// BLAKE2s-256 of `public_key`, the whole fingerprint, as 64 lower-case
/// hex digits.
pub fn fingerprint_hex(public_key: &[u8; 32]) -> String {
    link::fingerprint(public_key)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The paired remotes, in pairing order (oldest first), and the one
/// connected to (decision 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remotes {
    /// The remote connected to; `None`: none (the local socket).
    pub active: Option<RemoteId>,
    pub list: Vec<ServerRecord>,
}

impl Remotes {
    pub fn get(&self, id: RemoteId) -> Option<&ServerRecord> {
        self.list.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: RemoteId) -> Option<&mut ServerRecord> {
        self.list.iter_mut().find(|r| r.id == id)
    }

    /// The active remote's record.
    pub fn active_record(&self) -> Option<&ServerRecord> {
        self.active.and_then(|id| self.get(id))
    }

    /// The most recently paired remote, by `paired_at` and then list
    /// order, leaving out `except` (decision 8: the next active remote).
    /// Sets `id`'s nickname, as [`nickname`] keeps it. Whether it changed.
    pub fn set_nickname(&mut self, id: RemoteId, text: Option<&str>) -> bool {
        let kept = nickname(text);
        match self.get_mut(id) {
            Some(r) if r.nickname != kept => {
                r.nickname = kept;
                true
            }
            _ => false,
        }
    }

    /// The name each listed remote shows, in list order (plan 20261024_02,
    /// the 17:56 and 17:57 EDT amendments): its nickname if it has one;
    /// otherwise its base hostname, with `:port` added only when two or
    /// more listed remotes would otherwise show the same.
    pub fn display_names(&self) -> Vec<String> {
        let plain: Vec<String> = self
            .list
            .iter()
            .map(|r| r.nickname.clone().unwrap_or_else(|| r.default_name()))
            .collect();
        self.list
            .iter()
            .zip(&plain)
            .map(|(r, name)| {
                let clash = plain.iter().filter(|n| *n == name).count() > 1;
                match (&r.nickname, clash, split_port(r.host()).1) {
                    (None, true, Some(port)) => format!("{name}:{port}"),
                    _ => name.clone(),
                }
            })
            .collect()
    }

    pub fn most_recent(&self, except: &[RemoteId]) -> Option<RemoteId> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, r)| !except.contains(&r.id))
            .max_by_key(|(i, r)| (r.paired_at.unwrap_or(0), *i))
            .map(|(_, r)| r.id)
    }
}

/// One row of the Remotes section ([`Event::Remotes`](crate::Event::Remotes);
/// plan 20261028_01, decision 26).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInfo {
    pub id: RemoteId,
    pub name: String,
    /// Its first address, `host:port`.
    pub host: String,
    /// `3f9a 12c4`.
    pub fingerprint: String,
    /// Unix seconds; `None`: paired before the list.
    pub paired_at: Option<u64>,
    /// The remote in use.
    pub active: bool,
    /// The remote in use, and its `hello` answered on the open connection.
    pub online: bool,
    /// The whole fingerprint, BLAKE2s-256 of the daemon's key, as 64
    /// lower-case hex digits ([`fingerprint_hex`]; plan 20261024_02, the
    /// 2026-10-28 17:46 EDT amendment).
    pub fingerprint_hex: String,
    /// The daemon's fingerprint as six words, as `flankr pair` prints them
    /// ([`libflankr_proto::link::fingerprint_words_text`]).
    pub fingerprint_words: String,
    /// The owner's nickname for it; `None`: the default.
    pub nickname: Option<String>,
    /// The name shown ([`Remotes::display_names`]).
    pub display_name: String,
}

impl Remotes {
    /// The rows of the Remotes section, in pairing order; `online` says
    /// whether the active remote's connection is up.
    pub fn info(&self, online: bool) -> Vec<RemoteInfo> {
        self.list
            .iter()
            .zip(self.display_names())
            .map(|(r, display_name)| {
                let active = self.active == Some(r.id);
                RemoteInfo {
                    id: r.id,
                    name: r.name.clone(),
                    host: r.host().to_string(),
                    fingerprint: r.id.fingerprint(),
                    paired_at: r.paired_at,
                    active,
                    online: active && online,
                    fingerprint_hex: fingerprint_hex(&r.public_key),
                    fingerprint_words: link::fingerprint_words_text(&r.public_key),
                    nickname: r.nickname.clone(),
                    display_name,
                }
            })
            .collect()
    }
}

/// The `remotes` blob's layout (`remotes.toml`).
#[derive(Serialize, Deserialize)]
struct RemotesFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default, rename = "remote")]
    remotes: Vec<RemoteFile>,
}

#[derive(Serialize, Deserialize)]
struct RemoteFile {
    id: String,
    name: String,
    public_key: String,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paired_at: Option<u64>,
    key_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signing_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nickname: Option<String>,
}

/// The layout version of the `remotes` blob.
const REMOTES_VERSION: u32 = 1;

/// The `remotes` blob, as TOML (decision 6).
pub fn encode_remotes(remotes: &Remotes) -> Result<String, String> {
    let file = RemotesFile {
        version: REMOTES_VERSION,
        active: remotes.active.map(|id| id.to_string()),
        remotes: remotes
            .list
            .iter()
            .map(|r| RemoteFile {
                id: r.id.to_string(),
                name: r.name.clone(),
                public_key: STANDARD.encode(r.public_key),
                addresses: r.addresses.clone(),
                session_id: r.session_id.clone(),
                paired_at: r.paired_at,
                key_name: r.key_name.clone(),
                signing_key: r.signing_key.clone(),
                nickname: r.nickname.clone(),
            })
            .collect(),
    };
    toml::to_string(&file).map_err(|e| format!("the remotes list: {e}"))
}

/// Reads the `remotes` blob. A record whose `id` is not its key's is
/// refused, as is an `active` naming no record.
pub fn decode_remotes(text: &str) -> Result<Remotes, String> {
    let file: RemotesFile = toml::from_str(text).map_err(|e| format!("the remotes list: {e}"))?;
    if file.version != REMOTES_VERSION {
        return Err(format!(
            "the remotes list is version {}, not {REMOTES_VERSION}",
            file.version
        ));
    }
    let mut list = Vec::with_capacity(file.remotes.len());
    for r in file.remotes {
        let public_key = decode_key(&r.public_key, "a remote's public key")?;
        let id: RemoteId = r.id.parse()?;
        if id != RemoteId::of(&public_key) {
            return Err(format!("the remote {id}'s id is not its key's"));
        }
        list.push(ServerRecord {
            name: r.name,
            public_key,
            addresses: r.addresses,
            session_id: r.session_id,
            id,
            paired_at: r.paired_at,
            key_name: r.key_name,
            signing_key: r.signing_key,
            nickname: nickname(r.nickname.as_deref()),
        });
    }
    let active = file.active.map(|a| a.parse::<RemoteId>()).transpose()?;
    if let Some(id) = active
        && !list.iter().any(|r| r.id == id)
    {
        return Err(format!("the active remote {id} is not in the list"));
    }
    Ok(Remotes { active, list })
}

/// The single server record of before the list (`server.toml`, the
/// `server` blob): the public key in base64.
#[derive(Serialize, Deserialize)]
struct ServerFile {
    name: String,
    public_key: String,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

/// Reads the single server record of before the list (`server.toml`'s
/// layout, which the phone's `server` blob shares) as the remote it
/// migrates to: its key [`LEGACY_KEY_NAME`], no pairing time (decision 13).
pub fn decode_legacy_server(text: &str) -> Result<ServerRecord, String> {
    let file: ServerFile = toml::from_str(text).map_err(|e| format!("the server record: {e}"))?;
    let public_key = decode_key(&file.public_key, "the server's public key")?;
    let mut record = ServerRecord::new(file.name, public_key, file.addresses);
    record.session_id = file.session_id;
    record.key_name = LEGACY_KEY_NAME.to_string();
    Ok(record)
}

/// The list a store migrates to when it has no `remotes` blob yet: the
/// old record, if there was one, as the one remote and the active one;
/// otherwise empty (decision 13). The caller writes it, then deletes the
/// old record; with none, it deletes the [`LEGACY_KEY_NAME`] key.
pub fn migrated(server: Option<ServerRecord>) -> Remotes {
    match server {
        Some(record) => {
            log::info!(
                "keys: moved the paired server {} ({}) into the remotes list",
                record.name,
                record.id.fingerprint()
            );
            Remotes {
                active: Some(record.id),
                list: vec![record],
            }
        }
        None => Remotes::default(),
    }
}

/// Keys in files: each static key, base64, in `<dir>/key` (the key named
/// [`LEGACY_KEY_NAME`]) or `<dir>/key.<id>` (the key named
/// `noise.static.<id>`) with mode 0600, and the list in
/// `<dir>/remotes.toml`. A key file that anyone but its owner has any
/// permission on is refused. `<dir>/server.toml`, the single record of
/// before, is migrated on the first [`remotes`](KeyStore::remotes).
#[derive(Debug, Clone)]
pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FileKeyStore { dir: dir.into() }
    }

    /// `~/.config/flankr-companion/`.
    pub fn default_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config").join("flankr-companion")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file of the key named `key_name`.
    pub fn key_path(&self, key_name: &str) -> Result<PathBuf, String> {
        if key_name == LEGACY_KEY_NAME {
            return Ok(self.dir.join("key"));
        }
        match key_name
            .strip_prefix(LEGACY_KEY_NAME)
            .and_then(|s| s.strip_prefix('.'))
        {
            Some(rest)
                if !rest.is_empty()
                    && rest
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) =>
            {
                Ok(self.dir.join(format!("key.{rest}")))
            }
            _ => Err(format!("'{key_name}' is not a key name this store keeps")),
        }
    }

    fn server_path(&self) -> PathBuf {
        self.dir.join("server.toml")
    }

    fn remotes_path(&self) -> PathBuf {
        self.dir.join("remotes.toml")
    }

    /// Writes `contents` to `path` with mode 0600, through a temporary file
    /// renamed into place, creating the directory with mode 0700.
    fn write_private(&self, path: &Path, contents: &[u8]) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700));
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
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

/// Deletes a file; one that does not exist is not an error.
fn remove_optional(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
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
    /// The private key in `path`, if there is one.
    fn read_key(&self, path: &Path) -> Result<Option<[u8; 32]>, String> {
        let meta = match fs::metadata(path) {
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
        match read_optional(path)? {
            Some(text) => decode_key(&text, &path.display().to_string()).map(Some),
            None => Ok(None),
        }
    }
}

impl KeyStore for FileKeyStore {
    fn remotes(&mut self) -> Result<Remotes, String> {
        let path = self.remotes_path();
        if let Some(text) = read_optional(&path)? {
            let remotes = decode_remotes(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            // A migration that stopped between its two writes.
            remove_optional(&self.server_path())?;
            return Ok(remotes);
        }
        let server = match read_optional(&self.server_path())? {
            Some(text) => Some(
                decode_legacy_server(&text)
                    .map_err(|e| format!("{}: {e}", self.server_path().display()))?,
            ),
            None => None,
        };
        let had_server = server.is_some();
        let remotes = migrated(server);
        // The new list first; the old record only once it is written.
        self.set_remotes(&remotes)?;
        if had_server {
            remove_optional(&self.server_path())?;
        } else {
            // A key with no server: a pairing never stored.
            self.delete_static_key(LEGACY_KEY_NAME)?;
        }
        Ok(remotes)
    }

    fn set_remotes(&mut self, remotes: &Remotes) -> Result<(), String> {
        let text = encode_remotes(remotes)?;
        self.write_private(&self.remotes_path(), text.as_bytes())
    }

    fn static_key(&mut self, key_name: &str) -> Result<Option<Arc<dyn StaticKey>>, String> {
        let Some(private) = self.read_key(&self.key_path(key_name)?)? else {
            return Ok(None);
        };
        let keypair = StaticKeypair::from_private(private).map_err(|e| e.to_string())?;
        Ok(Some(Arc::new(keypair)))
    }

    fn generate_static_key(&mut self, key_name: &str) -> Result<Arc<dyn StaticKey>, String> {
        let path = self.key_path(key_name)?;
        let keypair = link::generate_keypair().map_err(|e| e.to_string())?;
        let text = format!("{}\n", STANDARD.encode(keypair.private));
        self.write_private(&path, text.as_bytes())?;
        Ok(Arc::new(keypair))
    }

    fn delete_static_key(&mut self, key_name: &str) -> Result<(), String> {
        remove_optional(&self.key_path(key_name)?)
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

    fn record(n: u8, paired_at: Option<u64>) -> ServerRecord {
        let mut r = ServerRecord::new(
            format!("m{n}"),
            [n; 32],
            vec![format!("m{n}.tail.ts.net:7347"), "127.0.0.1:7347".into()],
        );
        r.paired_at = paired_at;
        r
    }

    /// Today's layout: `server.toml` and `key`, as the old store wrote
    /// them.
    fn legacy(keys: &FileKeyStore, server: &[u8; 32]) -> [u8; 32] {
        let keypair = link::generate_keypair().unwrap();
        let text = format!("{}\n", STANDARD.encode(keypair.private));
        keys.write_private(&keys.dir.join("key"), text.as_bytes())
            .unwrap();
        let toml = format!(
            "name = \"m4\"\npublic_key = \"{}\"\naddresses = [\"m4.tail.ts.net:7347\"]\nsession_id = \"companion:s23\"\n",
            STANDARD.encode(server)
        );
        keys.write_private(&keys.server_path(), toml.as_bytes())
            .unwrap();
        keypair.public
    }

    #[test]
    fn ids_and_fingerprints_follow_the_proto_crate() {
        let key = [0u8; 32];
        let id = RemoteId::of(&key);
        assert_eq!(id.to_string(), link::remote_id(&key));
        assert_eq!(id.to_string(), "320b5ea99e653bc2");
        assert_eq!(id.fingerprint(), link::fingerprint_text(&key));
        assert_eq!(id.key_name(), "noise.static.320b5ea99e653bc2");
        assert_eq!("320b5ea99e653bc2".parse::<RemoteId>(), Ok(id));
        assert!("320B5EA99E653BC2".parse::<RemoteId>().is_err());
        assert!("320b5e".parse::<RemoteId>().is_err());
    }

    #[test]
    fn an_empty_store_reads_as_an_empty_list() {
        let (_dir, mut keys) = store();
        assert_eq!(keys.remotes().unwrap(), Remotes::default());
        assert!(keys.static_key(LEGACY_KEY_NAME).unwrap().is_none());
        // And says so again, from the list it wrote.
        assert!(keys.remotes_path().exists());
        assert_eq!(keys.remotes().unwrap(), Remotes::default());
    }

    #[test]
    fn a_list_of_three_round_trips_with_active() {
        let (_dir, mut keys) = store();
        let mut first = record(1, None);
        first.key_name = LEGACY_KEY_NAME.into();
        first.session_id = Some("companion:s23".into());
        let mut third = record(3, Some(1_793_212_400));
        third.signing_key = Some("c2lnbmluZw==".into());
        let remotes = Remotes {
            active: Some(record(2, None).id),
            list: vec![first, record(2, Some(1_793_212_345)), third],
        };
        keys.set_remotes(&remotes).unwrap();
        assert_eq!(keys.remotes().unwrap(), remotes);
        let text = fs::read_to_string(keys.remotes_path()).unwrap();
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(text.contains("[[remote]]"), "{text}");
        let mode = fs::metadata(keys.remotes_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_bad_list_is_refused() {
        let mut r = record(1, None);
        r.id = record(2, None).id;
        let text = encode_remotes(&Remotes {
            active: None,
            list: vec![r],
        })
        .unwrap();
        assert!(decode_remotes(&text).unwrap_err().contains("not its key's"));
        let text = encode_remotes(&Remotes {
            active: Some(record(2, None).id),
            list: vec![record(1, None)],
        })
        .unwrap();
        assert!(
            decode_remotes(&text)
                .unwrap_err()
                .contains("not in the list")
        );
    }

    #[test]
    fn todays_pairing_migrates_to_one_active_remote() {
        let (_dir, mut keys) = store();
        let server = [9u8; 32];
        let phone = legacy(&keys, &server);
        let remotes = keys.remotes().unwrap();
        let id = RemoteId::of(&server);
        assert_eq!(remotes.active, Some(id));
        assert_eq!(remotes.list.len(), 1);
        let r = &remotes.list[0];
        assert_eq!(r.public_key, server);
        assert_eq!(r.name, "m4");
        assert_eq!(r.addresses, ["m4.tail.ts.net:7347"]);
        assert_eq!(r.session_id.as_deref(), Some("companion:s23"));
        assert_eq!(r.paired_at, None);
        assert_eq!(r.key_name, LEGACY_KEY_NAME);
        // The old key is kept, under its old name and file.
        let key = keys.static_key(&r.key_name).unwrap().expect("the old key");
        assert_eq!(key.public(), phone);
        assert!(keys.dir.join("key").exists());
        assert!(!keys.server_path().exists(), "server.toml is gone");
        assert_eq!(keys.remotes().unwrap(), remotes);
    }

    #[test]
    fn both_files_read_the_list_and_delete_the_old_record() {
        let (_dir, mut keys) = store();
        let remotes = Remotes {
            active: Some(record(1, None).id),
            list: vec![record(1, None), record(2, Some(5))],
        };
        keys.set_remotes(&remotes).unwrap();
        legacy(&keys, &[9u8; 32]);
        assert_eq!(keys.remotes().unwrap(), remotes);
        assert!(!keys.server_path().exists());
        // The key of the pairing in the list is not touched.
        assert!(keys.dir.join("key").exists());
    }

    #[test]
    fn a_key_with_no_server_is_deleted_at_migration() {
        let (_dir, mut keys) = store();
        keys.generate_static_key(LEGACY_KEY_NAME).unwrap();
        assert_eq!(keys.remotes().unwrap(), Remotes::default());
        assert!(keys.static_key(LEGACY_KEY_NAME).unwrap().is_none());
    }

    #[test]
    fn each_remote_has_its_own_key_file() {
        let (_dir, mut keys) = store();
        let a = record(1, None);
        let b = record(2, None);
        let ka = keys.generate_static_key(&a.key_name).unwrap();
        let kb = keys.generate_static_key(&b.key_name).unwrap();
        assert_ne!(ka.public(), kb.public());
        let path = keys.key_path(&a.key_name).unwrap();
        assert_eq!(path, keys.dir.join(format!("key.{}", a.id)));
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        keys.delete_static_key(&a.key_name).unwrap();
        assert!(keys.static_key(&a.key_name).unwrap().is_none());
        let stored = keys.static_key(&b.key_name).unwrap().expect("b's key");
        assert_eq!(stored.public(), kb.public());
        assert!(keys.key_path("../server").is_err());
        assert!(keys.key_path("noise.static./x").is_err());
    }

    #[test]
    fn deleting_a_missing_key_is_not_an_error() {
        let (_dir, mut keys) = store();
        keys.delete_static_key(LEGACY_KEY_NAME).unwrap();
        keys.delete_static_key(&record(1, None).key_name).unwrap();
    }

    #[test]
    fn a_key_file_at_0644_is_refused() {
        let (_dir, mut keys) = store();
        let name = record(1, None).key_name;
        keys.generate_static_key(&name).unwrap();
        fs::set_permissions(
            keys.key_path(&name).unwrap(),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let err = keys.static_key(&name).err().expect("refused");
        assert!(err.contains("0644"), "{err}");
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused() {
        let (_dir, mut keys) = store();
        keys.write_private(&keys.dir.join("key"), STANDARD.encode([1u8; 16]).as_bytes())
            .unwrap();
        let err = keys.static_key(LEGACY_KEY_NAME).err().expect("refused");
        assert!(err.contains("16 bytes"), "{err}");
    }

    /// A remote at `address`, its key `[n; 32]`.
    fn at(n: u8, address: &str) -> ServerRecord {
        ServerRecord::new(format!("m{n}"), [n; 32], vec![address.into()])
    }

    #[test]
    fn the_base_hostname_is_the_first_label_or_the_whole_ip() {
        assert_eq!(base_hostname("m4.tail1a2b3c.ts.net:7347"), "m4");
        assert_eq!(base_hostname("m4:7347"), "m4");
        assert_eq!(base_hostname("m4.local"), "m4");
        assert_eq!(base_hostname("127.0.0.1:7347"), "127.0.0.1");
        assert_eq!(base_hostname("100.64.0.7"), "100.64.0.7");
        assert_eq!(base_hostname("[fd7a::1]:7347"), "[fd7a::1]");
    }

    #[test]
    fn a_default_name_adds_the_port_only_on_a_conflict() {
        let mut remotes = Remotes {
            active: None,
            list: vec![at(1, "m4.tail.ts.net:7347"), at(2, "mini.tail.ts.net:7348")],
        };
        // Not the default port, but no conflict: the base hostname alone.
        assert_eq!(remotes.display_names(), ["m4", "mini"]);
        // Two that would both show m4: each shows host:port, whatever the
        // port; the other is left alone.
        remotes.list.push(at(3, "m4.lan:7347"));
        assert_eq!(remotes.display_names(), ["m4:7347", "mini", "m4:7347"]);
        remotes.list[2].addresses = vec!["m4.lan:7400".into()];
        assert_eq!(remotes.display_names(), ["m4:7347", "mini", "m4:7400"]);
        // A nickname is shown as set, and ends the conflict it was in.
        assert!(remotes.set_nickname(remotes.list[2].id, Some("  Studio  ")));
        assert_eq!(remotes.display_names(), ["m4", "mini", "Studio"]);
        // A nickname equal to another's default: the default takes the
        // port, the nickname does not.
        assert!(remotes.set_nickname(remotes.list[2].id, Some("mini")));
        assert_eq!(remotes.display_names(), ["m4", "mini:7348", "mini"]);
        // Cleared (empty is the default): back to the conflict.
        assert!(remotes.set_nickname(remotes.list[2].id, Some("   ")));
        assert_eq!(remotes.list[2].nickname, None);
        assert_eq!(remotes.display_names(), ["m4:7347", "mini", "m4:7400"]);
        assert!(!remotes.set_nickname(remotes.list[2].id, None), "unchanged");
        // With no address, its name.
        let bare = Remotes {
            active: None,
            list: vec![ServerRecord::new("m9", [9; 32], Vec::new())],
        };
        assert_eq!(bare.display_names(), ["m9"]);
    }

    #[test]
    fn a_nickname_is_trimmed_cut_and_without_control_characters() {
        assert_eq!(nickname(None), None);
        assert_eq!(nickname(Some("")), None);
        assert_eq!(nickname(Some(" \t ")), None);
        assert_eq!(nickname(Some(" Mac\nmini ")).as_deref(), Some("Macmini"));
        let long = "x".repeat(NICKNAME_MAX + 10);
        assert_eq!(
            nickname(Some(&long)).map(|n| n.chars().count()),
            Some(NICKNAME_MAX)
        );
    }

    #[test]
    fn info_carries_the_whole_fingerprint_and_the_display_name() {
        let mut remotes = Remotes {
            active: Some(at(1, "m4.tail.ts.net:7347").id),
            list: vec![at(1, "m4.tail.ts.net:7347"), at(2, "m4.lan:7350")],
        };
        remotes.list[1].nickname = Some("Office".into());
        let info = remotes.info(true);
        let key = [1u8; 32];
        assert_eq!(info[0].fingerprint_hex, fingerprint_hex(&key));
        assert_eq!(info[0].fingerprint_hex.len(), 64);
        assert!(info[0].fingerprint_hex.starts_with(&link::remote_id(&key)));
        assert_eq!(
            info[0].fingerprint_words,
            link::fingerprint_words_text(&key)
        );
        assert_eq!(info[0].nickname, None);
        assert_eq!(info[0].display_name, "m4");
        assert_eq!(info[1].nickname.as_deref(), Some("Office"));
        assert_eq!(info[1].display_name, "Office");
        assert!(info[0].active && info[0].online && !info[1].active);
    }

    #[test]
    fn a_nickname_round_trips_and_an_old_list_reads_without_one() {
        let (_dir, mut keys) = store();
        let mut named = record(1, Some(5));
        named.nickname = Some("Studio".into());
        let remotes = Remotes {
            active: Some(named.id),
            list: vec![named, record(2, None)],
        };
        keys.set_remotes(&remotes).unwrap();
        let text = fs::read_to_string(keys.remotes_path()).unwrap();
        assert_eq!(text.matches("nickname = ").count(), 1, "{text}");
        assert_eq!(keys.remotes().unwrap(), remotes);
        // A list written before nicknames: none.
        let old = text.replace("nickname = \"Studio\"\n", "");
        let read = decode_remotes(&old).unwrap();
        assert!(read.list.iter().all(|r| r.nickname.is_none()));
    }

    #[test]
    fn the_next_active_is_the_most_recently_paired() {
        let remotes = Remotes {
            active: Some(record(1, None).id),
            list: vec![record(1, None), record(2, Some(20)), record(3, Some(10))],
        };
        let one = record(1, None).id;
        assert_eq!(remotes.most_recent(&[one]), Some(record(2, None).id));
        assert_eq!(
            remotes.most_recent(&[one, record(2, None).id]),
            Some(record(3, None).id)
        );
        // Undated ones: the later in the list.
        let undated = Remotes {
            active: None,
            list: vec![record(1, None), record(2, None)],
        };
        assert_eq!(undated.most_recent(&[]), Some(record(2, None).id));
        assert_eq!(undated.most_recent(&[one, record(2, None).id]), None);
    }
}
