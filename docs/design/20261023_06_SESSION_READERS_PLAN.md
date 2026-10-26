---
title: Session readers for agy and other agents, behind one agnostic trait
date: 2026-10-23
status: Implemented
covers:
  - "Reading the sessions of agents other than Claude Code and Codex,
    starting with agy (the Antigravity CLI), so their panes get the agent
    view, questions, turns, notices, spoken summaries and Gemini's session
    context like the others"
  - "Generalising `SessionReader` from JSONL lines to records, so a reader
    may read any store (JSONL files, SQLite databases) and yield the same
    reader-neutral `Item`s, with turns and questions computed above the
    readers"
  - "Recording fixtures for each new reader, read-only and with the
    owner's review, never from secrets"
  - "Not covered: writing to any agent's store; agents herdr does not
    detect; a remote herdr's sessions (not on this machine)"
sources:
  - "Owner (2026-10-23): \"we'll probably need to think about readers for
    the other sessions including agy's. that might be a different plan,
    and the traits should stay agnostic with the turns mapped to the same
    enum (likely modeled on Claude's or Codex's)\"; then \"Yes, create a
    Draft plan for readers for agy and other agents\""
  - "handlr at `68f6ef7`: `src/sessions/mod.rs` (`SessionReader` with
    `kind`, `locate(cwd, session_id) -> Option<Located>`,
    `parse_line(&str) -> Vec<Item>`, `tail`; `read_tail` over a file's
    lines; `Sessions::reader_for(agent)` matching `\"claude\"` and
    `\"codex\"`; `Item {ts, role, kind, tool, text, call_id}`;
    `Located {path, session_id, matched}` and `Located::quiet`),
    `src/sessions/{claude,codex,pending,redact,context}.rs`"
  - "herdr's agent list on 2026-10-23: agy panes report `agent: \"agy\"`
    and `agent_session` `{source: \"herdr:antigravity_cli\", kind: \"id\",
    value: <uuid>}`"
  - "Found read-only on 2026-10-23 (file names and table structure only,
    no rows read): agy keeps each conversation in
    `~/.gemini/antigravity-cli/conversations/<session id>.db`, SQLite in
    WAL mode (`-wal`, `-shm` beside it), with `annotations/<id>.pbtxt`,
    `presence/<id>.lock` and `brain/<id>/` alongside. Tables:
    `steps(idx, step_type, status, has_subtrajectory, metadata,
    error_details, permissions, task_details, render_info, step_payload,
    step_format)` with protobuf blobs, `trajectory_meta`, `gen_metadata`,
    `executor_metadata`, `parent_references`, `trajectory_metadata_blob`,
    `battle_mode_infos`. One conversation's steps counted by type and
    status: 14 (5), 15 (95), 101 (4), 132 (90, of which 3 with status 6),
    all `step_format` 0. The same directory holds
    `antigravity-oauth-token`, which no reader may open"
  - "Plan 20261023_04 (the overview), whose `Turn` (working, stopped,
    asking) is computed in the sessions module from `Item`s and `Located`
    alone, so it applies to every reader"
related:
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (the session readers, decisions 3 to 8, and redaction)
  - docs/design/20261023_03_SESSION_SUBSCRIPTIONS_PLAN.md (serials from a session's lines; a record store needs its own position)
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (Turn, computed above the readers)
  - docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md (`Sessions::reply_text`, which reads through the readers)
---

# Session readers for agy and other agents, behind one agnostic trait

## 1. Goal

Today only Claude Code and Codex panes have sessions handlr can read. An
agy pane shows "(no session to show)", gets no question card, no turn on
the overview, nothing read aloud and no context for Gemini. agy keeps its
conversations in SQLite, not JSONL, so it cannot be added as a third
line parser.

After this plan, a reader is anything that finds an agent's session and
yields the same `Item`s in order. Everything above the readers (questions,
turns, cards, redaction, summaries, serials) stays as it is and works for
any agent with a reader. agy is the first new one.

## 2. Decisions

1. **The trait reads records, not lines.** `SessionReader` keeps `kind`
   and `locate`, and replaces `parse_line` and the line-based `tail` with
   one method that reads the session's last records:

   ```rust
   pub trait SessionReader: Send + Sync {
       fn kind(&self) -> &'static str;
       fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located>;
       /// Records from the newest back, oldest first in the result, until
       /// `enough` says so or the reader's limits are reached.
       fn read_back(&self, at: &Located, enough: &dyn Fn(&[Item]) -> bool)
           -> Result<Read, String>;
       /// When the session last changed, for `Located::quiet`.
       fn modified(&self, at: &Located) -> Option<SystemTime>;
   }
   ```

   The JSONL readers share a `LineReader` helper that implements
   `read_back` over `parse_line` and `read_tail` as today, so Claude's and
   Codex's behaviour does not change. `Located::quiet` asks the reader for
   `modified`: a file's modification time for JSONL, the newer of the
   database's and its `-wal`'s for SQLite.
2. **`Item` stays the one shape,** with one addition: `pos: u64`, the
   record's position in its session, increasing, stable across reads (a
   JSONL line's number; agy's `steps.idx`). Plan 20261023_03's serials use
   it in place of "the JSONL line number" when that plan is built.
3. **Turns and questions stay above the readers.** `pending_question`,
   the overview's `Turn` and the card rules see only `Item`s, `Located`
   and herdr's status. No reader computes them. A reader that cannot tell
   a reply from a tool call leaves its items as `EntryKind::Other`, and
   they show as dim scrollback.
4. **Readers are chosen by herdr's agent and its session source.**
   `Sessions::reader_for` matches herdr's `agent` (`claude`, `codex`,
   `agy`) and, where herdr gives one, `agent_session.source`
   (`herdr:claude`, `herdr:antigravity_cli`). An agent with no reader is
   `no_reader`, as now.
5. **agy: SQLite, read-only, never blocking agy.** The reader opens
   `conversations/<id>.db` with `SQLITE_OPEN_READONLY` and reads through
   the WAL (it never uses `immutable`, which would miss the newest steps),
   with a 100 ms busy timeout, and closes it after each read. It reads
   only the `steps` table, newest `idx` first. It never opens any other
   file in `~/.gemini`, and never `antigravity-oauth-token`. The
   dependency is `rusqlite` with its `bundled` feature, pinned by version
   in the daemon crate only.
6. **agy's steps are decoded from their protobuf payloads** by field
   number, with no generated code: a small decoder for the wire format
   (varints, length-delimited fields) reads the fields that carry the
   text, the role and the tool call. Which `step_type` is which, and
   which field holds what, is found from fixtures (step 1) and written
   down in `docs/AGY_STEPS.md`, an append-only findings document (Rule
   3.6). Unknown step types give `EntryKind::Other` with no text, so a
   new agy version degrades to dim lines, never to wrong text.
7. **Fixtures are recorded with the owner, never from a live secret.**
   A fixture is a copy of one conversation's database, made read-only
   from a conversation the owner names, reviewed by the owner before it
   is committed, and replaced by a synthetic one if it holds anything
   private. The recording script never reads the OAuth token, `.private/`
   or `.env` files.
8. **The same path for other agents.** Each further agent herdr detects
   (for example the Gemini CLI or opencode) is an amendment to this plan:
   where its sessions are, how its records map to `Item`s, its fixtures,
   and its reader, behind the same trait.

## 3. Steps

1. **Fixtures and findings.** `scripts/record-agy-fixture.sh <session id>`
   copies one conversation's `.db` and `-wal` to a temporary directory and
   checkpoints the copy, so the fixture is one self-contained file. The
   owner names the conversation and reviews the copy; it goes under
   `tests/fixtures/sessions/agy/`. `docs/AGY_STEPS.md` records the step
   types and fields found, with the counts.
   **Done when:** two fixtures are committed with the owner's approval and
   `AGY_STEPS.md` names the step types for a user prompt, a reply, a tool
   call and a tool result.
2. **The trait.** `read_back`, `modified`, `LineReader`, `Item.pos`; the
   Claude and Codex readers on `LineReader`.
   **Done when:** every existing sessions, daemon and companion test
   passes unchanged, and `pos` is the line number for both.
3. **The protobuf decoder.** `src/sessions/proto_wire.rs`.
   **Done when:** tests decode varints, nested messages and strings from
   hand-written bytes, and a truncated blob is an error, not a panic.
4. **The agy reader.** `src/sessions/agy.rs`: locate by id
   (`conversations/<id>.db`), else the newest conversation whose
   `trajectory_meta` or annotation names the pane's cwd if that can be
   found, else none; `read_back` over `steps`; the mapping of step 1.
   **Done when:** the fixtures read as the expected items (a question
   asked at the end found by `pending_question`, a stopped turn by
   `Turn`), a database locked for writing is read without waiting more
   than 100 ms, and no file other than the `.db`, `-wal` and `-shm` is
   opened (checked with a fixture home holding a decoy token file).
5. **Live, read-only.** `handlr session peek --pane <agy pane>` (or the
   socket's `session.peek`) on a live agy pane, logged under
   `out/sessions/YYYYMMDD_NN_agy_live/`; then on the S23, an agy pane's
   agent view, its card on the overview, and a question card.
   **Done when:** that log shows agy entries and a pending question, and
   the S23 check passes.

Each step is one crate or subproject, one commit (Rule 3.1). 1 needs the
owner; 2 and 3 need nothing; 4 needs 1, 2 and 3; 5 needs 4.

## 4. Open questions for the owner

1. **Which agy conversations** may be copied as fixtures (step 1), or
   should the fixtures be synthetic from the start, built by a script
   that writes the same tables?
2. **Which other agents next**, and in what order? Gemini CLI, opencode,
   aider, or others you run in herdr.
3. **agy's `brain/` directory** may hold its plans or notes. Should the
   agent view show them, or only the conversation?

## Amendment — 2026-10-23 17:28 EDT: the repowiki agy session as the example, never committed

The owner (2026-10-23), answering open question 1: "you can use the repo
wiki session as an example. But don't commit it". This replaces decision
7's committed copies and step 1's "two fixtures are committed with the
owner's approval".

- **The example** is the agy conversation in the repowiki pane (herdr
  pane `w2:p4`, session `1225868f-05e4-41d8-85dd-c12a0851b232`). The
  recording script copies its `.db` and `-wal`, read-only, to
  `out/sessions/agy_example/` (gitignored) and checkpoints the copy. It
  is never committed, never pushed, and never copied anywhere else. Tests
  that need it are `#[ignore]` unless `HANDLR_AGY_EXAMPLE` points at it,
  as `live_naming.rs` does with `HANDLR_SNAPSHOT`.
- **The committed fixtures are synthetic:** `scripts/make-agy-fixture.rs`
  (or a test helper) writes a small database with the same tables and the
  step types and fields found in the example, holding made-up text. Those
  go under `tests/fixtures/sessions/agy/`.
- `docs/AGY_STEPS.md` records the structure found (step types, field
  numbers, which carries text, role and tool calls) and counts, never the
  example's text.

**Step 1, as amended. Done when:** the example is copied to
`out/sessions/agy_example/` and read by an ignored test, `AGY_STEPS.md`
names the step types for a user prompt, a reply, a tool call and a tool
result, and two synthetic fixtures are committed.

## Amendment — 2026-10-23 17:33 EDT: implementation has started; the document is now immutable

The owner approved the plan on 2026-10-23: "Yes, approve the session
readers plan and start with step 1, copying and mapping the agy example".
Open question 3 is answered by the owner's "It shouldn't include the
memory or anything like that. Just a session": agy's `brain/`, annotations
and summaries are never read, and its state snapshots (step type 23) are
left out. The document is now immutable; departures go to CHANGES.md.

## Amendment — 2026-10-25 02:54 EDT: implementation is complete

Every step is built and verified:
- **Step 1 (fixtures and findings):** `364df48`, `2807e4e`, `273168d`, `b93b05e`.
- **Step 2 (generic trait and Item.pos):** `fb97fef`.
- **Step 3 (protobuf wire decoder):** `671e251`.
- **Step 4 (agy SQLite reader):** `66ec882`.
- **Step 5 (live verification):** logged in `out/sessions/LOG.md` and
  `out/sessions/20261025_01_agy_live/`. `session.peek` and `agents.list`
  read all active agy panes directly from SQLite WAL without blocking.

`status` is set to `Implemented`.

## Amendment — 2026-10-26 11:32 EDT: exit code 0 is a successful tool call, not an error

This amends step 4 and `AGY_STEPS.md` regarding tool results and status mapping.

1. **Exit code 0 is not a failure:** A tool result from command execution (such as agy's `run_command`) containing `Command exited with code 0.` is a successful tool call. It must not be prefixed with `error:` or generate a `CardKind::Broke` ("Something broke") card, regardless of SQLite status integers.
2. **Mapped like in Claude:** When an agy tool result represents a command execution wrapped in an envelope (`Created At: ... \n Completed At: ... \n Command exited with code 0. \n\n Command output: ...`), the output is unwrapped to the actual command output (or empty string if `No output from the command.`), matching Claude's raw tool result presentation.
3. **Failures and non-zero exit codes:** Non-zero exit codes (such as `Command exited with code 1.`) represent failed execution and are prefixed with `error:` so they accurately trigger `is_error` and produce a `Broke` card.
4. **Tool result status codes:** agy SQLite status 5 (used in earlier conversations) and 3 (used in later conversations) both denote completed steps. Only status 6 (cancelled) and 7 (failed), or explicit error text (`Encountered error in step execution:`, `error:`), denote failures when not overridden by exit code 0.
5. **Guard in sections:** `src/sessions/sections.rs` (`is_error`) explicitly guards that tool result summaries reporting exit code 0 are never treated as errors.

