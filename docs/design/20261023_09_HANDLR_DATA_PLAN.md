---
title: handlr-data, the shared session data model with memory and redb stores
date: 2026-10-23
status: Draft
covers:
  - "A new crate, `crates/handlr-data`: the session data model shared by
    the daemon, the protocol and the companion (serials, sections, cards,
    turns, entries, links), and the stores that keep it"
  - "A `SectionStore` trait with two stores: in memory, and on disk in a
    redb file"
  - "The daemon caching each session's line index and sections in redb,
    so a restart resumes instead of rescanning; the phone keeping its
    scrollback in redb across app restarts, capped"
  - "Not covered: the sections, serials, subscriptions and history paging
    themselves (plan 20261023_03, which this builds on)"
sources:
  - "Owner (2026-10-23): \"should we use redb?\"; \"right, if in memory
    but we should still store sections by serial and have a full
    scrollback\"; \"whats stopping that plan, we should add a
    handlr-sessions crate and implement in-memory and redb stores, on
    daemon and app\"; \"it also means a -data crate (or that could be the
    -data crate) with types shared\"; \"yes, but new plan\""
  - "Plan 20261023_03 (session subscriptions), its amendments of 14:54 EDT
    (sections and serials, A1–A10) and of the evening (\"sections kept in
    memory for full scrollback\"), being built at the time of writing"
  - "handlr at `6bfbfc9`: `crates/libhandlr_proto/src/common.rs` (`Entry`,
    `Link`, `Turn`, `PendingQuestion`; the section types arrive with plan
    20261023_03), `src/sessions/` (the readers, `read_tail`, the line
    budget), `crates/handlr-companion/src/client.rs` (the agent view's
    entries, to become its section map)"
  - "redb (crates.io), an embedded key-value store in pure Rust: one file,
    ACID transactions, ordered keys, no C dependency, so it builds for
    Android as the rest of the app does"
related:
  - docs/design/20261023_03_SESSION_SUBSCRIPTIONS_PLAN.md (sections, serials, history; this moves its section map onto a store)
  - docs/design/20261023_06_SESSION_READERS_PLAN.md (the readers, which produce what is stored)
  - docs/design/20261023_07_SUBAGENTS_PLAN.md (child sessions, stored the same way)
---

# handlr-data, the shared session data model with memory and redb stores

## 1. Goal

Plan 20261023_03 gives sessions sections, each with a serial, and keeps
them in memory: the daemon rebuilds a session's line index from its file
when a tailer starts, and the phone holds every section it loads for the
life of an agent view. Two things are lost on a restart: the daemon
rescans a session file (35 MB for a long one) before it can answer, and
the phone forgets its scrollback and must page it all in again.

After this plan, the session data model lives in one crate that the
daemon, the protocol and the companion share, and the sections are kept
in a store: in memory as now, or on disk in a redb file, so both sides
resume where they were.

## 2. Decisions

1. **One crate, `crates/handlr-data`, for what a session is and where it
   is kept.** It holds the shared types and the stores. `libhandlr_proto`
   keeps the messages, methods, codes and the link; the daemon's readers
   and hub keep how a session file is read into these types.
2. **The shared types move into it:** `Serial`, `Section`, `SectionKind`,
   `CardBody`, `CardKind` (from plan 20261023_03), `Turn`, `Entry`,
   `EntryKind`, `Role`, `Link` and `PendingQuestion`. `libhandlr_proto`
   depends on `handlr-data` and re-exports them from `common` under the
   same names, so no user of the protocol changes. One definition serves
   the wire and the stores.
3. **The store:**

   ```rust
   pub trait SectionStore: Send + Sync {
       /// Puts `section` in `session`, replacing one with the same serial.
       fn put(&self, session: &SessionKey, section: &Section) -> Result<(), StoreError>;
       /// Up to `n` sections strictly before `before`, oldest first.
       fn before(&self, session: &SessionKey, before: Serial, n: usize) -> Result<Vec<Section>, StoreError>;
       /// Up to `n` sections strictly after `after`, oldest first.
       fn after(&self, session: &SessionKey, after: Serial, n: usize) -> Result<Vec<Section>, StoreError>;
       /// The newest serial held for `session`.
       fn newest(&self, session: &SessionKey) -> Result<Option<Serial>, StoreError>;
       /// Forgets everything about `session`.
       fn forget(&self, session: &SessionKey) -> Result<(), StoreError>;
   }
   pub struct SessionKey { pub session_id: String, pub file_hash: u32 }
   ```

   A batch of puts is one transaction (`put_many`), so a delivery is
   stored whole or not at all.
4. **Two stores.** `MemoryStore`: a `BTreeMap` per session on the packed
   serial, always built. `RedbStore`: one redb file, one table keyed by
   `(file_hash, packed serial)` holding the section serialised with
   `postcard` (compact, serde, no schema drift beyond the types), behind
   the crate's `redb` feature. `libhandlr_proto` never turns the feature
   on, so speaking the wire never pulls redb in; the daemon and the app
   do. redb and postcard are pinned to exact crates.io versions.
5. **The daemon caches the line index and sections** in
   `~/.local/state/handlr/sessions.redb` (directory 0700, file 0600). Each
   session's entry records the file's length and modification time when
   it was indexed; a tailer starting on that file resumes from the cached
   index when the file only grew, and rescans when it shrank, changed
   before the cached length, or was replaced. Sections are redacted
   before they are stored, as before they are sent, so the cache holds no
   more than a client sees.
6. **The phone keeps its scrollback** in `sessions.redb` in the app's data
   directory. Opening an agent view shows the stored sections at once,
   then subscribes with `after` set to the newest stored serial, so only
   what is newer comes over the link; paging back reads the store before
   asking `session.history`. The store is capped: the 20 most recently
   viewed sessions and 50 MB, the least recently viewed dropped first. A
   `session.delivery` with `reset` for a session forgets its stored
   sections first.
7. **Memory is the fallback.** If the redb file cannot be opened (a
   corrupt file, no space, a second daemon holding it), the store is a
   `MemoryStore` for that run, logged; a corrupt file is renamed aside
   (`sessions.redb.broken-<time>`) and a fresh one made on the next start.
   Tests use `MemoryStore`, and `RedbStore` over a temporary file.
8. **Versions.** `handlr-data` starts at 0.1.0. Moving the types behind a
   re-export changes no public API of `libhandlr_proto`, so its release
   is a patch (and, by the owner's rulings, appended fields and new
   items are additive too). handlr-companion and the app only change
   their manifests and lock files.

## 3. Steps

1. **`crates/handlr-data` with the types and `MemoryStore`.** The types
   move from `libhandlr_proto` (or, for the section types, from wherever
   plan 20261023_03 put them); the crate is added to the workspace in its
   first commit.
   **Done when:** the store's tests pass (puts replace by serial;
   `before`, `after` and `newest` across sessions; `forget`), and the
   types round-trip through serde as before.
2. **libhandlr_proto on handlr-data.** The protocol depends on it and
   re-exports the types from `common`; a patch release with its
   VERSIONS.md entry.
   **Done when:** every existing protocol, daemon and companion test
   passes unchanged.
3. **`RedbStore`** behind the `redb` feature, with `put_many` as one
   transaction, and the fallback and rename of decision 7.
   **Done when:** the same tests as step 1 pass against a temporary redb
   file, a reopened file returns what was put, and a corrupt file falls
   back to memory and is renamed.
4. **The daemon's cache** (decision 5), in the hub of plan 20261023_03.
   **Done when:** daemon tests show a restarted daemon resuming a grown
   session file without rescanning it (the reader is not called for the
   cached lines), and rescanning a file that shrank or was replaced.
5. **The phone's store** (decision 6), in the companion client, with the
   cap; the app opens it in its data directory.
   **Done when:** client tests show a view opening with stored sections
   before any delivery, subscribing with `after` set to the newest stored
   serial, a reset forgetting the session, and the cap dropping the
   least recently viewed; and on the S23, closing and reopening the app
   shows the scrollback at once, and paging back reads stored sections
   without asking the daemon.

Each step is one crate, one commit (Rule 3.1). 1 needs plan
20261023_03's step 2 (the section types); 2 needs 1; 3 needs 1; 4 needs
3 and plan 20261023_03's step 3; 5 needs 3 and plan 20261023_03's step 6.

## 4. Open questions for the owner

1. **The phone's cap:** 20 sessions and 50 MB, or different numbers?
2. **The daemon's cache location:** `~/.local/state/handlr/` as the plan
   says, or next to the config in `~/.config/handlr/`?
3. **Plan 20261023_03's latest amendment** says nothing persists across
   app restarts. Once this plan is approved, that line is replaced by
   decision 6 here; should that be recorded as an amendment there too, or
   is this plan's note enough?

## Amendment — 2026-10-23 23:26 EDT: approved; built once plan 20261023_03 lands

The owner approved the plan on 2026-10-23: "approve the existing and start
it". Its open questions take the plan's values until the owner says
otherwise: the phone's cap is 20 sessions and 50 MB; the daemon's cache is
in `~/.local/state/handlr/`; and plan 20261023_03's "nothing persists
across app restarts" is replaced by decision 6 here, noted in that plan by
its next amendment.

Building starts as soon as plan 20261023_03's work, being built in a
worktree at the time of writing, is on `main`: step 1 moves the section
types that work creates, and steps 4 and 5 build on its hub and client.
Starting before it lands would mean moving types out from under it. The
status becomes `In implementation` with step 1's first commit.
