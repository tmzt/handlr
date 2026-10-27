//! The daemon's one file watcher (plan 20261027_05, decision 2, and the
//! 12:37 EDT amendment, item 3).
//!
//! - It runs on `notify` (FSEvents on macOS, inotify on Linux, the
//!   platform's native watcher elsewhere) with `notify-debouncer-mini`,
//!   debounced [`DEBOUNCE`].
//! - It watches folders, not files, so an editor's save by rename is seen.
//!   A subscriber names paths and gets a `tokio::sync::watch` tick when one
//!   of them is created, changed or removed; a sibling's change is no tick.
//! - A path whose folder does not exist yet is watched from the nearest
//!   folder above it that does, and the watch moves down (or up) as folders
//!   are created or removed. [`Subscription::set_paths`] moves a
//!   subscriber's watch to other paths.
//! - If it cannot start, [`FileWatcher::start`] says why, and each user
//!   falls back to its own interval.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use ::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use tokio::sync::watch;

/// How long events are gathered before subscribers are told.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// The watcher; clones share it.
#[derive(Clone)]
pub struct FileWatcher {
    inner: Arc<Inner>,
}

struct Inner {
    /// `None` only while starting.
    debouncer: Mutex<Option<Debouncer<RecommendedWatcher>>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    next: u64,
    subs: HashMap<u64, Sub>,
    /// Each watched folder and how many targets it serves.
    dirs: HashMap<PathBuf, usize>,
}

struct Sub {
    targets: Vec<Target>,
    tx: watch::Sender<u64>,
}

/// One path a subscriber named.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    /// As named, made absolute.
    named: PathBuf,
    /// The folder watched for it: its own, or the nearest above that
    /// exists; canonical.
    dir: PathBuf,
    /// The path under `dir`'s canonical form, as events name it.
    path: PathBuf,
}

impl Target {
    fn of(named: &Path) -> Result<Target, String> {
        let named = if named.is_relative() {
            std::env::current_dir()
                .map_err(|e| format!("the working folder: {e}"))?
                .join(named)
        } else {
            named.to_path_buf()
        };
        let named = lexical(&named);
        let mut rest = vec![
            named
                .file_name()
                .ok_or_else(|| format!("{} names no file", named.display()))?
                .to_os_string(),
        ];
        let mut dir = named
            .parent()
            .ok_or_else(|| format!("{} has no folder", named.display()))?;
        while !dir.is_dir() {
            rest.push(
                dir.file_name()
                    .ok_or_else(|| format!("no folder above {} exists", named.display()))?
                    .to_os_string(),
            );
            dir = dir
                .parent()
                .ok_or_else(|| format!("no folder above {} exists", named.display()))?;
        }
        let dir = dir
            .canonicalize()
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut path = dir.clone();
        for part in rest.iter().rev() {
            path.push(part);
        }
        Ok(Target { named, dir, path })
    }

    /// Whether a change at `event` (normalized) touches this target: the
    /// path itself, or a missing folder on the way to it.
    fn touched_by(&self, event: &Path) -> bool {
        event == self.path || (event != self.dir && self.path.starts_with(event))
    }
}

/// `.` and `..` resolved by the text.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// An event's path with its folder canonical, as targets are kept (on
/// macOS a temporary folder's `/var` is `/private/var`).
fn normalize(event: &Path) -> PathBuf {
    match (event.parent(), event.file_name()) {
        (Some(parent), Some(name)) => match parent.canonicalize() {
            Ok(parent) => parent.join(name),
            Err(_) => event.to_path_buf(),
        },
        _ => event.to_path_buf(),
    }
}

impl FileWatcher {
    /// The platform's watcher, or why it can't start.
    pub fn start() -> Result<FileWatcher, String> {
        let inner = Arc::new(Inner {
            debouncer: Mutex::new(None),
            state: Mutex::new(State::default()),
        });
        let weak = Arc::downgrade(&inner);
        let debouncer = new_debouncer(DEBOUNCE, move |result: DebounceEventResult| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(events) => {
                    let paths: Vec<PathBuf> = events.iter().map(|e| normalize(&e.path)).collect();
                    inner.dispatch(&paths);
                }
                Err(e) => log::warn!("file watcher: {e}"),
            }
        })
        .map_err(|e| e.to_string())?;
        *inner.debouncer.lock().unwrap() = Some(debouncer);
        Ok(FileWatcher { inner })
    }

    /// A tick whenever one of `paths` is created, changed or removed.
    pub fn subscribe(&self, paths: &[PathBuf]) -> Result<Subscription, String> {
        let targets = paths
            .iter()
            .map(|p| Target::of(p))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.inner.state.lock().unwrap();
        self.inner.hold(&mut state, &targets)?;
        let id = state.next;
        state.next += 1;
        let (tx, rx) = watch::channel(0);
        state.subs.insert(id, Sub { targets, tx });
        Ok(Subscription {
            id,
            inner: Arc::downgrade(&self.inner),
            rx,
        })
    }

    /// How many folders are watched (tests).
    pub fn watched_folders(&self) -> usize {
        self.inner.state.lock().unwrap().dirs.len()
    }
}

impl Inner {
    /// Watches the targets' folders, or none of them.
    fn hold(&self, state: &mut State, targets: &[Target]) -> Result<(), String> {
        for (i, t) in targets.iter().enumerate() {
            if let Err(e) = self.add_dir(state, &t.dir) {
                for t in &targets[..i] {
                    self.release_dir(state, &t.dir);
                }
                return Err(e);
            }
        }
        Ok(())
    }

    fn add_dir(&self, state: &mut State, dir: &Path) -> Result<(), String> {
        if let Some(n) = state.dirs.get_mut(dir) {
            *n += 1;
            return Ok(());
        }
        let mut debouncer = self.debouncer.lock().unwrap();
        let debouncer = debouncer.as_mut().ok_or("the watcher is starting")?;
        debouncer
            .watcher()
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("watching {}: {e}", dir.display()))?;
        state.dirs.insert(dir.to_path_buf(), 1);
        Ok(())
    }

    fn release_dir(&self, state: &mut State, dir: &Path) {
        let Some(n) = state.dirs.get_mut(dir) else {
            return;
        };
        *n -= 1;
        if *n > 0 {
            return;
        }
        state.dirs.remove(dir);
        if let Some(debouncer) = self.debouncer.lock().unwrap().as_mut() {
            // A folder that was removed is no longer watched anyway.
            let _ = debouncer.watcher().unwatch(dir);
        }
    }

    /// Ticks the subscribers whose targets `paths` touch, then moves any
    /// target whose nearest existing folder changed (ticking it too).
    fn dispatch(&self, paths: &[PathBuf]) {
        let mut state = self.state.lock().unwrap();
        let mut moves = Vec::new();
        for (&id, sub) in &state.subs {
            let mut tick = paths
                .iter()
                .any(|p| sub.targets.iter().any(|t| t.touched_by(p)));
            for (i, t) in sub.targets.iter().enumerate() {
                if let Ok(now) = Target::of(&t.named)
                    && now != *t
                {
                    moves.push((id, i, now));
                    tick = true;
                }
            }
            if tick {
                sub.tx.send_modify(|n| *n += 1);
            }
        }
        for (id, i, now) in moves {
            if let Err(e) = self.add_dir(&mut state, &now.dir) {
                log::warn!("file watcher: {e}");
                continue;
            }
            let old = std::mem::replace(&mut state.subs.get_mut(&id).unwrap().targets[i], now);
            self.release_dir(&mut state, &old.dir);
        }
    }
}

/// A subscriber's paths and ticks; dropping it stops the watch.
pub struct Subscription {
    id: u64,
    inner: Weak<Inner>,
    rx: watch::Receiver<u64>,
}

impl Subscription {
    /// Waits for the next tick; `false` once the watcher is gone.
    pub async fn changed(&mut self) -> bool {
        self.rx.changed().await.is_ok()
    }

    /// The ticks so far.
    pub fn ticks(&self) -> u64 {
        *self.rx.borrow()
    }

    /// Watches `paths` instead; nothing changes when they are the same.
    pub fn set_paths(&self, paths: &[PathBuf]) -> Result<(), String> {
        let Some(inner) = self.inner.upgrade() else {
            return Ok(());
        };
        let targets = paths
            .iter()
            .map(|p| Target::of(p))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = inner.state.lock().unwrap();
        let Some(sub) = state.subs.get(&self.id) else {
            return Ok(());
        };
        if sub.targets == targets {
            return Ok(());
        }
        inner.hold(&mut state, &targets)?;
        let old = std::mem::replace(&mut state.subs.get_mut(&self.id).unwrap().targets, targets);
        for t in &old {
            inner.release_dir(&mut state, &t.dir);
        }
        Ok(())
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let mut state = inner.state.lock().unwrap();
        if let Some(sub) = state.subs.remove(&self.id) {
            for t in &sub.targets {
                inner.release_dir(&mut state, &t.dir);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Waits up to `within` for a tick; whether one came.
    async fn ticked(sub: &mut Subscription, within: Duration) -> bool {
        tokio::time::timeout(within, sub.changed())
            .await
            .unwrap_or(false)
    }

    const WAIT: Duration = Duration::from_secs(3);
    /// Long enough for a debounced event that should not come.
    const QUIET: Duration = Duration::from_millis(1200);

    #[tokio::test]
    async fn a_write_a_rename_over_and_a_removal_tick_and_a_sibling_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        std::fs::write(&file, "a").unwrap();
        let watcher = FileWatcher::start().unwrap();
        let mut sub = watcher.subscribe(std::slice::from_ref(&file)).unwrap();
        // FSEvents may report what happened just before the watch began.
        let _ = ticked(&mut sub, QUIET).await;

        std::fs::write(dir.path().join("sibling.toml"), "x").unwrap();
        assert!(!ticked(&mut sub, QUIET).await, "a sibling's write");

        std::fs::write(&file, "b").unwrap();
        assert!(ticked(&mut sub, WAIT).await, "a write");

        // An editor's save: a new file renamed over the old one.
        let tmp = dir.path().join(".config.toml.swp");
        std::fs::write(&tmp, "c").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        assert!(ticked(&mut sub, WAIT).await, "a rename over");

        std::fs::remove_file(&file).unwrap();
        assert!(ticked(&mut sub, WAIT).await, "a removal");

        std::fs::write(&file, "d").unwrap();
        assert!(ticked(&mut sub, WAIT).await, "a creation");

        drop(sub);
        assert_eq!(watcher.watched_folders(), 0);
    }

    #[tokio::test]
    async fn a_missing_folder_is_watched_from_above_and_followed_down() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join(".private/gemini_api_key");
        let watcher = FileWatcher::start().unwrap();
        let mut sub = watcher.subscribe(std::slice::from_ref(&key)).unwrap();
        let _ = ticked(&mut sub, QUIET).await;

        std::fs::create_dir(dir.path().join(".private")).unwrap();
        assert!(ticked(&mut sub, WAIT).await, "its folder created");
        let _ = ticked(&mut sub, QUIET).await;
        std::fs::write(&key, "k").unwrap();
        assert!(ticked(&mut sub, WAIT).await, "the key written");
        assert_eq!(watcher.watched_folders(), 1);
    }

    #[tokio::test]
    async fn set_paths_moves_the_watch() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let watcher = FileWatcher::start().unwrap();
        let mut sub = watcher.subscribe(&[a.path().join("k")]).unwrap();
        sub.set_paths(&[b.path().join("k")]).unwrap();
        assert_eq!(watcher.watched_folders(), 1);
        let _ = ticked(&mut sub, QUIET).await;
        std::fs::write(a.path().join("k"), "x").unwrap();
        assert!(!ticked(&mut sub, QUIET).await, "the old folder");
        std::fs::write(b.path().join("k"), "x").unwrap();
        assert!(ticked(&mut sub, WAIT).await, "the new folder");
    }
}
