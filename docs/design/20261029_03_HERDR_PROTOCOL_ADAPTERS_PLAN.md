---
title: herdr protocol adapters, one per herdr protocol version, from herdr's types generated once and committed
date: 2026-10-29
status: Draft
covers:
  - "herdr's protocol types generated once per supported herdr protocol
    version by a script (a new tool crate, `crates/herdr-wire-gen`, run
    with `cargo run -p herdr-wire-gen`) from herdr's release tags, and
    committed into libherdr_term as `src/herdr_v19.rs`, `src/herdr_v20.rs`
    and `src/herdr_v22.rs`, each with a header naming the tag, its commit
    and herdr's Apache-2.0 notice; libherdr_term loses its build script,
    `syn` and its need for the `vendor/herdr` submodule"
  - "A `ProtocolAdapter` trait in libherdr_term and one adapter per protocol
    version (19, 20, 22), each mapping its version's messages to flankr's
    own model (a pane's `libflankr_term::Grid` with its cursor, input,
    resize), written by hand and kept honest by the generated types"
  - "The daemon choosing the adapter before it connects, from the
    `protocol` number in herdr's JSON API `ping` answer; an unknown
    protocol uses today's `pane.read` poll, with no trial connections"
  - "A fixture test per version (handshake, a frame, input, resize) from
    recordings of that herdr version, made against headless test servers
    of herdr's release binaries under `~/.cache/flankr-herdr-test`, on
    scratch panes only, never the owner's live herdr"
  - "The name: protocol adapters (one per herdr protocol version), distinct
    from the screen drivers (one per agent) of plan 20261029_02, which read
    whichever grid the adapter gives, so herdr 0.8.0 panes get the grid
    route too"
  - "Not covered: herdr protocols before 19 or after 22; herdr over SSH
    (still local only, plan 20261027_07's 20:44 EDT amendment); herdr's
    optional surface encodings; changes to libflankr_term or
    libflankr_proto"
changes:
  - "docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md (In
    implementation, frozen; not edited). This plan replaces its decision 2
    (build-time extraction), the build side of its decision 14 (the
    submodule as the build's source) and the mechanism of its 23:42 EDT
    amendment, item 3 (build.rs reading `PROTOCOL_VERSION`); section 9
    lists what its frozen text keeps"
sources:
  - "The owner (2026-10-29), verbatim: \"since we used syn to collect
    structs should we build drivers per herdr version (including previous
    versions) instead of using it as a dep?\""
  - "The owner (2026-10-29), verbatim, to the lead's proposal (generate
    once and commit; one adapter per protocol version; choose the version
    from herdr's JSON API before connecting, unknown to the poll; a fixture
    test per version from recordings of test herdr servers; protocol
    adapters distinct from screen drivers): \"yes, write it as a draft\""
  - "herdr's tags in the `vendor/herdr` submodule (shallow, tags fetched),
    read only: `v0.8.0` (`346411fa21afd297f5ed3b3fa56f9e3fbf7654b7`,
    2026-08-03, `PROTOCOL_VERSION = 19`), `v0.8.2` (`9eb52145…`,
    2026-08-19, 20), `v0.9.0` (`b99002ac…`, 2026-09-07) to `v0.9.5`
    (`2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`, 2026-10-14), all 22;
    `v0.7.5` is 17. Files: `src/protocol/wire.rs` at each tag,
    `src/protocol/endpoint.rs` (from `v0.9.0` only),
    `src/server/client_transport.rs`, `src/server/headless.rs`,
    `src/server/socket_paths.rs`, `src/client/mod.rs`,
    `src/api/server.rs`, `src/api/schema/response.rs`, `LICENSE`
    (Apache-2.0; no `NOTICE` file at `v0.8.0` or at the pin `2713feb`)"
  - "`git diff --stat v0.8.2 v0.9.0 -- src/protocol/wire.rs`: 1470
    insertions, 241 deletions; `v0.8.0..v0.8.2`: 95 insertions, 3
    deletions; `v0.9.0..v0.9.5`: 264 lines changed in `wire.rs`, 101 in
    `endpoint.rs`"
  - "flankr at `c1c7543` (branch `main-terminal`), read 2026-10-29:
    `crates/libherdr_term/{Cargo.toml,build.rs,build/extract.rs,README.md}`,
    `crates/libherdr_term/src/{lib,attach,surface,convert,fake}.rs`,
    `crates/libherdr_term/tests/{extract,wire,attach,convert}.rs`,
    `src/server/herdr_attach.rs`, `src/server/terminal.rs`,
    `src/server/conn.rs:1759-1781`, `src/server/mod.rs:280-292`,
    `src/main.rs:20`, `:162-164` (`HERDR_SOCKET_PATH`), `VERSIONS.md`
    (libherdr_term 0.2.0, libherdr 0.3.1), `docs/HERDR_WIRE.md` (the
    protocol 22 audit, `cc88b3b8..2713feb`), `CHANGES.md` (the 2026-10-28
    00:39 EDT addendum: the owner's herdr 0.8.0 refused with `no_welcome`;
    the 0.9.5 test server under `~/.cache/flankr-herdr-test`)"
  - "The test herdr setup, read only: `~/.cache/flankr-herdr-test/bin/`
    (`herdr-macos-aarch64`, herdr 0.9.5; `herdr-test`, a wrapper that
    unsets `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH`,
    `HERDR_TAB_ID` and `HERDR_WORKSPACE_ID` and sets `XDG_CONFIG_HOME` and
    `XDG_STATE_HOME` under that folder); the owner's `/opt/homebrew/bin/herdr`
    reports `herdr 0.8.0`"
related:
  - "docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md (the crate this
    changes; In implementation)"
  - "docs/design/20261027_07_TERMINAL_MODE_PLAN.md (the terminal mode and
    the `pane.read` poll that stays the fallback; In implementation)"
  - "docs/design/20261029_02_SCREEN_DRIVERS_PLAN.md (Draft; its drivers read
    the grid this plan's adapters give, for every supported herdr)"
  - "docs/HERDR_WIRE.md (append-only findings; this plan adds sections for
    protocols 19 and 20)"
---

# herdr protocol adapters, one per herdr protocol version, from herdr's types generated once and committed

## 1. Goal

flankr's terminal view attaches to herdr as one of herdr's own clients
(plan 20261027_08), but only to a herdr that speaks protocol 22 (herdr
0.9.0 to 0.9.5 and HEAD). The owner's own herdr is 0.8.0, protocol 19,
and is served by the slower `pane.read` poll. This plan makes the attached
terminal work for each herdr release flankr chooses to support, by turning
the build-time extraction of herdr's types into a script that is run once
per supported herdr tag, committing its output, and writing one small
adapter per protocol version that maps that version's messages to flankr's
own model. The daemon learns herdr's protocol from herdr's JSON API before
it connects and picks the adapter; a protocol it does not know is served by
the poll, as today.

## 2. Findings

### 2.1 Why herdr 0.8.0 fails today

flankr's first message is `ClientMessage::EndpointControl { kind:
"endpoint.hello.v1", .. }` (`crates/libherdr_term/src/attach.rs`, the
hello built at `:340-358`). At `v0.8.0`, `ClientMessage`
(`src/protocol/wire.rs:341`) has 10 variants (`Hello`, `Input`,
`ClipboardImage`, `Resize`, `Detach`, `AttachTerminal`, `AttachScroll`,
`InputEvents`, `ObserveTerminal`, `ControlTerminal`), so the bincode
variant index of `EndpointControl` is unknown to it. herdr 0.8.0 logs
"failed to read client hello" and closes without a `Welcome`
(`src/server/client_transport.rs:505`), and flankr reports
`AttachError::Incompatible { code: "no_welcome", message: "unexpected end
of stream" }` (`attach.rs:369`). Plan 20261027_08's finding 2.6 ("a refused
hello is always readable") holds only for servers that have the endpoint
contract, that is protocol 22.

### 2.2 The three protocol versions in herdr's releases

| Protocol | Tags | `wire.rs` lines | Serde types in `wire.rs` | `endpoint.rs` |
|---|---|---|---|---|
| 17 and earlier | `v0.7.5` and earlier | | | no |
| 19 | `v0.8.0` | 2180 | 20 | no |
| 20 | `v0.8.2` | 2272 | 20 | no |
| 22 | `v0.9.0` to `v0.9.5` (and HEAD `2713feb`) | 3501 to 3675 | 58 | yes, generation 1 |

- **19 to 20** (`v0.8.0..v0.8.2`, +95 -3 in `wire.rs`): variants appended
  at the end of `ClientMessage` (`GraphicsTransmissionResult`,
  `InputPixels`) and of `ServerMessage` (`TerminalBell`, `GraphicsFile`,
  `GraphicsTransmissionStarted`, `GraphicsTransmissionRetired`), a variant
  `AppDirectGraphics` appended to `ClientLaunchMode`, and one field
  appended to a payload: `ServerMessage::MouseCapture { enabled, sgr_pixels
  }` (`v0.8.2` `wire.rs:717`). The appended field changes the bincode bytes
  of `MouseCapture`, so a decoder built from one version does not decode
  the other's `MouseCapture`: two generated modules are needed even though
  the mapping code is nearly the same.
- **20 to 22** (`v0.8.2..v0.9.0`, +1470 -241): a different protocol. 22
  adds the endpoint contract (`endpoint.rs`), the shell client
  (`ClientShellPaneInput`, `PaneSurface`, `PaneSurfacePatch`, per-pane
  rectangles, endpoint requests such as `pane.focus`) and renames the old
  `Hello` to `TerminalHello`.
- **Within 22** (`v0.9.0..v0.9.5` and on to the pin `2713feb`):
  `docs/HERDR_WIRE.md` (2026-10-27 23:50 EDT) found no change to any type
  reachable from `ClientMessage` or `ServerMessage` from `cc88b3b8`
  (2026-09-02, the endpoint contract's first commit) to `2713feb`, and
  herdr's nine golden hashes unchanged. `v0.9.0` (2026-09-07) and `v0.9.5`
  (2026-10-14) lie in that range by date; that they are on its history is
  to be confirmed from a full clone (step 2). One generated module serves
  every 22 release.

### 2.3 How a protocol 19 or 20 client gets one pane's cells

herdr 0.8.0 has no shell-client surface: its `Frame(FrameData)` is either
the whole herdr UI (an `App` client) or one terminal (a direct-attach
client). The direct-attach path is the one that fits flankr:

1. `ClientMessage::Hello { version: 19, cols, rows, cell_width_px: 0,
   cell_height_px: 0, requested_encoding: RenderEncoding::SemanticFrame,
   keybindings: ClientKeybindings::Server, launch_mode:
   ClientLaunchMode::TerminalAttach }`. The version must match exactly
   (`check_client_version`, `wire.rs:961`); a mismatch, a bad keybinding
   profile or a first message other than `Hello` is answered with
   `ServerMessage::Welcome { version, encoding, error: Some(..) }`
   (`client_transport.rs:528-579`). On success `Welcome { version: 19,
   encoding, error: None }` (`:582-587`).
2. `ClientMessage::ControlTerminal { target, takeover: false }`
   (`wire.rs:425`): `target` is a pane, terminal or agent target, resolved
   by `resolve_terminal_session_target` (`headless.rs:1782-1789`). The
   writable form: one controller per terminal; a second one without
   `takeover` is refused with `ServerShutdown { reason: "terminal attach
   failed: … already has an attached client; retry with --takeover" }`
   (`headless.rs:2614-2626`). On success the terminal is resized to the
   client's size and **locked against the desktop's resizes**
   (`direct_attach_resize_locks`, `headless.rs:2659-2668`) until the
   client leaves (`:1499-1504`). It does not move herdr's focus.
3. The server then renders that terminal alone at the client's size
   (`render_terminal_virtual`, `headless.rs:4105-4140`) into `FrameData {
   cells, width, height, cursor, hyperlinks, graphics }` and sends
   `ServerMessage::Frame` for a `SemanticFrame` client. Each `Frame` is
   the whole pane; there are no patches and no pane rectangles. herdr's
   own `herdr terminal session control` asks for `TerminalAnsi`
   (`src/client/mod.rs:912-941`); flankr asks for `SemanticFrame` so it
   gets cells. That the server honours `SemanticFrame` on this path is to
   be confirmed by step 5's recordings.
4. Input on this path: `ClientMessage::Input { data }` goes straight to
   the terminal's PTY as bytes (`apply_terminal_attach_input`,
   `headless.rs:2866-2872`); `AttachScroll { source: Wheel, direction,
   lines, column, row, modifiers }` routes a wheel by the pane's modes
   (`headless.rs:1791-1817`); `Resize { cols, rows, .. }` resizes the
   terminal (`headless.rs:2960-2995`); `Detach` leaves. herdr does not
   encode keys for this path: flankr would have to produce the bytes.
5. `ServerMessage::MouseCapture { enabled }` (`wire.rs:696`) says whether
   the attached terminal asks for mouse reporting; there is no
   alternate-screen flag and no scroll offset.

### 2.4 How the daemon can learn herdr's protocol before connecting

herdr's JSON API answers `{"id": .., "method": "ping", "params": {}}` with
`ResponseResult::Pong { version, protocol, capabilities }`, serialised as
`{"type": "pong", "version": "0.8.0", "protocol": 19, "capabilities": ..}`
(`src/api/schema/response.rs:42-50`; `src/api/server.rs:323-329` at
`v0.8.0`, `:559-566` at `2713feb`). `version` is
`crate::build_info::version()` (`0.9.5`, or `0.9.5-<channel>.<build>` on a
preview build) and `protocol` is `PROTOCOL_VERSION`. The daemon already
holds a JSON API client on herdr's socket (`--socket`, else
`HERDR_SOCKET_PATH`, else `config.toml`, else `~/.config/herdr/herdr.sock`,
`src/main.rs:162-164`, `:440`) and can send `ping` as a
`HerdrRequest::Custom` request, as `src/server/choice.rs:28` does for
`pane.send_text`; libherdr needs no release. The client socket is derived
from the API socket in the same way at 0.8.0 (`socket_paths.rs:50-57`) as
at 22, so `libherdr_term::client_socket_for` serves every version.

### 2.5 What the build-time extraction costs today

- `crates/libherdr_term/build.rs` and `build/extract.rs` (563 lines) need
  `syn` 2 (`full`, `visit-mut`), `quote`, `proc-macro2` and `prettyplease`
  as build and dev dependencies, and `vendor/herdr` checked out (or
  `HERDR_SRC`) for every build and test of the daemon.
- The extractor's lists (`SOURCES`, `KEPT_FNS`, `KEPT_IMPLS`, `OUTSIDE`,
  `REQUIRED`) are protocol 22's: `endpoint.rs`, `AgentStatus` and
  `ToastHerdrPosition` do not exist at 19 or 20, whose only outside type is
  `crate::input::WindowsKeyRecord` (`src/input/model.rs:7` at `v0.8.0`) and
  whose other `crate::` paths (`crate::input::{TerminalKey, TextCommit}`,
  `crate::raw_input::RawInputEvent`, `crate::protocol::ClientKeySource`)
  are in `impl` blocks or name a protocol type itself.
- Tests read herdr's own fixtures from the submodule
  (`tests/wire.rs:362`, `endpoint-hello-v1.json`, `endpoint-welcome-v1.json`,
  `endpoint-snapshot-v1.json`), and `src/fake.rs:17` finds the checkout.
- The daemon names herdr types directly: `libherdr_term::herdr::
  ClientPaneInputEvent` in `src/server/herdr_attach.rs:39`,
  `src/server/terminal.rs:78` and `src/server/conn.rs:1760`.

## 3. Terms

- A **protocol adapter** speaks one herdr client protocol version on
  herdr's client socket and turns it into flankr's model: a pane's
  `libflankr_term::Grid` with its `Cursor`, the pane's size, and input and
  resize in flankr's terms. There is one per herdr protocol version, in
  `libherdr_term`.
- A **screen driver** (plan 20261029_02) reads a pane's `Screen` and
  answers an agent's questions; there is one per agent, in the daemon. It
  never sees a protocol adapter, only the grid the daemon publishes from
  whichever adapter is attached, or the `pane.read` text when none is.

## 4. Decisions

1. **Supported protocols: 19, 20 and 22**, generated from the tags `v0.8.0`
   (19), `v0.8.2` (20) and `v0.9.5` (22). Protocols before 19 are not
   supported. Open question 1 asks whether 20 is worth carrying.
2. **Generate once, commit.** A new tool crate, `crates/herdr-wire-gen`
   (binary, `publish = false`, edition 2024), takes over
   `crates/libherdr_term/build/extract.rs` and its tests
   (`tests/extract.rs`). It is a workspace member but not a
   default member, so `cargo build` and `cargo test` without `-p` never
   build `syn` for it.
   - Its versions table, `crates/herdr-wire-gen/versions.toml`:

     ```toml
     [[version]]
     protocol = 19
     tag = "v0.8.0"
     commit = "346411fa21afd297f5ed3b3fa56f9e3fbf7654b7"
     profile = "direct"      # wire.rs + input/model.rs

     [[version]]
     protocol = 20
     tag = "v0.8.2"
     commit = "9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c"
     profile = "direct"

     [[version]]
     protocol = 22
     tag = "v0.9.5"
     commit = "2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01"
     profile = "endpoint"    # today's five files and lists
     ```

   - A profile is the extractor's lists for one protocol family:
     `endpoint` is today's `SOURCES`, `KEPT_FNS`, `KEPT_IMPLS`, `OUTSIDE`
     and `REQUIRED` unchanged; `direct` reads `src/protocol/wire.rs` and
     `src/input/model.rs`, keeps `write_message`, `read_message`,
     `read_exact_or_eof`, `Clone for CellData` and the `FramingError`
     impls, maps `crate::input::WindowsKeyRecord` and
     `crate::protocol::<Item>` to the generated module, and requires
     `PROTOCOL_VERSION`, `MAX_FRAME_SIZE`, `MAX_GRAPHICS_FRAME_SIZE`,
     `ClientMessage`, `ServerMessage`, `RenderEncoding`,
     `ClientKeybindings`, `ClientLaunchMode`, `CellData`, `CursorState`,
     `FrameData`, `AttachScrollSource`, `AttachScrollDirection` and
     `FramingError`. A kept item still naming another `crate::` path, a
     missing required item or a missing file fails the run with the item
     and the fix, as today.
   - It reads each file at the version's commit with `git -C <herdr> show
     <commit>:<path>` (no checkout is switched), checks that `<tag>`
     resolves to `<commit>`, and checks that the extracted
     `PROTOCOL_VERSION` equals `protocol`.
   - Commands:

     ```text
     cargo run -p herdr-wire-gen -- write            # all versions into crates/libherdr_term/src/
     cargo run -p herdr-wire-gen -- write --protocol 19
     cargo run -p herdr-wire-gen -- check            # regenerate in memory; fail if a committed file differs
     ```

     `--herdr <dir>` names the herdr clone, default `vendor/herdr`
     relative to the workspace root. `write` refuses to run on a herdr
     clone missing the tag and prints `git -C vendor/herdr fetch --depth 1
     origin tag <tag>`.
   - Output: `crates/libherdr_term/src/herdr_v19.rs`, `herdr_v20.rs`,
     `herdr_v22.rs`, formatted by prettyplease, each starting with:

     ```rust
     // @generated by herdr-wire-gen; do not edit. Regenerate with
     // `cargo run -p herdr-wire-gen -- write --protocol 19`.
     //
     // herdr's client protocol types, protocol version 19, from herdr
     // v0.8.0 (commit 346411fa21afd297f5ed3b3fa56f9e3fbf7654b7),
     // https://github.com/herdrdev/herdr, files src/protocol/wire.rs and
     // src/input/model.rs (SHA-256 of each as read: <hex>, <hex>).
     //
     // Copyright the herdr authors. Licensed under the Apache License,
     // Version 2.0 (the "License"); you may not use this file except in
     // compliance with the License. A copy is in LICENSE-herdr beside this
     // crate's Cargo.toml, or at http://www.apache.org/licenses/LICENSE-2.0.
     //
     // Changed from herdr's source: impl blocks other than those kept,
     // free functions other than the framing ones, test items and
     // cfg(windows) items removed; bincode::Decode and schemars::JsonSchema
     // derives removed; crate:: paths rewritten to this module.
     ```

   - Adding a version is one table row and one `write`; moving 22 to a
     later tag is changing its row, re-running `write`, and reviewing the
     diff of `herdr_v22.rs` (it should be the header only, if
     `docs/HERDR_WIRE.md`'s next section finds no reachable change).
3. **libherdr_term without a build script.** `build.rs`, `build/`, the
   `[build-dependencies]` and the `syn`/`quote`/`proc-macro2`/
   `prettyplease` dev-dependencies go; `HERDR_SRC` is no longer read. The
   generated files are modules `libherdr_term::herdr_v19`, `herdr_v20`
   and `herdr_v22`, each `pub mod` with today's `#[allow(dead_code,
   unused_imports, clippy::all, rustdoc::all, missing_docs,
   unreachable_pub)]`, and excluded from `cargo fmt` by a
   `#[rustfmt::skip]` on the `mod` item (the generator's prettyplease
   output stays byte-identical for `check`). herdr's three generation-1
   fixtures are copied into `crates/libherdr_term/tests/fixtures/herdr_v22/`
   (with the herdr commit they came from in a `SOURCE` file) so the tests
   need no submodule; `fake::herdr_root` goes.
4. **The submodule stays, for the generator only.** `vendor/herdr` stays
   in `.gitmodules` (shallow) and in the workspace `exclude`; building or
   testing any crate no longer needs it. Its pin is no longer what flankr
   speaks; `versions.toml` is. Open question 6 asks whether to drop it for
   a scratch clone.
5. **Licence.** `crates/libherdr_term/LICENSE-herdr` is herdr's `LICENSE`
   (the Apache License 2.0 text) copied verbatim from `v0.9.5`; herdr has
   no `NOTICE` file at any supported tag (checked at `v0.8.0` and `2713feb`;
   the generator checks each tag and fails if one appears, so it can be
   carried). Each generated file's header gives the attribution and the
   changes (Apache-2.0 section 4 (a)-(c)). The crate README's licence
   section names the three files and `LICENSE-herdr`; the rest of the
   crate stays flankr's.
6. **The adapter trait** (`crates/libherdr_term/src/adapter.rs`):

   ```rust
   /// A herdr client protocol version flankr has an adapter for.
   #[non_exhaustive]
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
   pub enum ProtocolVersion { V19, V20, V22 }

   impl ProtocolVersion {
       /// herdr's `PROTOCOL_VERSION` number, as `ping` reports it.
       pub fn from_protocol(protocol: u32) -> Option<ProtocolVersion>;
       pub fn protocol(self) -> u32;
   }

   /// The protocols with adapters, oldest first: [19, 20, 22].
   pub const SUPPORTED_PROTOCOLS: &[u32];

   /// What an adapter can carry; the daemon routes the rest as on a polled
   /// terminal (keys by `pane.send_keys`, `terminal.mouse` refused).
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub struct AdapterCaps {
       pub keys: bool,          // TermInput::Key / Text / Paste
       pub mouse: bool,         // TermInput::Mouse
       pub wheel: bool,         // TermInput::Wheel
       pub resize: bool,
       pub inactive: bool,      // can stay connected while no terminal is open
       pub retarget: bool,      // show() another pane on the same connection
       pub moves_focus: bool,   // show() moves herdr's focused pane
   }

   pub trait ProtocolAdapter: Send {
       fn version(&self) -> ProtocolVersion;
       fn caps(&self) -> AdapterCaps;
       /// herdr's version string from the handshake, if it gave one.
       fn server_version(&self) -> Option<&str>;
       /// Show `pane_id` at `cols` by `rows` (clamped by `clamp_surface`).
       fn show(&self, pane_id: &str, cols: u16, rows: u16) -> Result<(), AttachError>;
       fn resize(&self, cols: u16, rows: u16) -> Result<(), AttachError>;
       fn set_active(&self, active: bool) -> Result<(), AttachError>;
       fn input(&self, pane_id: &str, input: &[TermInput]) -> Result<(), AttachError>;
       fn detach(self: Box<Self>);
   }

   pub fn connect(
       version: ProtocolVersion,
       opts: &AttachOptions,
   ) -> Result<(Box<dyn ProtocolAdapter>, std::sync::mpsc::Receiver<AdapterEvent>), AttachError>;

   /// flankr's input, as the daemon's gates have passed it.
   #[non_exhaustive]
   pub enum TermInput {
       Key(KeyToken),
       Paste(String),
       Mouse(libflankr_proto::common::TerminalMouseParams),
       Wheel { lines: i32, col: u16, row: u16 },
   }

   /// One pane as the adapter last saw it, in flankr's model.
   pub struct PaneFrame {
       pub pane_id: String,
       pub grid: libflankr_term::Grid,     // cols x rows of the pane, cursor inside
       pub scroll: Option<PaneScroll>,     // None when the protocol has no offsets
       pub focused: bool,
   }
   pub struct PaneScroll { pub offset_from_bottom: u64, pub max_offset_from_bottom: u64 }

   #[non_exhaustive]
   pub enum AdapterEvent {
       Ready { server_version: Option<String> },
       /// Every pane in view after a frame, surface or patch.
       Frame { revision: u64, panes: Vec<PaneFrame> },
       ShowAnswer { pane_id: String, ok: bool, code: Option<String>, message: Option<String> },
       ActiveAnswer { active: bool, ok: bool, code: Option<String> },
       MouseCapture { enabled: bool },
       DesktopChange(DesktopChange),
       Closed { reason: Option<String> },
   }
   ```

   `AttachOptions`, `AttachError`, `client_socket_for`, `clamp_surface`,
   `KeyToken` and the colour and modifier conversions stay at the crate
   root and are shared by every adapter. `HERDR_PROTOCOL_VERSION` becomes
   the newest supported protocol (22, `herdr_v22::PROTOCOL_VERSION`);
   `ENDPOINT_PROTOCOL_GENERATION` stays (1).
7. **The protocol 22 adapter** (`src/v22/`) is today's code moved:
   `Attachment`, `Event`, `SurfaceView`, `PaneView`, `DesktopChange`,
   `Requested`, `METHOD_PANE_FOCUS` and `METHOD_SURFACE_SET` move from the
   crate root to `libherdr_term::v22`, using `herdr_v22`. Its
   `ProtocolAdapter` impl is a thin layer: `show` is `pane.focus` plus
   `ClientShellResize`; `set_active` is `client_shell.surface.set`;
   `input` turns `TermInput` into `ClientPaneInputEvent`s with today's
   conversions; each `Event::Surface` becomes `AdapterEvent::Frame` with a
   `PaneFrame` per `PaneSurfacePane` in view (today's `PaneView` to `Grid`
   conversion and pane cursor). Caps: all `true`. Behaviour on the wire is
   unchanged.
8. **The protocol 19 and 20 adapters** (`src/v19/`, `src/v20/`; one shared
   implementation `src/direct.rs`, generic over a small trait each
   generated module implements, since 19 and 20 differ only in appended
   variants and `MouseCapture.sgr_pixels`):
   - `connect` sends 2.3's `Hello` with the generated module's
     `PROTOCOL_VERSION`, reads `Welcome` (4 s timeout, today's
     `handshake_timeout`); an `error`, a `version` other than the one
     sent, or an `encoding` other than `SemanticFrame` is
     `AttachError::Incompatible { code: "refused" | "version" |
     "encoding", message, server_version: None }`. It then waits for
     `show`.
   - `show(pane_id, cols, rows)`: the first one sends `Resize` to the
     size, then `ControlTerminal { target: pane_id, takeover: false }`.
     A `ServerShutdown` instead of frames is `ShowAnswer { ok: false,
     code: "refused", message: reason }` followed by `Closed`. A connection
     controls one terminal: `show` of another pane on the same adapter is
     `AttachError::Endpoint { code: "retarget" }`, and the daemon detaches
     and connects again (`caps.retarget = false`).
   - Each `ServerMessage::Frame(FrameData)` becomes one `PaneFrame` for the
     controlled pane: the frame is the pane, `cells` row-major
     `width` by `height`, its cursor already pane-local; `scroll: None`;
     `Cursor.mouse_reporting` from the last `MouseCapture`,
     `alternate_screen` false. `revision` counts frames.
   - `resize` sends `Resize { cols, rows, 0, 0 }`. `input` carries only
     `TermInput::Wheel` as `AttachScroll { source: Wheel, direction: Up if
     lines > 0 else Down, lines: |lines| clamped 1-1000, column: Some(col),
     row: Some(row), modifiers: 0 }`; anything else is
     `AttachError::Endpoint { code: "unsupported" }`. Caps: `keys: false`,
     `mouse: false`, `wheel: true`, `resize: true`, `inactive: false`,
     `retarget: false`, `moves_focus: false`.
   - `set_active(false)` is `Detach` and closes; `detach` the same.
   - Every other `ServerMessage` variant is decoded (bincode cannot skip
     one) and ignored, except `ServerShutdown` (`Closed`) and
     `MouseCapture`.
9. **What plan 20261027_08 keeps and what this replaces.** That plan is
   frozen and is not edited. This plan:
   - replaces its **decision 2** (`build.rs`, the extraction at build time,
     `HERDR_SRC`, `$OUT_DIR/herdr_wire.rs`, the module `libherdr_term::herdr`,
     the `dump_herdr_types` example and `HERDR_WIRE_RS`) by decisions 2-3
     here; the kept and dropped items of its decision 2 are this plan's
     `endpoint` profile unchanged;
   - replaces the **build side of decision 14**: the submodule is the
     generator's source, not the build's (decision 4 here); and the
     **mechanism of the 23:42 EDT amendment, item 3** (`build.rs` reading
     `PROTOCOL_VERSION` from the pin): the version comes from
     `versions.toml` and each generated file. Its policy (22 tracks
     herdr's HEAD or latest release, each move audited in
     `docs/HERDR_WIRE.md` by protocol version) stays, applied to the
     22 row of `versions.toml`;
   - replaces **decision 1's** build dependencies (none now) and the
     **risk** "the build depends on the submodule" (it no longer does);
   - narrows **finding 2.6** as 2.1 here says;
   - keeps everything else for protocol 22 as written and amended: the
     endpoint generation 1 hello (decision 3), the takeover and focus rules
     (decision 5), the size rules (decision 6), the conversions (decision
     8), the cursor and link changes already released (decisions 9-10), the
     daemon's source switch and fallback (decision 11, now behind version
     selection), the input gates (decision 12), `[terminal] attach`
     (decision 13), and its amendments of 23:24, 23:32 and 23:42 EDT (one
     attachment kept open, a desktop change not forced back, the poll as
     the only fallback);
   - extends decision 11's attachment to protocols 19 and 20 with the
     differences of decision 8 here.
   The departures are recorded in `CHANGES.md` when this plan's
   implementation starts (Rule 1.7), naming plan 20261027_08 and its
   decisions 1, 2 and 14 and the 23:42 EDT amendment.
10. **Choosing the version before connecting.** In
    `src/server/herdr_attach.rs`, before each connect attempt (the first,
    and each reconnect after a close or a backoff), the daemon sends
    `ping` on herdr's JSON API with a 2 s timeout and reads `protocol` and
    `version` from the `pong`:
    - `ProtocolVersion::from_protocol(protocol)` is `Some(v)`: connect
      with `libherdr_term::adapter::connect(v, ..)`;
    - `None`, a `ping` error, or a `pong` without `protocol`: no
      connection is made to the client socket; terminals use the
      `pane.read` poll; the daemon logs once per herdr `version`: "herdr
      <version> speaks protocol <n>; flankr's attached terminal supports
      protocols 19, 20 and 22; using the poll", and pings again at the
      next backoff (1 s doubling to 30 s, as today) so an upgraded herdr
      is picked up.
    No adapter is ever tried against a server to see whether it works.
    A herdr whose `protocol` changes between pings (an upgrade under a
    running daemon) is reconnected with the new adapter after its
    attachment closes.
11. **The daemon's per-version behaviour** follows `caps()`:
    - `inactive: true` (22): one attachment for the daemon's run, inactive
      while no terminal is open, as today.
    - `inactive: false` (19, 20): connected only while a phone has a
      terminal open, as today's "detached while no terminal is open" path
      for a herdr without `client_shell.surface.set`
      (`herdr_attach.rs:476-488`); a switch to another session's pane
      detaches and connects again (`retarget: false`).
    - `keys: false`: `terminal.input` and `terminal.paste` on that terminal
      go through `pane.send_keys` / `pane.send_text` as on a polled
      terminal, under the same gates; `mouse: false`: `terminal.mouse` is
      `NOT_PERMITTED` ("mouse needs herdr 0.9"), and the tap arms typing as
      on a polled terminal; `wheel: true`: `terminal.scroll` goes through
      the adapter; its result's offsets are those `pane.scroll` reports
      when the adapter's frame has `scroll: None`.
    - `moves_focus: false`: no desktop-change detection; the `stale` flag
      is never set; the 23:32 EDT amendment's "not forced back" has nothing
      to act on, since herdr locks the pane at the phone's size until the
      phone detaches (2.3 item 2; open question 4).
    - The daemon no longer names herdr types: `herdr_attach.rs`,
      `terminal.rs` and `conn.rs` use `TermInput`, `PaneFrame` and
      `AdapterEvent`.
12. **Screen drivers get the grid on every supported herdr.** The daemon's
    `Published` view (plan 20261029_02, its finding 4 and `ScreenSource::
    Grid`) is built from `AdapterEvent::Frame`'s `PaneFrame`s, whatever the
    adapter. On herdr 0.8.0 the grid is there while a phone terminal shows
    the pane, the same condition as on 22; otherwise the drivers use the
    `pane.read` text as that plan says. Plan 20261029_02 needs no change
    for this; `attached_grid` reads `PaneFrame.grid` instead of cutting a
    `SurfaceView`.
13. **Recordings, made without touching the owner's herdr.**
    - A repository script, `scripts/herdr-test-servers.sh`, keeps one
      isolated herdr per version under `~/.cache/flankr-herdr-test/<tag>/`
      (`bin/herdr`, `config/`, `state/`, `run.sh`): it downloads herdr's
      macOS aarch64 release binary for `v0.8.0`, `v0.8.2` and `v0.9.5`
      from `https://github.com/herdrdev/herdr/releases/download/<tag>/`
      (the asset name confirmed in step 5), checks the SHA-256 written in
      the script after the first download, and checks `bin/herdr
      --version`. `run.sh` is today's `herdr-test` wrapper per version:
      it unsets `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH`,
      `HERDR_TAB_ID` and `HERDR_WORKSPACE_ID` and sets `XDG_CONFIG_HOME`
      and `XDG_STATE_HOME` to that version's folders, so its sockets are
      `~/.cache/flankr-herdr-test/<tag>/config/herdr/herdr.sock` and
      `herdr-client.sock`. The existing 0.9.5 setup at the folder's top
      level is left as it is.
    - Each server runs headless (`run.sh server`, in the background, no
      desktop client attached); the recorder creates one scratch workspace
      in it through that server's own JSON API, with one pane running a
      fixed script (`sh -c 'printf "\033[2J\033[H\033[1;31mred\033[0m
      plain\n\033[3;5Hcursor"; sleep 600'`, `TERM=xterm-256color`), and
      closes the workspace and stops the server at the end.
    - The recorder, `cargo run -p libherdr_term --example record --
      --protocol <n> --socket <client socket> --api <api socket> --pane
      <id> --out <dir>`, runs that version's adapter through a relay that
      writes every frame in both directions to
      `<dir>/<scenario>.frames` (records of `u8` direction, 0 client to
      server and 1 server to client, `u32` little-endian length, then the
      frame's bytes as on the socket) and `<scenario>.json` (`tag`,
      `commit`, `protocol`, `herdr --version`, `pane`, `cols`, `rows`,
      the script, the date). It refuses a `--socket` or `--api` path that
      is not under `~/.cache/flankr-herdr-test/`, and refuses to run when
      `HERDR_SOCKET_PATH` points at either.
    - The scenarios: `handshake` (connect, show the pane at 80x24, the
      first frame), `frame` (the script's screen: `red` in red, `plain`,
      the cursor at row 2, column 4), `input` (22: a key `a` press and
      release; 19 and 20: a wheel up of 3 lines), `resize` (to 100x30 and
      the next frame at that size).
    - Recordings go to `crates/libherdr_term/tests/fixtures/herdr_v<n>/`;
      evidence of each recording run (the server log tail and the
      recorder's output) to `out/herdr/<YYYYMMDD_NN>_recordings/` with its
      entry in `out/herdr/LOG.md` (Rule 3.5).

## 5. Steps

1. **The generator crate.** `crates/herdr-wire-gen` (decision 2), with
   `versions.toml`, the two profiles, `write` and `check`, added to the
   workspace `members` (not `default-members`) in its first commit; the
   extractor and `tests/extract.rs` are copied here from libherdr_term (the
   broken-copy test runs against both profiles) and removed from
   libherdr_term in step 3. libherdr_term is not changed in this step.
   **Done when:** `cargo test -p herdr-wire-gen` passes;
   `cargo run -p herdr-wire-gen -- write --out <scratch>` writes the
   three files from `vendor/herdr`'s tags, each reporting its
   `PROTOCOL_VERSION` (19, 20, 22); `herdr_v22.rs`'s body equals today's
   `$OUT_DIR/herdr_wire.rs` (built from `2713feb`) item for item; and fmt
   and clippy are clean for the crate.
2. **The audit for 19 and 20.** A new section in `docs/HERDR_WIRE.md`
   (append-only, its own commit) for protocols 19 and 20: the tags and
   commits, the types reachable from each version's `ClientMessage` and
   `ServerMessage`, the 19-to-20 differences of 2.2, the direct-attach
   path of 2.3 with line references, and, from a full-history scratch
   clone, whether `v0.9.0` and `v0.9.5` descend from `cc88b3b8` (so the 22
   audit covers them).
   **Done when:** the section is committed and names every type the 19
   and 20 adapters decode.
3. **libherdr_term 0.3.0: committed types and the adapter trait.** The
   three generated files and `LICENSE-herdr`; `build.rs`, `build/`, the
   build and `syn` dev dependencies, `HERDR_WIRE_RS` and
   `examples/dump_herdr_types.rs` removed; `herdr` becomes `herdr_v22`;
   the adapter module (decision 6) with `ProtocolVersion { V22 }` only;
   today's attachment moved to `v22` behind `ProtocolAdapter` (decision
   7); herdr's fixtures copied (decision 3); README updated (building,
   regeneration, licence); version 0.3.0 with its `VERSIONS.md` entry and
   the tag `libherdr_term-v0.3.0`. The daemon's mechanical path changes
   (`libherdr_term::herdr` to `herdr_v22`, the moved items to `v22::`) are
   their own commit right after (Rule 3.1), pushed together with this one
   so the branch builds; the daemon's behaviour changes wait for step 6.
   **Done when:** `cargo test -p libherdr_term` passes with
   `vendor/herdr` absent (moved aside for the run); every existing test
   passes unchanged apart from paths; `cargo run -p herdr-wire-gen --
   check` passes; fmt and clippy are clean; and `cargo tree -p
   libherdr_term` lists no `syn`.
4. **libherdr_term 0.3.1: the 19 and 20 adapters.** `src/direct.rs`,
   `src/v19/`, `src/v20/` (decision 8), `ProtocolVersion::{V19, V20}`,
   `SUPPORTED_PROTOCOLS = [19, 20, 22]`; the in-process fake server gains
   a direct-attach mode per version. Tests against the fakes: the `Hello`
   carries the version, `SemanticFrame` and `TerminalAttach`; a refusing
   `Welcome` is `Incompatible` with its code; `ControlTerminal` carries
   the pane and `takeover: false`; a `Frame` becomes one `PaneFrame` with
   the expected runs and cursor; a `ServerShutdown` after
   `ControlTerminal` is `ShowAnswer { ok: false }` then `Closed`; `show`
   of a second pane is `retarget`; `Resize` and `AttachScroll` carry the
   numbers; a v20 `MouseCapture { enabled, sgr_pixels }` decodes and a v19
   decoder refuses it. Version 0.3.1 with its `VERSIONS.md` entry and tag.
   **Done when:** `cargo test -p libherdr_term` passes these, fmt and
   clippy are clean.
5. **Recordings and fixture tests.** `scripts/herdr-test-servers.sh` in
   its own commit (repository-level, Rule 3.1); then, in libherdr_term,
   `examples/record.rs` and `tests/fixtures/herdr_v19/`, `herdr_v20/`,
   `herdr_v22/` with the four scenarios each (decision 13), and
   `tests/recorded.rs`, which replays each recording against that
   version's adapter: the client frames the adapter writes equal the
   recorded client frames byte for byte, and the server frames decode
   and map to the `PaneFrame` the scenario expects (`red` in red at row
   0, the cursor at row 2, column 4; 80x24, then 100x30 after `resize`).
   The recordings are made by an agent against the test servers only; if
   a step of 2.3 does not hold (the server sends `Terminal` ANSI frames
   to a `SemanticFrame` direct-attach client, or `ControlTerminal`
   refuses a pane id), the step stops and the owner is asked before the
   19/20 adapters change shape.
   **Done when:** `cargo test -p libherdr_term` replays all twelve
   recordings; the evidence and `out/herdr/LOG.md` entry exist; no
   process touched `~/.config/herdr` (the recorder's guard and the
   servers' logs show only `~/.cache/flankr-herdr-test/<tag>/` sockets).
6. **The daemon: version selection and the adapters.** Decisions 10-12 in
   `src/server/herdr_attach.rs`, `terminal.rs` and `conn.rs`: `ping`
   before each connect, `adapter::connect`, the caps-driven routing, the
   unknown-protocol log, `PaneFrame` into `Published`. Tests with the
   mock herdr's `ping` and the fakes: a `pong` with protocol 22 connects
   the 22 adapter and today's attachment tests still pass; 19 connects
   the direct adapter only when a terminal opens and detaches when it
   closes; a session switch on 19 reconnects; `terminal.input` on 19 goes
   to `pane.send_keys`; `terminal.mouse` on 19 is `NOT_PERMITTED`;
   `terminal.scroll` on 19 reaches the fake as `AttachScroll`; protocol 21
   or 23, a failed `ping` and a `pong` without `protocol` make no
   client-socket connection and serve the poll, with the log line once.
   **Done when:** those tests pass, `cargo clippy --all-targets` on the
   daemon adds no warnings, and on the owner's Mac (run by the owner, not
   an agent) a session's terminal on the phone, against the owner's herdr
   0.8.0, shows the pane's cells and cursor from the attachment (the
   daemon logs "Attached to herdr 0.8.0, protocol 19"), resizes with the
   phone, scrolls with a drag, and the desktop pane returns to its size
   when the phone leaves the terminal view.

## 6. Versioning

- **libherdr_term 0.2.0 to 0.3.0** (step 3). Not strictly additive under
  Rule 5.4: names are removed and moved. The `VERSIONS.md` entry:
  - **Breaking:** the module `herdr` is renamed `herdr_v22` (the same
    items, now generated once from herdr `v0.9.5` and committed).
  - **Breaking:** `HERDR_WIRE_RS` removed (the generated source is
    `src/herdr_v22.rs`).
  - **Breaking:** `Attachment`, `Event`, `SurfaceView`, `PaneView`,
    `DesktopChange`, `Requested`, `METHOD_PANE_FOCUS` and
    `METHOD_SURFACE_SET` move to `v22`.
  - **Breaking:** `fake::herdr_root` removed (feature `fake-server`).
  - **Changed:** `HERDR_PROTOCOL_VERSION` is now the newest protocol with
    an adapter (still 22), not the submodule pin's.
  - **Added:** `adapter::{ProtocolVersion, SUPPORTED_PROTOCOLS,
    AdapterCaps, ProtocolAdapter, connect, TermInput, PaneFrame,
    PaneScroll, AdapterEvent}`.
  Removing the build script, `syn` and the need for `vendor/herdr`
  changes no public item (build dependencies and the `HERDR_SRC`
  variable are not API) and is not listed. Tag `libherdr_term-v0.3.0`.
- **libherdr_term 0.3.0 to 0.3.1** (step 4). Strictly additive: new
  modules `herdr_v19`, `herdr_v20`, `v19`, `v20`, and `ProtocolVersion::
  {V19, V20}` on a `#[non_exhaustive]` enum (an open-enum variant, additive
  by the owner's ruling); `SUPPORTED_PROTOCOLS` gains 19 and 20 (a value,
  not a type). Tag `libherdr_term-v0.3.1`.
- **herdr-wire-gen 0.1.0**: a tool binary with no public API,
  `publish = false`; no `VERSIONS.md` entry and no tag, as for the
  daemon.
- **flankr** (the daemon) is an application: manifest and lock file only.
- libflankr_term, libflankr_proto, libherdr and flankr-companion are not
  changed.

## 7. Risks

- **19 and 20 are a different protocol, not an older copy of 22.** The
  direct-attach path controls one terminal per connection, locks its
  size, has no pane rectangles, scroll offsets or alternate-screen flag,
  and does not encode keys. The adapters are small (20 serde types), but
  their behaviour differs from 22's, and steps 4-5 rest on 2.3's reading
  of herdr 0.8.0's server, to be confirmed by the recordings.
- **The pane is held at the phone's size on 0.8.** While the phone's
  terminal is open, the owner's desktop shows that pane at the phone's
  size and cannot take it back by typing; it returns when the phone
  leaves the view (open question 4).
- **Pane ids.** `ControlTerminal`'s `target` is resolved as a pane,
  terminal or agent target; whether herdr 0.8.0 accepts the pane ids its
  JSON API gives flankr (`w1:p1`-style) is checked by the recordings.
- **Generated files drift from their tags.** Only if hand-edited;
  `herdr-wire-gen check` regenerates and compares, and the header names
  the command.
- **A herdr release reuses protocol 22 with a payload change.** As before:
  a decode error closes the attachment and the poll serves the terminal;
  each move of the 22 row is audited in `docs/HERDR_WIRE.md`.
- **Preview builds.** A preview herdr's `version` carries a suffix; the
  choice uses `protocol` only, so it is unaffected.

## 8. Open questions for the owner

1. **Which old versions:** 19 and 20, or 19 only? The owner runs 0.8.0
   (19); 0.8.2 (20) was one release and costs a second generated file and
   a few lines, since the adapters share one implementation.
2. **Protocol above 22:** the agreed rule sends an unknown protocol to the
   poll. herdr's endpoint generation 1 promises to stay available
   "indefinitely" whatever `PROTOCOL_VERSION` says; should a protocol
   above 22 use the 22 adapter (its hello is readable either way), or stay
   on the poll until a new row is generated (the plan)?
3. **Keep build-time extraction as an option** (a cargo feature that
   regenerates at build), or rely on `herdr-wire-gen check` only (the
   plan)?
4. **0.8's size lock:** accept that the desktop sees the pane at the
   phone's size while the phone's terminal is open (the plan), or use
   `ObserveTerminal` (read-only, no resize: the phone pans a desktop-sized
   pane, keys still through `pane.send_keys`)?
5. **Mouse on 0.8:** none (the plan: a tap arms typing, as on the poll),
   or flankr encodes SGR mouse reports itself into `Input { data }` while
   `MouseCapture` is on, which herdr 0.8 does not check against the pane's
   modes?
6. **The submodule:** keep `vendor/herdr` as the generator's source (the
   plan), or remove it and let the generator use a scratch clone at
   `~/.cache/flankr-herdr-gen/herdr`?
7. **When to drop a version:** when the owner's own herdr has moved past
   it (by an amendment naming the version and the commit that removes its
   files), or keep every version while its recordings pass?

## 9. README row

| [herdr protocol adapters, one per herdr protocol version, from herdr's types generated once and committed](docs/design/20261029_03_HERDR_PROTOCOL_ADAPTERS_PLAN.md) | 2026-10-29 | Draft | herdr's protocol types are generated once per supported herdr tag (`v0.8.0` protocol 19, `v0.8.2` 20, `v0.9.5` 22) by a new tool crate, `herdr-wire-gen`, and committed into libherdr_term as `herdr_v19.rs`, `herdr_v20.rs` and `herdr_v22.rs` with herdr's Apache-2.0 notice, so builds need no build script, `syn` or `vendor/herdr` (replacing plan 20261027_08's decision 2); a `ProtocolAdapter` per version maps its messages to flankr's grid, cursor, input and resize (22 is today's shell client; 19 and 20 control one terminal through herdr 0.8's direct attach); the daemon picks the adapter from herdr's JSON API `ping` (`protocol`) before connecting, unknown protocols use the poll; fixture tests replay recordings of headless test herdr servers; the screen drivers read the grid from any adapter, so herdr 0.8.0 panes get the grid route; libherdr_term 0.3.0 (breaking: `herdr` renamed `herdr_v22`, the attachment moved to `v22`) then 0.3.1. Draft: awaiting the owner's review of its open questions (which old versions, protocols above 22, 0.8's size lock and mouse, the submodule). |
