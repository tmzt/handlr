---
title: Pairing the phone with several flankr daemons, listed under Remotes in Settings
date: 2026-10-28
status: Draft
covers:
  - "The phone can be paired with several flankr daemons (**remotes**), each
    identified by its link static public key, each with its own phone-side
    Noise static key, its own record in the key store's secure store, and
    its own state database, `<data dir>/remotes/<id>/state.redb`"
  - "A **Remotes** section in Settings: one row per paired remote (name,
    host, a short fingerprint, when it was paired, which one is in use),
    an **X** on each row that removes it after a confirming modal, and a
    **+** that starts today's pairing flow and adds the new remote to the
    list"
  - "Removing a remote: its credentials and its state directory are
    deleted on the phone, and the daemon is asked to forget the phone
    (`link.unpair`), over the open link for the remote in use or over one
    short connection for any other; when it cannot be reached, the phone
    says so"
  - "Migration of today's single pairing (the `server` blob, or
    `server.toml` on the desktop) into the list on first start, and of any
    single state store into that remote's directory"
  - "Which settings are app-wide (`settings.toml`) and which belong to a
    remote"
  - "Not covered: **switching the active remote** (a later plan; the data
    model here is shaped for it); the signed pairing tokens of plan
    20261023_08, which change how a pairing is made, not how many are
    kept; anything on the daemon, which already pairs many devices"
sources:
  - "Owner (2026-10-28): \"plan support for pair multiple remote flankr
    daemons, only add the list to settings with an X to remove each one,
    switching active backend will come later, use same pair method wuth a
    new + button in settings in the Remotes section\""
  - "Owner (2026-10-28): \"also, one redb per backend for state, rem9v9ng
    backend deletes relevant state\""
  - "Owner (2026-10-28): \"add confirm when deleting\""
  - "flankr at `4c8a192` (branch `main-terminal`): `crates/flankr-companion/
    src/{keys, net, scan, client, lib}.rs`; `apps/companion/src/{keys, lib,
    ui, settings}.rs`; `apps/companion/src/settings/{screen, confirm,
    danger_row}.rs`; `apps/companion/SETTINGS_COPY.md`;
    `src/server/link/{mod, auth}.rs`; `crates/libflankr_proto/src/common.rs`
    (`LINK_UNPAIR`)"
related:
  - "docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md: the link, `IK`
    reconnects, `auth.toml`, and its 23:07 EDT amendment (pairing by public
    keys and the device code), which is the pairing method in the code
    today and which **+** reuses unchanged"
  - "docs/design/20261023_08_SIGNED_PAIRING_PLAN.md (Draft): replaces the
    device code with signed tokens. If built, it changes how one pairing is
    made; this plan's list holds its records the same way
    (`signing_key` is reserved below)"
  - "docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (In implementation):
    decision 3's sections, the confirming modal (`settings/confirm.rs`),
    *Unpair this device*, *Delete data* and *Clear cache*; CHANGES.md
    2026-10-24 12:33 and 14:14 EDT (modals open and close at once). This
    plan adds a section and leaves that plan's rows as they are, with the
    meanings stated in decision 9"
  - "docs/design/20261023_09_HANDLR_DATA_PLAN.md (Draft, approved by its
    23:26 EDT amendment, not built): decision 6 puts the phone's store in
    `sessions.redb` in the app's data directory. This plan puts it per
    remote instead (decision 6), which needs an amendment to that plan
    agreed with the owner (open question 3)"
---

# Pairing the phone with several flankr daemons, listed under Remotes in Settings

## 1. Goal

Today the phone holds one pairing: one server record and one static key.
Pairing again replaces the record (`net.rs` `begin_pairing` calls
`set_server` before the daemon has even accepted), and *Unpair this
device* deletes both. A phone used with a work Mac and a home Mac must
unpair and re-pair to move between them.

After this plan the phone keeps a **list of remotes**. Settings has a
**Remotes** section: each paired daemon on a row with its name, host,
fingerprint and pairing date, an **X** that removes it (after a
confirmation), and a **+** that pairs another the way pairing works now.
Each remote has its own credentials and its own state database, and
removing one deletes both. The app still connects to **one** remote, the
one already in use; choosing another is a later plan, and nothing stored
here needs migrating when it comes.

## 2. Findings in the code

1. **One pairing, two stores.** `KeyStore`
   (`crates/flankr-companion/src/keys.rs:19-33`) has one `static_key`, one
   `server`/`set_server`, and `forget`, which deletes both
   (`keys.rs:191-200` for files). `ServerRecord` (`keys.rs:36-42`) is
   `{name, public_key, addresses, session_id}`; it has no pairing time.
2. **On the phone** (`apps/companion/src/keys.rs`): the static key is a
   `NoiseStatic` in libmorph's key store under `KEY_NAME = "noise.static"`
   (`:28`), the server record a TOML blob `SERVER_BLOB = "server"` in the
   secure store (`:30`, `encode_server`/`decode_server` `:184-211`).
   `forget` deletes both (`:163-170`). On Android these are libmobile's
   Keystore-sealed stores; on macOS the login Keychain; elsewhere the
   desktop falls back to `FileKeyStore` in `~/.config/flankr-companion/`
   (`key` at mode 0600 and `server.toml`; `apps/companion/src/lib.rs:442-475`).
   No secret values were read for this plan.
3. **Pairing today** is the 23:07 EDT amendment's public-key method, not
   plan 20261023_08's tokens (that plan is Draft). `scan::read`
   (`crates/flankr-companion/src/scan.rs:31-56`) takes the `flankr://`
   URL, the bare `flankr1…` string or a QR data URL, and returns
   `Pairing {payload: PairingPayload, server}` with `server` the host's
   first DNS label. `Net::begin_pairing` (`net.rs:438-481`) makes the
   phone's key if it has none, builds the `flankrkey1…` device code over
   the server's key, **stores the server record at once**, and opens a
   10-minute window of `IK` retries every 2 s (`PAIRING_RETRY`,
   `PAIRING_WINDOW`, `net.rs:73-75`) until the owner pastes the code into
   `flankr pair`; the `hello` answered `paired: true` gives
   `Event::Paired`. A `Pair` input while connected ends the session
   (`End::Pair`, `net.rs:228-235`, `:580-584`) and moves to the new server.
4. **The app's pairing input**: the pairing screen's field and Pair button
   (`apps/companion/src/ui.rs`, "paste a pairing URL", `:343`), `--pair`
   on the desktop, and on Android `pair.txt` in the data directory, read
   and deleted **only at start** (`lib.rs:2366-2375`). All go through
   `App::open_url` (`lib.rs:539-551`) as `Input::Pair`. There is no camera
   scan in the app; the `flankr://` intent is not delivered yet
   (`lib.rs:2376-2380`).
5. **Connecting** (`net.rs:344-400`): `keys.server()` gives the one
   record; with none, the local socket (`connect_local`). The record's
   addresses are tried in order with `link::reconnect` (`IK`) and a
   10-second handshake timeout (`HANDSHAKE_TIMEOUT`, `net.rs:69`).
6. **Unpairing** (`client.rs:2284-2302`): online, the state sends
   `link.unpair` and waits up to `UNPAIR_WAIT` (5 s, `client.rs:664-666`);
   offline, or on no answer, it forgets the pairing here only
   (`Output::Forget {daemon: false}`). It also clears the closed views
   ("another daemon's sessions are not to be shown from the cache"). The
   daemon's `Link::unpair` (`src/server/link/mod.rs:357-367`) removes the
   calling key's `auth.toml` entry; `link.unpair` is refused on the local
   socket (`src/server/conn.rs:1184`). `AuthEntry`
   (`src/server/link/auth.rs:42-57`) already records `paired` (RFC 3339)
   per device, and a daemon holds any number of devices.
7. **No redb on the phone yet.** The cache is the closed views in memory
   (`client.rs:1125`, `closed: VecDeque<AgentView>`), with two `TODO(plan
   20261023_09)` notes to page them out to redb (`client.rs:2052`,
   `:2094`). `redb` is not in `Cargo.lock`. Plan 20261023_09's phone store
   (decision 6 and its 12:15 EDT amendment) is approved but unbuilt.
8. **Settings** (`apps/companion/src/settings.rs:56-87`) are one
   `settings.toml` in the data directory: sound, ding, microphone, general
   agent, tab follows focus, scrollbar, background, alerts, voice,
   appearance, light and dark theme. `delete_data` (`settings.rs:496`)
   removes everything in the data directory but the voice models.
9. **The settings screen** (`apps/companion/src/settings/screen.rs`):
   `Section` (`:107-121`) is Sound, Voice, Display, Background,
   Notifications, Battery, Device; rows are `ROW_H = 52` pt (`:81`);
   destructive rows are `DangerRow` (`danger_row.rs`), red, behind
   `Confirm` (`confirm.rs`: Markdown question, *Cancel* and a red action);
   the modal stack is `ModalTransition::Instant` (`screen.rs:317-321`), so
   modals open and close in one frame with no fade. `Asking` (`:211-235`)
   names the open modal; `SettingsHit` (`:171-185`) is what the app is
   told. There is no "+" or "X" control in the app yet.
10. **Public enums are closed.** `Input`, `Event` and `Output`
    (`client.rs:32`, `:450`, `:624`) have no `#[non_exhaustive]`, so a new
    variant is a breaking change. `flankr-companion` is at 0.1.0 and has
    no `VERSIONS.md` entry yet; `libflankr_proto` is at 0.4.2.

## 3. Decisions

### 3.1 A remote and its identity

1. **A remote is one paired daemon**, identified by its link static public
   key (the 32 bytes the phone pins for `IK`). Its **id** is the first 8
   bytes of BLAKE2s-256 over that key, written as 16 lower-case hex digits
   (`3f9a12c4d0e1b2a3`). The id names its directory and its key-store
   entry; it is never shown whole.
2. **Its fingerprint**, shown in the row, is the id's first 8 hex digits in
   two groups, `3f9a 12c4`: the same length and hash as the phone
   fingerprint the daemon logs (plan 20261023_08, 2.4 step 3). It is
   computed by one function, `libflankr_proto::link::fingerprint(key:
   &[u8; 32]) -> [u8; 32]` (BLAKE2s-256, the crate already has `blake2`
   behind `link`), so the daemon can print the same digits later.
3. **Its name** is what `scan::read` gives today (the host's first DNS
   label, `m4`) and is not unique: two remotes may both be `m4`. The row
   tells them apart by host and fingerprint.
4. **An id collision** (two keys with the same 8-byte prefix, about 1 in
   2^64 per pair) is refused at pairing time, with the error
   "a remote with the same id is already paired".

### 3.2 Credentials: one phone key per remote

5. **The phone has its own Noise static key for each remote.** A new
   remote's key is `noise.static.<id>` in the key store (`key.<id>` in
   `FileKeyStore`'s directory, mode 0600). Reasons: removing one remote
   deletes exactly its key and cannot break another pairing; and two
   daemons that compare their `auth.toml` files cannot tell they hold the
   same phone. The device code is already built per server
   (`DeviceCode::encode(&server_key)`), so pairing needs no other change.
   Each record names its key (`key_name`), so today's key keeps its name
   (`noise.static`, or the file `key`) and nothing in the key store is
   renamed: the key store lends keys and never exports them, so a rename
   would mean a new key and re-pairing.

### 3.3 The stored list

6. **One list, in one blob.** The secure store's `server` blob is replaced
   by a `remotes` blob (`remotes.toml` beside the keys in `FileKeyStore`),
   written whole, in this TOML layout:

   ```toml
   version = 1
   # The remote connected to. Absent: none (the local socket).
   active = "3f9a12c4d0e1b2a3"

   [[remote]]                       # in pairing order, oldest first
   id = "3f9a12c4d0e1b2a3"
   name = "m4"
   public_key = "<base64, 32 bytes>"
   addresses = ["m4.tail1a2b3c.ts.net:7347"]
   session_id = "companion:s23"     # optional, as today
   paired_at = 1793212345           # Unix seconds; absent when not known
   key_name = "noise.static"        # the key store entry of this remote's phone key
   # signing_key = "<base64>"       # reserved for plan 20261023_08, 2.5 step 3
   ```

   `ServerRecord` gains `id: RemoteId`, `paired_at: Option<u64>` and
   `key_name: String`, appended. A new type holds the list:

   ```rust
   #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
   pub struct RemoteId(pub [u8; 8]);        // Display: 16 hex digits
   impl RemoteId { pub fn of(public_key: &[u8; 32]) -> Self;
                   pub fn fingerprint(&self) -> String; }   // "3f9a 12c4"

   #[derive(Clone, Debug, Default, PartialEq, Eq)]
   pub struct Remotes { pub active: Option<RemoteId>, pub list: Vec<ServerRecord> }
   impl Remotes { pub fn get(&self, id: RemoteId) -> Option<&ServerRecord>;
                  pub fn active_record(&self) -> Option<&ServerRecord>; }
   ```

   `KeyStore` becomes:

   ```rust
   pub trait KeyStore: Send {
       fn remotes(&mut self) -> Result<Remotes, String>;
       fn set_remotes(&mut self, remotes: &Remotes) -> Result<(), String>;
       fn static_key(&mut self, key_name: &str) -> Result<Option<Arc<dyn StaticKey>>, String>;
       fn generate_static_key(&mut self, key_name: &str) -> Result<Arc<dyn StaticKey>, String>;
       /// Nothing stored is not an error.
       fn delete_static_key(&mut self, key_name: &str) -> Result<(), String>;
   }
   ```

   `server`, `set_server` and `forget` go: their callers in `net.rs` move
   to the list.
7. **Shaped for switching later.** The list already holds every remote
   with everything a connection needs, and `active` is a stored field, not
   implied by the list. A later plan that switches remotes only writes
   `active` and reconnects; no field, file or key changes.

### 3.4 Which remote the app connects to

8. **The active remote is the one in use today, and stays so.**
   - On migration (3.6), the existing pairing becomes the only remote and
     the active one.
   - With no remote (a fresh install, or every remote removed), the first
     pairing becomes active, as pairing does now: the pairing screen shows,
     the net thread pairs on its main connection, and the new remote is
     `active`.
   - With an active remote, **+** pairs **beside** it (3.5): the new remote
     is added to the list and does **not** become active. The app stays
     connected to the remote it was on.
   - Removing the active remote makes the **most recently paired** of the
     rest active (by `paired_at`, then list order) and connects to it; with
     none left, `active` is cleared and the app shows the pairing screen,
     as *Unpair this device* does now. This is the one automatic change of
     the active remote in this plan (open question 1).

### 3.5 The + button: the same pairing, beside the open connection

9. **+** opens the pairing screen (`ui.rs`) in an *add* mode: the title
   reads *Pair another remote*, the field and Pair button are as today,
   the device code and its notes show above them, and the header's "‹" or
   the system Back return to Settings. On Android the app also reads
   `pair.txt` when the add screen opens and on each resume while it is
   open (today it is read only at start), so `scripts/push-pair.sh` works
   without a restart.
10. **Pairing beside the active connection.** `Input::Pair(text)` while a
    remote is active and the pairing names a different key starts a
    **pairing probe** in the net thread, a separate Tokio task:
    - it makes the new remote's phone key (`noise.static.<id>`) and the
      device code, and gives `Event::PairingCode {code, server}` as now;
    - it tries `IK` to the new remote's addresses with a `hello` (the same
      `HelloParams` the state builds, from a new non-mutating
      `State::hello_request()`), every `PAIRING_RETRY` for
      `PAIRING_WINDOW`, as today's pairing does;
    - on a `hello` answered `paired: true`, it closes that connection,
      appends the remote to the list with `paired_at` now (not active),
      writes the list, and gives `Event::Paired {name}` and
      `Event::Remotes`;
    - the main connection and its state are untouched throughout;
    - nothing is written to the list before `paired: true`, so a pairing
      that is never accepted leaves no remote behind, only its key, which
      the probe deletes when it gives up or is cancelled;
    - one probe at a time: a second `Pair` replaces the first (its key
      deleted). Leaving the add screen sends `Input::CancelPairing`, which
      ends the probe.
    With no active remote, `Pair` works as today on the main connection,
    except that the record is written to the list only on `paired: true`
    (the code today writes it before; that bug-shaped order goes).
11. **Pairing a remote already in the list** (same public key) adds no row.
    The record's addresses and name are refreshed from the new pairing
    string and written; its key is reused, so the daemon, which already
    lists the phone, answers `paired: true` at once. The note says
    *Already paired with m4: its addresses are updated*. If that remote's
    key is missing from the key store, a new one is made under the same
    `key_name` and the owner pastes the new code as for a first pairing.
12. **At most 8 remotes.** + is drawn dim and takes no taps at 8, and the
    section's note says why. Each remote costs a key-store entry, a share
    of one blob and a directory; 8 is far past the use asked for and keeps
    the section short (open question 4).

### 3.6 Migration of today's pairing

13. **Once, at the first `remotes()` call** that finds no `remotes` blob:
    - if a `server` blob (or `server.toml`) exists, it becomes the one
      remote: `id` from its key, `paired_at` absent, `key_name =
      "noise.static"` (`"key"` maps to the same in `FileKeyStore`), and
      `active` = that id. The `remotes` blob is written first, and the
      `server` blob deleted only after the write succeeds; a start that
      finds both uses `remotes` and deletes `server`;
    - with no `server` blob, an empty list is written;
    - a `noise.static` key with no `server` record (a pairing never
      stored) is deleted, as unpairing would have.
    The migration is logged without key material: "keys: moved the paired
    server m4 (3f9a 12c4) into the remotes list".

### 3.7 One state database per remote

14. **Per-remote state lives in `<data dir>/remotes/<id>/state.redb`**,
    one redb file per remote (directory 0700, file 0600 where the platform
    has modes). `<data dir>` is libmorph's `ctx.data_dir()`, the same
    directory as `settings.toml`; on the desktop with `FileKeyStore` the
    keys stay in `~/.config/flankr-companion/` and only the state moves.
15. **This plan creates the file and owns its lifecycle**: a
    `RemoteState` in `flankr-companion` (`src/remote_state.rs`) that opens
    or creates the file with one table, `meta` (`remote` = the
    remote's public key in base64, `schema` = 1, `created` = Unix
    seconds), checks that `meta.remote` matches the remote it is opened
    for (a mismatch renames the file aside, `state.redb.mismatch-<time>`,
    and starts a fresh one), and closes it on drop. redb is pinned to one
    exact crates.io version.
16. **What it holds.** In this plan, only `meta`. The remote's session
    store of plan 20261023_09 (sections, outlines, places, paged-out
    closed views) is opened **in this file** when that plan is built, as
    more tables in the same database, instead of the app-wide
    `sessions.redb` its decision 6 names. That is a change to plan 09 to
    be agreed with the owner and written there as one amendment (open
    question 3); until then this plan writes nothing else to the file.
17. **Only the active remote's file is open**, held by the net thread for
    the life of its connection settings (not reopened per reconnect).
    Switching later closes one and opens another.
18. **If plan 09's app-wide store already exists** when this lands
    (`<data dir>/sessions.redb`), the migration of 3.6 renames it to the
    active remote's `state.redb` after writing `meta` into it; with no
    active remote it is deleted, since its sessions belong to no one.

### 3.8 App-wide and per-remote values

19. **App-wide** (`settings.toml`, unchanged): *All sound*, *Ding*,
    *Voice*, *Microphone*, *General agent (Gemini)*, *Tab follows focus*,
    *Scrollbar*, *Appearance*, *Light theme*, *Dark theme*, *Stay
    connected in the background*, *Alert when an agent needs you*; and the
    other data-directory files: `themes.toml`, `dictation_silence.txt`,
    `vocab.txt`, the voice models. *General agent* and *Tab follows focus*
    are sent to the daemon in `client.options` on every connection; they
    stay app-wide here (open question 2).
20. **Per remote**: its record in the `remotes` blob (name, public key,
    addresses, `session_id`, `paired_at`, `key_name`), its phone key, and
    its `state.redb` (today `meta` only; the session cache once plan 09 is
    built). The in-memory closed views are the active remote's and are
    cleared whenever the active remote changes, as unpairing does now
    (`client.rs:2290`).

### 3.9 Removing a remote (the X)

21. **X asks first.** A tap on a row's X opens the settings screen's
    confirming modal (`Confirm`, opening and closing at once, no fade, per
    CHANGES.md 2026-10-24 12:33 and 14:14 EDT), naming the remote by name,
    host and fingerprint and saying what goes: this phone's key for that
    pairing, its record, and its stored state and cache on this phone;
    and whether the daemon will be told (copy in 3.11). Buttons *Cancel*
    and *Remove* (red). A tap outside or Back is *Cancel*.
22. **Remove** sends `Input::RemoveRemote(id)`, and then:
    - **the daemon is asked to forget the phone**:
      - the active remote, online: `link.unpair` over the open link, as
        *Unpair this device* does (`UNPAIR_WAIT`, 5 s);
      - the active remote, offline: not asked; the modal said so first;
      - any other remote: a **revoke probe**, one `IK` connection to its
        addresses with its key (each address bounded by
        `HANDSHAKE_TIMEOUT`, the whole by 15 s), `hello`, `link.unpair`,
        `UNPAIR_WAIT` for the answer, then closed. The main connection is
        untouched;
    - **then, whatever the answer**, on the phone: its `state.redb` is
      closed if open (the active remote's: the connection is ended first),
      its directory `remotes/<id>/` is deleted, its key is deleted from
      the key store, and it is removed from the list, the list written
      last. A failure to delete the directory is logged and retried at the
      next start (a directory under `remotes/` with no list entry is
      deleted at start).
    - **then** `Event::RemoteRemoved {id, name, daemon}` (`daemon`: the
      daemon answered that it forgot the phone) and `Event::Remotes`; the
      app's note says which (copy in 3.11). For the active remote, 3.4's
      rule picks the next one.
23. **Order and safety.** The daemon is asked before the key is deleted,
    because the key is what lets the phone ask. Credentials are deleted
    before the list is written, so a crash between leaves a list entry
    whose key is missing; the next connection to it says "pair again" (as
    `net.rs:358-366` does now) and its X still works.

### 3.10 The Remotes section on the settings screen

24. **Place**: a new `Section::Remotes`, after *Battery* and before
    *Device*, which stays last (settings plan decision 3).
25. **The title line**: *REMOTES* in the section-title style on the left,
    and at its right a **+**, 44 pt touch target, glyph at the row label's
    size in the accent colour; dim and inactive at 8 remotes. An ⓘ beside
    the title opens `remotes` (3.11) in an `InfoModal`.
26. **A row per remote**, in pairing order, 52 pt (`ROW_H`):
    - line 1, the label size: the name, and after it, for the active one,
      a dim *in use*, or *in use · offline* while disconnected;
    - line 2, the subtitle size and dim: `<host:port of its first address>
      · <fingerprint> · paired <date>`, the date as `28 Oct 2026` in local
      time, *paired before this list* when `paired_at` is absent; cut with
      an ellipsis in the host when the line is too long;
    - at the right, an **X**, 44 pt touch target, glyph in the dim text
      colour (it is not red: the destructive word is in the modal's red
      button). A tap anywhere else on the row does nothing in this plan
      (the row is where switching will go).
27. **No remotes**: one dim note row, `remotes_empty`.
28. **Controls**, one per file as the settings plan does:
    `settings/remote_row.rs` (`RemoteRow`: two lines and an X, telling a
    tap on the X from one on the row) and the + on the section title in
    `screen.rs`. `Asking::RemoveRemote(RemoteId)` and
    `SettingsHit::{AddRemote, RemoveRemote(RemoteId)}` are added. The
    screen takes the list with `SettingsScreen::set_remotes(Vec<RemoteInfo>)`.

### 3.11 Copy, for `SETTINGS_COPY.md`

Added in the file's own style:

```markdown
# [Copy] section_remotes

- text: REMOTES

# [Copy] remotes

- title: Remotes
- text: |
    Each remote is a flankr daemon this phone is paired with. The phone
    pins the daemon's X25519 static key and reconnects with `Noise_IK`;
    for each remote it holds a separate static key of its own in
    libmorph's key store (Keystore-sealed on Android, the login Keychain
    on macOS), so removing one never touches another, and two daemons
    can't tell from their `auth.toml` that they list the same phone.
    The fingerprint is the first 8 hex digits of BLAKE2s-256 over the
    daemon's static key. flankr connects to one remote at a time, the one
    marked *in use*; choosing another comes later. Each remote's
    sessions are kept in its own `remotes/<id>/state.redb` in flankr's
    data directory, and go with it.

# [Copy] remote_in_use

- text: in use

# [Copy] remote_in_use_offline

- text: in use · offline

# [Copy] remote_detail

- [abc] host: the remote's first address, host:port
- [abc] fingerprint: the remote's fingerprint, as 3f9a 12c4
- [abc] date: when it was paired, as 28 Oct 2026
- text: |
    {{ [abc] host }} · {{ [abc] fingerprint }} · paired {{ [abc] date }}

# [Copy] remote_detail_undated

- [abc] host: the remote's first address, host:port
- [abc] fingerprint: the remote's fingerprint, as 3f9a 12c4
- text: |
    {{ [abc] host }} · {{ [abc] fingerprint }} · paired before this list

# [Copy] remotes_empty

- text: |
    No remote paired. Tap **+** to pair with a flankr daemon:
    `flankr pair` on the Mac prints the URL to paste.

# [Copy] remotes_full

- text: |
    8 remotes is the most this phone keeps: remove one to pair another.

# [Copy] remote_add

- text: Pair another remote

# [Copy] remote_already

- [abc] name: the remote's name
- text: |
    Already paired with {{ [abc] name }}: its addresses are updated.

# [Copy] remove_remote

- title: Remove remote
- [abc] name: the remote's name
- [abc] host: the remote's first address, host:port
- [abc] fingerprint: the remote's fingerprint
- text: |
    Remove **{{ [abc] name }}** (`{{ [abc] host }}`, {{ [abc] fingerprint }})
    from this phone? This phone's key for that pairing, its record and
    its stored sessions and cache on this phone are deleted. To use it
    again, pair it again with `flankr pair`.

# [Copy] remove_remote_tell

- [abc] name: the remote's name
- text: |
    flankr asks {{ [abc] name }} to forget this phone (`link.unpair`), so
    its `auth.toml` no longer lists it.

# [Copy] remove_remote_try

- [abc] name: the remote's name
- text: |
    flankr connects to {{ [abc] name }} once, for up to 15 seconds, to ask
    it to forget this phone (`link.unpair`). If it can't be reached, it
    still lists this phone until the entry is removed from its
    `auth.toml`.

# [Copy] remove_remote_offline

- [abc] name: the remote's name
- text: |
    {{ [abc] name }} is offline: it will still list this phone until the
    entry is removed from its `auth.toml`.

# [Copy] remove_remote_next

- [abc] next: the remote that will be in use after
- text: |
    It's the remote in use: flankr connects to {{ [abc] next }} next.

# [Copy] remove_remote_last

- text: |
    It's the last remote: flankr returns to the pairing screen.

# [Copy] remove_remote_action

- text: Remove

# [Copy] remote_removed

- [abc] name: the remote's name
- text: Removed {{ [abc] name }}; it no longer lists this phone.

# [Copy] remote_removed_here

- [abc] name: the remote's name
- text: |
    Removed {{ [abc] name }} on this phone; it lists this phone until the
    entry is removed from its `auth.toml`.
```

The modal's text is `remove_remote`, then one of `remove_remote_tell`
(the active remote, online), `remove_remote_offline` (the active remote,
offline) or `remove_remote_try` (any other), then, for the active remote,
`remove_remote_next` or `remove_remote_last`, each as its own paragraph.
The pairing screen's add-mode title is `remote_add`.

### 3.12 The settings plan's own rows, with several remotes

29. **Unpair this device** keeps its row and copy, and acts on the active
    remote: it is the same as that remote's X (the same `RemoveRemote`
    path), with its own modal. With no remote it is dim and inactive.
30. **Delete data** asks every remote to forget the phone (the active one
    over its link, the others by revoke probes, all at once, each bounded
    as in 3.9), then deletes every remote's key, the `remotes` blob and
    everything in the data directory but the voice models, `remotes/`
    included, as now.
31. **Clear cache (n MB)** clears the **active remote's** cache only: the
    closed views in memory (all of which are the active remote's), and,
    once plan 09's tables are in `state.redb`, that file's session tables,
    compacted. *n* is the active remote's cache. Other remotes' files are
    left; they go with their X or *Delete data*.

## 4. Versions (Rule 5)

- **`libflankr_proto` 0.4.2 → 0.4.3**: `link::fingerprint` added.
  Strictly additive: a patch.
- **`flankr-companion` 0.1.0 → 0.2.0**, **Breaking**:
  - `KeyStore`'s methods change (`server`, `set_server`, `forget` removed;
    `remotes`, `set_remotes`, `delete_static_key` added; `static_key` and
    `generate_static_key` take a key name);
  - new variants in the closed enums: `Input::RemoveRemote(RemoteId)`,
    `Input::CancelPairing`; `Event::Remotes(Vec<RemoteInfo>)`,
    `Event::RemoteRemoved {id, name, daemon}`; `Output::Forget` becomes
    `Output::Remove {id, daemon}` (renamed, a field changed);
  - added: `RemoteId`, `Remotes`, `RemoteInfo {id, name, host,
    fingerprint, paired_at, active, online}`, `RemoteState`;
  - `ServerRecord` gains `id`, `paired_at`, `key_name` (appended fields,
    additive by the owner's ruling, but this release is breaking anyway);
  - behaviour: a second pairing no longer replaces the first.
  If plan 20261023_08 is built first and takes 0.2.0, this is 0.3.0.
- **`companion`** and **`flankr`** are applications: manifests and lock
  file only. The daemon does not change.

## 5. Steps

Each step is one crate or subproject and one commit (Rule 3.1), and passes
`cargo fmt -p <crate> -- --check`, `cargo clippy -p <crate> --all-targets`
and `cargo test -p <crate>` (Rule 3.4).

1. **`libflankr_proto`: `link::fingerprint`** (decision 2), 0.4.3 with its
   `VERSIONS.md` entry and tag.
   **Done when:** the function's output for a fixed key equals BLAKE2s-256
   computed independently in the test, and the crate's tests pass.
2. **`flankr-companion`: the list in `FileKeyStore`** (3.1–3.3, 3.6):
   `RemoteId`, `Remotes`, `ServerRecord`'s new fields, the new `KeyStore`
   trait, `remotes.toml` and `key.<id>`, and the migration from
   `server.toml` and `key`.
   **Done when:** tests show an empty store reads as an empty list; a list
   of three round-trips with `active`; a directory holding today's
   `server.toml` and `key` reads as one active remote with `key_name`
   naming the old key and the same public key, and `server.toml` is gone
   afterwards; both files present reads the list and deletes
   `server.toml`; `delete_static_key` of a missing key is not an error;
   a key at mode 0644 is still refused.
3. **`flankr-companion`: `RemoteState`** (3.7): redb pinned, the `meta`
   table, the mismatch rename, the directory layout and the start-time
   sweep of directories with no list entry.
   **Done when:** a temporary data directory gets
   `remotes/<id>/state.redb` with `meta` written once and read back; a
   file whose `meta.remote` names another key is renamed aside and a fresh
   one made; dropping the handle lets the directory be deleted and
   reopened; an orphan directory is deleted by the sweep; an existing
   `sessions.redb` is moved into the active remote's directory.
4. **`flankr-companion`: the net thread and the state** (3.4, 3.5, 3.9,
   3.12): connecting to `active`; pairing with no active remote writing
   the list only on `paired: true`; the pairing probe beside an active
   connection; `CancelPairing`; re-pairing a listed remote; the limit of
   8; `RemoveRemote` for the active remote (online and offline) and for
   another (the revoke probe); the next active remote; `Event::Remotes`
   on every change and at start; *Clear cache* on the active remote;
   *Delete data*'s revoke-all; 0.2.0 with its `VERSIONS.md` entry and tag.
   **Done when:** against an in-process link server with a temporary
   `auth.toml` (as `tests/link.rs` does for the daemon), tests show: a
   second pairing while connected to the first adds a second remote and
   the first connection never drops; an unaccepted probe adds nothing and
   deletes its key when the window ends or on `CancelPairing`; pairing the
   same server again adds no row and updates the addresses; a ninth
   pairing is refused; removing a non-active reachable remote calls
   `link.unpair` with its key (the server's `auth.toml` no longer lists
   it), deletes its key and directory, and leaves the active connection
   up; removing an unreachable one gives `daemon: false` within 15 s and
   still deletes everything; removing the active remote connects to the
   most recently paired remaining one, or to none with an empty list.
5. **`apps/companion`: the key store and the screens** (3.2, 3.6,
   3.10–3.12): `VaultKeyStore` on the `remotes` blob and `noise.static.<id>`
   keys, with the migration from the `server` blob; the Remotes section,
   `RemoteRow`, the +, the ⓘ, `Asking::RemoveRemote` and its modal; the
   pairing screen's add mode and `pair.txt` read on opening and resume;
   the copy in `SETTINGS_COPY.md`.
   **Done when:** app tests round-trip the `remotes` blob and migrate a
   `server` blob (memory vault); place the Remotes section between Battery
   and Device; draw a row's two lines and its X, and tell an X tap from a
   row tap; open the modal with the right paragraphs for an online active,
   offline active and other remote, cancel it, and confirm it to
   `SettingsHit::RemoveRemote`; dim the + at 8; show `remotes_empty`; and
   on the S23 (with the owner's go-ahead): the existing pairing appears as
   one remote after the update with no re-pairing, + pairs a second Mac
   (or a second daemon on another port) while the first stays connected,
   and X on it, confirmed, removes it from the list and from that
   daemon's `auth.toml`.
6. **Documents**: this plan's row in the README table (in this plan's
   first commit); the `CHANGES.md` addendum naming this plan when
   implementation starts; plan 20261023_09's amendment for decision 16 if
   the owner agrees (open question 3).
   **Done when:** the README lists this plan with its status, and
   `CHANGES.md` has the addendum.

Order: 1 needs nothing; 2 needs 1; 3 needs nothing; 4 needs 2 and 3; 5
needs 4.

## 6. Risks

1. **Migration on the phone is one-way and touches the only pairing.** A
   bug in step 5's migration could lose the `server` record. Guarded by
   writing `remotes` before deleting `server`, by never renaming or
   deleting `noise.static` during migration, and by testing against a
   memory vault holding today's exact blob layout.
2. **A probe and the main connection at once** double the link traffic
   during pairing and add a second task touching the key store. The key
   store calls are short and serialised through the net thread; the probe
   holds only its own key name.
3. **Deleting an open redb file.** On Unix the delete succeeds while a
   handle is open and the file lives on until it closes, which would leak
   space and could be reopened by a racing write. The active remote's
   connection and `RemoteState` are dropped before the directory is
   deleted, and only the active remote's file is ever open.
4. **Revoking is best effort.** An unreachable daemon keeps the phone's
   key in `auth.toml`; that key is deleted on the phone, so nothing can
   use the entry, but the owner sees a stale device in `flankr` until it
   is removed there. The copy says this before and after.
5. **Per-remote keys change today's assumption of one phone key.** Any
   code that reads `noise.static` directly (none found outside `keys.rs`)
   would miss the new keys. Step 5 greps for `KEY_NAME`.
6. **Plan 09 not yet amended.** If plan 09 is built from its current text
   first, it creates `sessions.redb` app-wide; decision 18's move covers
   that, at the cost of one more migration path to test.
7. **Two remotes named alike** (`m4` twice) could be confused in the
   modal; the host and fingerprint are in both the row and the modal.

## 7. Open questions for the owner

1. **Removing the active remote**: switch to the most recently paired
   remaining one (as written), or go to "no remote in use" and leave the
   choice to the switching plan?
2. **General agent and Tab follows focus**: app-wide (as written), or per
   remote, since each daemon has its own `[llm]` and herdr?
3. **Plan 20261023_09's phone store** in each remote's `state.redb`
   (decision 16) instead of its app-wide `sessions.redb`: agree, so that
   plan gets one amendment saying so? Its cap of 20 sessions and 50 MB
   would then be per remote, or shared across them?
4. **The limit of 8 remotes**: enough, or another number?
5. **Pairing beside the open connection** (decision 10) keeps the app on
   its remote while + pairs another. The simpler alternative is today's
   behaviour, where pairing moves the connection to the new remote and
   makes it active. Keep the probe?
6. **Unpair this device** with several remotes: keep it as the active
   remote's X (as written), or drop the row now that each remote has an X?
7. **Showing the daemon's own fingerprint**: should `flankr pair` (or a
   status command) print the daemon's fingerprint, so the row's digits
   can be checked against the Mac? That is a daemon change, outside this
   plan.
8. **The settings plan** is frozen; this plan adds a section to its
   decision 3. Should that plan get a short amendment pointing here, as it
   did for *Tab follows focus*?
