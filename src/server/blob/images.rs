//! The `images` store (docs/design/20261028_05_BINARY_FILES_PLAN.md,
//! decision 5): uploads written only to the fixed, protected folder
//! `<temp>/flankr-<uid>/images/<pane>/`, its references, and its cleanup.
//!
//! Every folder is opened with `O_NOFOLLOW` and checked to be the daemon's
//! own with mode 0700; a file is written as `<name>.part` with `O_EXCL`,
//! mode 0600, and `linkat`ed to its final name once its size and SHA-256
//! match. The phone's display name is never used in a path and never
//! logged. All of it is blocking; callers run it on the blocking pool.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use libflankr_blob::{Code, DAEMON_IMAGES_ACTIVE, ImageType, REFERENCE_LEN};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

/// How long a DONE's reference may be named by a send (decision 5.6).
pub const IMAGE_REF_TTL: Duration = Duration::from_secs(30 * 60);
/// A `.part` older than this is deleted by cleanup.
pub const PART_MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// A stored image older than this (by mtime) is deleted by cleanup.
pub const IMAGE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The folder's largest total, incoming image included.
pub const STORE_MAX_BYTES: u64 = 500 * 1024 * 1024;
/// How often cleanup runs besides at start and before each ACCEPT.
pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// The longest `<pane>` folder name.
const PANE_DIR_MAX: usize = 64;

/// One live reference (decision 5.6).
#[derive(Debug, Clone)]
pub struct Stored {
    /// The image's absolute path.
    pub path: PathBuf,
    pub pane_id: String,
    pub principal: String,
    pub at: Instant,
}

/// Why a reference cannot be used by a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefError {
    /// Unknown, used, or another principal's or pane's.
    NotYours,
    /// Older than [`IMAGE_REF_TTL`].
    Expired,
}

/// The store, one per daemon.
pub struct ImageStore {
    /// `temp_dir()` as read at start (or the test hook's folder).
    temp: PathBuf,
    uid: u32,
    /// Set once the store finds a folder that is not private, or a temp
    /// path a pasted path could not hold: every OFFER is then `io`.
    off: AtomicBool,
    refs: Mutex<HashMap<[u8; REFERENCE_LEN], Stored>>,
    /// Incoming images across connections (`DAEMON_IMAGES_ACTIVE`).
    active: Arc<AtomicUsize>,
}

/// Holds one of the daemon's incoming-image places until dropped.
pub struct ActiveSlot(Arc<AtomicUsize>);

impl Drop for ActiveSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// `pane_id` as a folder name: every character outside `[A-Za-z0-9_-]`
/// replaced by `_`, cut to 64 characters (decision 5.1).
pub fn pane_dir_name(pane_id: &str) -> String {
    pane_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(PANE_DIR_MAX)
        .collect()
}

/// The stored file's name: `<YYYYMMDD-HHMMSS>-<hash8>.<ext>`, the local
/// time `now`.
pub fn file_name(now: SystemTime, sha256: &[u8; 32], ty: ImageType) -> String {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: localtime_r writes only into `tm`, a valid, owned value;
    // `secs` is a valid time_t.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let stamp = if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        format!("{secs}")
    } else {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    };
    format!("{stamp}-{}.{}", libflankr_blob::hex(&sha256[..4]), ty.ext())
}

fn io_code(e: impl std::fmt::Display) -> (Code, String) {
    (Code::IO, e.to_string())
}

impl ImageStore {
    /// The store under `temp` (the daemon's `temp_dir()`), for `uid`.
    pub fn new(temp: PathBuf, uid: u32) -> ImageStore {
        let store = ImageStore {
            temp,
            uid,
            off: AtomicBool::new(false),
            refs: Mutex::new(HashMap::new()),
            active: Arc::new(AtomicUsize::new(0)),
        };
        // A path with a space or a quote could not be pasted as one word
        // (decision 7.4): the store is off.
        let text = store.temp.to_string_lossy();
        if text.contains([' ', '"', '\'']) || text.chars().any(char::is_control) {
            store.turn_off(&format!(
                "the temporary folder {text} holds a space or a quote; images are refused"
            ));
        }
        store
    }

    /// `<temp>/flankr-<uid>/images`.
    pub fn root(&self) -> PathBuf {
        self.temp
            .join(format!("flankr-{}", self.uid))
            .join("images")
    }

    pub fn is_off(&self) -> bool {
        self.off.load(Ordering::SeqCst)
    }

    fn turn_off(&self, why: &str) {
        if !self.off.swap(true, Ordering::SeqCst) {
            log::warn!("{why}");
        }
    }

    /// One of the daemon's `DAEMON_IMAGES_ACTIVE` places, or `None` (busy).
    pub fn take_slot(&self) -> Option<ActiveSlot> {
        let n = self.active.fetch_add(1, Ordering::SeqCst);
        if n >= DAEMON_IMAGES_ACTIVE {
            self.active.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(ActiveSlot(Arc::clone(&self.active)))
    }

    /// Opens `name` under `parent` without following a link, creating it
    /// 0700 when missing, and checks it is the daemon's own with mode 0700
    /// exactly; otherwise the store is off.
    fn private_dir(&self, parent: &OwnedFd, name: &str, shown: &Path) -> Result<OwnedFd, Code> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match rustix::fs::openat(parent, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => {
                match rustix::fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(e) => {
                        log::warn!("the image folder {} cannot be made: {e}", shown.display());
                        return Err(Code::IO);
                    }
                }
                rustix::fs::openat(parent, name, flags, Mode::empty()).map_err(|e| {
                    self.refuse_dir(shown, &e.to_string());
                    Code::IO
                })?
            }
            Err(e) => {
                // A link (ELOOP, or ENOTDIR on some systems) or anything
                // else that is not a plain folder.
                self.refuse_dir(shown, &e.to_string());
                return Err(Code::IO);
            }
        };
        let stat = rustix::fs::fstat(&fd).map_err(|_| Code::IO)?;
        let mode = stat.st_mode as u32 & 0o7777;
        if stat.st_uid != self.uid || mode != 0o700 {
            self.turn_off(&format!(
                "the image folder {} is not private: owner {}, mode {:o}; images are refused",
                shown.display(),
                stat.st_uid,
                mode
            ));
            return Err(Code::IO);
        }
        Ok(fd)
    }

    fn refuse_dir(&self, shown: &Path, why: &str) {
        self.turn_off(&format!(
            "the image folder {} is not private: it cannot be opened as a folder ({why}); \
             images are refused",
            shown.display()
        ));
    }

    /// The `images` folder, made safely.
    fn images_dir(&self) -> Result<OwnedFd, Code> {
        if self.is_off() {
            return Err(Code::IO);
        }
        let temp = rustix::fs::open(
            &self.temp,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| {
            log::warn!(
                "the temporary folder {} cannot be opened: {e}",
                self.temp.display()
            );
            Code::IO
        })?;
        let mine = format!("flankr-{}", self.uid);
        let mine_path = self.temp.join(&mine);
        let mine_fd = self.private_dir(&temp, &mine, &mine_path)?;
        self.private_dir(&mine_fd, "images", &self.root())
    }

    /// A new `.part` for `pane_id`, to be written and then [`Part::finish`]ed.
    pub fn create_part(&self, pane_id: &str) -> Result<Part, Code> {
        let images = self.images_dir()?;
        let pane = pane_dir_name(pane_id);
        let folder = self.root().join(&pane);
        let dir = self.private_dir(&images, &pane, &folder)?;
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).map_err(|_| Code::IO)?;
        let part_name = format!("{}.part", libflankr_blob::hex(&nonce));
        let fd = rustix::fs::openat(
            &dir,
            part_name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| {
            log::warn!("an image cannot be written in {}: {e}", folder.display());
            Code::IO
        })?;
        Ok(Part {
            dir,
            folder,
            part_name,
            file: Some(std::fs::File::from(fd)),
            written: 0,
        })
    }

    /// Remembers a stored image under a new reference.
    pub fn remember(&self, path: PathBuf, pane_id: &str, principal: &str) -> [u8; REFERENCE_LEN] {
        let mut refs = self.refs.lock().unwrap();
        refs.retain(|_, s| s.at.elapsed() < IMAGE_REF_TTL);
        loop {
            let mut r = [0u8; REFERENCE_LEN];
            if getrandom::getrandom(&mut r).is_err() {
                // No randomness: a counter-free fallback from the clock.
                let n = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                r = n.to_be_bytes();
            }
            if let std::collections::hash_map::Entry::Vacant(v) = refs.entry(r) {
                v.insert(Stored {
                    path: path.clone(),
                    pane_id: pane_id.to_string(),
                    principal: principal.to_string(),
                    at: Instant::now(),
                });
                return r;
            }
        }
    }

    /// The image `reference` names, if `principal` holds it for `pane_id`
    /// and it is live; not consumed.
    pub fn check(
        &self,
        reference: &str,
        pane_id: &str,
        principal: &str,
    ) -> Result<PathBuf, RefError> {
        let r = libflankr_blob::parse_reference(reference).ok_or(RefError::NotYours)?;
        let refs = self.refs.lock().unwrap();
        let s = refs.get(&r).ok_or(RefError::NotYours)?;
        if s.principal != principal || !crate::watch::pane_ids_match(&s.pane_id, pane_id) {
            return Err(RefError::NotYours);
        }
        if s.at.elapsed() >= IMAGE_REF_TTL {
            return Err(RefError::Expired);
        }
        Ok(s.path.clone())
    }

    /// Forgets `references` (a send used them; decision 5.6).
    pub fn consume(&self, references: &[String]) {
        let mut refs = self.refs.lock().unwrap();
        for r in references {
            if let Some(r) = libflankr_blob::parse_reference(r) {
                refs.remove(&r);
            }
        }
    }

    /// The paths live references name, which cleanup keeps.
    fn live_paths(&self) -> Vec<PathBuf> {
        self.refs
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.at.elapsed() < IMAGE_REF_TTL)
            .map(|s| s.path.clone())
            .collect()
    }

    /// Cleanup (decision 5.7), making room for `incoming` bytes: `Err`
    /// [`Code::NO_SPACE`] when it cannot. A store that was never made is
    /// clean.
    pub fn cleanup(&self, incoming: u64) -> Result<Cleaned, Code> {
        self.cleanup_at(incoming, SystemTime::now())
    }

    /// As [`cleanup`](Self::cleanup), as of `now` (tests).
    pub fn cleanup_at(&self, incoming: u64, now: SystemTime) -> Result<Cleaned, Code> {
        let mut cleaned = Cleaned::default();
        if self.is_off() {
            return Err(Code::IO);
        }
        // Only an existing store is walked; making it is create_part's.
        if !self.root().exists() {
            return Ok(cleaned);
        }
        let images = self.images_dir()?;
        let live = self.live_paths();
        let mut files: Vec<(SystemTime, u64, String, String)> = Vec::new();
        let mut total = 0u64;
        for pane in list(&images)? {
            let stat = match rustix::fs::statat(&images, pane.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if FileType::from_raw_mode(stat.st_mode as _) != FileType::Directory {
                odd(&images, &pane, &mut cleaned);
                continue;
            }
            let shown = self.root().join(&pane);
            let Ok(dir) = self.private_dir(&images, &pane, &shown) else {
                continue;
            };
            let mut kept = 0;
            for name in list(&dir)? {
                let Ok(stat) = rustix::fs::statat(&dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                else {
                    continue;
                };
                if FileType::from_raw_mode(stat.st_mode as _) != FileType::RegularFile {
                    odd(&dir, &name, &mut cleaned);
                    continue;
                }
                let mtime =
                    SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_mtime.max(0) as u64);
                let size = stat.st_size.max(0) as u64;
                let age = now.duration_since(mtime).unwrap_or_default();
                let stale = if name.ends_with(".part") {
                    age >= PART_MAX_AGE
                } else {
                    age >= IMAGE_MAX_AGE && !live.contains(&shown.join(&name))
                };
                if stale {
                    if rustix::fs::unlinkat(&dir, name.as_str(), AtFlags::empty()).is_ok() {
                        cleaned.count += 1;
                        cleaned.bytes += size;
                    }
                    continue;
                }
                kept += 1;
                total += size;
                if !name.ends_with(".part") && !live.contains(&shown.join(&name)) {
                    files.push((mtime, size, pane.clone(), name));
                }
            }
            if kept == 0 {
                let _ = rustix::fs::unlinkat(&images, pane.as_str(), AtFlags::REMOVEDIR);
            }
        }
        // Oldest first, until the incoming image fits.
        files.sort();
        let limit = STORE_MAX_BYTES.saturating_sub(incoming);
        for (_, size, pane, name) in files {
            if total <= limit {
                break;
            }
            let Ok(dir) = self.private_dir(&images, &pane, &self.root().join(&pane)) else {
                continue;
            };
            if rustix::fs::unlinkat(&dir, name.as_str(), AtFlags::empty()).is_ok() {
                total -= size;
                cleaned.count += 1;
                cleaned.bytes += size;
            }
        }
        if cleaned.count > 0 {
            log::info!(
                "Image cleanup deleted {} file(s), {} bytes",
                cleaned.count,
                cleaned.bytes
            );
        }
        if incoming > 0 && total > limit {
            return Err(Code::NO_SPACE);
        }
        Ok(cleaned)
    }
}

/// What a cleanup deleted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cleaned {
    pub count: usize,
    pub bytes: u64,
}

/// The names in `dir`, but `.` and `..`.
fn list(dir: &OwnedFd) -> Result<Vec<String>, Code> {
    let entries = Dir::read_from(dir).map_err(|_| Code::IO)?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().ok().map(str::to_string))
        .filter(|n| n != "." && n != "..")
        .collect())
}

/// Something that is neither a regular file nor a folder: unlinked, never
/// followed, logged once.
fn odd(dir: &OwnedFd, name: &str, cleaned: &mut Cleaned) {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if rustix::fs::unlinkat(dir, name, AtFlags::empty()).is_ok() {
        cleaned.count += 1;
        if !LOGGED.swap(true, Ordering::Relaxed) {
            log::info!("Image cleanup deleted something that was not a file or a folder");
        }
    }
}

/// An image being written: `<pane>/<random>.part`, deleted when dropped
/// unless [`finish`](Self::finish)ed.
pub struct Part {
    dir: OwnedFd,
    folder: PathBuf,
    part_name: String,
    file: Option<std::fs::File>,
    written: u64,
}

impl Part {
    /// Appends `data`.
    pub fn write(&mut self, data: &[u8]) -> Result<(), (Code, String)> {
        let file = self.file.as_mut().ok_or((Code::IO, "closed".into()))?;
        file.write_all(data).map_err(io_code)?;
        self.written += data.len() as u64;
        Ok(())
    }

    /// Synced and linked to its final name, `name`, and the `.part`
    /// unlinked: the image's absolute path. A file of that name with the
    /// same size is the same image, kept as it is.
    pub fn finish(mut self, name: &str) -> Result<PathBuf, (Code, String)> {
        let file = self.file.take().ok_or((Code::IO, "closed".into()))?;
        file.sync_all().map_err(io_code)?;
        drop(file);
        let linked = rustix::fs::linkat(
            &self.dir,
            self.part_name.as_str(),
            &self.dir,
            name,
            AtFlags::empty(),
        );
        match linked {
            Ok(()) => {}
            Err(rustix::io::Errno::EXIST) => {
                let same = rustix::fs::statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW)
                    .is_ok_and(|s| {
                        FileType::from_raw_mode(s.st_mode as _) == FileType::RegularFile
                            && s.st_size.max(0) as u64 == self.written
                    });
                if !same {
                    return Err((Code::IO, format!("{name} exists")));
                }
            }
            Err(e) => return Err(io_code(e)),
        }
        // Drop unlinks the .part.
        Ok(self.folder.join(name))
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        self.file = None;
        let _ = rustix::fs::unlinkat(&self.dir, self.part_name.as_str(), AtFlags::empty());
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    fn uid() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    fn store(dir: &Path) -> ImageStore {
        ImageStore::new(dir.to_path_buf(), uid())
    }

    const PNG: [u8; 16] = [
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ];

    fn upload(s: &ImageStore, pane: &str, bytes: &[u8]) -> PathBuf {
        let mut part = s.create_part(pane).unwrap();
        part.write(bytes).unwrap();
        let sha = libflankr_blob::sha256(bytes);
        part.finish(&file_name(SystemTime::now(), &sha, ImageType::Png))
            .unwrap()
    }

    #[test]
    fn an_upload_lands_in_the_protected_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let path = upload(&s, "w1:p6", &PNG);
        let pane = dir.path().join(format!("flankr-{}/images/w1_p6", uid()));
        assert_eq!(path.parent().unwrap(), pane);
        let name = path.file_name().unwrap().to_str().unwrap();
        // <YYYYMMDD-HHMMSS>-<hash8>.png
        assert_eq!(name.len(), 15 + 1 + 8 + 4, "{name}");
        assert!(name.ends_with(&format!(
            "-{}.png",
            libflankr_blob::hex(&libflankr_blob::sha256(&PNG)[..4])
        )));
        for d in [
            dir.path().join(format!("flankr-{}", uid())),
            dir.path().join(format!("flankr-{}/images", uid())),
            pane.clone(),
        ] {
            assert_eq!(
                std::fs::metadata(&d).unwrap().mode() & 0o7777,
                0o700,
                "{d:?}"
            );
        }
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
        // No .part is left.
        let names: Vec<_> = std::fs::read_dir(&pane).unwrap().flatten().collect();
        assert_eq!(names.len(), 1);
        // The same image in the same second is the same file.
        let again = upload(&s, "w1:p6", &PNG);
        assert_eq!(again, path);
        assert_eq!(std::fs::read_dir(&pane).unwrap().count(), 1);
    }

    #[test]
    fn a_dropped_part_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let mut part = s.create_part("p1").unwrap();
        part.write(b"half").unwrap();
        drop(part);
        let pane = dir.path().join(format!("flankr-{}/images/p1", uid()));
        assert_eq!(std::fs::read_dir(&pane).unwrap().count(), 0);
    }

    #[test]
    fn a_symlink_or_an_open_folder_turns_the_store_off() {
        for case in ["mine_link", "images_link", "pane_link", "mode_0755"] {
            let dir = tempfile::tempdir().unwrap();
            let elsewhere = tempfile::tempdir().unwrap();
            let mine = dir.path().join(format!("flankr-{}", uid()));
            let images = mine.join("images");
            match case {
                "mine_link" => std::os::unix::fs::symlink(elsewhere.path(), &mine).unwrap(),
                "images_link" => {
                    std::fs::create_dir(&mine).unwrap();
                    std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    std::os::unix::fs::symlink(elsewhere.path(), &images).unwrap();
                }
                "pane_link" => {
                    std::fs::create_dir_all(&images).unwrap();
                    for d in [&mine, &images] {
                        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))
                            .unwrap();
                    }
                    std::os::unix::fs::symlink(elsewhere.path(), images.join("p1")).unwrap();
                }
                _ => {
                    std::fs::create_dir(&mine).unwrap();
                    std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                }
            }
            let s = store(dir.path());
            assert!(matches!(s.create_part("p1"), Err(Code::IO)), "{case}");
            assert!(s.is_off(), "{case}");
            // Off for the run: a later, fixed folder is still refused.
            assert!(s.create_part("p2").is_err(), "{case}");
            assert_eq!(
                std::fs::read_dir(elsewhere.path()).unwrap().count(),
                0,
                "{case}: nothing written through the link"
            );
        }
    }

    #[test]
    fn a_folder_of_another_owner_turns_the_store_off() {
        let dir = tempfile::tempdir().unwrap();
        let s = ImageStore::new(dir.path().to_path_buf(), uid() + 1);
        // The folder is made by this uid, so it is not the "daemon's".
        assert!(s.create_part("p1").is_err());
        assert!(s.is_off());
    }

    #[test]
    fn a_temp_path_with_a_space_turns_the_store_off() {
        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("a b");
        std::fs::create_dir(&spaced).unwrap();
        let s = store(&spaced);
        assert!(s.is_off());
        assert!(s.create_part("p1").is_err());
    }

    #[test]
    fn pane_names_are_cleaned_and_cut() {
        assert_eq!(pane_dir_name("w1:p6"), "w1_p6");
        assert_eq!(pane_dir_name("../x y"), "___x_y");
        assert_eq!(pane_dir_name(&"a".repeat(100)).len(), 64);
    }

    #[test]
    fn references_are_per_principal_and_pane_and_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let path = upload(&s, "p1", &PNG);
        let r = libflankr_blob::hex(&s.remember(path.clone(), "p1", "paired:s23"));
        assert_eq!(s.check(&r, "p1", "paired:s23"), Ok(path));
        assert_eq!(s.check(&r, "p2", "paired:s23"), Err(RefError::NotYours));
        assert_eq!(s.check(&r, "p1", "paired:tab"), Err(RefError::NotYours));
        assert_eq!(s.check("zz", "p1", "paired:s23"), Err(RefError::NotYours));
        s.consume(std::slice::from_ref(&r));
        assert_eq!(s.check(&r, "p1", "paired:s23"), Err(RefError::NotYours));
    }

    #[test]
    fn cleanup_by_age_size_and_live_reference() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert_eq!(s.cleanup(0), Ok(Cleaned::default()), "no store yet");
        let a = upload(&s, "p1", &PNG);
        let mut other = PNG.to_vec();
        other.push(1);
        let b = upload(&s, "p2", &other);
        // A stale .part.
        let part = s.create_part("p1").unwrap();
        std::mem::forget(part);
        let now = SystemTime::now();
        // An hour on: the .part goes, the images stay.
        let c = s.cleanup_at(0, now + PART_MAX_AGE).unwrap();
        assert_eq!(c.count, 1);
        assert!(a.exists() && b.exists());
        // Eight days on: a live reference keeps `a`; `b` goes, and its
        // empty pane folder with it.
        let r = s.remember(a.clone(), "p1", "x");
        let c = s
            .cleanup_at(0, now + IMAGE_MAX_AGE + Duration::from_secs(1))
            .unwrap();
        assert_eq!(c.count, 1);
        assert!(a.exists() && !b.exists());
        assert!(!b.parent().unwrap().exists());
        // Over the size: the oldest unreferenced go first; with only a
        // live one there is no room.
        s.consume(&[libflankr_blob::hex(&r)]);
        let c = s.cleanup_at(STORE_MAX_BYTES, now).unwrap();
        assert_eq!(c.count, 1, "a made room");
        assert!(!a.exists());
        let a2 = upload(&s, "p1", &PNG);
        let _r = s.remember(a2.clone(), "p1", "x");
        assert_eq!(s.cleanup_at(STORE_MAX_BYTES, now), Err(Code::NO_SPACE));
        assert!(a2.exists());
    }
}
