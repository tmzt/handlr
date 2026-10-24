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
