//! Registry of panes the user has asked to be notified about.
//!
//! A watch is purely local state: it decides which `pane.agent_status_changed`
//! subscriptions the event connection carries, and which incoming events are
//! worth speaking. Because herdr closes any connection that receives a second
//! `events.subscribe`, every change here bumps a version that the event
//! supervisor watches so it can reconnect with the new subscription set.

use std::collections::BTreeSet;
use std::sync::Mutex;
use tokio::sync::watch;

/// Panes currently being watched, plus a change signal for the event supervisor.
#[derive(Debug)]
pub struct WatchRegistry {
    panes: Mutex<BTreeSet<String>>,
    version_tx: watch::Sender<u64>,
    version_rx: watch::Receiver<u64>,
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchRegistry {
    pub fn new() -> Self {
        let (version_tx, version_rx) = watch::channel(0);
        Self {
            panes: Mutex::new(BTreeSet::new()),
            version_tx,
            version_rx,
        }
    }

    /// Normalize a pane id for storage and comparison.
    fn normalize(pane_id: &str) -> String {
        pane_id.trim().to_lowercase()
    }

    /// Start watching a pane. Returns `false` if it was already watched.
    pub fn watch(&self, pane_id: &str) -> bool {
        let id = Self::normalize(pane_id);
        if id.is_empty() {
            return false;
        }
        let added = {
            let mut panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
            panes.insert(id)
        };
        if added {
            self.bump();
        }
        added
    }

    /// Stop watching a pane. Returns `false` if it was not being watched.
    ///
    /// Accepts either spelling of the id, so "p6" unwatches "w1:p6".
    pub fn unwatch(&self, pane_id: &str) -> bool {
        let id = Self::normalize(pane_id);
        let removed = {
            let mut panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
            let matches: Vec<String> = panes
                .iter()
                .filter(|p| pane_ids_match(p, &id))
                .cloned()
                .collect();
            for m in &matches {
                panes.remove(m);
            }
            !matches.is_empty()
        };
        if removed {
            self.bump();
        }
        removed
    }

    /// Stop watching every pane. Returns how many watches were dropped.
    pub fn clear(&self) -> usize {
        let dropped = {
            let mut panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
            let n = panes.len();
            panes.clear();
            n
        };
        if dropped > 0 {
            self.bump();
        }
        dropped
    }

    /// Whether events for this pane should be announced.
    pub fn is_watched(&self, pane_id: &str) -> bool {
        let id = Self::normalize(pane_id);
        let panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
        panes.iter().any(|p| pane_ids_match(p, &id))
    }

    /// Currently watched pane ids, in stable order.
    pub fn watched(&self) -> Vec<String> {
        let panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
        panes.iter().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        let panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
        panes.is_empty()
    }

    /// Receiver that fires whenever the watch set changes.
    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.version_rx.clone()
    }

    fn bump(&self) {
        self.version_tx.send_modify(|v| *v = v.wrapping_add(1));
    }
}

/// Compare two pane ids tolerantly: herdr reports fully-qualified ids
/// (`w1:p6`) while the model often says just `p6`.
pub fn pane_ids_match(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a.eq_ignore_ascii_case(b)
        || a.to_lowercase().ends_with(&format!(":{}", b.to_lowercase()))
        || b.to_lowercase().ends_with(&format!(":{}", a.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_and_unwatch() {
        let reg = WatchRegistry::new();
        assert!(reg.is_empty());

        assert!(reg.watch("w1:p6"));
        assert!(!reg.watch("w1:p6"), "second watch is a no-op");
        assert_eq!(reg.watched(), vec!["w1:p6"]);

        assert!(reg.is_watched("w1:p6"));
        // Short and long spellings both resolve.
        assert!(reg.is_watched("p6"));
        assert!(!reg.is_watched("p1"));

        assert!(reg.unwatch("p6"), "short id unwatches the qualified one");
        assert!(reg.is_empty());
        assert!(!reg.unwatch("p6"), "unwatching twice is a no-op");
    }

    #[test]
    fn test_watch_normalizes_and_rejects_blank() {
        let reg = WatchRegistry::new();
        assert!(reg.watch("  W1:P6  "));
        assert_eq!(reg.watched(), vec!["w1:p6"]);
        assert!(!reg.watch("   "));
        assert_eq!(reg.watched().len(), 1);
    }

    #[test]
    fn test_clear_reports_count() {
        let reg = WatchRegistry::new();
        reg.watch("w1:p1");
        reg.watch("w1:p6");
        assert_eq!(reg.clear(), 2);
        assert_eq!(reg.clear(), 0);
    }

    #[test]
    fn test_changes_signal_fires_only_on_real_change() {
        let reg = WatchRegistry::new();
        let rx = reg.subscribe_changes();
        let start = *rx.borrow();

        reg.watch("w1:p6");
        assert_eq!(*rx.borrow(), start + 1);

        reg.watch("w1:p6"); // duplicate: no reconnect needed
        assert_eq!(*rx.borrow(), start + 1);

        reg.unwatch("w1:p6");
        assert_eq!(*rx.borrow(), start + 2);

        reg.unwatch("w1:p6"); // absent: no reconnect needed
        assert_eq!(*rx.borrow(), start + 2);
    }

    #[test]
    fn test_pane_ids_match() {
        assert!(pane_ids_match("w1:p6", "p6"));
        assert!(pane_ids_match("p6", "w1:p6"));
        assert!(pane_ids_match("W1:P6", "w1:p6"));
        assert!(!pane_ids_match("w1:p6", "p61"));
        assert!(!pane_ids_match("w1:p6", ""));
    }
}
