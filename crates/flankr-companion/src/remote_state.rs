//! One state database per remote (plan 20261028_01, decisions 14-18):
//! `<data dir>/remotes/<id>/state.redb`.
//!
//! In this plan the file holds one table, `meta`: `remote` (the remote's
//! public key in base64), `schema` (1) and `created` (Unix seconds). The
//! data plan's session tables (plan 20261023_09) go in this file when that
//! plan is built, capped per remote at 50 MB with least-recently-used
//! eviction and no session count (plan 20261028_01, the 13:06 EDT
//! amendment); nothing else is written here until then.
//!
//! The directories are made 0700 and a new file 0600 as they are created,
//! never loosened and then tightened (plan 20261028_01, the 14:37 EDT
//! amendment, item 4).
//!
//! Only the active remote's file is open, held by the `net` thread for the
//! life of its connection settings. It is closed (dropped) before its
//! directory is deleted.

use std::fs::{self, DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::keys::{RemoteId, Remotes, ServerRecord};

/// The `meta` table: names to values, all text.
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

/// The layout of `state.redb` this crate writes.
pub const STATE_SCHEMA: u32 = 1;

/// The file name of a remote's state database.
pub const STATE_FILE: &str = "state.redb";

/// The data plan's app-wide store, if it was ever made (decision 18).
pub const SESSIONS_FILE: &str = "sessions.redb";

/// What `meta` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMeta {
    pub remote: [u8; 32],
    pub schema: u32,
    pub created: u64,
}

/// A remote's open state database. Dropping it closes the file.
pub struct RemoteState {
    db: Database,
    path: PathBuf,
    id: RemoteId,
}

impl std::fmt::Debug for RemoteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteState")
            .field("path", &self.path)
            .field("id", &self.id)
            .finish()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The prefix of a file left by an older build that moved another
/// remote's `state.redb` aside; such a file is deleted.
const MISMATCH_PREFIX: &str = "state.redb.mismatch-";

/// Creates `dir` and any missing parents with mode 0700, as they are made
/// (no window in which one is wider). One that exists is left as it is.
fn private_dir(dir: &Path) -> Result<(), String> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// Opens `path` for reading and writing, creating it with mode 0600 if it
/// is missing: the file is never there with a wider mode.
fn private_file(path: &Path) -> Result<fs::File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|e| db_err(path, e))
}

/// Deletes the `state.redb.mismatch-*` files in `dir`. The names deleted.
fn delete_mismatched(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut deleted = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(MISMATCH_PREFIX) {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => deleted.push(name),
            Err(e) => log::warn!("state: deleting {}: {e}", entry.path().display()),
        }
    }
    deleted
}

fn db_err(path: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}

impl RemoteState {
    /// `<data dir>/remotes/`.
    pub fn remotes_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("remotes")
    }

    /// `<data dir>/remotes/<id>/`.
    pub fn dir(data_dir: &Path, id: RemoteId) -> PathBuf {
        Self::remotes_dir(data_dir).join(id.to_string())
    }

    /// `<data dir>/remotes/<id>/state.redb`.
    pub fn file(data_dir: &Path, id: RemoteId) -> PathBuf {
        Self::dir(data_dir, id).join(STATE_FILE)
    }

    /// Opens or creates `remote`'s database, writing `meta` into a new
    /// one. A file whose `meta.remote` names another key is deleted (it
    /// holds another daemon's data) and a fresh one made.
    pub fn open(data_dir: &Path, remote: &ServerRecord) -> Result<RemoteState, String> {
        let dir = Self::dir(data_dir, remote.id);
        private_dir(&Self::remotes_dir(data_dir))?;
        private_dir(&dir)?;
        let path = dir.join(STATE_FILE);
        let state = Self::open_file(&path, remote)?;
        match state.meta()? {
            Some(meta) if meta.remote == remote.public_key => Ok(state),
            Some(_) => {
                drop(state);
                log::warn!("state: {} is another remote's; deleted", path.display());
                fs::remove_file(&path).map_err(|e| db_err(&path, e))?;
                let state = Self::open_file(&path, remote)?;
                state.write_meta(&remote.public_key)?;
                Ok(state)
            }
            None => {
                state.write_meta(&remote.public_key)?;
                Ok(state)
            }
        }
    }

    fn open_file(path: &Path, remote: &ServerRecord) -> Result<RemoteState, String> {
        let file = private_file(path)?;
        let db = Database::builder()
            .create_file(file)
            .map_err(|e| db_err(path, e))?;
        Ok(RemoteState {
            db,
            path: path.to_path_buf(),
            id: remote.id,
        })
    }

    fn write_meta(&self, remote: &[u8; 32]) -> Result<(), String> {
        write_meta(&self.db, &self.path, remote)
    }

    /// The remote this database is for.
    pub fn id(&self) -> RemoteId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The `meta` table, or `None` if it was never written.
    pub fn meta(&self) -> Result<Option<StateMeta>, String> {
        let path = &self.path;
        let read = self.db.begin_read().map_err(|e| db_err(path, e))?;
        let table = match read.open_table(META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(db_err(path, e)),
        };
        let get = |name: &str| -> Result<Option<String>, String> {
            Ok(table
                .get(name)
                .map_err(|e| db_err(path, e))?
                .map(|v| v.value().to_string()))
        };
        let Some(remote) = get("remote")? else {
            return Ok(None);
        };
        let remote: [u8; 32] = STANDARD
            .decode(remote.trim())
            .ok()
            .and_then(|k| k.try_into().ok())
            .ok_or_else(|| db_err(path, "meta.remote is not a 32-byte key in base64"))?;
        let schema = get("schema")?.and_then(|s| s.parse().ok()).unwrap_or(0);
        let created = get("created")?.and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok(Some(StateMeta {
            remote,
            schema,
            created,
        }))
    }

    /// Deletes `<data dir>/remotes/<id>/`, which must not be open. Nothing
    /// there is not an error.
    pub fn delete(data_dir: &Path, id: RemoteId) -> Result<(), String> {
        let dir = Self::dir(data_dir, id);
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", dir.display())),
        }
    }

    /// Deletes every directory under `<data dir>/remotes/` that has no
    /// entry in `remotes` (decision 22: one left by a delete that failed),
    /// and any `state.redb.mismatch-*` file an older build left in a
    /// listed one. Returns the ids of the directories deleted.
    pub fn sweep(data_dir: &Path, remotes: &Remotes) -> Vec<String> {
        let root = Self::remotes_dir(data_dir);
        let Ok(entries) = fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut swept = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let listed = name
                .parse::<RemoteId>()
                .is_ok_and(|id| remotes.get(id).is_some());
            if listed {
                for file in delete_mismatched(&entry.path()) {
                    log::info!("state: deleted remotes/{name}/{file}");
                }
                continue;
            }
            if !entry.path().is_dir() {
                continue;
            }
            match fs::remove_dir_all(entry.path()) {
                Ok(()) => {
                    log::info!("state: deleted remotes/{name}, which no remote lists");
                    swept.push(name);
                }
                Err(e) => log::warn!("state: deleting remotes/{name}: {e}"),
            }
        }
        swept
    }

    /// Moves the data plan's app-wide `<data dir>/sessions.redb`, if there
    /// is one, into the active remote's directory as its `state.redb`,
    /// after writing `meta` into it; with no active remote it is deleted
    /// (decision 18). The active remote's own file, if any, is replaced:
    /// in this plan it holds only `meta`.
    pub fn adopt_sessions(data_dir: &Path, active: Option<&ServerRecord>) -> Result<(), String> {
        let old = data_dir.join(SESSIONS_FILE);
        if !old.exists() {
            return Ok(());
        }
        let Some(remote) = active else {
            log::info!("state: deleted {SESSIONS_FILE}; no remote is in use");
            return fs::remove_file(&old).map_err(|e| db_err(&old, e));
        };
        {
            let db = Database::create(&old).map_err(|e| db_err(&old, e))?;
            write_meta(&db, &old, &remote.public_key)?;
        }
        let dir = Self::dir(data_dir, remote.id);
        private_dir(&Self::remotes_dir(data_dir))?;
        private_dir(&dir)?;
        let new = dir.join(STATE_FILE);
        // Narrowed before it moves, so `state.redb` is never wider.
        fs::set_permissions(&old, fs::Permissions::from_mode(0o600))
            .map_err(|e| db_err(&old, e))?;
        fs::rename(&old, &new).map_err(|e| db_err(&old, e))?;
        log::info!(
            "state: moved {SESSIONS_FILE} into remotes/{}/{STATE_FILE}",
            remote.id
        );
        Ok(())
    }
}

/// Writes `meta` for `remote`: its key, [`STATE_SCHEMA`], and now as
/// `created` unless one is there.
fn write_meta(db: &Database, path: &Path, remote: &[u8; 32]) -> Result<(), String> {
    let write = db.begin_write().map_err(|e| db_err(path, e))?;
    {
        let mut table = write.open_table(META).map_err(|e| db_err(path, e))?;
        let created = table
            .get("created")
            .map_err(|e| db_err(path, e))?
            .map(|v| v.value().to_string());
        table
            .insert("remote", STANDARD.encode(remote).as_str())
            .map_err(|e| db_err(path, e))?;
        table
            .insert("schema", STATE_SCHEMA.to_string().as_str())
            .map_err(|e| db_err(path, e))?;
        if created.is_none() {
            table
                .insert("created", now_secs().to_string().as_str())
                .map_err(|e| db_err(path, e))?;
        }
    }
    write.commit().map_err(|e| db_err(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(n: u8) -> ServerRecord {
        ServerRecord::new(format!("m{n}"), [n; 32], vec!["127.0.0.1:7347".into()])
    }

    #[test]
    fn meta_is_written_once_and_read_back() {
        let data = tempfile::tempdir().unwrap();
        let r = remote(1);
        let state = RemoteState::open(data.path(), &r).unwrap();
        let path = data
            .path()
            .join("remotes")
            .join(r.id.to_string())
            .join("state.redb");
        assert_eq!(state.path(), path);
        assert!(path.exists());
        let meta = state.meta().unwrap().expect("meta");
        assert_eq!(meta.remote, r.public_key);
        assert_eq!(meta.schema, STATE_SCHEMA);
        assert!(meta.created > 0);
        drop(state);
        // Reopened: the same meta, not written again.
        let again = RemoteState::open(data.path(), &r).unwrap();
        assert_eq!(again.meta().unwrap(), Some(meta));
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn new_directories_and_files_are_private_from_the_start() {
        let data = tempfile::tempdir().unwrap();
        let r = remote(1);
        let state = RemoteState::open(data.path(), &r).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&RemoteState::remotes_dir(data.path())), 0o700);
        assert_eq!(mode(&RemoteState::dir(data.path(), r.id)), 0o700);
        assert_eq!(mode(state.path()), 0o600);
        // The file is opened with its mode, then handed to redb.
        let fresh = data.path().join("fresh.redb");
        let file = private_file(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o600, "0600 as created");
        drop(Database::builder().create_file(file).unwrap());
        assert_eq!(mode(&fresh), 0o600);
    }

    #[test]
    fn another_remotes_file_is_deleted() {
        let data = tempfile::tempdir().unwrap();
        let r = remote(1);
        let other = remote(2);
        // Another remote's file where r's should be.
        {
            let path = RemoteState::file(data.path(), r.id);
            private_dir(path.parent().unwrap()).unwrap();
            let db = Database::create(&path).unwrap();
            write_meta(&db, &path, &other.public_key).unwrap();
        }
        let state = RemoteState::open(data.path(), &r).unwrap();
        assert_eq!(state.meta().unwrap().unwrap().remote, r.public_key);
        let names: Vec<String> = fs::read_dir(RemoteState::dir(data.path(), r.id))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["state.redb"], "nothing kept aside");
    }

    #[test]
    fn the_sweep_deletes_mismatch_files_left_in_listed_directories() {
        let data = tempfile::tempdir().unwrap();
        let kept = remote(1);
        drop(RemoteState::open(data.path(), &kept).unwrap());
        let dir = RemoteState::dir(data.path(), kept.id);
        fs::write(dir.join("state.redb.mismatch-1793188800"), b"old").unwrap();
        fs::write(dir.join("other"), b"kept").unwrap();
        let remotes = Remotes {
            active: Some(kept.id),
            list: vec![kept.clone()],
        };
        assert!(RemoteState::sweep(data.path(), &remotes).is_empty());
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["other", "state.redb"]);
    }

    #[test]
    fn a_closed_file_can_be_deleted_and_made_again() {
        let data = tempfile::tempdir().unwrap();
        let r = remote(1);
        let state = RemoteState::open(data.path(), &r).unwrap();
        drop(state);
        RemoteState::delete(data.path(), r.id).unwrap();
        assert!(!RemoteState::dir(data.path(), r.id).exists());
        // Nothing there: not an error.
        RemoteState::delete(data.path(), r.id).unwrap();
        let state = RemoteState::open(data.path(), &r).unwrap();
        assert!(state.meta().unwrap().is_some());
    }

    #[test]
    fn the_sweep_deletes_directories_no_remote_lists() {
        let data = tempfile::tempdir().unwrap();
        let kept = remote(1);
        let orphan = remote(2);
        drop(RemoteState::open(data.path(), &kept).unwrap());
        drop(RemoteState::open(data.path(), &orphan).unwrap());
        fs::create_dir_all(RemoteState::remotes_dir(data.path()).join("junk")).unwrap();
        let remotes = Remotes {
            active: Some(kept.id),
            list: vec![kept.clone()],
        };
        let mut swept = RemoteState::sweep(data.path(), &remotes);
        swept.sort();
        let mut expected = vec![orphan.id.to_string(), "junk".to_string()];
        expected.sort();
        assert_eq!(swept, expected);
        assert!(RemoteState::dir(data.path(), kept.id).exists());
        assert!(!RemoteState::dir(data.path(), orphan.id).exists());
        // No remotes/ at all: nothing to do.
        let empty = tempfile::tempdir().unwrap();
        assert!(RemoteState::sweep(empty.path(), &remotes).is_empty());
    }

    #[test]
    fn an_app_wide_sessions_file_moves_to_the_active_remote() {
        let data = tempfile::tempdir().unwrap();
        let r = remote(1);
        let old = data.path().join(SESSIONS_FILE);
        {
            let db = Database::create(&old).unwrap();
            const SESSIONS: TableDefinition<&str, &str> = TableDefinition::new("sessions");
            let w = db.begin_write().unwrap();
            w.open_table(SESSIONS)
                .unwrap()
                .insert("s1", "kept")
                .unwrap();
            w.commit().unwrap();
        }
        RemoteState::adopt_sessions(data.path(), Some(&r)).unwrap();
        assert!(!old.exists());
        let state = RemoteState::open(data.path(), &r).unwrap();
        assert_eq!(state.meta().unwrap().unwrap().remote, r.public_key);
        // Its tables came with it.
        const SESSIONS: TableDefinition<&str, &str> = TableDefinition::new("sessions");
        let read = state.db.begin_read().unwrap();
        let table = read.open_table(SESSIONS).unwrap();
        assert_eq!(table.get("s1").unwrap().unwrap().value(), "kept");

        // With no active remote, it is deleted.
        let data = tempfile::tempdir().unwrap();
        let old = data.path().join(SESSIONS_FILE);
        drop(Database::create(&old).unwrap());
        RemoteState::adopt_sessions(data.path(), None).unwrap();
        assert!(!old.exists());
        assert!(!RemoteState::remotes_dir(data.path()).exists());
    }
}
