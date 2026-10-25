//! The paired clients, `~/.config/flankr/auth.toml`
//! (plan 20261022_02, decision 6, and the 18:20 EDT amendment).
//!
//! ```toml
//! [[client]]
//! name = "s23"
//! key = "x25519:<public key, base64>"
//! paired = "2026-10-22T11:30:00-04:00"
//! protocols = ["companion"]       # "companion", "harness"
//! agents = ["claude", "codex"]    # optional; narrows [[agents]], never widens
//! approve = true                  # optional; default true for companion, false for harness
//! ```
//!
//! The file is read again for every new link connection and by the
//! revocation check, so editing it takes effect without a restart. A file
//! that cannot be read, does not parse, repeats a name or a key, or is
//! writable by group or others authorizes nobody.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use libflankr_proto::common::protocols;
use libflankr_proto::link::KEY_LEN;
use serde::{Deserialize, Serialize};

use crate::core::principal::Principal;

/// The prefix of a key in `auth.toml`.
pub const KEY_PREFIX: &str = "x25519:";

/// Short protocol names, as `protocols` lists them.
pub const COMPANION: &str = "companion";
pub const HARNESS: &str = "harness";

const HEADER: &str = "# Paired flankr clients (`flankr pair`). Delete an entry to revoke it.\n";

/// One paired client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthEntry {
    pub name: String,
    /// `x25519:<public key, base64>`.
    pub key: String,
    /// When it was paired, RFC 3339 in local time.
    pub paired: String,
    /// What it may say `hello` with: `companion`, `harness`.
    #[serde(default)]
    pub protocols: Vec<String>,
    /// Narrows `[[agents]]`; never widens it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<String>>,
    /// May it approve confirmations. Defaults to true for a companion
    /// connection and false for a harness one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve: Option<bool>,
}

/// The short name of a `hello` protocol: `flankr.companion` → `companion`.
pub fn short_protocol(protocol: &str) -> &str {
    protocol
        .strip_prefix("flankr.")
        .or_else(|| protocol.strip_prefix("handlr."))
        .unwrap_or(protocol)
}

/// A key as `auth.toml` writes it.
pub fn format_key(key: &[u8; KEY_LEN]) -> String {
    format!("{KEY_PREFIX}{}", STANDARD.encode(key))
}

impl AuthEntry {
    /// A new entry, paired now.
    pub fn new(name: &str, key: &[u8; KEY_LEN], protocols: Vec<String>) -> Self {
        AuthEntry {
            name: name.to_string(),
            key: format_key(key),
            paired: now_rfc3339(),
            protocols,
            agents: None,
            approve: None,
        }
    }

    /// The public key, or `None` if the entry's key is malformed.
    pub fn public_key(&self) -> Option<[u8; KEY_LEN]> {
        let b64 = self.key.trim().strip_prefix(KEY_PREFIX)?;
        STANDARD.decode(b64).ok()?.try_into().ok()
    }

    /// Whether this client may say `hello` with `protocol` (full or short).
    pub fn allows(&self, protocol: &str) -> bool {
        let short = short_protocol(protocol);
        self.protocols
            .iter()
            .any(|p| short_protocol(p).eq_ignore_ascii_case(short))
    }

    /// Whether a connection speaking `protocol` may approve confirmations.
    pub fn can_approve(&self, protocol: &str) -> bool {
        self.approve
            .unwrap_or_else(|| short_protocol(protocol) == short_protocol(protocols::COMPANION))
    }

    /// The principal of a connection speaking `protocol`.
    pub fn principal(&self, protocol: &str) -> Principal {
        Principal {
            id: format!("paired:{}", self.name),
            agents: self.agents.clone(),
            can_approve: self.can_approve(protocol),
        }
    }
}

/// The whole file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFile {
    #[serde(rename = "client", default)]
    pub clients: Vec<AuthEntry>,
}

impl AuthFile {
    /// Read `path`. A missing file is empty.
    pub fn load(path: &Path) -> Result<AuthFile, String> {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AuthFile::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o022 != 0 {
            return Err(format!(
                "{} has mode {mode:04o}; it must not be writable by group or others (chmod 600 {})",
                path.display(),
                path.display()
            ));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parse and check: unique names, unique well-formed keys.
    pub fn parse(text: &str) -> Result<AuthFile, String> {
        let file: AuthFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut names = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for entry in &file.clients {
            if !names.insert(entry.name.as_str()) {
                return Err(format!("the name '{}' is used twice", entry.name));
            }
            let key = entry
                .public_key()
                .ok_or_else(|| format!("'{}' has a malformed key", entry.name))?;
            if !keys.insert(key) {
                return Err(format!("the key of '{}' is used twice", entry.name));
            }
        }
        Ok(file)
    }

    pub fn find_key(&self, key: &[u8; KEY_LEN]) -> Option<&AuthEntry> {
        self.clients
            .iter()
            .find(|c| c.public_key().as_ref() == Some(key))
    }

    pub fn find_name(&self, name: &str) -> Option<&AuthEntry> {
        self.clients.iter().find(|c| c.name == name)
    }
}

/// Why `add` refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// The name is in the file, and `replace` was not asked for.
    NameInUse(String),
    /// The key is already paired, under another name.
    KeyInUse(String),
    /// The file could not be read or written.
    File(String),
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddError::NameInUse(name) => write!(
                f,
                "the name '{name}' is already paired (use --replace to re-pair it)"
            ),
            AddError::KeyInUse(name) => write!(f, "this key is already paired as '{name}'"),
            AddError::File(e) => f.write_str(e),
        }
    }
}

/// Add `entry` to the file at `path`.
///
/// A new name is appended, leaving the rest of the file as it is. With
/// `replace`, an entry of the same name is replaced: its key, `paired` and
/// `protocols` are the new entry's, and its `agents` and `approve` are
/// kept, so re-pairing never widens what a device may do. The file is then
/// rewritten (comments are not kept). Returns the entry replaced.
pub fn add(path: &Path, entry: AuthEntry, replace: bool) -> Result<Option<AuthEntry>, AddError> {
    let mut file = AuthFile::load(path).map_err(AddError::File)?;
    let key = entry
        .public_key()
        .ok_or_else(|| AddError::File("malformed key".to_string()))?;
    if let Some(other) = file.find_key(&key)
        && (other.name != entry.name || !replace)
    {
        return Err(AddError::KeyInUse(other.name.clone()));
    }
    let Some(index) = file.clients.iter().position(|c| c.name == entry.name) else {
        append(path, &entry).map_err(AddError::File)?;
        return Ok(None);
    };
    if !replace {
        return Err(AddError::NameInUse(entry.name));
    }
    let old = file.clients[index].clone();
    file.clients[index] = AuthEntry {
        agents: old.agents.clone(),
        approve: old.approve,
        ..entry
    };
    rewrite(path, &file).map_err(AddError::File)?;
    Ok(Some(old))
}

/// Remove the entry whose key is `key` from the file at `path` (plan
/// 20261024_02, decision 6: `link.unpair`). The file is rewritten, as by a
/// replace. Returns the entry removed, or `None` when no entry has the key
/// (the file is then left as it is).
pub fn remove(path: &Path, key: &[u8; KEY_LEN]) -> Result<Option<AuthEntry>, String> {
    let mut file = AuthFile::load(path)?;
    let Some(index) = file
        .clients
        .iter()
        .position(|c| c.public_key().as_ref() == Some(key))
    else {
        return Ok(None);
    };
    let old = file.clients.remove(index);
    rewrite(path, &file)?;
    Ok(Some(old))
}

fn open_private(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).mode(0o600);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    options.open(path)
}

fn ensure_dir(path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
        && !dir.exists()
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn entry_toml(entry: &AuthEntry) -> Result<String, String> {
    toml::to_string(&AuthFile {
        clients: vec![entry.clone()],
    })
    .map_err(|e| e.to_string())
}

fn append(path: &Path, entry: &AuthEntry) -> Result<(), String> {
    ensure_dir(path)?;
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut text = String::new();
    if existing.is_empty() {
        text.push_str(HEADER);
    }
    if !existing.is_empty() && !existing.ends_with("\n\n") {
        text.push_str(if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        });
    } else if existing.is_empty() {
        text.push('\n');
    }
    text.push_str(&entry_toml(entry)?);
    let mut file = open_private(path, true).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn rewrite(path: &Path, file: &AuthFile) -> Result<(), String> {
    ensure_dir(path)?;
    let mut text = String::from(HEADER);
    for entry in &file.clients {
        text.push('\n');
        text.push_str(&entry_toml(entry)?);
    }
    let tmp = path.with_extension("toml.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut out = open_private(&tmp, false).map_err(|e| format!("{}: {e}", tmp.display()))?;
    out.write_all(text.as_bytes())
        .and_then(|_| out.sync_all())
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))
}

/// Now, as RFC 3339 in local time: `2026-10-22T11:30:00-04:00`.
pub fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: localtime_r writes only into `tm`, which is a valid, owned
    // value; `now` is a valid time_t.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return format!("@{now}");
    }
    let offset = tm.tm_gmtoff / 60;
    let sign = if offset < 0 { '-' } else { '+' };
    let offset = offset.abs();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        offset / 60,
        offset % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;
    use crate::core::policy::{self, Denied};

    fn key(n: u8) -> [u8; KEY_LEN] {
        [n; KEY_LEN]
    }

    #[test]
    fn entries_append_and_names_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        assert_eq!(AuthFile::load(&path).unwrap(), AuthFile::default());

        add(
            &path,
            AuthEntry::new("s23", &key(1), vec![COMPANION.into()]),
            false,
        )
        .unwrap();
        add(
            &path,
            AuthEntry::new("t3", &key(2), vec![HARNESS.into()]),
            false,
        )
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let file = AuthFile::load(&path).unwrap();
        assert_eq!(file.clients.len(), 2);
        assert_eq!(file.find_key(&key(2)).unwrap().name, "t3");
        assert_eq!(file.find_name("s23").unwrap().public_key(), Some(key(1)));
        assert!(file.find_key(&key(9)).is_none());

        // The same name again is refused, and the file is unchanged.
        let before = std::fs::read_to_string(&path).unwrap();
        let err = add(&path, AuthEntry::new("s23", &key(3), vec![]), false).unwrap_err();
        assert_eq!(err, AddError::NameInUse("s23".to_string()));
        // So is a key already paired under another name.
        let err = add(&path, AuthEntry::new("new", &key(2), vec![]), false).unwrap_err();
        assert_eq!(err, AddError::KeyInUse("t3".to_string()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn replace_swaps_the_key_and_keeps_narrowing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        add(
            &path,
            AuthEntry::new("s23", &key(1), vec![COMPANION.into()]),
            false,
        )
        .unwrap();
        // The owner narrows the entry by hand.
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace(
                "protocols = [\"companion\"]",
                "protocols = [\"companion\"]\nagents = [\"claude\"]\napprove = false",
            ),
        )
        .unwrap();
        add(
            &path,
            AuthEntry::new("t3", &key(2), vec![HARNESS.into()]),
            false,
        )
        .unwrap();

        let old = add(
            &path,
            AuthEntry::new("s23", &key(5), vec![COMPANION.into()]),
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(old.public_key(), Some(key(1)));
        let file = AuthFile::load(&path).unwrap();
        assert_eq!(file.clients.len(), 2);
        let s23 = file.find_name("s23").unwrap();
        assert_eq!(s23.public_key(), Some(key(5)));
        assert_eq!(s23.agents, Some(vec!["claude".to_string()]));
        assert_eq!(s23.approve, Some(false));
        assert!(file.find_key(&key(1)).is_none());
        assert_eq!(file.find_name("t3").unwrap().public_key(), Some(key(2)));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        // Replacing a name not in the file adds it.
        assert_eq!(
            add(&path, AuthEntry::new("new", &key(6), vec![]), true).unwrap(),
            None
        );
        assert_eq!(AuthFile::load(&path).unwrap().clients.len(), 3);
    }

    #[test]
    fn remove_takes_out_one_entry_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        for (name, n) in [("s23", 1), ("t3", 2)] {
            add(
                &path,
                AuthEntry::new(name, &key(n), vec![COMPANION.into()]),
                false,
            )
            .unwrap();
        }
        let removed = remove(&path, &key(1)).unwrap().unwrap();
        assert_eq!(removed.name, "s23");
        let file = AuthFile::load(&path).unwrap();
        assert_eq!(file.clients.len(), 1);
        assert!(file.find_key(&key(1)).is_none());
        assert_eq!(file.find_name("t3").unwrap().public_key(), Some(key(2)));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        // A key not in the file changes nothing.
        let before = std::fs::read_to_string(&path).unwrap();
        assert_eq!(remove(&path, &key(9)).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_file_with_repeats_or_open_to_writes_authorizes_nobody() {
        let k1 = format_key(&key(1));
        let k2 = format_key(&key(2));
        let twice_name = format!(
            "[[client]]\nname = \"a\"\nkey = \"{k1}\"\npaired = \"x\"\n\n[[client]]\nname = \"a\"\nkey = \"{k2}\"\npaired = \"x\"\n"
        );
        assert!(AuthFile::parse(&twice_name).unwrap_err().contains("twice"));
        let twice_key = format!(
            "[[client]]\nname = \"a\"\nkey = \"{k1}\"\npaired = \"x\"\n\n[[client]]\nname = \"b\"\nkey = \"{k1}\"\npaired = \"x\"\n"
        );
        assert!(AuthFile::parse(&twice_key).unwrap_err().contains("twice"));
        let bad_key = "[[client]]\nname = \"a\"\nkey = \"rsa:nope\"\npaired = \"x\"\n";
        assert!(AuthFile::parse(bad_key).unwrap_err().contains("malformed"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        add(&path, AuthEntry::new("a", &key(1), vec![]), false).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o622)).unwrap();
        assert!(AuthFile::load(&path).unwrap_err().contains("writable"));
        // Readable by others is fine: it holds public keys only.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(AuthFile::load(&path).is_ok());
    }

    #[test]
    fn approve_defaults_by_protocol() {
        let mut entry = AuthEntry::new("s23", &key(1), vec![COMPANION.into(), HARNESS.into()]);
        assert!(entry.can_approve(protocols::COMPANION));
        assert!(!entry.can_approve(protocols::HARNESS));
        entry.approve = Some(false);
        assert!(!entry.can_approve(protocols::COMPANION));
        entry.approve = Some(true);
        assert!(entry.can_approve(protocols::HARNESS));

        let p = entry.principal(protocols::HARNESS);
        assert_eq!(p.id, "paired:s23");
        assert!(p.can_approve);
    }

    #[test]
    fn protocols_are_matched_by_short_name() {
        let entry = AuthEntry::new("t3", &key(1), vec![HARNESS.into()]);
        assert!(entry.allows(protocols::HARNESS));
        assert!(entry.allows("harness"));
        assert!(!entry.allows(protocols::COMPANION));
    }

    #[test]
    fn agents_narrow_and_never_widen() {
        let mut entry = AuthEntry::new("s23", &key(1), vec![HARNESS.into()]);
        entry.agents = Some(vec!["claude".to_string(), "codex".to_string()]);
        let principal = entry.principal(protocols::HARNESS);
        let agents = vec![
            AgentConfig::new("claude", vec!["continue".into()], false),
            AgentConfig::new("omp", vec!["continue".into()], false),
        ];
        assert!(policy::check(&principal, &agents, "claude", "p1", "continue").is_ok());
        // omp is configured, but out of this entry's reach.
        assert!(matches!(
            policy::check(&principal, &agents, "omp", "p2", "continue"),
            Err(Denied::OutOfReach { .. })
        ));
        // codex is in the entry, but has no [[agents]] entry: never widened.
        assert!(matches!(
            policy::check(&principal, &agents, "codex", "p3", "continue"),
            Err(Denied::UnconfiguredAgent { .. })
        ));
        // No list leaves [[agents]] as it is.
        entry.agents = None;
        let principal = entry.principal(protocols::HARNESS);
        assert!(policy::check(&principal, &agents, "omp", "p2", "continue").is_ok());
    }

    #[test]
    fn timestamps_are_rfc3339() {
        let t = now_rfc3339();
        assert_eq!(t.len(), 25, "{t}");
        assert_eq!(&t[10..11], "T");
        assert!(matches!(&t[19..20], "+" | "-"), "{t}");
    }
}
