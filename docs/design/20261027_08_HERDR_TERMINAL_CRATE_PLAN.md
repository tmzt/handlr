---
title: libherdr_term, attaching to herdr as a client from herdr's own protocol types
date: 2026-10-27
status: Draft
covers:
  - A new crate, `crates/libherdr_term`, that attaches to the local herdr
    server as one of herdr's own clients (the stable endpoint contract,
    generation 1, on herdr's client socket) and gives the daemon one pane's
    cells, cursor, size and mouse mode, and sends it keys, text, paste,
    mouse and wheel events
  - herdr's protocol types compiled from a git submodule, `vendor/herdr`,
    pinned to one herdr commit; herdr itself is not changed
  - Conversions from herdr's types into flankr's own (`libflankr_term`,
    `libflankr_proto`) with `From`/`Into`, so those crates stay free of
    herdr types and the link protocol changes only additively
  - Takeover by design: the attached client resizes herdr's tab to the
    phone's size and moves herdr's focus to the session's pane
    (`tab_follows_focus`); what happens when the desktop client is used
    again and when the phone detaches
  - The daemon's switch from the `pane.read` poll to the attached stream,
    keeping the poll as the fallback
  - Not covered: herdr over SSH (plan 20261027_07's 20:44 EDT amendment:
    local mode only); herdr's optional surface encodings (`surface_delta`,
    `surface_reuse`, `surface_scroll`); Kitty graphics; popups
sources:
  - The owner (2026-10-27), verbatim: "we aren't changing the herdr impl so
    adding a libherdr_term crate makes sense, our own protocol should be
    able to wrap it via Into, we'll use a submodule for the herdr
    internals. write a plan first"
  - The owner (2026-10-27), verbatim, while this plan was drafted: "herdr is
    designed to allow the remote to take over and resizes to match, unlike
    the transcript mode, it's likely that the tab follows focus will be
    enabled and we'll have to send tab change requests (it wasnt clear in
    the plan if it uses a ssh connection per tab)"
  - herdr at `~/src/herdr`, commit `2713feb56e9c9db33992db0a262c68f7426d46dd`
    (2026-10-27, crate 0.9.5, Apache-2.0), read only; nothing was run and no
    socket was touched. Files and lines cited in section 2.
  - herdr's git history as gathered by the lead (the local clone is
    shallow): `PROTOCOL_VERSION` 2 to 22 between 2026-04-26 and 2026-09-01;
    `src/protocol/wire.rs` changed 8 more times after 22 without a bump
    (one a `feat!`, c411883e, 2026-09-24); `src/protocol/endpoint.rs` added
    in cc88b3b8 (2026-09-02) at `ENDPOINT_PROTOCOL_GENERATION = 1`, which
    has not changed since, only new `.v1` kinds added
  - flankr: `crates/libflankr_term/src/{lib,frame,mirror,screen,ansi}.rs`
    (`Color`, `flags`, `Run`, `Row`, `Grid`, `Frame`, `RowDiff`, `Mirror`,
    `Screen`; the frame bytes at `frame.rs:23-37`),
    `crates/libflankr_proto/src/common.rs:1300-1385` (the `terminal.*`
    params), `companion.rs:86-96` (`TerminalInfo`), `src/server/terminal.rs`
    (the poller, `scroll_request` at `:461`, `parse_keys` at `:808`),
    `src/transport.rs:96-98` (`tab_follows_focus`), `VERSIONS.md`
    (libflankr_proto 0.4.1, libherdr 0.3.1)
related:
  - docs/design/20261027_07_TERMINAL_MODE_PLAN.md (the terminal mode this
    feeds; its 14:23 EDT amendment on herdr's binary protocol, its 20:44 EDT
    amendment: local mode only, libflankr_proto 0.4.0)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3,
    its 21:58 EDT amendment: a drag scrolls like a wheel, a tap arms typing)
  - docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md (the
    `tab_follows_focus` client option)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (the link the frames
    ride on)
---

# libherdr_term, attaching to herdr as a client from herdr's own protocol types

## 1. Goal

The terminal view on the phone shows the session's pane as herdr's own
client would: exact cells, colours and attributes, the cursor, the pane's
real size, and the TUI's mouse mode; and a drag, a tap or a key reaches the
pane the way herdr's own client would send it, including real mouse events
for TUIs that turned mouse reporting on. flankr gets this by attaching to
the local herdr server as a herdr client, with herdr's own protocol types
compiled from a pinned submodule, without changing herdr.

## 2. Findings

All paths are in `~/src/herdr` at `2713feb`.

### 2.1 Attaching

1. **The client socket.** herdr serves its binary client protocol on
   `herdr-client.sock` beside the JSON API socket `herdr.sock`
   (`src/session.rs:171-187`). The path is derived from the API socket by
   inserting `-client` before `.sock` when `HERDR_SOCKET_PATH` is set
   (`src/server/socket_paths.rs:23-57`), else `HERDR_CLIENT_SOCKET_PATH`,
   else the session's data directory. flankr already knows the API socket
   (libherdr `transport.rs:21`), so the client socket is that path with
   `-client` inserted. The socket is mode 0600 (`socket_paths.rs:12`).
2. **Framing.** Every message is `[u32 little-endian length][bincode
   payload]`, bincode 2 through serde with `bincode::config::standard()`
   (varint integers, little endian, no limit), the decoder required to
   consume the whole payload (`src/protocol/wire.rs:1656-1716`). Frames
   over 2 MiB are refused (`MAX_FRAME_SIZE`, `wire.rs:24`); a client reads
   server frames up to 32 MiB (`MAX_GRAPHICS_FRAME_SIZE`, `wire.rs:29`).
   herdr uses bincode 2.0.1 and serde 1.0.229 (`Cargo.lock`).
3. **Three handshakes, two protocols.** The first message decides
   (`src/server/client_transport.rs:670-935`, handshake timeout 4 s at
   `:42`):
   - `ClientMessage::TerminalHello { version, cols, rows, .. }`
     (`wire.rs:466`): the *direct terminal* path. `version` must equal
     `PROTOCOL_VERSION = 22` exactly (`wire.rs:20`, `check_client_version`
     at `wire.rs:1749-1771`), else `ServerMessage::Welcome { error }`. It is
     followed by `AttachTerminal`, `ObserveTerminal` or `ControlTerminal`.
     The file says so itself: "`PROTOCOL_VERSION` still guards
     same-install direct-terminal and internal operations"
     (`wire.rs:6-8`).
   - `ClientMessage::EndpointControl { kind: "endpoint.hello.v1", data }`
     (`wire.rs:616`) where `data` is the JSON of `EndpointClientHello`
     (`src/protocol/endpoint.rs:45-77`): `generation`, `cell_width_px`,
     `cell_height_px`, `surface_size { cols, rows }`, `pixel_mouse`,
     `direct_graphics`, `endpoint_keybindings`, `mouse_capture`,
     `surface_active` (default true), `surface_reuse`, `surface_delta`,
     `surface_scroll` (default false), and the codec lists. The server
     refuses with an `EndpointServerWelcome` whose `error.code` is
     `invalid_hello`, `unsupported_generation` (generation is not 1),
     `no_common_core` (the client does not list all four of
     `shell.snapshot.v1`, `shell.surface.v1`, `shell.input.semantic.v1`,
     `shell.blob.v1`, `endpoint.rs:136-150`) or `invalid_surface`
     (`client_transport.rs:738-792`). Otherwise it answers
     `EndpointControl { kind: "endpoint.welcome.v1" }` with the methods
     the client lane serves and the capabilities (`endpoint.rs:152-176`,
     `client_transport.rs:829-847`). This is the **stable endpoint
     contract**: "Generation 1 is the compatibility floor for Local, SSH,
     and Cloud shell endpoints and must remain available indefinitely"
     (`endpoint.rs:1-9`); `ClientMessage` and `ServerMessage` have their
     "variant order frozen for endpoint generation 1"
     (`wire.rs:461`, `:1381`), and herdr's own tests pin the bincode bytes
     of `PaneSurface` and `PaneSurfacePatch` by SHA-256
     (`wire.rs:2583`, `:2630`) and every message tag
     (`wire.rs:1957`, `:2720`).
   - `ClientShellHello` (`wire.rs:557`): the pre-contract shell hello,
     refused with "this client predates the stable endpoint protocol"
     (`client_transport.rs:793-805`).
4. **What attaching as a shell does (the takeover).** A shell client that
   connects with `surface_active: true` becomes the server's foreground
   client at once (`src/server/headless.rs:2023-2029`): window title, bell,
   keybindings, host cell size and pixel-mouse availability follow it
   (`headless.rs:798-830`). It then claims its tab's geometry if no client
   owns it yet (`claim_unowned_shell_tab_geometry`,
   `src/server/headless/client_views.rs:714-734`). Any *interaction* (a
   key, text, paste or mouse event that is not a release,
   `headless.rs:3396-3400`) promotes it to foreground again and takes the
   tab's geometry unconditionally (`headless.rs:2430-2433`,
   `client_views.rs:693-712`); so do the endpoint requests in
   `shell_endpoint_claims_geometry`, `pane.focus` among them
   (`client_views.rs:245-257`). The geometry controller's surface size
   resizes every pane of that tab (`resize_tab_surface`); when it is the
   only active shell client, every tab of every workspace
   (`client_views.rs:565-597`, `:613-631`). This is the takeover the owner
   describes: the last client to interact wins the tab's size. (It is not
   herdr's `--takeover` flag, which belongs to the direct-terminal attach
   and is not used here.)
5. **When the desktop client is used again.** The desktop client's next
   interaction makes it foreground and geometry controller again, and the
   tab's panes go back to the desktop's size. flankr's surface keeps
   arriving at the phone's size, but its panes' PTYs are now the
   desktop's, so herdr clips or pads them into flankr's rectangles until
   flankr interacts again. There is no lock and no notification; it is a
   tug of war won by the last interaction.
6. **When flankr detaches** (or sets `client_shell.surface.set { active:
   false }`): its geometry claims are dropped, each viewed tab goes to the
   first remaining viewer and is resized to it, and the most recently
   active remaining shell client becomes foreground
   (`surface_interest.rs:83-93`, `client_views.rs:633-690`,
   `headless.rs:912-918`, `:1028-1041`). With the desktop client still
   attached, the desktop's size comes back without the owner touching it.
7. **Focus.** A shell client views one tab: the tab its own
   `ClientShellLocation` focuses (`client_views.rs:63-90`), seeded from
   herdr's global focus at connect (`headless.rs:2001-2010`). It changes
   it with endpoint requests: `ClientMessage::ClientShellEndpointRequest
   { boot_id, request }` (`wire.rs:592`) where `request` is the JSON
   of herdr's API request (`{"id", "method", "params"}`, decoded at
   `client_transport.rs:97-117`), answered by
   `ServerMessage::ClientShellEndpointResponseChunk` (`wire.rs:1489`).
   `tab.focus`, `pane.focus` and `pane.zoom` are client-lane methods
   (`src/api/schema.rs:77`, `:115`, `:96`). A focus request moves the
   client's own location, then **also moves herdr's global active tab**
   (`set_default_shell_target_from_client`, `client_views.rs:203-213`,
   called for every request at `:922`) and runs the API method, so
   `pane.focus` focuses the pane for herdr as a whole. The request needs
   an active surface (`endpoint_requests.rs:68-76`), one at a time
   (`endpoint_busy`, `:59-67`), and the `boot_id` of the current server
   boot (`stale_boot`, `:28-37`), which arrives in the first snapshot
   (`EndpointControl { kind: "shell.snapshot.v1" }`, JSON
   `ClientShellSnapshot`, `endpoint.rs:110-115`, `wire.rs:916-951`).
   **So a shell cannot show a pane on another tab without moving herdr's
   focus there.** With `tab_follows_focus` on, as the owner expects, that
   is what is wanted; with it off, the stream is only usable for panes on
   herdr's active tab.
8. **Read-only observers exist, on the other protocol.** `ObserveTerminal
   { target }` (`wire.rs:533`, handled at `headless.rs:1304-1324`) watches
   one pane without becoming foreground, resizing or taking input, and
   herdr documents it as the interface "for third-party bridges"
   (`herdr terminal session observe`, `src/client/terminal_sessions.rs`,
   `docs/next/website/src/content/docs/persistence-remote.mdx:138-150`).
   It is on the direct terminal path: exact `PROTOCOL_VERSION` match, and
   frames are ANSI bytes (`TerminalFrame`, `wire.rs:1326-1338`), not
   cells. `ControlTerminal` (`wire.rs:539`) is its writable form: one
   controller per terminal, the pane resized to the controller and locked
   against the desktop's resizes (`headless.rs:1737-1840`). The owner's
   ruling that takeover is by design makes the stable shell path the
   better fit; observe remains the fallback named in open question 3.

### 2.2 How herdr's remote mode maps connections

One SSH session per attached client, not per tab. `herdr --remote <target>`
is "a remote thin-client launcher over SSH command stdio"
(`src/remote/attach.rs:1`): it runs `ssh <target> exec herdr --session
<name> remote-client-bridge` (`attach.rs:417-438`), the host side pumps that
ssh's stdio into **one** client-socket connection (`src/remote/host.rs:6-37`),
and the local thin client reaches it through one per-process forward socket,
`herdr-remote-<pid>-<target>-<session>.sock` (`attach.rs:3633-3642`). Tabs
are not connections: the one shell connection views one tab at a time and
switches with `tab.focus`/`pane.focus` requests on that same connection
(2.1.7). flankr's terminal is local only (plan 20261027_07, 20:44 EDT), so
none of this is used; it is recorded because the earlier plan was unclear.

### 2.3 What a shell client receives

- `ServerMessage::PaneSurface(PaneSurfaceFrame)` (`wire.rs:1469`,
  `:1268-1280`): `boot_id`, `projection_revision`, `surface_revision`,
  `frame: FrameData`, `panes: Vec<PaneSurfacePane>`, `splits`, `popup`,
  `graphics`. `FrameData` (`wire.rs:763-776`) is `cells: Vec<CellData>`
  row-major, `width`, `height`, `cursor: Option<CursorState>`,
  `hyperlinks`, `graphics`. It is the viewed tab only, "without sidebar,
  tab bar, or overlays", at the client's `surface_size`.
- `ServerMessage::PaneSurfacePatch(PaneSurfacePatch)` (`wire.rs:1497`,
  `:1299-1309`): `base_surface_revision`, `surface_revision`, `rows:
  Vec<PaneSurfacePatchRow { x, y, cells }>` (a changed span of cells at a
  surface column and row, `wire.rs:1289-1295`), the `panes` whose content
  changed, and the final `cursor`. Applied in order against the last full
  surface (`src/server/render_stream.rs:343`). Without `surface_delta`,
  `surface_reuse` and `surface_scroll` in the hello these two are the whole
  surface stream (`render_stream.rs:179-230`).
- `CellData` (`wire.rs:697-710`): `symbol: String` (a grapheme), `fg: u32`,
  `bg: u32`, `modifier: u16`, `skip: bool` (the second half of a wide
  glyph), `hyperlink: Option<u32>`. Colours (`wire.rs:1509-1574`): tag byte
  `0x00` with `0` the default and `1..=16` the named colours in the order
  Black, Red, Green, Yellow, Blue, Magenta, Cyan, Gray, DarkGray, LightRed,
  LightGreen, LightYellow, LightBlue, LightMagenta, LightCyan, White;
  `0x01_0000NN` palette `NN`; `0x02RRGGBB` RGB. `modifier` is ratatui's
  `Modifier` bits (BOLD 1, DIM 2, ITALIC 4, UNDERLINED 8, SLOW_BLINK 16,
  RAPID_BLINK 32, REVERSED 64, HIDDEN 128, CROSSED_OUT 256) with an
  underline style in bits 12-15 (`wire.rs:1576-1600`).
- `CursorState { x, y, visible, shape }` (`wire.rs:749-758`), surface
  relative; `shape` is a DECSCUSR parameter.
- **One pane can be cut from the surface.** `PaneSurfacePane`
  (`wire.rs:1090-1104`) gives each pane's `pane_id`, `rect`, `inner_rect`
  (the terminal cells, inside borders and scrollbar gutter), `scroll {
  offset_from_bottom, max_offset_from_bottom, viewport_rows }`, `focused`,
  `mouse_reporting`, `sgr_pixel_mouse` and `alternate_screen_active`. The
  session's pane is the `inner_rect` of the entry with its `pane_id`; its
  size is `inner_rect.width` by `inner_rect.height`, which is the PTY's size
  while flankr holds the tab's geometry; the cursor is the pane's when it
  falls inside that rectangle.
- Also on the connection: `EndpointControl` kinds (snapshot, agent view,
  agent completions, presentation sync, health ping), `MouseCapture`,
  `ClientShellKeyboardReportAll`, `Notify`, `SemanticNotification`,
  `WindowTitle`, `TerminalBell`, `Clipboard`, `ServerShutdown`
  (`wire.rs:1384-1504`). Unknown `EndpointControl` kinds are to be ignored
  (`endpoint.rs:8`). Every `ServerMessage` variant must decode, since
  bincode cannot skip a variant it does not know: the whole enum and every
  type under it are needed.

### 2.4 What a shell client sends

- `ClientMessage::ClientShellPaneInput { pane_id, events:
  Vec<ClientPaneInputEvent> }` (`wire.rs:580-583`), at most 4096 events
  per message (`client_transport.rs:118`), targeted by **pane id**, not by
  herdr's focus or surface coordinates. It reaches the pane only if the
  pane is in the client's viewed tab (the focused pane only, when the tab
  is zoomed) and no popup covers the tab (`headless.rs:2394-2424`,
  `client_views.rs:493-519`).
- `ClientPaneInputEvent` (`wire.rs:161-182`): `Key { code: ClientKeyCode,
  modifiers: u8, kind: Press|Repeat|Release, repeat_count,
  shifted_codepoint, generated_text, tracks_release, physical_key_id,
  windows_record }`, `TextCommit(String)`, `Paste(String)`, `Mouse { kind:
  ClientMouseKind, position: ClientMousePosition, geometry, modifiers: u8,
  lines: u16 }`. `ClientMouseKind` is `Down/Up/Drag(Left|Right|Middle)`,
  `Moved`, `ScrollUp/Down/Left/Right` (`wire.rs:94-103`);
  `ClientMousePosition::Cell { column, row }` is **pane-local**, the
  surface position minus the pane's `inner_rect` origin
  (`src/client/shell/mouse.rs:2338-2342`). Modifiers are crossterm's bits
  (Shift 1, Ctrl 2, Alt 4).
- herdr encodes keys for the pane's negotiated keyboard protocol and mouse
  events against the pane's VT modes (`src/server/pane_input.rs:229-355`).
  A button or motion event is dropped when the TUI has not turned mouse
  reporting on (the encoder returns nothing), so a shell never gets stray
  bytes. **The wheel covers both cases of the L3 drag:**
  `ScrollUp`/`ScrollDown` go through `apply_scroll`
  (`pane_input.rs:128-185`), which by the pane's `wheel_routing()`
  (`src/pane.rs:1696-1700`, `:3732`) sends a mouse report
  (`MouseReport`), arrow keys in an alternate screen without mouse
  (`AlternateScroll`), or scrolls herdr's own scrollback (`HostScroll`).
  Wheel events are interactions, so they take the geometry and foreground
  like keys.
- `ClientShellResize { cell_width_px, cell_height_px, surface_size,
  pixel_mouse }` (`wire.rs:571`) changes the surface size; `Detach`
  (`wire.rs:506`) leaves cleanly.

### 2.5 Compiling herdr's types without changing herdr

herdr is a binary-only crate: `src/main.rs` declares `mod protocol;` with
no lib target, so no other crate can depend on it, and adding one would
change herdr (not allowed). Its workspace member `crates/ghostty-vt` needs
zig (`crates/ghostty-vt/build.rs:6-70`); herdr pins Rust 1.96.1
(`rust-toolchain.toml`) and edition 2021.

The protocol files are not standalone. `wire.rs` (3675 lines, tests from
`:1773`) has these outside references:

| Reference | Where | Used by |
|---|---|---|
| `crate::input::WindowsKeyRecord` | `src/input/model.rs:7-15` | `ClientPaneInputEvent::Key` field (on the wire) |
| `crate::api::schema::AgentStatus` | `src/api/schema/common.rs:247-254` | `ClientShellTab`, `ClientShellPane`, `ClientShellAgent` fields (on the wire, JSON snapshot) |
| `crate::config::ToastHerdrPosition` | `src/config/model.rs:75-82` | `SemanticNotification` field (on the wire) |
| `crate::input::{TerminalKey, TextCommit}`, `crate::raw_input::RawInputEvent`, `crate::terminal_theme`, `crate::config::CustomCommandAction`, `crossterm`, `ratatui` | `raw_input.rs` (3801 lines) and more | `impl` blocks only (conversions herdr's server and client use) |
| `crate::build_info::version`, `crate::api::schema::AgentViewSetParams` | `endpoint.rs:128`, `:162` | free functions only |

The options:

- **(a) `#[path]` includes of the protocol files.** Fails as is: the
  `impl` blocks need `raw_input.rs`, `input/`, `terminal_theme.rs`, ratatui
  0.30 and crossterm 0.29, whose own `crate::` references go further
  (`raw_input.rs:34-36`, `:250`, `crate::platform`). Shimming those modules
  means hand-writing look-alikes of herdr's internals. Worse, an included
  file's `#[cfg(test)] mod tests` and `#[cfg(any(windows, test))]` items
  compile under flankr's own `cargo test` and reference `crate::pane`,
  `crate::terminal` and `crate::ghostty`.
- **(b) A build script that extracts the types from the submodule.**
  Parse `wire.rs` and `endpoint.rs` with `syn`, keep the type definitions
  and the few serde helpers, drop `impl` blocks and test items, map the
  three wire-visible outside types to their definitions extracted from
  their own files, and refuse to build if any other `crate::` path is left.
  The output is herdr's own definitions, field for field and variant for
  variant, regenerated from whatever commit the submodule is at. Build
  dependencies: `syn` 2 (`full`, `visit-mut`), `quote` 1, `prettyplease`
  0.2. Runtime: `serde`, `serde_json`, `bincode` 2.0.1 (`serde`), `base64`
  0.22 (not needed by the kept items; listed only if the extractor keeps
  `surface_delta` later). No zig, no ratatui, no crossterm, no tokio from
  herdr. The edition difference does not matter for type definitions
  (herdr's protocol files use no `gen` identifier and no macros).
- **(c) A lib target on herdr via a path dependency.** Changes herdr; not
  allowed. It would also pull ghostty-vt and zig.
- **(d) Re-declaring the types by hand in flankr.** About 75 types (every
  `ServerMessage` payload must decode, 2.3), with no link to herdr's source
  except a drift test; this is what the owner's "a submodule for the herdr
  internals" asks to avoid.

**(b) is chosen** (decision 2).

### 2.6 Drift

- The stable surface is the endpoint contract: `EndpointClientHello` and
  `EndpointServerWelcome` are JSON with serde defaults
  (`endpoint.rs:5-8`); the enum variant order of `ClientMessage` and
  `ServerMessage` is frozen; the bincode bytes of the surface messages are
  pinned by herdr's own golden hashes. What is *not* promised in words is
  that payload structs reached only through those variants never gain a
  field; the 8 unbumped `wire.rs` changes after `PROTOCOL_VERSION = 22`
  (lead's finding) must be audited against the gen-1 payloads (step 1).
- A refused hello is always readable: `EndpointControl` is the last
  variant of both enums with a frozen two-string payload, and the welcome
  is JSON, so an incompatible herdr answers in a form flankr decodes
  (`unsupported_generation`, `no_common_core`).
- A payload change inside generation 1 would show as a bincode decode
  error or trailing bytes on a surface message (`wire.rs:1700-1713`), not
  at the handshake. libherdr_term treats that as incompatibility, closes,
  and the daemon falls back to the poll.

## 3. Feasibility verdict

**Feasible, by takeover, as the owner intends.** flankr attaches to the
local herdr over the stable endpoint contract (generation 1) as one more
herdr client, exactly as herdr's own thin and remote clients do. That
**does change the owner's desktop session, by design**: while the phone's
terminal is open flankr is herdr's foreground client, its tab is laid out
at the phone's size (every tab, if the desktop client is not attached), and
herdr's focus moves to the session's pane (`tab_follows_focus`). The desktop
gets its size back the moment the owner types or clicks there, and for good
when the phone leaves the terminal view and flankr detaches. In return the
phone gets herdr's own cells, cursor, exact pane size and mouse mode, and
real mouse and wheel input that herdr encodes against the TUI's modes.

Two limits stand. With `tab_follows_focus` off, a pane on a tab other than
herdr's active one cannot be streamed without moving herdr's focus, so the
daemon uses the `pane.read` poll for it. And a pane in a split tab gets only
its share of the phone's surface (open question 1 on zooming). The poll of
plan 20261027_07 stays as the fallback for both, for an incompatible herdr,
and for a missing submodule build.

## 4. Decisions

1. **Crate.** `crates/libherdr_term` (`libherdr_term`), version 0.1.0,
   edition 2024, licence note: contains code generated from herdr
   (Apache-2.0), with herdr's `LICENSE` referenced from the crate README.
   It depends on `libflankr_term`, `libflankr_proto`, `serde`, `serde_json`,
   `bincode` 2.0.1 (`serde` feature) and `unicode-width` 0.2; build
   dependencies `syn`, `quote`, `prettyplease`. It does not depend on
   `libherdr` or tokio: its connection is a blocking `UnixStream` on one
   named thread per attachment, as herdr's own client does, and it hands
   events to the caller through `std::sync::mpsc`; the daemon bridges that
   to its async side. It is added to the workspace's `members` and
   `default-members`, and `vendor/herdr` to the workspace `exclude`.
2. **herdr's types from the submodule** (2.5 (b)). `build.rs` reads, from
   `../../vendor/herdr` (overridable by `HERDR_SRC`):
   `src/protocol/wire.rs`, `src/protocol/endpoint.rs`,
   `src/input/model.rs`, `src/api/schema/common.rs`,
   `src/config/model.rs`. It keeps:
   - from `wire.rs` and `endpoint.rs`: every `const`, every `type` alias,
     every `struct` and `enum` deriving `Serialize` or `Deserialize`, and
     `FramingError`; the functions `serialize_graphics_bytes`,
     `deserialize_graphics_bytes`, `deserialize_client_shell_agent_status`,
     `default_true`, `write_message`, `read_message`, `read_exact_or_eof`;
     the impls `Clone for CellData` and `Display`, `Error`,
     `From<io::Error>` for `FramingError`;
   - `WindowsKeyRecord` from `input/model.rs`, `AgentStatus` from
     `common.rs` and `ToastHerdrPosition` from `config/model.rs`, with
     `schemars::JsonSchema` removed from their derives, and the paths
     `crate::input::WindowsKeyRecord`, `crate::api::schema::AgentStatus`
     and `crate::config::ToastHerdrPosition` rewritten to them;
   - and drops every item under `#[cfg(test)]`, `#[cfg(windows)]` or
     `#[cfg(any(windows, test))]`, every other `impl` and function, and
     `bincode::Decode` from derives (flankr decodes through serde only).
   If a kept item still names a `crate::` path, a listed item is missing,
   or a listed file is absent, the build fails with the item and the fix
   ("map it in build.rs" or "run `git submodule update --init
   vendor/herdr`"). The output is `$OUT_DIR/herdr_wire.rs`, included as
   `libherdr_term::herdr` (`#[allow(dead_code, clippy::all)]`), with
   `cargo:rerun-if-changed` on each source. An example,
   `cargo run -p libherdr_term --example dump_herdr_types`, prints it for
   review when the pin moves. The crate's `[lib]` sets `test = false` and
   `doctest = false` and its tests live in `tests/`, so no herdr test code
   is ever compiled.
3. **The protocol spoken: endpoint generation 1, as a shell client.** The
   hello is `EndpointClientHello { generation: 1, cell_width_px: 0,
   cell_height_px: 0, surface_size, pixel_mouse: false, direct_graphics:
   false, endpoint_keybindings: false, mouse_capture: false,
   surface_active: true, surface_reuse: false, surface_delta: false,
   surface_scroll: false, snapshot_codecs: ["shell.snapshot.v1"],
   surface_codecs: ["shell.surface.v1"], input_codecs:
   ["shell.input.semantic.v1"], blob_codecs: ["shell.blob.v1"] }`. A
   welcome whose `generation != 1` or whose `error` is set is
   `AttachError::Incompatible { code, message, server_version }`. The
   direct-terminal path (`TerminalHello`, `PROTOCOL_VERSION`) is not used
   (open question 3).
4. **One herdr connection per phone connection, retargeted.** The daemon
   opens one shell connection for a link connection (`Conn`) when it opens
   its first terminal and closes it when that connection has no terminal
   open. A phone shows one terminal at a time; opening or switching to
   another session's terminal on the same phone retargets the same herdr
   connection with `pane.focus`. Not one per daemon: two phones have
   different sizes and may show different sessions, and herdr gives each
   client one location and one size. Not one per terminal: every active
   shell client becomes foreground and claims its tab, so two connections
   for one phone would fight each other as well as the desktop. A
   terminal of the same connection that is not shown gets no stream (its
   `Mirror` keeps its last frame and is refreshed by the poll if it is
   ever read).
5. **Takeover and focus.**
   - On open: connect with the phone's surface (decision 6), wait for the
     first `shell.snapshot.v1` (its `boot_id`), then send the endpoint
     request `{"id": "<n>", "method": "pane.focus", "params": {"pane_id":
     "<pane>"}}`. That moves flankr's view, herdr's active tab and herdr's
     focused pane to the session (2.1.7) and claims the tab's geometry
     (2.1.4).
   - On a session switch within the same phone connection: the same
     request for the new pane, one request in flight at a time
     (`endpoint_busy` is retried after the previous answer).
   - With the phone's `tab_follows_focus` client option off: no stream is
     opened for a pane that is not on herdr's active tab (read with
     `pane.list`); that terminal is served by the poll, and `pane.focus` is
     never sent.
   - flankr never re-claims the geometry on a timer. It claims on open, on
     a switch, and on every input it sends (which herdr treats as an
     interaction); so after the owner uses the desktop, the phone shows
     herdr's clipped view until the owner touches the phone's terminal,
     and the daemon sets `TerminalCursor`'s `stale` flag (decision 8)
     while a `MouseCapture` or surface tells it the desktop took over
     (open question 2).
   - On close (the phone leaves the terminal view, closes the terminal or
     drops the link): `ClientMessage::Detach`, then the socket is closed.
     herdr gives the tab back to the desktop client (2.1.6).
   - `pane.zoom` is not sent (open question 1).
6. **Surface size.** `surface_size` is the phone's `cols_hint` and
   `rows_hint` from `terminal.open` (`common.rs:1300-1311`), clamped to
   20-500 columns and 5-200 rows; without hints, 80 by 24. A later
   `terminal.resize { terminal_id, cols, rows }` (rotation, zoom) sends
   `ClientShellResize`. The pane's size reported to the phone is its
   `inner_rect` size, not the surface size.
7. **The library's API** (all in `libherdr_term`):

   ```rust
   pub struct AttachOptions {
       pub socket: PathBuf,          // herdr-client.sock
       pub cols: u16, pub rows: u16, // surface_size
   }
   pub fn client_socket_for(api_socket: &Path) -> PathBuf;

   pub struct Attachment { /* writer half, request ids, thread handle */ }
   impl Attachment {
       pub fn connect(opts: &AttachOptions) -> Result<(Attachment, Receiver<Event>), AttachError>;
       pub fn focus_pane(&self, pane_id: &str) -> Result<(), AttachError>;
       pub fn resize(&self, cols: u16, rows: u16) -> Result<(), AttachError>;
       pub fn input(&self, pane_id: &str, events: Vec<herdr::ClientPaneInputEvent>) -> Result<(), AttachError>;
       pub fn detach(self);
   }

   pub enum Event {
       Ready { boot_id: String, server_version: String },
       Surface(SurfaceView),        // after every full surface or patch
       FocusAnswer { ok: bool, code: Option<String> },
       MouseCapture { enabled: bool },
       Closed { reason: Option<String> },
   }

   /// The tab as flankr last applied it, herdr's own types inside.
   pub struct SurfaceView {
       pub surface_revision: u64,
       pub frame: herdr::FrameData,
       pub panes: Vec<herdr::PaneSurfacePane>,
   }
   impl SurfaceView {
       pub fn pane(&self, pane_id: &str) -> Option<PaneView<'_>>;
   }
   pub struct PaneView<'a> {
       pub pane: &'a herdr::PaneSurfacePane,
       pub cells: Vec<&'a [herdr::CellData]>,      // inner_rect rows
       pub cursor: Option<herdr::CursorState>,     // pane-local, if inside
   }

   pub enum AttachError {
       Io(std::io::Error),
       Framing(String),                             // herdr::FramingError
       Incompatible { code: String, message: String, server_version: Option<String> },
       Endpoint { code: String, message: String },  // a request's error
       Closed,
   }
   ```

   Patches are applied in order: a patch whose `base_surface_revision` is
   not the held `surface_revision` closes the attachment with
   `AttachError::Framing`, and the daemon reconnects once (a full surface
   follows a new connection).
8. **Conversions into flankr's types, `From`/`Into`.** They live in
   `libherdr_term` (it owns the generated herdr types, so
   `impl From<herdr::X> for libflankr_term::Y` is allowed by the orphan
   rule); `libflankr_term` and `libflankr_proto` never see herdr:
   - `impl From<herdr::Color32> for libflankr_term::Color`, where
     `Color32(u32)` is a local newtype over `CellData.fg`/`bg`: `0` to
     `Color::DEFAULT`; `0x00_0000NN` with `NN` in 1-16 to
     `Color::palette(NN - 1)`; `0x01_0000NN` to `Color::palette(NN)`;
     `0x02RRGGBB` to `Color::rgb`; anything else to `DEFAULT`.
   - `impl From<herdr::Modifier16> for u8` (`libflankr_term::flags`):
     BOLD 1 to `BOLD`, DIM 2 to `DIM`, ITALIC 4 to `ITALIC`, UNDERLINED 8
     or a non-zero underline style to `UNDERLINE`, REVERSED 64 to
     `INVERSE`, CROSSED_OUT 256 to `STRIKETHROUGH`; blink is dropped;
     HIDDEN 128 makes the run's text spaces.
   - `impl From<PaneView<'_>> for libflankr_term::Grid`: each row's cells
     become runs of one style, `skip` cells emit nothing, adjacent equal
     styles merge, trailing default-style spaces are cut, so equal rows are
     equal values as `parse_ansi` makes them (`ansi.rs:38`); `cols` and
     `rows` are the `inner_rect` size; `cursor` (decision 9) from the pane
     cursor and the `PaneSurfacePane` flags.
   - `impl From<herdr::CursorState> for libflankr_term::Cursor`.
   - flankr to herdr, for input: `impl From<&libflankr_proto::common::TerminalMouseParams>
     for herdr::ClientPaneInputEvent` (decision 10), and
     `impl From<libflankr_proto::common::TerminalScrollParams> for
     herdr::ClientPaneInputEvent` (`lines > 0` is `ScrollUp`, `< 0`
     `ScrollDown`, `lines.unsigned_abs()` clamped to 1-1000 as `lines`,
     position `Cell { column: 0, row: 0 }` or the drag's start cell when the
     phone sends it), and `impl From<KeyToken> for herdr::ClientPaneInputEvent`
     for the tokens `parse_keys` already produces (`terminal.rs:808`): a
     character is `TextCommit`, `enter`, `escape`, `tab`, `backspace` and
     the arrows are `Key { kind: Press }` followed by the same with
     `Release`, `ctrl+<a-z>` is `Key { code: Char, modifiers: 2 }`.
9. **The cursor, additive in `libflankr_term` (0.1.0 to 0.1.1).**
   - `pub struct Cursor { pub col: u16, pub row: u16, pub visible: bool,
     pub shape: u8, pub mouse_reporting: bool, pub alternate_screen: bool,
     pub stale: bool }`, a new type.
   - `Grid` gains an appended `pub cursor: Option<Cursor>` (`serde(default)`),
     `Grid::from_lines` sets `None`, so `parse_ansi` grids are unchanged.
   - `Frame` gains an appended `pub cursor: Option<Cursor>`, encoded as a
     trailer after the rows of `frame.rs:23-37`: `u8 1`, `u16 col`, `u16
     row`, `u8 bits` (bit 0 visible, 1 mouse reporting, 2 alternate
     screen, 3 stale, 4-7 zero), `u8 shape`: 7 bytes. `Frame::decode`
     reads the trailer when present; any other trailing bytes stay
     `DecodeError::TrailingBytes`. `Mirror` sends the cursor in every frame
     of a terminal whose grids carry one, and a cursor change alone makes
     a frame with no rows. `Screen` keeps the newest cursor.
   - An older phone never gets a trailer: the daemon writes it only when
     the phone's `terminal.open` asked (decision 10).
   - Rule 5: appended fields and a new type, additive by the owner's
     ruling; a patch bump with its `VERSIONS.md` entry and the tag
     `libflankr_term-v0.1.1`.
10. **The link protocol, additive in `libflankr_proto` (0.4.1 to 0.4.2).**
    - `TerminalOpenParams` gains appended `#[serde(default)] cursor: bool`
      (the phone decodes the trailer) and `TerminalOpenResult` gains
      appended `#[serde(default)] cursor: bool` (frames on this terminal
      carry it) and `#[serde(default)] source: TerminalSource` with
      `#[non_exhaustive] enum TerminalSource { #[default] Poll, Attached }`.
    - `TerminalInfo` gains appended `#[serde(default)] mouse: bool`: the
      daemon serves `terminal.mouse`.
    - New method `terminal.mouse` (`common::methods::TERMINAL_MOUSE`) with
      `TerminalMouseParams { terminal_id: u32, kind: TerminalMouseKind,
      col: u16, row: u16, #[serde(default)] modifiers: u8 }` and
      `#[non_exhaustive] enum TerminalMouseKind { Down(Button),
      Up(Button), Drag(Button), Moved }`, `enum Button { Left, Right,
      Middle }`; result `{}`. Pane-local cells, as herdr expects.
    - New method `terminal.resize { terminal_id, cols, rows }` -> `{}`.
    - `terminal.scroll` is unchanged on the wire; on an attached terminal
      the daemon sends it as a wheel event (decision 8) instead of
      `pane.scroll`, so a mouse-reporting TUI scrolls itself, and its
      result's `offset_from_bottom`/`max_offset_from_bottom` come from the
      next surface's `PaneSurfacePane.scroll`.
    - Rule 5: a new method, new types and appended fields: additive, a
      patch bump with its `VERSIONS.md` entry and tag
      `libflankr_proto-v0.4.2`. `companion::VERSION` stays 1 and the link
      prologue `flankr.link/1`. (The owner's 0.4.0 minor was a one-off
      ruling for plan 20261027_07 and is not repeated here.)
11. **The daemon's source switch.** `src/server/terminal.rs` gains a
    per-connection `herdr_attachment: Option<HerdrAttach>` beside its
    terminal table. `terminal.open` on a local herdr (`herdr_is_local`,
    `terminal.rs:150`) with the attachment allowed (decision 13) tries the
    attachment first; on `AttachError::Incompatible`, a connect error or a
    pane not on the active tab with `tab_follows_focus` off it uses the
    poll as today and answers `source: Poll`. While attached, each
    `Event::Surface` whose pane view changed becomes a `Grid` by `Into`
    and goes through the same `Mirror` and the same 8 frames a second
    cap; the 2 s re-read and the `pane.scroll_changed` wake-ups are not
    used for that terminal. If the attachment closes, the terminal falls
    back to the poll without closing (`source` is not re-announced; the
    phone sees `cursor: None` frames). The incompatibility is logged once
    per daemon run with herdr's `server_version` and code.
12. **Input over the attachment, gated as `terminal.input` is.**
    `terminal.input`, `terminal.scroll` and `terminal.mouse` all require
    `[terminal] input`, the agent's `terminal_input`, a principal that may
    approve and a live arm (plan 20261027_07 decision 6, as the L3 21:58
    EDT amendment changed it); `terminal.mouse` and the wheel extend the
    arm as keys do. On an attached terminal, keys go as
    `ClientShellPaneInput` events (decision 8) after `parse_keys` has
    refused what it refuses today (signals without `terminal_signals`,
    control characters, over 64 bytes); on a polled terminal they go
    through `pane.send_keys` as today. `terminal.mouse` on a polled
    terminal is `NOT_PERMITTED` ("mouse needs the attached terminal").
    Mouse events are not logged one by one; one `terminal_input` notice per
    arm window records that the terminal was clicked, as keys are.
13. **Config.** `[terminal] attach = true` (default true; false keeps the
    poll only). No other new keys.
14. **The submodule.** `vendor/herdr`, URL
    `https://github.com/herdrdev/herdr`, pinned to
    `2713feb56e9c9db33992db0a262c68f7426d46dd` until step 1's audit
    chooses the commit matching the owner's installed herdr. `.gitmodules`
    sets `shallow = true`; it is not recursive (herdr's `vendor/` is
    inline, not submodules). A checkout without it builds nothing of
    `libherdr_term` and the build error says how to fetch it.

## 5. Steps

1. **The submodule and its audit.** Add `vendor/herdr` (decision 14) and
   the workspace `exclude` in its own commit (repository-level files,
   Rule 3.1). Fetch herdr's full history in a scratch clone (not the
   submodule) and, for every commit from cc88b3b8 to the pin, list the
   changes to `wire.rs`, `endpoint.rs` and the three outside types that
   touch a type reachable from `ClientMessage` or `ServerMessage`; record
   them in a new findings document, `docs/HERDR_WIRE.md` (append-only,
   Rule 3.6), with the owner's installed herdr version (`herdr --version`,
   run by the owner) and the commit it matches. Move the pin to that
   commit if it differs.
   **Done when:** `git submodule status` shows the pin, `docs/HERDR_WIRE.md`
   lists each gen-1 payload change since cc88b3b8 (or says there is
   none), and the pin is the owner's installed version.
2. **The crate and the extracted types.** `crates/libherdr_term` with
   `build.rs` per decision 2, `lib.rs` exposing `herdr`, and the
   `dump_herdr_types` example; added to the workspace in the same commit.
   Tests in `tests/wire.rs`: herdr's golden SHA-256 hashes for
   `PaneSurface` (`wire.rs:2560-2584`, `7c016f7b…`) and `PaneSurfacePatch`
   (`:2597-2631`, `0814b99a…`) built from the same values, so the
   extracted types encode byte for byte as herdr's; the `ClientMessage`
   and `ServerMessage` tags of `wire.rs:1957` and `:2720`; herdr's
   fixtures `tests/fixtures/endpoint-hello-v1.json`,
   `endpoint-welcome-v1.json` and `endpoint-snapshot-v1.json` from the
   submodule decode (`endpoint.rs:262-300`); the build fails cleanly with
   `HERDR_SRC` pointing at a copy where one kept type names `crate::foo`.
   **Done when:** `cargo test -p libherdr_term` passes these, and
   `cargo fmt`/`clippy -p libherdr_term --all-targets` are clean.
3. **Attaching and the surface.** `Attachment::connect`, the reader
   thread, patch application, `SurfaceView::pane`, `focus_pane`,
   `resize`, `detach` (decisions 3, 5, 6, 7). Tests against an in-process
   fake server on a socketpair that speaks the extracted types: the hello
   carries generation 1 and the four codecs; a refused welcome is
   `Incompatible` with its code; a full surface then two patches gives the
   expected cells and pane cursor; a patch on the wrong base closes;
   `focus_pane` sends the `pane.focus` request with the snapshot's
   `boot_id` and reports the answer; `Detach` is sent on drop.
   **Done when:** those tests pass, and on the owner's Mac (run by the
   owner, not by an agent), `cargo run -p libherdr_term --example attach --
   <pane_id> 80 24` prints the pane's rows and cursor, the desktop's tab
   resizes to 80x24 while it runs and returns to its size after Ctrl-C.
4. **Conversions.** Decision 8's `From` impls, with tests: every named,
   palette and RGB colour; each modifier bit; a wide glyph's skip cell; a
   hidden run; trailing blanks cut; a row of `CellData` equal to what
   `parse_ansi` makes of herdr's `visible_ansi` for the same content
   (fixture written by hand); the input conversions for every
   `TerminalMouseKind`, both wheel directions and each named key token.
   **Done when:** `cargo test -p libherdr_term` passes them.
5. **The cursor in libflankr_term.** Decision 9: `Cursor`, the appended
   fields, the trailer, `Mirror` and `Screen`; release 0.1.1 with its
   `VERSIONS.md` entry and tag. Tests: a frame without a trailer decodes
   as before; with one, the cursor round-trips; a cursor-only change makes
   a frame with no rows; the existing 1000-sequence round trip still ends
   equal with cursors on.
   **Done when:** `cargo test -p libflankr_term` passes, old frame bytes
   decode unchanged.
6. **The link protocol.** Decision 10 in `libflankr_proto`; release 0.4.2
   with its `VERSIONS.md` entry and tag. Tests: the appended fields default
   from older JSON; `TerminalMouseParams` round-trips each kind.
   **Done when:** `cargo test -p libflankr_proto` passes.
7. **The daemon, read side.** Decisions 4, 5, 11, 13 in
   `src/server/terminal.rs` and `conn.rs`: one attachment per connection,
   `pane.focus` on open and switch, the `tab_follows_focus` rule, the
   fallback, the cursor trailer when asked, `source` in the answer. Tests
   with the fake server of step 3 behind a test hook: opening a terminal
   attaches and focuses; a surface change sends a diff with the cursor; a
   second session on the same connection retargets the same attachment;
   closing the last terminal detaches; an incompatible fake falls back to
   the mock herdr poll and answers `source: Poll`; with
   `tab_follows_focus` off and the pane on another tab, no attachment is
   made.
   **Done when:** those tests pass and `cargo clippy --all-targets` on the
   daemon adds no warnings.
8. **The daemon, input side.** Decision 12: keys, wheel and mouse over the
   attachment, `terminal.resize`, the gates. Tests: each of the existing
   refusals still refuses; an armed `terminal.mouse` reaches the fake as
   `ClientShellPaneInput` with pane-local cells; `terminal.scroll` reaches
   it as a wheel event; on a polled terminal `terminal.mouse` is
   `NOT_PERMITTED` and `terminal.scroll` still uses `pane.scroll`.
   **Done when:** those tests pass.
9. **The companion crate and the app.** `flankr-companion` sends
   `cursor: true`, keeps the cursor, sends `terminal.resize` when the fit
   size changes and `terminal.mouse` for a tap while the pane reports mouse
   (`Cursor.mouse_reporting`), else the tap arms as today; the app draws
   the cursor (block, underline or bar by `shape`, hidden when not
   `visible` or `stale`).
   **Done when:** on the S23, a Claude Code session's terminal shows the
   cursor where Claude Code's prompt is, the desktop tab takes the phone's
   size while the view is open and returns when it is left, and in a
   mouse-reporting TUI (for example `htop`) a tap selects the row under
   the finger and a drag scrolls the TUI itself.

## 6. Risks

- **The tug of war is visible to the owner.** While the phone's terminal
  is open, the desktop's tab is at the phone's size; using the desktop
  takes it back and leaves the phone clipped until it is touched. This is
  herdr's design; the `stale` bit makes it visible on the phone.
- **Alone on the server, flankr resizes every tab** (2.1.4), not only the
  session's. Every agent's TUI redraws at the phone's size, and back when
  the phone detaches.
- **Gen-1 payloads may still change.** The contract freezes variant order
  and herdr's tests pin a few encodings; a field added to a payload struct
  would break decoding at a surface message. The audit of step 1 and the
  golden hashes of step 2 bound it; the fallback keeps the view working.
- **The build depends on the submodule.** A fresh checkout without
  `git submodule update --init vendor/herdr` does not build the daemon.
  Only the daemon depends on `libherdr_term`; the phone builds do not.
- **Global focus moves.** `pane.focus` changes herdr's active tab and
  focused pane for everyone, including the desktop, every time the phone
  opens or switches a terminal. With `tab_follows_focus` on this is
  wanted; it is still a side effect of reading.
- **Split tabs give small panes.** A pane in a three-way split gets a third
  of the phone's surface (open question 1).
- **Generated code licence.** The extracted types are herdr's Apache-2.0
  code compiled into flankr; the crate's README must carry the notice.

## 7. Open questions for the owner

1. Should flankr send `pane.zoom` after `pane.focus`, so the session's pane
   fills the phone's whole surface? Zoom is tab state herdr shares, so the
   desktop sees the pane zoomed too until it unzooms; without it a pane in
   a split gets its share of the phone's size.
2. When the desktop takes the tab back, should the phone (a) show the
   clipped view with the stale mark until touched (the plan), (b) re-claim
   on its own after some idle seconds, or (c) close the stream and fall
   back to the poll until touched?
3. If the installed herdr ever refuses generation 1, or a gen-1 payload
   changes under the pin, is the `pane.read` poll enough as the fallback,
   or should libherdr_term also speak herdr's documented read-only bridge,
   `herdr terminal session observe` (ANSI frames, no takeover, matched to
   the installed binary because it is the installed binary)?
4. Is `[terminal] attach = true` the right default, given the takeover, or
   should attaching be opt-in for the first release?
5. The pin: track the owner's installed herdr version (moved by hand, with
   step 1's audit, when herdr is upgraded), or the newest herdr release?


## Amendment — 2026-10-27 23:24 EDT: the terminal sized to the phone, keyboard included; herdr's private protocol only for the terminal

The owner (2026-10-27): "resize term to fit phone (including ime offset).
the json protocol should still be used for summary/overview, only use the
private protocol for terminal/mouse/wheel". Where this disagrees with the
body, it wins.

1. **The terminal fits the phone** (replaces decision 6's "the phone's
   `cols_hint` and `rows_hint` … without hints, 80 by 24"). The app works
   out the columns and rows that fit the terminal area at its current font
   size: the width of the view, and the height from below the header to
   the top of the bottom bar **less the soft keyboard's height while it is
   shown** (the IME inset libmorph reports). It sends them in
   `terminal.open` and again in `terminal.resize` whenever they change:
   the keyboard shown or hidden, a pinch that settles on a new size (sent
   once the pinch ends, not during it), rotation, and the cards over the
   terminal changing height. The daemon passes them on as
   `ClientShellResize` (`surface_size`), clamped as decision 6 says, at
   most twice a second. The phone's first frame after a resize is drawn at
   the new size without panning, since it fits; pan stays for a pane
   narrower than the surface would make it. With the poll (no attachment)
   the size cannot be set, and the view pans as today.
2. **herdr's private protocol only for the terminal.** The attachment of
   decisions 3-5 is opened only while a phone has a terminal open, and
   carries only that terminal: its surface, cursor and size, its keys,
   text and paste, mouse and wheel, and the `pane.focus` endpoint request
   that points the attachment's own view at the session's pane (2.1.7).
   Everything else stays on herdr's JSON API as it is: the snapshot, agent
   status and the event subscription, `pane.read` for transcripts and
   detection, the summaries and the overview, `tab_follows_focus` for the
   general agent's `focus_project`, and `send_keys`/`run_command` for
   sends. No JSON API use moves to the attachment.
3. **Open question 1 narrows.** The surface is herdr's whole tab, so the
   session's pane fills the phone only when it is alone in its tab. In a
   tab split into several panes, the pane gets its share of the phone-sized
   tab unless it is zoomed (`pane.zoom`, which the desktop would also
   see). The question stays for the owner: zoom the pane while the phone's
   terminal is open, or leave splits as they are.

The plan stays `Draft`.
