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

## flankr-companion 0.4.0 — 2026-10-28 (the commit tagged `flankr-companion-v0.4.0`)

The settings follow-up (`docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md`,
the living section's 2026-10-28 entries). Not strictly additive:

- **Breaking:** `Input::SetNickname { id, nickname }` sets or clears a
  remote's nickname, kept in the remotes list (`Input` is exhaustive).
- **Breaking:** `Event::PairingWords` gains `fingerprint_hex` (BLAKE2s-256 of
  the daemon's key, 64 hex digits; a field on an exhaustive enum's variant).
- **Added:** `RemoteInfo::fingerprint_hex`, `fingerprint_words`, `nickname`
  and `display_name` (the nickname, or else the base hostname, with `:port`
  only when two listed remotes would otherwise show the same name).
- **Added:** `ServerRecord::nickname`; `Remotes::set_nickname`,
  `Remotes::display_names`; `keys::nickname`, `base_hostname`,
  `fingerprint_hex`, `NICKNAME_MAX` (40).

## libflankr_proto 0.5.1 — 2026-10-28 (the commit tagged `libflankr_proto-v0.5.1`)

Plan 20261028_03 (the session drawer), step 1. Strictly additive:

- **Added:** `AgentInfo::idle_ms: Option<u64>` (milliseconds since the
  session file or any of its subagents' files was last written, on the
  daemon's clock), `AgentInfo::subagents_running: Option<u32>` (subagents
  not marked finished and written within 60 s; absent when the agent's
  reader does not read subagents) and `AgentInfo::summary: Option<String>`
  (the first line of the latest Finished card's spoken summary, redacted, at
  most `TURN_TEXT_MAX` characters). Appended, defaulted when absent and left
  off the wire when `None`, so older lists parse and older clients ignore
  them.

## flankr-companion 0.5.0 — 2026-10-28 (the commit tagged `flankr-companion-v0.5.0`)

Plan 20261028_03 (the session drawer), steps 3 and 4, with the 18:48 and
18:51 EDT amendments. Not strictly additive:

- **Breaking:** `Input` and `Event` are `#[non_exhaustive]`; matches need a
  wildcard arm. Later variants are patch bumps.
- **Breaking:** `Input::SwitchRemote(RemoteId)`, `Input::DrawerShown(bool)`
  and `Input::ToRemote { remote, input }`; `Event::Background { remote,
  event }` carries every event of a remote not in use.
- **Breaking (behaviour):** every listed remote keeps its own link (a
  connection manager); those not in use are in the daemon's background
  mode. `RemoteInfo::online` is now each remote's own link, not only the
  active one's. Switching closes no link.
- **Added:** `DRAWER_LIST_INTERVAL` (3 s): while the drawer shows, the active
  remote's agents are listed at once and every 3 s, also with an agent view
  open.
- **Added:** the `activity` module: `Activity`, `NeedsInput`, `Signals`,
  `activity()`, `short_age()`, `RECENT`, `TOOL_WAIT`, `SUBAGENT_LIVE`.
- **Added:** `State::replay`, `State::leave_foreground`,
  `State::trade_terminals`; `Remotes::info_with`; `Cross::shown`.
- libflankr_proto 0.5.0 → 0.5.1 (semver-compatible; `AgentInfo`'s
  `idle_ms`, `subagents_running`, `summary`).

## libflankr_proto 0.5.2 — 2026-10-28 (the commit tagged `libflankr_proto-v0.5.2`)

Plan 20261028_04 (plugins), step 1. Strictly additive:

- **Added:** `companion::methods::{PLUGIN_CALL, PLUGIN_RESULT, PLUGIN_NOTIFY,
  PLUGIN_SET_ENABLED}` (`plugin.call`, `plugin.result`, `plugin.notify`,
  `plugin.set_enabled`); `PluginCall`, `PluginAccepted`, `PluginResult`,
  `PluginNotify`, `PluginInfo`, `PluginMenuItem`, `PluginSetEnabledParams`,
  `PluginSetEnabledResult`; `PLUGIN_MAX_BYTES` (256 KiB),
  `PLUGIN_NS_MAX_BYTES`, `PLUGIN_METHOD_MAX_BYTES`, `is_plugin_ns()` and
  `is_plugin_method()`; `envelope::codes::{PLUGIN_UNKNOWN (-32030),
  PLUGIN_DISABLED (-32031), PLUGIN_TOO_LARGE (-32032)}`.
- **Added:** `HelloResult::plugins` and `ServerCapabilities::plugins`
  (`Option<Vec<PluginInfo>>`), appended, defaulted when absent and left off
  the wire when `None`, so an older daemon's objects still read (no plugins).

## dev.flankr.plugins.planning 0.1.0 — 2026-10-28 (the commit tagged `dev.flankr.plugins.planning-v0.1.0`)

Plan 20261028_04 (plugins), step 3. The plugin's first release, disabled by
default:

- **Added:** the methods `plans.list {}` → `{schema: 1, root, plans: [...]}`
  and `plans.show {name}` → `{schema: 1, plan: {...}}`, in tpt's schema-1
  shapes (list items: `file`, `base`, `date_from_name`, `nn`, `name`,
  `kind`, `title`, `date`, `status`, `status_base`, `status_by {ref}`,
  `last_amendment`, `readme {row, status, decision}`, `error`; `show` adds
  `covers`, `sources`, `related`, `living_sections`, `extra`, `headings`,
  `amendments [{index, line, timestamp, subject, living_section}]`); the
  errors -32110 (no docs/design plans) and -32111 (no plan of that name).
- **Added:** the menu item `plans` ("Plans in this project", a modal, both
  screens).

## flankr-companion 0.5.1 — 2026-10-28 (the commit tagged `flankr-companion-v0.5.1`)

Plan 20261028_04 (plugins), step 5. Strictly additive (`Input` and `Event`
are `#[non_exhaustive]`):

- **Added:** `Input::PluginCall { ns, id, method, params, session }` (sent as
  `plugin.call`; a call refused at once, offline, malformed, over 256 KiB,
  refused by the daemon or cut off by the connection closing, is answered
  as `Event::PluginResult` with the error) and
  `Input::SetPluginEnabled { ns, enabled }` (`plugin.set_enabled`).
- **Added:** `Event::Plugins(Vec<PluginInfo>)` (after each `hello`, empty
  from an older daemon; on each `server.capabilities` with `plugins`; after
  a toggle is answered or refused; last in `State::replay`),
  `Event::PluginResult(PluginResult)` and `Event::PluginNotify(PluginNotify)`.
- **Added:** `State::plugins()`; `client::plugin_refusal()`: plugin inputs
  for a remote not in use are refused (plugins act on the remote in use
  only).
- libflankr_proto 0.5.1 → 0.5.2 (semver-compatible).

## libflankr_proto 0.5.3 — 2026-10-28 (the commit tagged `libflankr_proto-v0.5.3`)

Plan 20261028_04 (plugins), its 2026-10-28 22:03 EDT amendment (title-bar
icons). Strictly additive:

- **Added:** `PluginMenuItem::icon` (`Option<String>`), appended, defaulted
  when absent and left off the wire when `None`: one of the app's icon
  names (e.g. `"plans"`); an item with an icon is a title-bar icon.
- **Added:** `PluginMenuItem::opens` documents `"menu"` (a popup menu of the
  device half's items, anchored to the title icon); an app that does not
  know a value opens it as `"modal"`.

## dev.flankr.plugins.planning 0.1.1 — 2026-10-28 (the commit tagged `dev.flankr.plugins.planning-v0.1.1`)

Plan 20261028_04 (plugins), its 2026-10-28 22:03 EDT amendment, item 5.
Additive:

- **Changed (additive):** the menu item `plans` ("Plans in this project",
  both screens) gains `icon: "plans"` (☑, a title-bar icon) and
  `opens: "menu"` (a popup of the ten newest plans and All plans…); an app
  without `"menu"` opens it as the modal, as in 0.1.0. Methods unchanged.

## flankr-companion 0.5.2 — 2026-10-29 (the commit tagged `flankr-companion-v0.5.2`)

Plan 20261028_06 (the bottom bar updated), step 2. Strictly additive (a
field appended to `Settings`, which has `Default`):

- **Added:** `Settings::speak_gemini` (`bool`, `true` by default),
  *Speak agent (Gemini) responses*: off, a `turn.say` for the running turn
  still gives `Event::Conversation` (the card shows the line) but no
  `Event::Say`. What is being said is not cut off; `Input::Skip` does that.
  *Speak coding responses* stays `Input::ReadAloud`.

## libflankr_proto 0.5.4 — 2026-10-29 (the commit tagged `libflankr_proto-v0.5.4`)

Plan 20261028_05 (binary transfers), step 1, and its 23:43 EDT amendment,
item 2. Strictly additive:

- **Added:** link kind 2: `link::KIND_BINARY` (0x2), `Kind::Binary`,
  `Message::Binary(Vec<u8>)` (both enums `#[non_exhaustive]`),
  `LinkStream::send_binary` and `LinkWriter::send_binary`; `recv` reads
  kinds 0, 1 and 2, `recv_message` still kind 0 only.
- **Added:** `companion::BlobInfo` (`version`, `namespaces`, `read`,
  `image_max`, `file_max`, `git_max`, `chunk_max`, `images_per_send`;
  `serves()`), and `HelloResult::blob` / `ServerCapabilities::blob`
  (`Option<BlobInfo>`), appended, defaulted when absent and left off the
  wire when `None`.
- **Added:** `AgentsSendParams::parts` (`Vec<SendPart>`), appended,
  defaulted, left off the wire when empty; `common::SendPart`
  (`#[non_exhaustive]`: `Text`, `Image`).
- **Added:** `methods::TERMINAL_PASTE` (`terminal.paste`),
  `TerminalPasteParams`, `TERMINAL_PASTE_TEXT_MAX` (8192),
  `TERMINAL_PASTE_IMAGES_MAX` (4).

## libflankr_blob 0.1.0 — 2026-10-29 (the commit tagged `libflankr_blob-v0.1.0`)

Plan 20261028_05, step 2. New crate: the kind-2 encoding (`BlobMessage`,
`Body`, `Offer`, `Subcommand`, `DecodeError`), `Namespace`, `TransferId`,
`Ids`, `Code` and its 20 codes, the limits of 4.4, the
`Receiving`/`Sending` state machines and `Step`, `ImageType` sniffing,
`GitRequest`/`Rev`, `ReadPattern` and `always_refused` (moved from the
daemon), `sha256`, `hex`, `parse_reference`.

## flankr-companion 0.5.3 — 2026-10-29 (the commit tagged `flankr-companion-v0.5.3`)

Plan 20261028_06 (the bottom bar updated), its 2026-10-29 00:43 EDT
amendment, item 4. Strictly additive (a variant on the `#[non_exhaustive]`
`Input`):

- **Added:** `Input::StopReading`: what is being said stops
  (`Event::StopSpeaking` while speaking), and every reply waiting to be
  read (for its summary or its turn) and every held spoken notice is
  dropped, so nothing follows it. Replies that come after are read as
  usual. `Input::Skip` is unchanged: it stops only the current one.

## flankr-companion 0.5.4 — 2026-10-29 (the commit tagged `flankr-companion-v0.5.4`)

Plan 20261028_05 (binary transfers), step 7, and its 23:43 EDT amendment,
item 2. Strictly additive (`Input` and `Event` are `#[non_exhaustive]`; a
field appended to `CompanionConfig`, which has `new`):

- **Added:** `Input::AttachImage`, `DetachImage`, `RetryImage`,
  `RequestFile`, `RequestGit`, `CancelTransfer`, `TerminalPasteText`,
  `TerminalPasteImages`.
- **Added:** `Event::TransferStarted`, `TransferProgress`, `ImageReady`,
  `FileReady`, `TransferFailed`, `Attachments`, `Blob`.
- **Added:** `State::set_preview_cache`, `blob`, `attachments`, `binary`,
  `binary_written`, `take_binary`; `CompanionConfig::cache_dir`;
  `blob::{Attachment, PreviewCache, PREVIEW_CACHE_MAX, PREVIEW_CACHE_FOLDER,
  token}`.
- Submit sends `agents.send` `parts` when the draft names uploaded images;
  link kind 2 is read and written (two-queue writer).
- Takes libflankr_blob 0.1.0 (new dependency).

## libflankr_proto 0.5.5 — 2026-10-29 (the commit tagged `libflankr_proto-v0.5.5`)

Plan 20261029_01 (choice questions), step 1. Strictly additive:

- **Added:** `PendingQuestion::choices` (`Option<ChoiceSet>`), appended,
  defaulted when absent and left off the wire when `None`; when present,
  `answers` is empty and `text` is the first question's text.
- **Added:** `common::ChoiceSet` (`id`, `questions`, `source`),
  `ChoiceQuestion` (`header`, `text`, `multi`, `other`, `options`) and
  `ChoiceOption` (`label`, `detail`, `preview`), all `#[non_exhaustive]`,
  each with `new` and `within_bounds`; the bounds `CHOICE_QUESTIONS_MAX`
  (4), `CHOICE_OPTIONS_MAX` (8), `CHOICE_HEADER_MAX` (32),
  `CHOICE_LABEL_MAX` (120), `CHOICE_DETAIL_MAX` (400), `CHOICE_PREVIEW_MAX`
  (4000), `CHOICE_PREVIEW_LINES` (60) and `CHOICE_ID_MAX` (128).
- **Added:** `methods::CHOICE_ANSWER` (`choice.answer`, companions only),
  `ChoiceAnswerParams` (`pane_id`, `id`, `answers`), `ChoiceAnswer`
  (`picked`, `other`; `Default`), `ChoiceAnswerResult`
  (`#[non_exhaustive]`, tagged by `status`: `answered`, `mismatch { got }`,
  `no_result`) and `CHOICE_RESULT_WAIT_S` (5).
- **Added:** `envelope::codes::CHOICE_STALE` (-32040),
  `CHOICE_UNSUPPORTED` (-32041) and `SCREEN_MISMATCH` (-32042);
  `codes::ALL` is unchanged.

## flankr-companion 0.5.5 — 2026-10-29 (the commit tagged `flankr-companion-v0.5.5`)

Plan 20261029_01 (choice questions), step 5. Strictly additive (a field
appended to `AgentCard`; a variant each on the `#[non_exhaustive]` `Input`
and `Event`):

- **Added:** `AgentCard::choices` (`Option<ChoiceSet>`): a live Question
  card's choices, from its pending question, while its text is that
  question's; `None` for every other card and from an older daemon.
- **Added:** `Input::Choose { id, answers }`: `choice.answer` for the open
  view's pane; one at a time.
- **Added:** `Event::ChoiceAnswered { id, pane_id, outcome, got, code }`:
  `answered` (the card goes at once), `mismatch` (`got`: what the agent
  recorded), `no_result`, or `failed` (`got`: the reason; `code`: the
  daemon's error code when it gave one), also for a closed link or no
  answer in `CHOICE_ANSWER_WAIT`.
- **Added:** `client::CHOICE_ANSWER_WAIT` (20 s).
- Takes libflankr_proto 0.5.5.

## libflankr_proto 0.5.6 — 2026-10-29 (the commit tagged `libflankr_proto-v0.5.6`)

Plan 20261023_07 (subagents), step 2. Strictly additive (fields appended;
defaulted when absent; left off the wire when empty or `None`):

- **Added:** `AgentInfo::children` (`Vec<ChildInfo>`): at most
  `CHILDREN_MAX` (16), running first, then finished, newest first within
  each.
- **Added:** `common::ChildInfo` (`id`, `short`, `description`, `kind`,
  `background`, `finished`, `started`, `started_by`, `turn`; `Default`).
- **Added:** `child: Option<String>` on `SessionPeekParams`,
  `SpeechSummarizeParams` and `SessionSubscribeParams`; without it the main
  session is read, as before; an unknown child is `UNKNOWN_AGENT`.

## flankr-companion 0.5.6 — 2026-10-29 (the commit tagged `flankr-companion-v0.5.6`)

Plan 20261023_07 (subagents), step 4. Strictly additive (variants on the
`#[non_exhaustive]` `Input`; fields appended to `Event::AgentView`):

- **Added:** `Input::OpenChild(String)` and `Input::CloseChild` (back to the
  parent).
- **Added:** `Event::AgentView::child` (`Option<String>`) and
  `Event::AgentView::children` (`Vec<ChildInfo>`).
- Takes libflankr_proto 0.5.6.

## dev.flankr.plugins.planning 0.1.2 — 2026-10-29 (the commit tagged `dev.flankr.plugins.planning-v0.1.2`)

Plan 20261028_04 (plugins), its 2026-10-29 02:10 EDT amendment, item 6.
Additive:

- **Added:** two read-only general agent tools. `planning_plans_list {}`:
  the plans and specs in docs/design, newest first by last update, each
  with its name, NN, date, title, abbreviated status (Draft, Hold,
  InImpl, Impl, Succd, Rtrd) and update time. `planning_plan_show
  { name }`: one plan's front matter, covers and amendments, as
  `plans.show` gives them. Offered while the plugin is enabled and
  `agent_tools` is not `false`. Methods and the menu unchanged.

## flankr-companion 0.5.7 — 2026-10-29 (the commit tagged `flankr-companion-v0.5.7`)

Plan 20261028_06 (bottom bar), its 2026-10-29 15:09 EDT amendment, item 5.
Strictly additive (a variant on the `#[non_exhaustive]` `Input`):

- **Added:** `Input::ContinueByVoice`: the typed prompt open (the message
  or Gemini's) goes on in voice mode, its text kept as its first part, and
  `Event::Arm` is sent. Nothing changes with the microphone setting off or
  no typed prompt open.

## flankr-companion 0.5.8 — 2026-10-30 (the commit tagged `flankr-companion-v0.5.8`)

Plan 20261028_03 (session drawer), its 2026-10-30 13:15 EDT amendment.
Strictly additive:

- **Added:** `State::restore_agents(Vec<AgentInfo>)`: a list kept from
  an earlier run is given as `Event::Agents` until the daemon lists;
  nothing once it has.
- **Added:** `State::take_new_list()`: the agent list, once, after the
  daemon gives one that differs from what showed.
- **Added:** `RemoteState::agents()` and `RemoteState::set_agents(&[AgentInfo])`:
  the remote's last agent list, kept in its `state.redb` (an `agents`
  table).

## flankr-companion 0.6.0 — 2026-10-30 (the commit tagged `flankr-companion-v0.6.0`)

Plan 20261023_02 (dictated messages), its 2026-10-30 13:37 EDT amendment.
Not strictly additive: no API item changes, but when `State` gives
`Event::Say` does (Rule 5.4, behaviour a caller can observe):

- **Breaking (behaviour):** while a voice prompt is collecting (a dictated
  message, or Gemini's prompt, in `PromptMode::Voice`), replies read aloud,
  the daemon's notices, `turn.say` lines and a tapped turn's "stopped"
  sentence are held in the notice queue (cap 8, the oldest dropped) and
  given as `Event::Say`, in order, once the prompt is submitted, cleared or
  closed, under the queue's usual rules. The prompt's own speech (read-back,
  "The message is empty/full.", refusals, a failed send), confirmation
  questions and `Input::Replay` are said at once, as before.
- **Breaking (behaviour):** `Input::StartDictation`, a voice
  `Input::StartGeneralAgentPrompt`, `Input::ContinueByVoice` and a
  suggestion that starts a message give `Event::StopSpeaking` before
  `Event::Arm` while speech plays.

## libflankr_proto 0.5.7 — 2026-10-30 (the commit tagged `libflankr_proto-v0.5.7`)

Plan 20261023_05 (spoken summaries), its 2026-10-30 13:52 EDT amendment,
item 2. Strictly additive:

- **Added:** `methods::SPEECH_CATCH_UP` (`speech.catch_up`), companions
  only: one spoken catch-up for the speech held during a voice input,
  offered while the daemon has a spoken summaries model (`LlmInfo::summaries`)
  and under the same `client.options` (`spoken_output`) as
  `speech.summarize`.
- **Added:** `common::SpeechCatchUpParams` (`held`, `said`),
  `common::HeldSpeech` (`pane_id`, `ts`, `text`), `common::SaidInput`
  (`text`, `pane_id`) and `common::SpeechCatchUpResult` (`summary`); each
  `Default`, optional fields defaulted when absent and left off the wire
  when `None`.

## flankr-companion 0.7.0 — 2026-10-30 (the commit tagged `flankr-companion-v0.7.0`)

Plan 20261023_02 (dictated messages), its 2026-10-30 13:52 EDT amendment.
Not strictly additive: when and what `State` gives as `Event::Say` after
a voice input changes (Rule 5.4, behaviour a caller can observe):

- **Breaking (behaviour):** when a voice prompt ends (submitted, cleared
  or closed) with speech held, `State` sends one `speech.catch_up` with
  the held items, oldest first (a reply read aloud named by its pane and
  `ts`, a notice by its pane), and what was said (the message as sent and
  its target pane, or Gemini's prompt; none when cancelled or cleared).
  Its summary is given as one `Event::Say` in place of the held items,
  under the queue's usual rules. While it is made the held items stay
  held, and speech that arrives meanwhile waits behind it, joining the
  next voice input's catch-up if one starts. The held items are given as
  they are, as in 0.6.0, when read aloud is off, the daemon makes no
  summaries (`LlmInfo::summaries`), the client is offline, the request is
  refused or fails, the answer has no `summary`, the link closes, or no
  answer comes in `CATCH_UP_WAIT`. Nothing held asks nothing; *Stop
  reading* and *All sound* off drop a catch-up being made.
- **Changed:** `State::held_notices()` also counts the items a catch-up is
  being made for.
- **Added:** `client::CATCH_UP_WAIT` (8 s).
- libflankr_proto 0.5.6 → 0.5.7 (semver-compatible).

## flankr-speech 0.1.1 — 2026-10-30 (the commit tagged `flankr-speech-v0.1.1`)

A fix, with no public API change (the owner's report of 2026-10-30 15:12
EDT, "tapped the record button and got no response"). The crate's first
entry here: 0.1.0 is the version it was introduced at, not separately
tagged.

- **Fixed:** `read_aloud` and `complexity` no longer panic on a word
  ending in a character of more than one byte (`—`, `…`, `→`, `✓`): the
  check for an attached `am`/`pm` split such a word inside that
  character. The word is now left as it is. The daemon's
  `speech.summarize` task and the companion client's own reading of a
  reply, when no summary came, both panicked on such a reply; the client's
  panic ended its `net` thread.

## flankr-companion 0.7.1 — 2026-10-30 (the commit tagged `flankr-companion-v0.7.1`)

Plan 20261023_02 (dictated messages), its 2026-10-30 15:10 EDT amendment.
Strictly additive: both variants are on `#[non_exhaustive]` enums, and a
caller that gives neither sees no change.

- **Added:** `Event::CatchUp { text }`: when a voice input's catch-up
  comes, its summary (trimmed) is given at once for the app's *While you
  spoke* card, beside the `Event::Say` of the same text that follows in
  the queue's turn. Never given for the fallback (none made, refused,
  failed, late, or the link closed).
- **Added:** `Input::Muted(bool)`: the app's mute. Muted, no reply is read
  aloud and no Gemini line is spoken, except during voice input, where
  they are held as unmuted so its catch-up is still asked for. With it,
  the app gives `Input::ReadAloud` and `Settings::speak_gemini` as they
  are, not turned off for mute. Not muted until given.

## flankr-companion 0.7.2 — 2026-10-30 (the commit tagged `flankr-companion-v0.7.2`)

Plan 20261022_03 (harness protocol), living section L3, the 2026-10-30
15:20 EDT amendment, item 5. Strictly additive: the variant is on the
`#[non_exhaustive]` `Input`, the new enum is itself `#[non_exhaustive]`,
and a caller that gives neither sees no change.

- **Added:** `Input::DropSpeech(SpeechOf)`: a card dismissed on the phone
  drops its speech waiting to be said, and stops it if it is being said;
  other speech is left as it is.
- **Added:** `SpeechOf`, whose speech that is: `Card(Serial)`, the open
  view's card at that section serial (the replies on its line, waiting to
  be summarized, read aloud or held for a catch-up), or `CatchUp`, the
  catch-up last given as `Event::CatchUp`.

## flankr-companion 0.7.3 — 2026-10-30 (the commit tagged `flankr-companion-v0.7.3`)

Plan 20261023_03 (session subscriptions) and plan 20261024_02 (the
settings screen), their 2026-10-30 15:20 EDT amendments: the last view
cached, shown at start and refreshed on connect, and *At startup*'s last
session. Additive: the new fields are appended to a struct
(`RemoteInfo`) and to a variant of the `#[non_exhaustive]` `Event`, which
the owner's ruling of 2026-10-23 counts as additive ("appending to a
struct is additive"); a pattern that names every field of
`Event::AgentView` without `..` has to name the two new ones.

- **Added:** `State::restore_view(KeptView)`: a session's view kept from
  an earlier run goes into the cache of closed views; opening its pane
  (`Input::OpenAgent`), even offline, shows it at once. Once online its
  subscription resumes where the kept view ends and deliveries merge in
  by serial. Gives `Event::Cache` when the cache's size changed.
- **Added:** `Event::AgentView::cached`: the view shows what was kept,
  and no delivery has come since; the first one clears it.
- **Added:** `Event::AgentView::ended`: the restored session is gone (no
  longer in the daemon's agent list, or its subscription said there is
  no session); the view holds nothing. A view opened as before is never
  `ended`.
- **Added:** `State::take_kept_view()`: once, the view to keep, after the
  agent view closed or `Input::Background(true)` came with it open: its
  outline and the sections of the last screenful and a screenful either
  side (the newest without a window), at most `KEPT_SECTIONS` (200) or
  `KEPT_BYTES` (512 KiB) of section JSON. None for a subagent's view or
  an ended one.
- **Added:** `State::take_last_session()`: once, the last session's
  change: `Some(Some(pane))` when a view opens or the app goes to the
  background with it open, `Some(None)` on `Input::CloseAgent`.
- **Added:** `State::lists(pane_id)`: whether the agent list as last
  known has the pane.
- **Added:** `KeptView` (with `pane_id()`, `session_id()`, `kept_at()`,
  `sections()`, `set_kept_at()`), `KEPT_SECTIONS`, `KEPT_BYTES`, and
  `LastSession { pane_id, shown }`.
- **Added:** `RemoteState::view()`, `set_view()`, `last_session()` and
  `set_last_session()`: `state.redb`'s new `view` table (one entry,
  replaced each time) and `last_session` table (deleted with `None`).
- **Added:** `RemoteInfo::last_session`: the remote's last open session
  from its `state.redb`, while its pane is in the remote's agent list as
  last known. The `net` thread keeps both tables as the view opens,
  closes and goes to the background, and gives `Event::Remotes` when a
  last session changes.

## libflankr_proto 0.5.8 — 2026-10-30 (the commit tagged `libflankr_proto-v0.5.8`)

Plan 20261029_01 (choice questions), its 2026-10-30 16:42 EDT amendment,
item 1. Strictly additive:

- **Added:** `methods::CHOICE_CANCEL` (`choice.cancel`), companions only,
  offered as `choice.answer` is: cancels the pane's open choice question,
  as the agent's own dismiss does.
- **Added:** `common::ChoiceCancelParams` (`id`, the question's
  `ChoiceSet::id`, and `pane_id`) and `common::ChoiceCancelResult`
  (`cancelled`: false when the agent's screen did not show the question
  and nothing was sent).

## flankr-companion 0.7.4 — 2026-10-30 (the commit tagged `flankr-companion-v0.7.4`)

Plan 20261029_01 (choice questions), its 2026-10-30 16:42 EDT amendment,
item 3. Strictly additive: the variant is on the `#[non_exhaustive]`
`Input`, and the new outcomes come only after it.

- **Added:** `Input::CancelChoice { id }`: `choice.cancel` for the open
  view's pane, refused here as `Input::Choose` is (no view, a subagent's
  view, offline, or a choice request already on its way), one choice
  request at a time with it.
- **Added:** `Event::ChoiceAnswered`'s outcomes for a cancel: `cancelled`,
  the question then ended as the agent's own dismiss ends it (its live
  card goes at once), and `not_cancelled`, with `got` empty when the
  agent's screen did not show the question, or the reason and the
  daemon's `code` when the request did not go through.

## libflankr_proto 0.5.9 — 2026-10-30 (the commit tagged `libflankr_proto-v0.5.9`)

Plan 20261024_03 (the general agent chat), its 2026-10-30 22:08 EDT
amendment, item 4. Strictly additive:

- **Added:** `companion::methods::TURN_PROGRESS` (`turn.progress`), a
  notification from the daemon while a turn runs.
- **Added:** `companion::TurnProgress` (`turn_id`; `tool`, the tool the
  engine is calling, empty once it returned and the engine thinks again;
  `label`, its plain name to show, e.g. "Looking at recent commits…").

## flankr-companion 0.7.5 — 2026-10-30 (the commit tagged `flankr-companion-v0.7.5`)

Plan 20261024_03 (the general agent chat), its 2026-10-30 22:08 EDT
amendment, item 3. Strictly additive: the variants are on the
`#[non_exhaustive]` `Event`, and libflankr_proto 0.5.8 → 0.5.9 is
semver-compatible.

- **Added:** `Event::TurnStarted { turn_id }`, given as each
  `turn.start` is sent, before the user's `Event::Conversation` line.
- **Added:** `Event::TurnProgress { label: Option<String> }`, from
  `turn.progress` for the running turn not tapped to interrupt:
  `Some(label)` while Gemini runs a tool ("Working…" when the daemon
  gave none), `None` once it is thinking again. Never spoken.
