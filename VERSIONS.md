# VERSIONS

Each change of the crate version and the public API changes it carries
(AGENTS.md, Rule 5). Internal changes are not listed, and there is no entry
without a version change. Entries are appended, oldest first, and never
edited.

## 0.1.0 — 2026-08-31 (`a53c2a2`, tag `v0.1.0`)

Initial version. Recorded retroactively on 2026-10-22 from the commit; the
tag was added on 2026-10-22.

- `handlr` is an application: the voice CLI binary. Its library target exists
  for the binary and tests, and has no public API under Rule 5.5.
- The workspace crates `libherdr` (`88625e2`) and `libinfer` (`2b85d0a`)
  were introduced at 0.1.0 as path dependencies of handlr, not separately
  released or tagged.

## libherdr 0.2.0 — 2026-10-23 (the commit tagged `libherdr-v0.2.0`)

The first separately numbered version of the workspace crate `libherdr`
(plan 20261022_03, the 16:23 EDT amendment). It is still a path dependency
of handlr, not published. Its tags carry the crate's name, so they are not
confused with handlr's own `v<version>` tags.

- **Breaking:** `PaneInfo` gains `agent_session: Option<AgentSession>`,
  taken out of `extra`. `PaneInfo` is an exhaustive struct.
- **Breaking:** `HerdrRequest::InspectPane` gains `source: ReadSource`. It
  is an exhaustive enum variant.
- **Breaking:** `MockState` gains `read_log`. It is a public struct.
- **Breaking, in behaviour:** `pane.read` now sends `lines`, herdr's own
  parameter name, as well as the old `count`. herdr therefore honours the
  requested line count, which it did not before.
- **Added:**
  - `AgentSession` (`source`, `agent`, `kind`, `value`, and `id()`);
  - `ReadSource` (`visible`, `recent`, `recent_unwrapped`, `detection`);
  - `HerdrRequest::inspect_pane_from` and
    `HerdrClient::inspect_pane_from(pane_id, lines, source)`.

## libhandlr_proto 0.1.1 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.1.1`)

The first separately numbered version of the workspace crate
`libhandlr_proto`, which was 0.1.0 from its first commit. It is a path
dependency of handlr and handlr-companion, not published. Its tags carry the
crate's name, as libherdr's do. Strictly additive (plan 20261023_05,
section 4, as amended at 15:45 EDT):

- **Added:**
  - `common::methods::SPEECH_SUMMARIZE` (`"speech.summarize"`);
  - `common::SpeechSummarizeParams` (`pane_id`, `ts`, `text`) and
    `common::SpeechSummarizeResult` (`spoken: Option<String>`, `complex`);
  - `envelope::codes::UNAVAILABLE` (-32015). `codes::ALL` is unchanged, so
    its type stays `[i64; 15]`.

## libhandlr_proto 0.1.2 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.1.2`)

Strictly additive (plan 20261023_05, the 16:28 EDT amendment, A4 and A6):

- **Added:**
  - `common::methods::CLIENT_OPTIONS` (`"client.options"`);
  - `common::ClientOptionsParams` (`spoken_output`), whose unknown fields
    are ignored.

## libhandlr_proto 0.2.0 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.2.0`)

Not strictly additive (plan 20261023_05, the 16:56 EDT amendment, "both
versions in the answer"):

- **Breaking:** `common::SpeechSummarizeResult` loses `spoken` and gains
  `read` (`String`, always present: the reply under the reading rules)
  and `summary` (`Option<String>`: Gemini's, when one was made).

## libhandlr_proto 0.3.0 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.3.0`)

Not strictly additive (plan 20261023_04, the 17:04 EDT amendment, A4):

- **Breaking:** `common::AgentInfo` gains `turn: Option<Turn>`, left out
  of the wire when absent and read as absent when missing. `AgentInfo` is
  an exhaustive struct.
- **Added:**
  - `common::Turn`, `Working { line }`, `Stopped { line }` or
    `Asking { question }`, an object tagged by `state` on the wire
    (`{"state": "stopped", "line": "Done."}`);
  - `common::TURN_TEXT_MAX` (200), the longest text a `Turn` carries.

## libhandlr_proto 0.3.1 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.3.1`)

Additive, by the owner's ruling (2026-10-23: "I would consider it
additive, just patch level"; Rule 5.4 would count a field added to a
public struct as not strictly additive):

- **Added:** `common::ClientOptionsParams::background` (`bool`, defaulted
  on the wire): the companion's link sleeps, and the daemon sends it only
  what needs the user.

## libhandlr_proto 0.3.2 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.3.2`)

Additive:

- **Added:** `common::PingParams` (`active: Option<bool>`), `ping`'s params:
  the companion says whether the app is in front.

## libhandlr_proto 0.3.3 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.3.3`)

Additive, by the owner's ruling ("appending to a struct is additive"):

- **Added:**
  - `common::Link` (`text`, `url`): a repowiki link for a `docs/…` path;
  - `common::Entry::links` (`Vec<Link>`, defaulted and left out of the
    wire when empty);
  - `common::PendingQuestion::about_plan` (`bool`), `plan`
    (`Option<String>`, the plan's spoken name) and `links`
    (`Vec<Link>`), each defaulted and left out of the wire when empty:
    the companion offers approve, improve and implement for a question
    about a plan (plan 20261022_03, the 21:57 EDT amendment).

## libhandlr_proto 0.3.4 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.3.4`)

Additive, by the owner's ruling ("appending to a struct is additive"):

- **Added:** `common::Entry::origin` (`Option<String>`, defaulted and left
  out of the wire when absent): an entry that is context rather than the
  user's or the agent's own words, and where it is from: `"task"`,
  `"peer:<name>"`, `"agent:<id>"`, `"recap"` or `"compact"`.

## libherdr 0.3.0 — 2026-10-23 (the commit tagged `libherdr-v0.3.0`)

Not strictly additive (plan 20261023_03, step 1: herdr's focus and pane
lifecycle, pushed):

- **Breaking:** `events::Subscription` gains `PaneFocused`, `TabFocused`,
  `PaneUpdated` and `PaneClosed` (`pane.focused`, `tab.focused`,
  `pane.updated`, `pane.closed`, with no fields). `Subscription` is an
  exhaustive enum.
- **Breaking:** `Subscription::pane_id` returns `Option<&str>`: `None`
  for the four new subscriptions, which herdr does not scope to a pane.
- **Breaking:** `events::HerdrEvent` gains `PaneFocused { pane_id,
  workspace_id }`, `TabFocused { tab_id, workspace_id }`,
  `PaneUpdated(Box<PaneInfo>)` and `PaneClosed { pane_id, workspace_id }`,
  decoded from `pane_focused`, `tab_focused`, `pane_updated` and
  `pane_closed` (or the dotted spellings). `HerdrEvent` is an exhaustive
  enum; these events were `Other` before.
- **Added:** `Subscription::focus_and_panes()`, the four as an array.

## libhandlr_proto 0.3.5 — 2026-10-23 (the commit tagged `libhandlr_proto-v0.3.5`)

Additive, by the owner's rulings ("appending to a struct is additive";
"same with open enums, not closed"): new methods, new types, and two
new enums that are open (plan 20261023_03, step 2, as amended at 14:54
and 22:55 EDT):

- **Added:**
  - `common::methods::SESSION_SUBSCRIBE`, `SESSION_UNSUBSCRIBE`,
    `SESSION_HISTORY` and the notification `SESSION_DELIVERY`;
  - `common::Serial` (`file`, `line`, `slot`), with `new`, `file_hash`
    (FNV-1a), `to_u64`, `from_u64`, `same_file`, `PartialOrd` (`None`
    across files), `Display` and `FromStr` (`"3fa9c21e:412.2"`), and
    serde as that string; `common::ParseSerialError`;
    `common::SERIAL_LINE_MAX`;
  - `common::Section` (`serial`, `kind`, `entries`, `card`),
    `common::CardBody` (`card`, `live`, `title`, `text`, `confirm_id`,
    `about_plan`, `links`);
  - `common::SectionKind` (`turn`, `text`, `tool`, `card`, `context`)
    and `common::CardKind` (`approval`, `question`, `broke`, `finished`),
    both `#[non_exhaustive]` with an `Unknown` variant that any other
    kind reads as;
  - `common::SessionSubscribeParams`, `SessionSubscribeResult`,
    `SessionDelivery` (with `removed`, the serials of sections to drop),
    `SessionRef`, `SessionUnsubscribeParams`, `SessionHistoryParams`,
    `SessionHistoryResult`, and `common::delivery_reasons`;
  - `common::SUBSCRIBE_SECTIONS_DEFAULT` (50), `SECTIONS_MAX` (200),
    `HISTORY_SECTIONS_DEFAULT` (100), `SUBSCRIPTIONS_MAX` (8);
  - `envelope::codes::TOO_MANY_SUBSCRIPTIONS` (-32012),
    `SUBSCRIPTION_NOT_FOUND` (-32013) and `SESSION_CHANGED` (-32014).
    `codes::ALL` is unchanged, so its type stays `[i64; 15]`.

## libhandlr_proto 0.3.6 — 2026-10-24 (the commit tagged `libhandlr_proto-v0.3.6`)

Additive, by the owner's rulings ("appending to a struct is additive";
new items are additive): appended struct fields, all serde-defaulted so
that messages without them read as before, and new types, constants and
a method (plan 20261024_01, step 1, as amended at 00:03 EDT):

- **Added:**
  - `common::Section::index` (default 0, left out of the wire when 0);
  - `common::Slot` (`index`, `serial`, `kind`, `lines`, `chars`, `rev`);
  - `common::SLOTS_MAX` (2000), `LIVE_INLINE` (8) and
    `CONTENT_LIMIT_DEFAULT` (50);
  - `common::SessionSubscribeParams::outline` and `from`;
  - `common::SessionDelivery::slots`, `removed_index`, `first` and
    `last`;
  - `common::methods::SESSION_CONTENT` (`session.content`), with
    `common::SessionContentParams` (`subscription_id`, `from`, `to`,
    `limit`, `slots_only`) and `common::SessionContentResult`
    (`sections`, `slots`, `removed_index`, `first`, `last`, `more`).
- `envelope::codes::SESSION_CHANGED` (-32014), already there since
  0.3.5, is also what `session.content` answers for a file the
  subscription has moved off; its value is unchanged.

## libhandlr_proto 0.3.7 — 2026-10-24 (the commit tagged `libhandlr_proto-v0.3.7`)

Additive, by the owner's ruling ("appending to a struct is additive"):
two appended, serde-defaulted fields (plan 20261024_01, the CHANGES.md
departure of 2026-10-24 on the daemon epoch):

- **Added:**
  - `common::SessionSubscribeParams::epoch`, the epoch of the outline a
    resuming client holds;
  - `common::SessionDelivery::epoch`, the epoch an outline delivery's
    indexes are of.
- **Changed meaning, in outline mode only:** `SessionDelivery::reset`
  drops only the outline; contents stay valid by serial. Outline mode
  came in 0.3.6 and has no client yet.

## libhandlr_proto 0.3.8 — 2026-10-24 (the commit tagged `libhandlr_proto-v0.3.8`)

Additive, by the owner's rulings ("appending to a struct is additive", and
plan 20261024_02, decisions 6 and 7: "a patch release"): a new method and
an appended, serde-defaulted field (plan 20261024_02, step 2):

- **Added:**
  - `common::methods::LINK_UNPAIR` (`link.unpair`), with
    `common::LinkUnpairParams` and `common::LinkUnpairResult`, both `{}`:
    over a paired link, the daemon forgets the calling device and closes
    the link;
  - `common::ClientOptionsParams::general_agent`, true when left out:
    while false, the daemon starts no Gemini turn for the client.
- **Changed:** `ClientOptionsParams::default()` has `general_agent: true`;
  the other options still default to off.

## libhandlr_proto 0.3.9 — 2026-10-24 (the commit tagged `libhandlr_proto-v0.3.9`)

Additive, by the owner's rulings ("appending to a struct is additive", and
the CHANGES.md entries of 2026-10-24 12:06 EDT: "a patch release"): four
appended, serde-defaulted fields and a constant (plan 20261022_03, the L3
amendments of 2026-10-24 11:54, 12:06 and 12:27 EDT):

- **Added:**
  - `common::Entry::text`, the entry's full redacted text, up to the
    reader's limits; `summary` stays the 500-character form;
  - `common::CardBody::points`, a point-by-point summary of a Finished
    card's reply, at most `common::CARD_POINTS_MAX` (6) lines;
  - `common::CardBody::spoken`, the text to read aloud for the card;
  - `common::Slot::bookend`, true on a Finished card's outline slot, for
    the scrollbar's section labels.

## libflankr_proto 0.3.10 — 2026-10-27 (the commit tagged `libflankr_proto-v0.3.10`)

The first release under the crate's new name (plan 20261025_01); 0.3.9 was
libhandlr_proto's. Additive, by the owner's rulings ("appending to a struct
is additive", and plan 20261027_02, decision 7: "libflankr_proto gets a
patch release"): two new types, an appended, serde-defaulted field, a
notification and an error code (plan 20261027_02, step 3, with the
amendment of 00:59 EDT naming them `llm`):

- **Added:**
  - `companion::LlmInfo { general_agent, summaries, reason }`, what the
    daemon's `[llm]` makes available, with `LlmInfo::available()`;
  - `companion::HelloResult::llm`, `None` from an older daemon, which
    means both available;
  - `companion::methods::SERVER_CAPABILITIES` (`server.capabilities`),
    daemon to phone, with `companion::ServerCapabilities { llm }`, sent
    whenever what the daemon offers changes;
  - `envelope::codes::LLM_UNAVAILABLE` (-32016): a turn refused because
    the general agent is not available, its message the reason.
    `codes::ALL` is unchanged.

## libflankr_proto 0.3.11 — 2026-10-27 (the commit tagged `libflankr_proto-v0.3.11`)

Additive, by the owner's ruling ("appending to a struct is additive"): two
appended, serde-defaulted fields (plan 20261027_04, step 3, with the
amendments of 11:20 EDT and the settings plan's of 11:27 EDT):

- **Added:**
  - `companion::TurnStartParams::session: Option<String>`, the pane id of
    the session the phone has open, so it is the turn's current agent;
    `None` from the overview and from older apps;
  - `common::ClientOptionsParams::tab_follows_focus: bool`, off when left
    out: when on, the general agent's `focus_project` also moves herdr's
    focused tab.

## libflankr_proto 0.3.12 — 2026-10-27 (the commit tagged `libflankr_proto-v0.3.12`)

Additive, by the owner's ruling ("appending to a struct is additive"): one
appended, serde-defaulted field (plan 20261023_02, the 2026-10-27 13:25 EDT
amendment):

- **Added:** `common::AgentsSendParams::confirmed: bool`, off when left out
  and not written when off: the user approved the send in the app already,
  so a companion that may approve sends it without a confirmation.

## libflankr_proto 0.3.13 — 2026-10-27 (the commit tagged `libflankr_proto-v0.3.13`)

Additive, by the owner's ruling ("appending to a struct is additive", and
plan 20261027_06, decision 5: "a patch release"): one new type and one
appended, serde-defaulted field (plan 20261027_06, step 2):

- **Added:**
  - `common::Answer { label, style, sends, starts, command, answers }`, one
    suggested answer to a pending question, its texts filled by the
    daemon's `SUGGESTIONS.md` rules;
  - `common::PendingQuestion::answers: Vec<Answer>`, empty when left out
    and not written when empty: the answers to offer, in order; empty from
    an older daemon.
- **Changed (documentation only):** `PendingQuestion::about_plan` and
  `PendingQuestion::plan` now say what the daemon fills them with (the
  `plan` rule matched, and the plan as its answers name it).

## libflankr_proto 0.4.0 — 2026-10-27 (the commit tagged `libflankr_proto-v0.4.0`)

A minor bump by the owner's ruling for this change only (plan 20261027_07,
the 20:44 EDT amendment), although every change below is additive: the
terminal mode's protocol (plan 20261027_07, step 2, with the 14:23 EDT
amendment's split flag byte). `companion::VERSION` stays 1 and the link's
prologue `flankr.link/1`.

- **Added:**
  - `link`: the flag byte is two nibbles, flags low (bit 0 more, bits 1-3
    reserved) and kind high (0 JSON-RPC, 1 terminal frame, 2-14 reserved,
    15 extended); `0x00`/`0x01` are unchanged. `KIND_JSON`, `KIND_TERMINAL`,
    `KIND_EXTENDED`; `Kind { Json, Terminal }` and `Message { Json(Vec<u8>),
    Terminal(Vec<u8>) }`, both non-exhaustive; `LinkStream::recv`,
    `send_terminal` and `send_kind`; `LinkReader::recv`;
    `LinkWriter::send_terminal` and `send_kind`. `recv` is `BadFrame` on a
    reserved flag bit, a reserved or extended kind, or a kind that changes
    mid-message.
  - `common::methods::TERMINAL_OPEN`, `TERMINAL_CLOSE`, `TERMINAL_ACK`,
    `TERMINAL_INPUT_ARM`, `TERMINAL_INPUT`, `TERMINAL_CLOSED`, and their
    types `TerminalOpenParams { pane_id, cols_hint, rows_hint }`,
    `TerminalOpenResult { terminal_id, cols, rows }`,
    `TerminalCloseParams`, `TerminalAckParams { terminal_id, seq }`,
    `TerminalInputArmParams`, `TerminalInputArmResult { expires_in_s }`,
    `TerminalInputParams { terminal_id, keys }`, `TerminalClosedParams
    { terminal_id, reason }` and the open `TerminalClosedReason { PaneClosed,
    AgentChanged, InputRevoked, Closed, Unknown }`; `TERMINAL_INPUT_MAX`
    (64) and `TERMINAL_INPUT_ARM_SECS` (60).
  - `companion::TerminalInfo { input }`, and `terminal: Option<TerminalInfo>`
    appended to `companion::HelloResult` and `companion::ServerCapabilities`,
    `None` when left out and not written when `None`.
- **Unchanged:** `recv_message` reads kind 0 only and is `BadFrame` on any
  other flag byte, exactly as before.

## libherdr 0.3.1 — 2026-10-27 (the commit tagged `libherdr-v0.3.1`)

Additive, by the owner's ruling ("appending to a struct is additive") and
plan 20261027_07's 20:44 EDT amendment ("a patch"): one appended,
serde-defaulted field on a request variant, and two new functions (plan
20261027_07, the 14:23 EDT amendment's ANSI read):

- **Added:**
  - `ReadFormat { Text, Ansi }` (default `Text`, serialised `"text"` and
    `"ansi"`), re-exported from the crate root;
  - `HerdrRequest::InspectPane::format: ReadFormat`, `Text` when left out
    and not sent when `Text`, so existing `pane.read` requests are
    byte-identical;
  - `HerdrRequest::inspect_pane_as(pane_id, lines, source, format)` and
    `HerdrClient::inspect_pane_as(pane_id, lines, source, format)`.

## libflankr_proto 0.4.1 — 2026-10-27 (the commit tagged `libflankr_proto-v0.4.1`)

Additive (a new method and its types; plan 20261022_03's L3 amendment on
the terminal view's drag):

- **Added:** `common::methods::TERMINAL_SCROLL` (`terminal.scroll`),
  `TerminalScrollParams { terminal_id, lines }` (positive back into the
  scrollback) and `TerminalScrollResult { offset_from_bottom,
  max_offset_from_bottom }`, the pane scrolled as a mouse wheel does, gated
  as `terminal.input` is.

## libflankr_term 0.1.1 — 2026-10-28 (the commit tagged `libflankr_term-v0.1.1`)

This is the crate's first entry: 0.1.0, its first release (plan
20261027_07, step 1), had none.

Additive (a new type and appended fields, by the owner's ruling that
appending to a struct is additive; plan 20261027_08, decision 9):

- **Added:** `Cursor { col, row, visible, shape, mouse_reporting,
  alternate_screen, stale }`.
- **Added:** `Grid::cursor: Option<Cursor>` (appended, `serde(default)`).
  `Grid::from_lines`, and so `parse_ansi`, give `None`.
- **Added:** `Frame::cursor: Option<Cursor>` (appended, `serde(default)`),
  written by `Frame::encode` as a 7-byte trailer after the rows when
  `Some` (`u8 1, u16 col, u16 row, u8 bits, u8 shape`; bits 0-3 visible,
  mouse reporting, alternate screen, stale; bits 4-7 zero) and read by
  `Frame::decode` when present. A frame without a cursor encodes byte for
  byte as in 0.1.0, and 0.1.0 bytes decode unchanged; any other trailing
  bytes are still `DecodeError::TrailingBytes`.
- **Added:** `Frame::encode_with(cursor: bool)` and
  `Frame::encoded_len_with(cursor: bool)`: `encode_with(false)` leaves the
  trailer out, for a phone that did not ask for it. `CURSOR_TRAILER_LEN`
  (7).
- **Added:** `Screen::cursor()`, the newest frame's cursor.
- `Mirror` carries a grid's cursor in every frame made from it, and a
  grid that differs from the last only in its cursor makes a frame with no
  rows.

## libflankr_proto 0.4.2 — 2026-10-28 (the commit tagged `libflankr_proto-v0.4.2`)

Additive (new methods and types, appended fields and open enums, by the
owner's ruling that appending to a struct is additive, and the same with
open enums; plan 20261027_08, decision 10):

- **Added:** `TerminalOpenParams::cursor: bool` (appended, default false,
  not written when false): the phone reads libflankr_term 0.1.1's cursor
  trailer.
- **Added:** `TerminalOpenResult::cursor: bool` (appended, default false,
  not written when false): this terminal's frames carry the trailer; and
  `TerminalOpenResult::source: TerminalSource` (appended, default `Poll`).
- **Added:** `TerminalSource { Poll, Attached, Unknown }`, open
  (`non_exhaustive`; a source this crate does not know reads as
  `Unknown`), snake case.
- **Added:** `companion::TerminalInfo::mouse: bool` (appended, default
  false, not written when false): the daemon serves `terminal.mouse`.
- **Added:** `common::methods::TERMINAL_MOUSE` (`terminal.mouse`) with
  `TerminalMouseParams { terminal_id, kind, col, row, modifiers }`, the
  kind flattened into the object (`{"terminal_id": 1, "kind": "down",
  "button": "left", "col": 4, "row": 2}`; `modifiers` defaults to 0 and is
  not written when 0); `TerminalMouseKind { Down { button }, Up { button },
  Drag { button }, Moved, Unknown }`, open; `MouseButton { Left, Right,
  Middle }`. Answered `{}`.
- **Added:** `common::methods::TERMINAL_RESIZE` (`terminal.resize`) with
  `TerminalResizeParams { terminal_id, cols, rows }`, answered `{}`.

`companion::VERSION` stays 1 and the link prologue `flankr.link/1`.

## libherdr_term 0.2.0 — 2026-10-28 (the commit tagged `libherdr_term-v0.2.0`)

The crate's first entry; 0.1.0 (plan 20261027_08, steps 2-4, with the
fake server and typed characters as key events of steps 7-8) had none. Not
strictly additive (plan 20261022_03's L3 amendment on the keys row):

- **Breaking:** `convert::KeyToken` gains `Modified { code, modifiers }`
  (crossterm bits: Ctrl 2, Alt 4), for `alt+<char>`, `ctrl+alt+<a-z>` and
  `ctrl+`/`alt+` before the named keys; `KeyToken` is an exhaustive enum.

## libflankr_proto 0.4.3 — 2026-10-28 (the commit tagged `libflankr_proto-v0.4.3`)

Strictly additive (plan 20261028_01, step 1 and the 13:03 EDT amendment's
item 7), behind the `link` feature:

- **Added:** `link::fingerprint(key: &[u8; 32]) -> [u8; 32]`, BLAKE2s-256
  over a link static public key.
- **Added:** `link::remote_id(key) -> String`, the first 8 bytes of the
  fingerprint as 16 lower-case hex digits; `link::REMOTE_ID_LEN` (8).
- **Added:** `link::fingerprint_text(key) -> String`, the shown
  fingerprint, the first 8 hex digits in two groups (`3f9a 12c4`).
- **Added:** `link::confirmation_code(key) -> String`, six zero-padded
  decimal digits: the first 20 bits of BLAKE2s-256 over
  `link::CONFIRMATION_CONTEXT` (`b"flankr.pair.confirm/1"`) and the key,
  modulo 1,000,000.

`companion::VERSION` stays 1 and the link prologue `flankr.link/1`.

## flankr-companion 0.2.0 — 2026-10-28 (the commit tagged `flankr-companion-v0.2.0`)

The crate's first entry; 0.1.0 had none. Several paired remotes (plan
20261028_01, steps 2-4). Not strictly additive:

- **Breaking:** `KeyStore`: `server`, `set_server` and `forget` removed;
  `remotes`, `set_remotes` and `delete_static_key` added; `static_key` and
  `generate_static_key` take a key name.
- **Breaking:** `ServerRecord` gains `id: RemoteId`, `paired_at:
  Option<u64>`, `key_name: String` and `signing_key: Option<String>`
  (reserved); `ServerRecord::new` and `host` added.
- **Breaking:** `Input::CancelPairing` and `Input::RemoveRemote(RemoteId)`
  added to the closed `Input`; `Input::Unpair` now removes the active remote.
- **Breaking:** `Event::Remotes(Vec<RemoteInfo>)`, `Event::RemoteRemoved
  { id, name, daemon }` and `Event::RemoteUpdated { id, name }` added to the
  closed `Event`; `Event::PairingCode` gains `confirmation`; `Event::Unpaired`
  is given when the last remote is removed.
- **Breaking:** `Output::Forget { daemon }` becomes `Output::Remove { id,
  daemon }`.
- **Breaking:** `CompanionConfig` gains `data_dir` and `pairing_window`.
- **Breaking (behaviour):** a second pairing no longer replaces the first:
  it pairs beside the open connection and is added to the list; each remote
  gets its own phone key (`noise.static.<id>`); nothing is stored until the
  daemon accepts; the single `server` record is migrated into the `remotes`
  list on first read.
- **Added:** `RemoteId`, `Remotes`, `RemoteInfo`, `RemoteState` (redb
  4.4.0, a `meta` table per remote), `LEGACY_KEY_NAME`, `MAX_REMOTES`,
  `REVOKE_TIMEOUT`, `State::hello_request`, `keys::{encode_remotes,
  decode_remotes, decode_legacy_server, migrated}`.

## libflankr_proto 0.5.0 — 2026-10-28 (the commit tagged `libflankr_proto-v0.5.0`)

Plan 20261028_01, the six-word pairing step (amendments of 13:59, 14:37,
14:40 and 14:44 EDT).

- **Breaking:** `link::confirmation_code` and `link::CONFIRMATION_CONTEXT`
  removed (the six-digit code is dropped).
- **Breaking:** `PairingPayload` gains `offer: Option<[u8; 16]>`;
  `PAIRING_VERSION` is now 3, written when `offer` is `Some` (version 2 is
  still written without one, and both are read). Decoders of 0.4.3 and
  earlier refuse a version 3 payload as unknown.
- **Added:** `LinkError::Pairing(PairMessageError)` (`LinkError` is
  non-exhaustive).
- **Added:** `OFFER_LEN`, `PAIRING_VERSION_NO_OFFER`, `generate_offer`.
- **Added:** `pairing_words`, `pairing_words_text`, `fingerprint_words`,
  `fingerprint_words_text`, `PAIRING_WORDS_CONTEXT`, `WORD_COUNT` (six
  BIP-39 English words from 66 bits of BLAKE2s-256).
- **Added:** the NK pairing request: `PAIR_PATTERN`, `MODE_PAIR` (0x03),
  `PAIR_MESSAGE_VERSION`, `PAIR_TIMEOUT_DEFAULT_SECS`,
  `PAIR_TIMEOUT_MAX_SECS`, `PairRequest`, `PairAck`, `PairRefusal`,
  `PairResult`, `PairMessageError`, `PairResponse`, `PairIncoming`,
  `Incoming`, `request_pairing`, `recv_pair_result`, `accept_pair`,
  `accept_incoming`, `send_pair_result`.
- New dependency: `bip39` 3 (no default features, CC0-1.0), under the
  `link` feature.

## flankr-companion 0.3.0 — 2026-10-28 (the commit tagged `flankr-companion-v0.3.0`)

Plan 20261028_01, the six-word pairing step (the 14:37, 14:42 and 14:44 EDT
amendments) and the review's changes (14:37 EDT, item 4). Not strictly
additive:

- **Breaking:** `libflankr_proto` 0.4.3 → 0.5.0.
- **Breaking:** `Event::PairingCode { code, server, confirmation }` replaced
  by `Event::PairingWords { server, address, words: Option<String>,
  fingerprint, expires_in_secs: Option<u16>, fallback:
  Option<DeviceCodeFallback> }`; the six-digit confirmation code is gone.
  `DeviceCodeFallback { code, replace }` added; `replace` says the code must
  be pasted with `flankr pair --replace`.
- **Breaking (behaviour):** `Input::Pair` with a version 3 link sends the NK
  pairing request; the remote, its key and `state.redb` are stored only
  after the daemon's `Added`; a refusal, expiry, dropped connection or
  `Input::CancelPairing` (which closes the request's connection) gives
  `Event::PairFailed` and deletes the new key. A version 2 link, or a daemon
  not reached, uses the device-code fallback.
- **Breaking (behaviour):** a link for the remote being paired keeps the
  pending key and request; a listed remote's new name and addresses are
  saved only after a verified `IK` connection to them (`Event::RemoteUpdated`
  only then, else `Event::PairFailed`); `state.redb` and its directory are
  created 0600/0700, and `state.redb.mismatch-*` files are deleted.
- **Added:** `CHECK_TIMEOUT` (30 s), `RESULT_GRACE` (10 s).

## flankr-audio 0.2.0 — 2026-10-28 (the commit tagged `flankr-audio-v0.2.0`)

- **Breaking**: libmorph 0.4.2 (rev 0b288e8) → 0.5.0 (tag v0.5.0, c9b432d).
  `AudioSession::spawn_morph` (feature `morph`) takes `libmorph::CaptureFormat`
  and returns `libmorph::Sink` and `libmorph::Source` from libmorph 0.5.0,
  which is not semver-compatible with 0.4 (Rule 5.5). No other API change.
