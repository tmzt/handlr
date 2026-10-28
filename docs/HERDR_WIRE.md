# herdr wire compatibility

This file records, for each update of the `vendor/herdr` pin, what changed in
herdr's client protocol since the previous pin and whether a `libherdr_term`
decoder built from the older pin still talks to the newer herdr (plan
`docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md`, step 1 and its
23:42 EDT amendment). It is append-only (AGENTS.md Rule 3.6): one section per
pin update, named by the herdr `PROTOCOL_VERSION` the pin moves to. Earlier
sections are never edited; a correction is a later section.

## 2026-10-27 23:50 EDT — Protocol version 22: pin 2713feb, audit since generation 1 (cc88b3b8)

### The pin

- Commit `2713feb56e9c9db33992db0a262c68f7426d46dd`, 2026-10-27 01:00
  +0300, "fix: install opencode and kilo integrations under
  XDG_CONFIG_HOME (#5585)"; herdr's HEAD on 2026-10-27.
- Crate version `0.9.5` (`Cargo.toml`).
- `PROTOCOL_VERSION = 22` (`src/protocol/wire.rs:20`).
- `ENDPOINT_PROTOCOL_GENERATION = 1` (`src/protocol/endpoint.rs:15`).

### Method

From a full-history clone of `https://github.com/herdrdev/herdr` (not the
shallow submodule), every commit in `cc88b3b8..2713feb` that touches
`src/protocol/wire.rs`, `src/protocol/endpoint.rs`,
`src/protocol/surface_delta.rs`, `src/input/model.rs`,
`src/api/schema/common.rs` or `src/config/model.rs` was read. For
`wire.rs` and `endpoint.rs`, every `struct` and `enum` deriving
`Serialize`/`Deserialize` (plus `const` and `type` items), with derive lines
and comments removed and field-level serde attributes kept, was extracted at
each of those commits and diffed against the previous one. The three outside
types were compared at `cc88b3b8` and `2713feb` and through each commit that
touches their files.

Reachability: a type counts as on the wire if `ClientMessage` or
`ServerMessage` reaches it through bincode (a variant payload or a field), or
if it is JSON carried inside `EndpointControl { kind, data }` (hello,
welcome, snapshot, agent view, agent completions).

### PROTOCOL_VERSION and generation history

`PROTOCOL_VERSION` (guards only the same-install direct-terminal path,
`TerminalHello`, by exact match):

| Version | Date | Commit |
|---|---|---|
| 1 | 2026-04-21 | `3397e1ce` add persistent server-client sessions |
| 2 | 2026-04-26 | `e9547a8e` |
| 3 | 2026-05-07 | `b0aa7b2c` |
| 4 | 2026-05-08 | `793f127e` |
| 5 | 2026-05-13 | `46003090` |
| 6 | 2026-05-15 | `73dba975` |
| 7 | 2026-05-19 | `e7f94a3f` |
| 8 | 2026-05-20 | `ed478be9` |
| 9 | 2026-05-21 | `06bcd06a` |
| 10 | 2026-05-23 | `9817820a` |
| 11 | 2026-05-24 | `420925d1` |
| 12 | 2026-05-30 | `fc4e1bbf` |
| 13 | 2026-06-06 | `4001fd2e` |
| 14 | 2026-06-13 | `d35c642e` |
| 15 | 2026-06-29 | `8b74fb8a` |
| 16 | 2026-07-05 | `2bc1724c` |
| 17 | 2026-07-17 | `e0758c32` |
| 18 | 2026-07-23 | `e7fc85bf` |
| 19 | 2026-08-01 | `b76adc15` |
| 20 | 2026-08-08 | `6f311498` |
| 21 | 2026-08-25 | `d79fd746` |
| 22 | 2026-09-01 | `99c23cd1` fix(windows): preserve native pane key identity |

No bump since `99c23cd1`, although `wire.rs` changed 8 times after it:
`cc88b3b8` itself and the 7 commits in the table below.

`ENDPOINT_PROTOCOL_GENERATION = 1` since `cc88b3b8` (2026-09-02, "feat: add
stable client endpoint compatibility (#3509)"), unchanged through the pin.
`cc88b3b8` appended `EndpointControl { kind: String, data: String }` as the
last variant of both `ClientMessage` (tag 20) and `ServerMessage` (tag 20),
added `#[serde(other)] Unknown` to one enum and the
`deserialize_client_shell_agent_status` fallback on the three `AgentStatus`
fields, all **without** bumping `PROTOCOL_VERSION`. So herdr at `99c23cd1`
is also protocol 22 but has no `EndpointControl` variant: it cannot decode
the generation-1 hello (tag 20 is past its last `ClientMessage` variant).
`PROTOCOL_VERSION = 22` therefore does not mean "speaks generation 1"; only
a welcome answering the hello does.

### Changes since cc88b3b8

| Date | Commit | Subject | File: type and change | Wire effect |
|---|---|---|---|---|
| 2026-09-03 | `2ae8b91c` | fix(windows): preserve dead-key composition in kitty panes (#3548) | `wire.rs`: `impl ClientPaneInputEvent` only (`to_raw_input_event_with_windows_source`) and tests. `input/model.rs`: private field on `TerminalKey` (not a wire type); `WindowsKeyRecord` unchanged | None. Server-side interpretation of `windows_record` only |
| 2026-09-04 | `a2b12897` | feat: enable kitty graphics by default | `config/model.rs`: config defaults; `ToastHerdrPosition` unchanged | None |
| 2026-09-06 | `9e9bc8a1` | feat: manage multiple ssh machines from one client (#3670) | `endpoint.rs`: `EndpointClientHello.surface_active: bool` added, `#[serde(default = "default_true")]`; `EndpointServerWelcome.capabilities: Vec<String>` added, `#[serde(default)]`; new consts for the surface-interest, presentation-fence and health-check capabilities and their control kinds. New client-lane method `client_shell.surface.set` (`tests/fixtures/endpoint-method-shapes-v1.json`). `common.rs`: `AgentStatus` unchanged | None in bincode. Hello and welcome are JSON; both new fields default, and neither struct has `deny_unknown_fields`, so old and new peers read each other's JSON. Behaviour: herdr before this commit ignores `surface_active` (every shell is active) and does not offer `client_shell.surface.set` |
| 2026-09-07 | `48281813` | feat: make ui.pane_borders a three-state auto/always/off mode (#3234) | `config/model.rs`: config only | None |
| 2026-09-12 | `9ad65d90` | fix: suppress Windows dead-key fallback characters in remote panes (#3972) | `wire.rs`: tests only. `input/model.rs`: `TerminalKey` private field renamed; `WindowsKeyRecord` unchanged | None |
| 2026-09-13 | `fc1cb77f` | feat: add Letta Code detection and restore (#3107) | `config/model.rs`: config only | None |
| 2026-09-14 | `3b478e69` | fix: scope agent views to the selected machine (#3784) | `endpoint.rs`: new JSON struct `EndpointAgentViewProjection { boot_id, revision, view: Option<Value> }`, sent as `EndpointControl` kind `endpoint.agent-view.v1`, behind capability `agent_view_projection` | None. A new named control; unknown kinds are ignored (`endpoint.rs:8`) |
| 2026-09-14 | `d8b36691` | fix: avoid resending terminal cells for metadata updates (#4137) | `wire.rs`: `CellData` loses derived `Clone` for a hand-written `impl Clone` (fields unchanged). `endpoint.rs`: `EndpointClientHello.surface_reuse: bool`, `#[serde(default)]`. New `surface_reuse.rs` encoding as `EndpointControl` kind `endpoint.surface-reuse.v1` | None. Optional encoding, sent only when the hello sets `surface_reuse`; flankr sends `false` |
| 2026-09-16 | `18061191` | fix: keep remote surface updates incremental (#4209) | `wire.rs`: `bincode::Decode` added to the derives of `CellData`, `CursorState`, `PaneSurfacePane`, `PaneSurfaceScrollMetrics`, `PaneSurfaceSplitDirection`, `SurfaceRect`, the `SurfaceGraphics*` key/target/source/format/placement types, `ClientShellPopupSize` (fields unchanged). `endpoint.rs`: `EndpointClientHello.surface_delta: bool`, `#[serde(default)]`. `surface_delta.rs` added: its own frozen binary layout, sent as `EndpointControl` kind `endpoint.surface-delta.v1` | None. Derives do not change the serde encoding; the delta encoding is opt-in and flankr sends `false` |
| 2026-09-19 | `cc7c696c` | feat: add configurable pane screen and scrollback clearing (#4383) | `config/model.rs`: config only | None |
| 2026-09-21 | `28360107` | fix: distinguish agent completion from startup and session changes (#4457) | `endpoint.rs`: new JSON struct `EndpointAgentCompletions { boot_id, revision, completions: BTreeMap<String, u64> }`, `EndpointControl` kind `endpoint.agent-completions.v1`, capability `agent_completions` | None. A new named control, ignorable |
| 2026-09-22 | `46a50918` | fix: stagger restored agent startups (#4487) | `config/model.rs`: config only | None |
| 2026-09-24 | `c411883e` | feat!: replace the custom graphics api with optimized native kitty rendering (#4561) | `wire.rs`: `SurfaceGraphicsAsset.data: Vec<u8>` gains `#[serde(serialize_with = "serialize_graphics_bytes", deserialize_with = "deserialize_graphics_bytes")]`, which uses `serialize_bytes` for bincode. The breaking part (`feat!`) is the removal of the JSON socket API's graphics methods (`src/api/schema/*`), not the client wire | None. Bincode's byte slice is the same length prefix plus bytes as `Vec<u8>`; herdr's test `graphics_bulk_codec_preserves_legacy_wire_and_json` asserts old and new encode identically and decode each other. `SurfaceGraphicsAsset` is reached from every `PaneSurfaceFrame` through `graphics: SurfaceGraphicsScene { assets, .. }` |
| 2026-09-28 | `80c0c072` | perf: send scrolling pane output as row shifts and skip blank cells (#4711) | `endpoint.rs`: `EndpointClientHello.surface_scroll: bool`, `#[serde(default)]`. `surface_delta.rs` and new `surface_scroll.rs`: opt-in encodings as `EndpointControl` kinds `endpoint.surface-delta.v1` and `endpoint.surface-scroll.v1` | None. Opt-in; flankr sends `false` |
| 2026-09-28 | `7f89b11a` | feat: support multiple prefix keys (#4653) | `config/model.rs`: config only | None |
| 2026-09-28 | `d5680d84` | fix: capture Windows host default colors at startup (#4710) | `wire.rs`: variant `HostDefaultColor { kind: ClientHostDefaultColorKind, color: ClientHostColor }` appended at the end of `ClientInputEvent` (both payload types existed before) | None for flankr. `ClientInputEvent` is not reached from `ClientMessage` or `ServerMessage` at `cc88b3b8` or at the pin (the shell sends `ClientPaneInputEvent` in `ClientShellPaneInput`). Even if it were, an appended variant only breaks an older peer that receives it, and flankr never sends it |
| 2026-10-05 | `49848eff` | fix: require opt-in for cross-elevation Windows access (#4779) | `config/model.rs`: config only | None |
| 2026-10-07 | `a124eed7` | refactor: route all pane key encoding through libghostty (#5017) | `wire.rs`: dead `impl` function `from_crossterm` removed; tests. `input/model.rs`: `impl` code; `WindowsKeyRecord` unchanged | None |
| 2026-10-08 | `abf0d07c` | fix: map non-latin layout ctrl chords to their base keys (#4961) | `input/model.rs`: `impl` code; `WindowsKeyRecord` unchanged | None |
| 2026-10-13 | `241b99cd` | refactor: share cli and api command contracts (#5212) | `common.rs`: moved code; `AgentStatus` unchanged (`Idle, Working, Blocked, Done, Unknown`, `snake_case`) | None |

The three outside types are identical at `cc88b3b8` and `2713feb`:
`WindowsKeyRecord { key_down: bool, repeat_count: u16, virtual_key_code:
u16, virtual_scan_code: u16, unicode: u16, control_key_state: u32 }`
(`src/input/model.rs:6-14`), `AgentStatus` (`src/api/schema/common.rs:245-253`,
was `:152-160`), `ToastHerdrPosition { TopLeft, TopRight, BottomLeft,
BottomRight }` (`src/config/model.rs:71-81`).

No field of any bincode-encoded type reachable from `ClientMessage` or
`ServerMessage` was added, removed, reordered or retyped, and no variant was
added to, removed from or reordered in either enum or any enum under them.
The only variant added anywhere (`ClientInputEvent::HostDefaultColor`) is
outside the reachable set.

### herdr's own pins on the encoding

The test module of `wire.rs` (from `:1774` at the pin) hashes bincode
(`bincode::config::standard()`) encodings with SHA-256 (`encoded_sha256`,
`:1858`). All nine hashes are the same at `cc88b3b8` and `2713feb`:

| Test (line at pin) | SHA-256 |
|---|---|
| `client_shell_resize_roundtrip` (`:1928`) | `676d6376202750e72c45ff511e256b6154d3d20c0ee088d3792fa3a69d9704b9` |
| `client_shell_pane_input_roundtrips_semantic_and_windows_keys` (`:2116`) | `f558384bb53dfd2baf1fa72e1709d88be6891da79e51f88513905afc085065e6` |
| `client_shell_endpoint_messages_roundtrip` (`:2331`), request | `de5693585a01f6b0d5ee07c51b6ddf79ee9f67dbf183255822d31f35210f5ffb` |
| `client_shell_endpoint_messages_roundtrip`, response | `bc14dbb5263d3097fe6d3e70a4b6d71aa9c2fa4ae3206d692a7182512bffdd1d` |
| `client_clipboard_image_roundtrip` (`:2366`) | `1c02110be0671faf318b3f4d8f507748d5a0a98cd474832669c5290d97ef19ab` |
| `server_frame_roundtrip_nontrivial` (`:2503`, hash at `:2583`), `PaneSurface` | `7c016f7b21ddb5ac79212cf65a968b93eb292b5305b941263e89ffaa40158ee3` |
| `pane_surface_patch_roundtrip` (`:2598`, hash at `:2630`), `PaneSurfacePatch` | `0814b99a1dc6eaf7918424aa416c066509cbfb73b72344a809c27cde78cb6dbd` |
| `client_shell_graphics_payload_codec_is_frozen` (`:2635`) | `49c4efec0f1456c8ca4112ddf6ead1ab75d0224007576c2ccc18c3fca55a69f0` |
| `server_graphics_roundtrip` (`:2941`) | `28a420f92e0e05e6760a8c140baf307c360c6d1b1aa68027481b324f87e22c44` |

The tag tests `client_message_wire_tags_reflect_current_order` (`:1957`)
and `client_shell_server_message_tags_are_frozen` (`:2720`) are unchanged
since `cc88b3b8`. Tags of the messages flankr uses: `ClientMessage`
`Detach` 4, `ClientShellResize` 12, `ClientShellPaneInput` 13,
`ClientShellEndpointRequest` 15, `ClientShellMouseCapture` 19,
`EndpointControl` 20; `ServerMessage` `PaneSurface` 13,
`ClientShellEndpointResponseChunk` 18, `PaneSurfacePatch` 19,
`EndpointControl` 20. herdr's `AGENTS.md:92` at the pin calls these
digests, the tag tests and `tests/fixtures/endpoint-method-shapes-v1.json`
compatibility contracts that are never re-blessed for a wire change. The
endpoint fixtures `endpoint-hello-v1.json`, `endpoint-welcome-v1.json` and
`endpoint-snapshot-v1.json` are unchanged since `cc88b3b8`; the method-shape
fixture only gained the `client_shell.surface.set` entry (`9e9bc8a1`), and
the `pane.focus` and `tab.focus` shapes are unchanged.

### Verdict

A decoder built at `2713feb` is compatible with every herdr from `cc88b3b8`
(2026-09-02) to `2713feb` (2026-10-27) for the messages flankr uses: the
generation-1 hello and welcome, the snapshot, `PaneSurface` and
`PaneSurfacePatch`, `ClientShellEndpointRequest` and
`ClientShellEndpointResponseChunk`, `ClientShellPaneInput`,
`ClientShellResize`, `Detach` and `ClientShellMouseCapture` /
`MouseCapture`. Every bincode type reachable from `ClientMessage` and
`ServerMessage` encodes byte for byte the same across the range, and every
JSON change in the hello and welcome is a field with a serde default.

Limits inside that range:

- herdr from `cc88b3b8` up to the commit before `9e9bc8a1` (2026-09-02 to
  2026-09-05) ignores `surface_active` and has no `client_shell.surface.set`
  method, and its welcome has no `capabilities`. Detaching uses `Detach`,
  which works throughout; `client_shell.surface.set` is called only when the
  welcome's `methods` lists it.
- The optional encodings (`surface_reuse`, `surface_delta`,
  `surface_scroll`) and the named controls added in the range (agent view,
  agent completions, presentation sync, health) are not requested by flankr
  and are ignored if seen.

Outside the range: herdr before `cc88b3b8`, including `99c23cd1` (also
`PROTOCOL_VERSION = 22`), has no `EndpointControl` and cannot decode the
hello, so the attachment fails at connect and the daemon uses the poll.
Newer herdr is covered only by a later section of this file.
