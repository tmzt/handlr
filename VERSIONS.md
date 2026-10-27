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
