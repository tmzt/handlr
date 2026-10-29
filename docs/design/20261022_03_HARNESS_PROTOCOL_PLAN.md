---
title: The handlr.harness protocol, session peek and the MCP adapter
date: 2026-10-22
status: In implementation
covers:
  - The `handlr.harness` protocol for programs: agent state, sending with
    confirmation, and peeking at an agent's session
  - Session readers that find an agent's own transcript from the pane's
    cwd, for Claude Code and Codex, and return its last entries
  - `handlr mcp`, an MCP server over stdio that maps onto the harness
    protocol
  - Not covered: the T3 Code plugin itself (its own plan, once its plugin
    surface is known); agent supervision, plan inference and the local
    model (a later plan, which adds methods and events here)
sources:
  - Owner's discussion (2026-10-22): a T3 Code plugin uses handlr as the
    proxy for agent state, to peek at sessions and keep agents on track,
    through a JSON API different from the phone's; a session is both the
    pane's scrollback and the agent's own transcript, found from herdr's
    cwd, with the last few entries exposed; an MCP server as another
    client.
  - herdr's snapshot carries each pane's `cwd` (`libherdr::protocol::PaneInfo`).
  - Claude Code session files on this machine (2026-10-22):
    `~/.claude/projects/<cwd with "/" replaced by "-">/<session uuid>.jsonl`,
    one JSON object per line; `type` is `user`, `assistant`, `system`,
    `attachment` and bookkeeping types; `user` and `assistant` lines carry
    `message.content` as a string or a list of blocks (`text`, `thinking`,
    `tool_use`, `tool_result`), with `cwd` and `timestamp` on each line.
  - Codex session files on this machine (2026-10-22, CLI 0.147.0):
    `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`; the
    first line is `session_meta` with `payload.cwd`; `response_item` lines
    have `payload.type` `message`, `reasoning`, `function_call` and
    `function_call_output`.
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (this depends on
    it)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (carries this
    protocol between machines)
---

# The handlr.harness protocol, session peek and the MCP adapter

## 1. Goal

A program, such as a T3 Code plugin or a coding agent through MCP, can ask
handlr which agents are running and in what state, read the last few
entries of what each agent has been doing in its own transcript, and ask
for a command to be sent to one. Sending goes through the same policy core
and confirmation broker as the phone, so a program can ask but, unless
configured otherwise, cannot approve its own request; the phone does.

## 2. Decisions

1. **`handlr.harness` version 1.** JSON-RPC 2.0 over the daemon socket,
   or over the link for a client paired with `protocols = ["harness"]`.
   It has the common methods of plan 01 and adds:

   | Method | Params | Result |
   |---|---|---|
   | `agents.list` | `{with_session?: bool}` | plan 01's `AgentRef`s, each with `session: {kind, path, updated} \| null` when asked |
   | `session.peek` | `{pane_id, entries?, scrollback_lines?}` | `{session: {kind, path, entries: [Entry]} \| null, scrollback}` |

   `entries` is 1–50 (default 10); `scrollback_lines` is 0–200 (default
   40), read with `agents.read`. Events are plan 01's topics; the
   supervision plan adds its own.

2. **Entries**, the same shape for every agent:

   ```rust
   pub struct Entry {
       pub ts: String,              // RFC 3339, from the file
       pub role: Role,              // User | Assistant | Tool
       pub kind: EntryKind,         // Text | ToolUse | ToolResult
       pub tool: Option<String>,    // tool or function name
       pub summary: String,         // at most 500 chars, cut at a char boundary with "…"
   }
   ```

   Thinking and reasoning blocks are left out. A `tool_use` summary is the
   tool name and its input as compact JSON, cut to length. A `tool_result`
   summary is its text, cut to length, with `is_error` shown as a leading
   `error: `.

3. **Session readers.** `src/sessions/` in handlr:

   ```rust
   pub trait SessionReader: Send + Sync {
       fn kind(&self) -> &'static str;                  // "claude_code", "codex"
       fn locate(&self, cwd: &Path) -> Option<PathBuf>;  // the newest session for this cwd
       fn tail(&self, path: &Path, n: usize) -> Result<Vec<Entry>, String>;
   }
   ```

   The reader is chosen by the pane's agent name: `claude` → Claude Code,
   `codex` → Codex; any other agent has no session, and `session.peek`
   returns only the scrollback.
   - **Claude Code.** `locate` maps the cwd to the project directory by
     replacing every `/` with `-` (and `.` with `-`), and takes the
     `*.jsonl` there with the newest modification time. If that directory
     does not exist, it scans `~/.claude/projects/*/` for files whose
     first line with a `cwd` names this cwd. `tail` reads `user` and
     `assistant` lines into entries per decision 2, skipping every other
     `type`.
   - **Codex.** `locate` walks `~/.codex/sessions/` from the newest day
     backwards, at most 14 days, and takes the newest rollout whose
     `session_meta.payload.cwd` is this cwd. `tail` reads `response_item`
     lines: `message` (by its role), `function_call` → `ToolUse`,
     `function_call_output` → `ToolResult`, skipping `reasoning`.
   - **Reading from the end.** `tail` reads the last 256 KiB of the file
     and parses whole lines in it, reading a further 256 KiB back (up to 2
     MiB) only if that gave fewer than `n` entries. Session files grow to
     many megabytes, and are never read whole.
   - Unknown line types and malformed lines are skipped, not errors: both
     formats change between agent versions.

4. **Where sessions are read.** Session files are read on the machine the
   daemon runs on. A pane on a remote herdr (SSH transport) has no local
   session file; `session.peek` returns `session: null` for it until a
   later plan runs handlr next to that herdr.

5. **Harness principals.** Local harness clients follow plan 01 (same uid,
   `can_approve` from `local_approve`). Paired ones follow `auth.toml`
   (plan 02), `approve` defaulting to `false`.

6. **`handlr mcp`.** An MCP server over stdio, using the official Rust MCP
   SDK (`rmcp`, pinned to its current release when this step starts). It
   connects to the daemon socket as `local:mcp` speaking
   `handlr.harness`, and exposes tools that map one to one:

   | MCP tool | Harness method |
   |---|---|
   | `list_agents` | `agents.list {with_session: true}` |
   | `peek_session` | `session.peek` |
   | `read_pane` | `agents.read` |
   | `send_to_agent` | `agents.send`; a `pending` result tells the model the user must approve it on their phone, and gives the `confirm_id` |
   | `confirmation_status` | waits up to 30 s for `confirm.resolved` on a `confirm_id` and returns the outcome, or `pending` |

   If the daemon is not running, every tool returns an error that says so.
   The adapter holds no state of its own.

## 3. Steps

1. **Session readers.** `src/sessions/` per decisions 2 and 3, with
   fixtures cut from real files (paths and text replaced) under
   `tests/fixtures/sessions/`: Claude Code with string and block content,
   tool use and results, an error result, and bookkeeping lines; Codex
   with messages, function calls and outputs and reasoning; a file whose
   last 256 KiB holds fewer than `n` entries; a malformed line.
   **Done when:** `cargo test -p handlr sessions` passes, and a test reads
   a 50 MiB generated file's last 10 entries without reading more than
   512 KiB of it.

2. **The harness methods.** The `harness` types in `libhandlr_proto` and
   the two methods in the daemon.
   **Done when:** `handlr ctl session.peek '{"pane_id":"…"}'` on a pane
   running Claude Code in this repository returns its last 10 user,
   assistant and tool entries, matching the end of its JSONL file, plus
   the scrollback; on a Codex pane, the same from its rollout; on a shell
   pane, `session: null` and the scrollback.

3. **`handlr mcp`.** Decision 6.
   **Done when:** with `handlr mcp` configured as an MCP server in Claude
   Code, the model lists the agents with their sessions, peeks one, and
   calls `send_to_agent`, which returns `pending`; approving on the phone,
   or with `handlr ctl confirm.reply` under `local_approve`, sends the
   command once, and `confirmation_status` then reports `sent`.

## 4. Open questions

1. Is the cwd enough to pick the session when two agents of the same kind
   run in one directory? The newest file may belong to the other pane.
   Matching on the session id an agent shows in its pane, where it shows
   one, would settle it.
2. Should `session.peek` also return the agent's current plan file once
   the supervision plan infers it, or is that a supervision method?
3. Which of T3 Code's needs belong here and which in its plugin, once its
   plugin surface is known?

## Amendment — 2026-10-22 12:06 EDT: action labels, status by label, and a skill

Agreed with the owner. This amendment changes decisions 1, 5 and 6 and
steps 2 and 3, and adds step 4. It follows plan 01's amendment of the same
time (approval phrases, action labels and lifetimes). Where it disagrees
with the body, this amendment wins.

**Labels are required here (decision 1).** In `handlr.harness`,
`agents.send` requires `label` (plan 01's format). A request without one
gets `-32602` (invalid params). The confirmations a program asks for may
wait up to an hour, and the label is what lets the program, and the user,
tell them apart.

**Principals (decision 5).** Harness principals cannot approve. Local ones
can, but only with the approval phrase (plan 01), which handlr never
gives to the requester.

**MCP tools (decision 6), replacing two rows of the table.**
- `send_to_agent {pane_id, command, label}` maps to `agents.send`, with
  `label` required. A `pending` result tells the model that the user
  approves it on their phone, and repeats the label.
- `confirmation_status {label}` (or `confirm_id`) maps to `confirm.status`.
  It waits up to 30 s for a resolution, and otherwise returns `pending`.
- The adapter exposes no approving tool.

**Step 4 (new): the handlr skill.** `skills/handlr/SKILL.md` in this
repository is a Claude Code skill used alongside `handlr mcp`. It tells
the model to:
- write a label that names the action and why, in lowercase words joined
  by `-`;
- tell the user the request is pending on their phone, giving the label;
- check back with `confirmation_status` by label, including after its
  context has been compacted;
- never ask the user for the approval phrase, and never try to approve
  its own request.

The README explains how to install it into `~/.claude/skills/`.

**Done when:** with the skill and `handlr mcp` installed in Claude Code,
asking it to have another agent compact gives a `send_to_agent` call
whose label describes the action; the phone speaks the label; after
`/compact` in the requesting session, the model finds the outcome by
label.

**Step 3's Done when** approves on the phone, or with `handlr ctl
confirm.reply` given the approval phrase, in place of `local_approve`.

## Amendment — 2026-10-22 16:23 EDT: sessions found by herdr's agent_session id

Agreed with the owner. This amendment replaces how decision 3 locates a
session, adds the session id to results, and settles open question 1.
Where it disagrees with earlier text, this amendment wins.

herdr's snapshot carries `agent_session` on each pane, for example
`{"source": "herdr:claude", "agent": "claude", "kind": "id", "value":
"<uuid>"}`. It does so for Claude Code and for the Antigravity CLI
(`agy`). On 2026-10-22, the value for the pane running this session
matched both `CLAUDE_CODE_SESSION_ID` in its environment and the name of
its JSONL file.

- **`libherdr`.** `PaneInfo` gains `agent_session:
  Option<AgentSession>`, with `source`, `agent`, `kind` and `value`. It is
  taken out of `extra`. `PaneInfo` is an exhaustive public struct, so this
  is a minor version bump of libherdr under Rule 5.4, with its
  `VERSIONS.md` entry.
- **By id.** `locate` takes the pane's `agent_session.value` when `kind`
  is `id`.
  - Claude Code: `~/.claude/projects/<encoded cwd>/<id>.jsonl`, and if
    that file does not exist (the cwd changed), `~/.claude/projects/*/<id>.jsonl`.
  - Codex: the rollout whose file name ends in `-<id>.jsonl`, searched
    back at most 14 days.
- **By cwd, as a fallback.** The newest-by-cwd rule of decision 3 applies
  only when herdr gives no `agent_session`. Results found that way are
  marked `matched: "cwd"`; the others are marked `matched: "id"`.
- **Results.** The `session` object in `agents.list` and in
  `session.peek` gains `session_id` and `matched`.
- **Antigravity.** A reader for `agy` sessions is left for later. Its file
  format has not been looked at.

Open question 1 is settled: the session id identifies the session even
when several agents of the same kind share a directory.

## Amendment — 2026-10-23 02:28 EDT: approved; session context for Gemini first; living sections

The owner approved this plan, and asked to keep it open for amendments.
This amendment does four things:
- reorders the steps;
- adds the session context for Gemini, pending questions and the agent
  view;
- adds `recent-unwrapped` for shell panes;
- names living sections.

The status stays `Draft` until the first implementing commit sets it to
`In implementation` (Rule 1.5).

**Why.** On 2026-10-23, `herdr pane read` of a Claude Code pane returned
exactly the visible frame: 43 lines, byte-identical for `visible` and for
`recent` with `--lines 200`. A screenshot of the same pane in Termux
matched it. Claude Code draws full-screen, so herdr holds no scrollback
for it. Gemini's turn context, the last 50 terminal lines, therefore never
contains an agent's earlier replies or the question it is waiting on. The
agent's transcript does, which is what decision 3's readers already read.

**Living sections (Rule 1.6).** These stay open after implementation
starts. New entries are appended as amendments, and an entry is fixed
once it is implemented:
- **L1, session sources:** which agents have readers and how each is found
  (decision 3, the 16:23 EDT amendment, and this one).
- **L2, Gemini's session context:** what goes into the turn, its limits,
  and redaction (decision 7 below).
- **L3, the agent view:** what the companion shows (decision 9 below).
- **The open questions** (section 4).

**7. The session context for Gemini (new; living section L2).**
`HerdrContextContributor` adds a `### Agent sessions` section to the turn
for some agents:
- **Which agents:** the focused agent; any agent the utterance names, by
  the naming plan's names and aliases; and any agent with a pending
  question.
- **What each gets:** its spoken name, its last 6 `Text` entries (user and
  assistant, in order, each cut to 800 characters), one line counting the
  tool calls in between ("12 tool calls"), and its pending question if any.
- **Limits:** 6 KB per agent and 12 KB in all. The oldest entries are
  dropped first, and the pending question is always kept.
- **Redaction, before anything is added:**
  - strings that look like secrets are replaced with `[redacted]`: API
    keys (`AIza…`, `sk-…`, `ghp_…`, `xox…`), PEM blocks, JWTs, and runs
    of more than 40 base64 or hex characters;
  - any line that mentions a path under `.private/` or a `.env` file has
    its value removed.
- **Shell and other panes without a session** keep the terminal tail, now
  read with `source: recent-unwrapped` and 80 lines (`libherdr` gains the
  `source` parameter).

**8. Pending questions (new).** An agent's pending question is the final
paragraph of its last assistant `Text` entry, when:
- that paragraph ends with `?`, or contains a line that does; and
- no user entry follows it; and
- the pane's `agent_status` is `idle` or `blocked`.

It is carried as `pending_question: {text, ts}` (the text cut to 400
characters, after redaction) in:
- the session object of `agents.list`;
- `session.peek`;
- the Gemini context.

When a watched agent goes from `working` to `idle` or `blocked` with a
new pending question, the companion is sent a notice: "<spoken name> is
asking: <the question, cut to 200 characters>". It goes through the
existing notice queue and its rules for when notices are spoken.

**9. The agent view (new; living section L3).**
- **Access:** `session.peek` is also available to companion principals.
  It is read-only.
- **The view:** the app shows an agent's entries in a terminal style: a dark
  theme, the user's turns on a grey band, the agent's replies in plain
  text, tool calls as dim one-line summaries, and the pending question
  highlighted at the end.
- **Opening it:** tap the focus line, or an agent's notice.
- **The font:** libmsdf's terminal preset (Noto Sans Mono with the symbol
  page, requested of libmsdf on 2026-10-23). Until it is released, the app
  uses its current font.
- **Answering:** the view's "Take a message" button starts dictation (plan
  20261023_02) with this agent as the fixed target.

**Steps, reordered (replacing section 3's order):**
1. **Session readers.** The old step 1, with the 16:23 EDT lookup by id.
   It adds the pending-question rule (decision 8) and the redaction
   (decision 7), each with tests:
   - the 2026-10-23 transcript's last reply, ending in a question, gives a
     pending question;
   - each secret pattern is redacted.
2. **Gemini's session context** (decision 7) and `recent-unwrapped` for
   shell panes.

   **Done when:**
   - a test turn with the mock context contains the `### Agent sessions`
     section within its limits;
   - live, asking the phone "what is the handlr agent waiting on?" while
     this session ends on a question gets that question back in substance.
3. **Pending-question notices** (decision 8).

   **Done when:** a watched mock agent going idle with a question gives
   one notice, and the same question does not give a second.
4. **The agent view** (decision 9), in the companion protocol,
   `handlr-companion` and `apps/companion`.

   **Done when,** on the S23: open the handlr agent's view, see its last
   replies and its pending question, and take a message to it from there.
5. **The harness methods:** the old step 2.
6. **`handlr mcp`:** the old step 3.

## Amendment — 2026-10-23 02:34 EDT: the agent view as terminal scrollback with priority cards (living section L3)

On the owner's description (2026-10-23). This replaces the "view" bullet
of decision 9. Access, opening, the font and answering stand.

**One flow per agent.**
- **Scrollback formatted as a terminal.** The agent's transcript (from the
  step 1 readers) is laid out at the phone's width, wrapped by the app, so
  nothing breaks at a fixed column:
  - dark background with the bright terminal colours;
  - the user's turns on a grey band;
  - the agent's replies in plain text;
  - tool calls as dim one-line summaries;
  - diff lines in green and red, errors in red;
  - the terminal font when libmsdf 0.5.0 is released.
- **Finger scrolling** through the whole session, with momentum, as in
  Termux and herdr today. The view follows the newest entry at the bottom
  until the user scrolls up, and follows again once scrolled back to the
  bottom.

**Priority cards, in the same flow.** A card sits at its place in the
conversation, but is visually distinct: a raised panel, an accent edge,
larger text, and its actions on it. Cards, highest priority first:
1. **Approval needed:** a handlr confirmation pending for this agent, with
   Approve and Deny.
2. **The agent is asking:** its pending question (decision 8), with "Take a
   message".
3. **Something broke:** a tool result with `is_error`, failing tests in a
   tool result, or the pane's `agent_status` being `blocked`.
4. **Finished:** the agent's turn ended (`working` to `idle`), with the
   final reply's first paragraph.

A card is **live** until it is resolved:
- the confirmation is resolved;
- a user entry follows the question;
- a later tool result succeeds;
- the agent works again.

Once resolved, it fades to ordinary scrollback styling, so only live cards
stand out.

**Detection** is by these rules, from data handlr already has: the broker,
the readers, herdr's status. Nothing extra is computed.

**Open questions, in this living section:**
- L3-1. Should the highest live card also stay **pinned** at the edge of the
  screen while the user scrolls back, or appear only in the flow?
- L3-2. **Rules only, or a model too?** A model (Gemini now, or the local
  model later) could flag "the thing that matters right now" in replies
  that match no rule, at the cost of a call per agent turn.
- L3-3. What else did the owner mean to add? The description ended mid-
  sentence ("So make…").

**Step 4's "Done when"** gains:
- on the S23, the handlr agent's view scrolls back through its session in
  terminal format;
- a pending confirmation and a pending question each show as a live card
  in the flow, and fade when resolved.

## Amendment — 2026-10-23 02:34 EDT: L3-1 settled: pinned to the bottom, scrolling overrides (living section L3)

On the owner's answer: "pinned to bottom but scroll overrides". This
settles open question L3-1.

- **Pinned to the bottom.** The highest-priority live card is pinned to the
  bottom of the agent view, above the buttons, whether or not its place in
  the flow is on screen. With several live cards, only the highest is
  pinned; the others stay in the flow.
- **Scrolling overrides.** The pin never moves the view or takes a scroll.
  While the user's finger is scrolling back, the pinned card shrinks to a
  one-line bar (its kind and summary), so it doesn't cover the
  scrollback. Tapping the bar jumps to the card in the flow. Once the view
  is back at the bottom, the card shows in full again. New entries never
  pull the view down while the user is scrolled back.
- **When it goes:** when the card is resolved, it leaves the pin, and the
  next live card, if any, takes its place.

Step 4's "Done when" gains:
- with a live card and the view scrolled back, the bar shows, scrolling is
  never interrupted, and tapping the bar jumps to the card.

## Amendment — 2026-10-23 02:35 EDT: L3-3 settled: nothing more to add (living section L3)

The owner's description ended mid-sentence because the companion app
restarted during an install. The rest was only "make a plan", which became
the amendment of the agent view itself. There is nothing more to add, and
open question L3-3 is closed.

## Amendment — 2026-10-23 02:36 EDT: L3-2: rules for now, a model decided later (living section L3)

On the owner's answer: "rules for now, decide on a model later".

- **Built now:** priority cards are detected by the four rules only.
- **Deferred:** a model-flagged "worth a look" card stays in L3 as a
  later entry, to be added by amendment once the rules have been lived
  with. It would rank below the four rules, run only when an agent's turn
  ends, and read only the final reply, preferably on the local model.

## Amendment — 2026-10-23 02:52 EDT: implementation started; steps 1, 2 and 3 done

The status is `In implementation`. Outside the living sections L1, L2 and
L3 and the open questions, this document is now immutable (AGENTS.md, Rules
1.5 and 1.6).

**Commits:**
- libherdr: `0a51286` (rustfmt only), then `786a09a` (0.2.0, tag
  `libherdr-v0.2.0`, `agent_session` and the read source);
- `libhandlr_proto`, then handlr, in this batch.

**Decided where the plan was open,** recorded under the living sections
where they belong:
- **L1, session sources:**
  - **The reader's interface.** `locate(cwd, session_id)` returns the
    path, the id and how it was matched.
  - **No fallback after an id.** When herdr gives a session id and no file
    matches it, there is no session, rather than the newest file for the
    directory.
  - **Claude Code's directory names.** Every character that is not an
    ASCII letter or digit becomes `-`.
  - **Codex's 14 days** are the 14 newest day directories.
  - **Skipped as not conversation:**
    - Claude Code sidechain, meta and compaction-summary lines, and lines
      whose origin is a task notification;
    - user text that is only a harness tag;
    - Codex developer and system messages.
- **L2, Gemini's session context:**
  - **Which agents:** the focused one, those the utterance names (by full
    name, or by project or tab as whole words, and by the bare agent name
    only when it is unique), and idle or blocked agents with a pending
    question.
  - **Over the limit,** the oldest entry of any agent is dropped first.
  - **The tool calls** are one line: "(N tool calls in between)".
  - **Terminal tails:** a pane with a session block gets no terminal tail
    unless it is blocked, since permission prompts show on screen and not
    in the transcript. Every tail is read with `recent_unwrapped` and 80
    lines.
- **Decision 8, pending questions:**
  - tool entries after the reply do not clear the question;
  - trailing `* _ \` " ' ) ]` and curly quotes are ignored when looking
    for the `?`;
  - a question notice replaces the status notice for the same event;
  - repeats are suppressed per pane and per connection.
- **Redaction:**
  - PEM blocks;
  - `.private/` and `.env` lines;
  - API keys by prefix (`AIza`, `sk-`, the GitHub token prefixes,
    `xox?-`);
  - JWTs;
  - hex runs over 40 characters;
  - mixed base64 runs over 40 characters.

  A tool result whose call names a `.private/` or `.env` path is
  redacted whole.

**Still to do:**
- **Step 2's live check:** ask the phone what the handlr agent is waiting
  on.
- **Step 4**, the agent view, which the owner has approved.
- **Steps 5 and 6.**

## Amendment — 2026-10-23 03:11 EDT: step 4 built; how the agent view behaves (living section L3)

Step 4's code is committed in this batch. Its S23 checks follow on the
phone. This records under L3 what the plan left open.

- **`session.peek`:**
  - **Who may call it:** companion connections and the local daemon
    socket. A paired harness gets -32001 until step 5.
  - **Arguments:** `entries` is 1 to 200, default 40.
  - **No session:** a shell, a remote herdr or a missing file give
    `session: null`.
  - **In `agents.list`:** `AgentInfo.session` carries the session id, how
    it was matched, and the pending question.
- **Refreshing:**
  - a peek when the view opens, and again at once after a reconnect;
  - then every 5 s while the agent works or is blocked, and every 15 s
    while it is idle;
  - at once on that agent's notice or status change;
  - never two within 5 s, and one in flight at a time.

  Polling is needed because notices only come for watched panes. The view
  asks for 200 entries.
- **The cards as built:**
  1. **Approval:** each queued confirmation for the pane. Its card answers
     that confirmation even while another is being asked.
  2. **Question:** placed at the reply it belongs to.
  3. **Broken:** either the pane is blocked, or the last tool result of the
     last turn starts `error: ` or shows failing tests (`test result:
     FAILED`, `N failed` with N > 0, or `FAIL` / `--- FAIL` lines).
  4. **Finished:** the status is idle or done and the last text is a reply.
     It is not shown when a Question card is already at that reply.

  A card that the rules no longer find is resolved, and stays as one dim
  line at its place. At most 32 are kept.
- **Scrolling** is the app's own, since libcontrols has no scroll view:
  - a 10 pt slop tells a drag from a tap;
  - a fling slows as e^(−3.2 t), with no fling after a 100 ms rest;
  - the view is clipped, and only visible rows are drawn;
  - scrolled back, the view stays on its top entry as old entries leave
    the window.
- **The pin:** the flow always ends above the full pinned card, so
  switching between the card and its one-line bar never moves the flow.
  Tapping the bar puts the card a third of the way down the view.
- **"Take a message"** starts dictation in the view, with that agent as
  the fixed target. "Take a message" said while the view is open targets
  it too.
- **Closing:** the Back button, Android's Back, or Escape on the desktop.
- **Not done:** opening the view from a notice, since notices are spoken,
  with nothing on screen to tap.

## Amendment — 2026-10-23 16:02 EDT: the agent view's controls, suggestions, message card, read aloud and scroll hold (living section L3)

Agreed with the owner on 2026-10-23, in the order asked, and built that
day. It records in this living section what the CHANGES.md entries of
15:01, 15:11, 15:23, 15:29, 15:39, 15:49 and 15:59 EDT recorded as
departures. Where they disagree with the 03:11 EDT amendment, this one
wins: it replaces that amendment's bottom row (Back and "Take a message"),
its "Back button" under **Closing**, and its "Take a message" button.

**The header.** Back is a `‹` glyph (U+2039, from the terminal face) at the
left of the title row; the left 44 pt of the header is its tap area.
Android's Back and Escape still close the view.

**The bottom row controls the audio** ("the bottom row to control the
audio, with the left having a square play button, the right a square
record button"; "play doesn't become send, record becomes send"; "the
record/send action should always be in the bottom right per Fitz"). All
its buttons are 52 pt squares:
- not taking a message: play at the left, record (a white dot on blue) at
  the right;
- taking a message: play at the left, cancel (`✕`) in the middle, send
  (`➤`, on green) in record's place at the bottom right.

Scratch has no button; "scratch that" still works by voice. A microphone
glyph for record is for later; no bundled atlas has one.

**Suggested responses** ("a \"suggested response\" row with yes, no,
continue, compact (if needed)"; "just above the bottom buttons, plain text
in a different color"; "suggestions that belong in a card can go there"):
- `handlr_companion::Suggestion` is `yes`, `no`, `cont` and `cmpct`, sent
  to the view's pane by one `agents.send` (yes and no as messages,
  `continue` and `/compact` as commands);
- the row, plain text just above the bottom row, shows `cont` while the
  pinned live card is Something broke or Finished, and nothing while
  working, taking a message, or when the card has its own buttons;
- a Question card's actions are Yes, No and a record button; an
  Approval card keeps Approve and Deny;
- yes green, no red, the commands yellow;
- `cmpct` waits for the context's use, which the daemon does not report
  yet; Claude's numbered options (`1.`, `2.`) as taps are for later.

**The message card** ("the entered text should be on a pinned card with
the scrolling pane pushed up and out of the way. the same applied to an
inline message, it expands the card being responded to with the same
buttons in place, hiding the global ones"):
- while a message is taken, its text is on a card pinned above the bottom
  row; the agent's pinned card makes way, and the flow ends above it;
- a Question card's record button takes a message in reply to that card:
  the card is pinned at the bottom, expanded with the message under its
  title and text and the same row inside it, and the screen's row is
  hidden.

**Read aloud** ("add a play button (dull/bright) on the left, this will
play or not the incoming white text boxes as voice"; "tapping the play
while it's speaking should stop the current playback (skip)"):
- play, dull when off and bright when on, reads the view's new replies
  (assistant text only, never tool calls, their output or code blocks)
  from when it is turned on, after speaking, listening and confirmations;
- while something is being said, play shows `▶│` and a tap skips to the
  next reply;
- what is read follows the reading rules: a link as its text, a path as
  "the" and the English words of its last part, inline code as its words,
  a number or version as it is, a commit hash not at all;
- complex replies are summarised by the daemon under plan 20261023_05.

**Own approvals** ("a separate approval isn't needed for UI entered
buttons or when text has been buffered for review first"): a tapped
suggestion, and a message submitted from the view by its button or its
silence, approve their own confirmation; the daemon still makes it. Plan
20261023_02's side of this is its 15:23 EDT CHANGES.md entry.

**Scrolled back, the view holds** ("if I'm scrolled back, don't force the
scroll"): when the entry at the window's top leaves the 200-entry window,
the view stays on the next entry still there; entries are matched by time,
role and kind, so a summary that changes between peeks keeps the place.

**Questions at the end of a task** ("if the question stops as a task, it
should be a pinned card"): a reply's final question is pending (decision
8) while the pane is `done` as well as `idle` or `blocked`; its Question
card is the pinned one.

**"(loading session)"** shows until the session first answers; "(no
session to show)" only after it answered with nothing.

**Code layout** ("one control per file"): the controls are component
structs in `apps/companion/src/agent/` (`control_row.rs`,
`dictation_card.rs`, `square_button.rs`); plan 20261023_04 moves those
used by more than one screen to `src/controls/`.

**Implemented by:** `f76ca33`, `3bc21df` (back, suggestions); `ae8b110`
(scroll hold); `a68beee`, `0496df6`, `0ec7a2e` (Submit waits, read aloud,
play and record); `979e732`, `c8ebd95` (own approvals, the audio row, the
message card); `73668af` (record becomes send); `3ba67c9` (the reading
rules); `d8e62e2` (skip); `2e70b69`, `41cfda1` (loading); `05561dd`
(questions when done).

## Amendment — 2026-10-23 16:16 EDT: a tapped Yes or No carries its question; the pinned card is shown once (living section L3)

The owner (2026-10-23): "the \"yes\" should carry the context handle when
typed, either a phrase such as \"Yes, install it on the S23\" or the bip
handle. the button should only show \"yes\""; and "got two cards instead of
one with the same content". This adds to the 16:02 EDT amendment's
**Suggested responses** and replaces the 03:11 EDT amendment's "Tapping
the bar puts the card a third of the way down the view".

- **Yes and No answering a question** send the answer with the question's
  context (`handlr_companion::answer_text`), from the last sentence of the
  pending question that asks: "Install it on the S23?" sends "Yes, install
  it on the S23" or "No, don't install it on the S23"; "Should I …",
  "Shall I …", "Do you want me to …" and the like are turned into the
  request ("Yes, write the amendment now"); a question that is not a
  request sends "Yes, re: is that what you meant". Only the message
  filter's characters are kept, at most 200. The buttons still read Yes
  and No. With no pending question, they send "yes" and "no" as before.
- **The pinned card is shown once,** at the bottom; its place in the flow
  is kept with no height, and tapping its bar while scrolled back returns
  to the bottom.

## Amendment — 2026-10-23 16:19 EDT: buttons press in (living section L3)

The owner (2026-10-23): "a visual effect like pressing it should have like
a depress effect or something like that". Every button in the agent view
(the bottom row's squares, a card's buttons, the message card's controls)
and every suggestion is drawn pressed in while a finger is on it: 2 pt
lower and its colour at 70%. A tap's press shows for 120 ms after the
finger lifts, so a quick tap is seen; a drag past the 10 pt slop is not a
press. `Look` and `depressed` are in `apps/companion/src/agent/square_button.rs`.

## Amendment — 2026-10-23 16:19 EDT: buttons press in, withdrawn (living section L3)

The owner (2026-10-23): "actually, that's better to recommend to
libcontrols so it can plan the feature, it doesn't belong in handlr (the
press effect)". This withdraws the previous amendment, "buttons press in":
its code (`b54544b`) is reverted, and a pressed look is recommended to
libcontrols to plan as its own feature. The app takes it from a libcontrols
release when there is one.

## Amendment — 2026-10-23 16:25 EDT: when continue is suggested (living section L3)

The owner (2026-10-23), asking why `cont` showed after a finished turn: "got
it, it should probably also show after an idle period has passed, such as
30s". This replaces the 16:02 EDT amendment's "the row … shows `cont`
while the pinned live card is Something broke or Finished":
- with a live **Something broke** card, `cont` shows at once;
- after a **Finished** turn, or with no live card, it shows only once the
  agent has been quiet for 30 s (no new entries and no change of status),
  so a turn that simply finished is not offered a continue;
- never while working, taking a message, or when the pinned card has its
  own buttons (a question's Yes and No, an approval's Approve and Deny).

## Amendment — 2026-10-23 16:32 EDT: a question ending a quiet turn is pending whatever herdr says (living section L3)

The owner (2026-10-23), after no Question card appeared: "I didn't get the
question card, is it delayed?", then "yes" to the fix. herdr reports a
Claude Code pane `working` while one of its background agents runs, after
its turn has ended, so decision 8's status test hid the question. A reply's
final question is now also pending when the turn has visibly ended: that
reply is the session's last entry and the session file has been unwritten
for 5 s (`pending::TURN_QUIET`, `Located::quiet`), whatever herdr's
status. Every place that finds the pending question uses it: the agent
view, the notices and Gemini's session context.

Not yet: the Finished card follows herdr's `idle` or `done` in the
client's rules, so a turn that ended without a question while a background
agent runs shows no Finished card. That needs the daemon to tell the
client the turn ended, a protocol field, which waits for
libhandlr_proto's next release.

## Amendment — 2026-10-23 16:39 EDT: a tapped answer dismisses its card and shows as typed input (living section L3)

The owner (2026-10-23), after a second tap on a card that was still shown
sent a plain "yes": "tapping yes/no on a card should dismiss the card on
the phone side (so immediate) after the tap animation plays. let's convert
the responses to typed amber text with the input box".
- A tapped Yes or No dismisses its Question card at once, on the phone,
  before the daemon or the session has answered; a second tap has no card
  to land on. If the send is refused, the card comes back.
- Every response sent from the view (Yes, No, cont) shows as typed input:
  amber text on the terminal grid after a `›` prompt, in a bordered input
  box above the controls (`apps/companion/src/agent/input_box.rs`), at
  most three lines. It goes when the session shows it as the user's entry,
  when the agent works, or when the send is refused.
- "after the tap animation plays": the press effect is libcontrols' to plan
  (the 16:23 EDT amendment); until it ships, the card goes on the tap.

## Amendment — 2026-10-23 16:39 EDT: a correction to the previous amendment's reference (living section L3)

The previous amendment cites "the 16:23 EDT amendment" for the press
effect; the amendment it means is "buttons press in, withdrawn", at 16:19
EDT.

## Amendment — 2026-10-23 16:44 EDT: answering a question skips its reading (living section L3)

The owner (2026-10-23): "if an answer is to a spoke question, it should
skip the question playback". When a Yes or No answers the pending
question, the reply it belongs to is not read aloud: if it is waiting to be
read (for its summary or behind other speech) it is dropped, and if it is
being read, the speech stops (`Event::StopSpeaking`), as skip does.

## Amendment — 2026-10-23 17:33 EDT: a dictated message shows in the flow once sent (living section L3)

The owner (2026-10-23): "Spoken messages should be included as text in the
flow after they're submitted". A dictated message sent to the viewed agent
shows as typed input (the 16:38 EDT amendment's amber input box) from the
moment it is sent until the session shows it as the user's entry in the
flow, where it then stays as ordinary scrollback.

## Amendment — 2026-10-23 17:34 EDT: a correction to the previous amendment's reference (living section L3)

The previous amendment cites "the 16:38 EDT amendment's amber input box";
that amendment, "a tapped answer dismisses its card and shows as typed
input", is at 16:39 EDT.

## Amendment — 2026-10-23 17:39 EDT: code diffs drawn as the terminal interface draws them, never spoken (living section L3)

Agreed with the owner on 2026-10-23 (by voice; "code depths" read as "code
diffs", "the quad to be" as "the Claude TUI"): "we should include the code
[diffs] in the same way that they're shown in the [Claude TUI]"; "diffs
are never spoken"; "skip the 'changed three files' unless it's a summing up
of a long action and in prose"; then "Yes, write the diff amendment into
L3". This replaces, for edits, the 02:34 EDT amendment's "diff lines in
green and red".

1. **What is drawn.** An edit (Claude Code's `Edit`, `MultiEdit` and
   `Write`) is drawn from its result's `structuredPatch` hunks, as the
   terminal interface draws it:
   - a header, `Update(<path>)`, or `Write(<path>)` for a new file,
     with the path shortened to its last two parts;
   - each hunk's lines numbered from `oldStart` and `newStart`: removed
     lines red on a dark red band, added lines green on a dark green
     band, context dim;
   - a footer, `⎿ N additions, M removals`.
2. **Limits.** At most 20 lines of an edit show in the flow; tapping it
   shows all of its hunks, up to 400 lines, and tapping again folds it.
3. **The protocol.** `Entry` gains `diff: Option<Diff>`, with
   `Diff { path, kind (update or write), hunks: [{old_start, old_lines,
   new_start, new_lines, lines}] }`, each line redacted as entries are,
   and at most 400 lines. It goes in the next libhandlr_proto release
   after 0.3.0. Edits from agents whose readers give no hunks (Codex's
   patches, agy's tool results) stay as green and red lines until their
   reader supplies hunks.
4. **Diffs are never spoken, and edits are not announced.** Read aloud
   never reads an edit: its hunks never go to the reading rules or to
   Gemini, and what is spoken does not count or describe edits ("changed
   three files"). The one exception is the agent's own prose: a reply that
   sums up a long piece of work in prose is read or summarised like any
   other reply, its mentions of files included. The `spoken_summary`
   prompt in `ADDITIONAL_PROMPTS.md` gets the same rule (say what was done
   in the agent's own terms; never list or count edits); since plan
   20261023_05 is implemented, that change is recorded in CHANGES.md when
   it is made.
5. **One control per file:** `apps/companion/src/agent/diff_block.rs`.

## Amendment — 2026-10-23 19:09 EDT: a cut question expands when tapped (living section L3)

The owner (2026-10-23): "clicking an abbreviated question should expand
the text and card". The pinned card shows its first three lines and cuts
the rest with "…". When it is cut, a tap on the card (not on its buttons)
shows all of its text, growing the card upward as far as leaves the flow
120 pt; a second tap folds it. A different pinned card starts folded. A
card whose text fits has nothing to expand, and a tap on it does nothing.

## Amendment — 2026-10-23 21:57 EDT: plan questions get approve, improve and implement; repowiki links (living section L3)

Agreed with the owner on 2026-10-23 (by voice): "we detect when it's
asking about a plan and add in a[pprove], improve, implement option. Also,
can we add something like Repo Wiki prefix as a configuration option and
then create URLs from that?", then "Yes, write both as one L3 amendment and
build them".

**1. Plan questions.** The daemon marks a pending question
`about_plan` when it names a plan: a path or link to
`docs/design/…_PLAN.md`, or one of the words "plan", "amendment",
"draft" or "approve". It also gives the plan's spoken name, as the reading
rules read its path ("the overview screen plan"), as `plan` when one path
names it. `PendingQuestion` gains `about_plan: bool` and
`plan: Option<String>` (additive; libhandlr_proto's next patch).

A Question card that is `about_plan` offers **approve**, **improve** and
**implement** in place of Yes and No:
- **approve** sends "Approve the <plan>" ("Approve the plan" with no name);
- **improve** opens the message card with "Improve the <plan>: " already
  in it, to dictate the rest;
- **implement** sends "Approve the <plan> and start building it".

A tap is its own approval and dismisses the card, as Yes does. The
buttons read just the three words.

**2. Repowiki links.** A new `[repowiki] url` setting (for example
`"http://timothys-mac-mini-1.tailcd7df.ts.net:8088"`) turns on links;
without it nothing changes. The daemon maps an agent's directory to its
repowiki site id by repowiki's own rule (the directory's parent and own
name joined with `--`: `src--handlr`), and gives each entry and
question a `links` list: for every `docs/…` path in its text,
`{text, url}` with `url` = `<url>/#/<site>/<path without .md>`. In the
agent view such a path is drawn underlined in the link colour, and a tap
opens the URL in the phone's browser. Opening a link needs an outbound
"open this link" call that libmobile and libmorph do not have yet (they
only receive links); until a release has it, a tap does nothing and the
link is still shown. `Entry` and `PendingQuestion` gain `links`
(additive). The `.md` is left off, as repowiki's default route
(`/README`) is; the S23 check confirms it.

## Amendment — 2026-10-23 22:05 EDT: plain http and https links are links too (living section L3)

The owner (2026-10-23): "we should also detect other http/https links and
make them clickable". This adds to the previous amendment (2026-10-23 21:57 EDT), item
2: besides `docs/…` paths (which need `[repowiki] url`), every
`http://` or `https://` URL in an entry or the pending question is a
link, whether or not `[repowiki] url` is set. Only those two schemes
(never `file:`, `content:`, `intent:`, `javascript:` or others). A URL
ends at whitespace, without the punctuation that ends or wraps a sentence
(`.,;:!?)]}>"'`), keeping a `)` that balances a `(` inside it; a
Markdown link `[text](url)` is shown as its text. They are drawn and
tapped like the repowiki links.

## Amendment — 2026-10-23 22:58 EDT: labels fit their buttons; a plan question's words are text (living section L3)

The owner (2026-10-23): "button labels shouldn't be bigger than the
buttons, Yes/No are buttons but suggestions (approve, impl) can be plain
text". This replaces, in the 21:57 EDT amendment's item 1, the three words
as buttons:
- a plan question's **approve**, **improve** and **implement** are plain
  text, lower case, in green, the link blue and yellow, like the
  suggestion row; Yes and No stay buttons;
- a labelled button's text is 17 pt, or smaller to fit (down to 9 pt), so
  it never outgrows the button.

## Amendment — 2026-10-23 23:00 EDT: "discuss", not "improve" (living section L3)

The owner (2026-10-23): "improve was a typo, discuss would be better".
This replaces "improve" in the 21:57 EDT amendment and in the amendment
before this one: the plan question's middle word is **discuss**, and it
takes a message starting "Discuss the <plan>: ".

## Amendment — 2026-10-23 23:54 EDT: the scrollback is a ribbon of sections in serial order (living section L3)

The owner (2026-10-23), after scrolling back on the S23 with session
subscriptions (plan 20261023_03): "i was able to scroll back to start of
session but way too quickly"; "its just not consistent, it should be a
ribbon made up of in order sections"; then "yes" to this amendment. This
replaces how the agent view holds its place and lays itself out: L3-1's
"kept on the entry at its top", plan 20261023_03's flattening of sections
into the app's entries and cards (`Sections::flatten` feeding
`AgentScreen::set_content`), and the 22:55 EDT amendment's re-layout of
the whole flow on each change. What a section shows, the cards' rules, the
pinned card and the history paging (A7) stand.

Found on the S23 before this amendment. The app gets the sections
flattened into one entry list, resent whole on every change. It holds its
place by searching that list for the entry that was at the window's top by
time, role and kind, which several entries can share (a reply's parallel
tool calls), so the view can jump. It lays out every entry's text on each
change, and the phone logged 235 skipped frames, and a fling advances by
its speed times the frame time, so slow frames make it leap. The "loading
earlier…" row comes and goes at the top through the same search.

1. **The ribbon.** The app keeps the view's sections by serial, in serial
   order, as the client does. The scrollback is those sections laid end
   to end: each section is one block (its entries, or its card), and the
   ribbon's height is the sum of the blocks.
2. **Each block laid out once.** A block's height, rows, fills, buttons
   and links are laid out when its section arrives and cached by serial.
   They are laid out again only when that section is replaced with
   different content, when the width or scale changes, or for the pinned
   card's place. Adding, replacing or removing a section touches only its
   own block and the running heights after it.
3. **The place is a serial.** Scrolled back, the view's position is the
   serial of the block at the window's top and the distance from that
   block's top. Sections paged in above it, or a section replaced below
   it, never move it. If its section is removed, the place moves to the
   next section still held, at the same point on screen. Following the
   bottom is unchanged.
4. **The marker row has a fixed place.** The ribbon starts with one row
   of fixed height that is always there: "loading earlier…" while a page
   is on its way, "start of session" once the first line is reached, and
   blank otherwise. Its text changing never moves the view.
5. **The client gives sections.** `handlr-companion` gives the app the
   view's sections in serial order (with the hide rule for a just-answered
   card applied, and the pinned card's serial), and marks which serials
   changed since the last event, in place of the flattened entries and
   cards. The memory guard (rows only for the window and `KEEP_SCREENS`
   on each side) applies per block.
6. **A fling moves evenly.** A frame's time is capped at 1/30 s for the
   fling (in place of 0.1 s), so a slow frame cannot throw the view.

Tests: a page of sections landing above the window does not move it; two
sections whose entries share a time do not confuse the place; replacing a
section below the window does not move it, and removing the one at the
top keeps the next one at the same point; the marker row changing does not
move the view; a fling over the same speed covers the same distance
whether or not a page lands during it; and a delivery of one section lays
out one block (counted). On the S23: scrolling back through a long session
to its start moves smoothly at the finger's speed, with no jumps as pages
land.

## Amendment — 2026-10-24 00:05 EDT: a one-line "jump to end" message below the scrollback (living section L3)

The owner (2026-10-24): "the UI will have a jump to end one line message
like -- (message) -- below the scroll"; then "yes" to this amendment. It
adds to the previous amendment (the ribbon, 2026-10-23 23:54 EDT).

1. **When.** While the view is scrolled back (not following), one line
   sits just below the scrollback, above the suggestions and the
   controls. While following, it is hidden.
2. **What.** Dim text, centred: `-- <newest message> --`, the first line
   of the newest section's text (a turn's prompt, a reply, a tool call's
   one-line summary, or a card's title), cut to fit with "…". When
   sections have arrived since the view left the bottom, it leads with
   their count: `-- 3 new: <newest message> --`. With nothing new, it
   reads `-- jump to end --`.
3. **Tap.** Tapping it jumps to the bottom and follows again, as
   scrolling to the bottom does.
4. **Fixed height.** The line's height is reserved for the scrollback's
   window whether or not it shows, so it appearing or going never moves
   the scrollback.
5. **Source.** The newest section is always held, from its delivery: in
   outline mode (plan 20261024_01), its contents come inline with the
   newest slots.

Tests: the line is hidden while following and shown when scrolled back;
its text for nothing new, one new section and several; a long message
cut with "…"; a tap follows again at the bottom; and showing or hiding it
does not move the scrollback.

## Amendment — 2026-10-24 01:06 EDT: the jump line scrolls to the end in 2 s on an ease-in-out curve (living section L3)

The owner (2026-10-24): "jump to end should be a scroll, if new content
inject it below the old ribbon end and keep going"; "it should be the
proper speed for a constant time scroll, as in it should take 2s
regardless of distance"; "possibly with an accel curve"; then "yes". This
replaces item 3 of the 00:05 EDT amendment ("tapping it jumps to the
bottom"), and the tap's swap and jump in plan 20261024_01's departure of
00:36 EDT, item 3 (recorded in CHANGES.md).

1. **A timed scroll.** Tapping the jump line scrolls to the end in exactly
   2 s, whatever the distance. The position follows a cubic ease-in-out
   from where the view was to the end: it speeds up through the first
   second and slows through the second, with zero speed at both ends. The
   speed comes from the distance.
2. **A growing end.** The end is read again each frame. If it grows during
   the scroll, the curve keeps its timing and lands on the new end at 2 s.
3. **A touch stops it,** as it catches a fling. Contents are not fetched
   while it runs; placeholders fill in once it lands.
4. **Following again** once it lands.
5. **New content goes below.** While a new outline is held aside, the tap
   does not swap the ribbon. The new outline's sections whose serials come
   after the old ribbon's newest are added below its end, and the scroll
   carries on into them. Once the view is at the bottom and following, the
   rest of the new outline is merged in silently, and the view stays at the
   bottom.
6. **The label** is unchanged: `-- new content --` while a new outline is
   held aside, and `-- N new: … --` or `-- jump to end --` otherwise.

Tests: the scroll takes 2 s from one screen back and from 300; its
speed is zero at both ends and highest midway; an end that grows during
it is reached at 2 s; a touch stops it; it follows at the end; and with a
new outline held aside, the newer sections go below the old end, the
scroll reaches them, and the merge at the bottom does not move the view.

## Amendment — 2026-10-24 01:07 EDT: the jump line says what is at the end (living section L3)

The owner (2026-10-24): "the new should reflect what is, such as done or a
question"; then "yes". This replaces the jump line's text in item 2 of
the 00:05 EDT amendment (`-- <newest message> --`, `-- N new: … --`) and
item 6 of the previous amendment (`-- new content --`). The tap, the 2 s
scroll and new content going below all stand.

While scrolled back, the line shows the state at the session's end. The
first that holds wins:
1. a question is waiting: `-- question: <its first line> --`, in the
   question card's accent;
2. approval is needed: `-- approval: <the command> --`, in the approval
   card's accent;
3. something broke: `-- broke: <the error's first line> --`, in red;
4. the turn is finished: `-- done: <the reply's first line> --`, in green;
5. the agent is working: `-- working: <the newest section's first line> --`,
   dim;
6. nothing has arrived since the view left the bottom: `-- jump to end --`,
   dim.

The states come from the live cards and herdr's status, as the pinned
card and the overview's turn do. Each line is cut to fit with "…". A new
outline held aside has no text of its own: the state line covers it.

Tests: each state's text and colour, the order when several hold, a long
line cut with "…", and `-- jump to end --` when nothing is new.

## Amendment — 2026-10-24 10:40 EDT: placeholders as faint bars with a slight shimmer (living section L3)

The owner (2026-10-24): "outline boxes shouldn't look like missing content,
they should be light gray almost clear, maybe with a slight shimmer"; then
"Approve the plan". This replaces the placeholder of plan 20261024_01,
decision 7 ("a dim placeholder of its height, with a short '…' row"), as
built in step 5.

1. **No "missing" look:** no dim block and no "…" row.
2. **Faint bars:** a section's placeholder is faint rounded bars, one per
   estimated line, at the text's colour at about 6% opacity. The last bar
   of a block is shorter, as a paragraph's last line is. A card's
   placeholder is one faint rounded panel of its estimated height.
3. **A slight shimmer:** a soft band, about 4% brighter at its centre,
   sweeps across the visible placeholders from left to right every 1.6 s,
   the same phase for all. It runs only while a placeholder is on screen.
4. **Contents replace it in place,** and the view does not move, as now.

Tests: placeholders draw bars, not text; a card's draws one panel; the
shimmer's band moves with time and wraps every 1.6 s; and no frames are
asked for the shimmer when no placeholder is on screen.

## Amendment — 2026-10-24 11:09 EDT: two-finger scroll at 5×, and a proportional scrollbar while scrolling (living section L3)

The owner (2026-10-24): "lets make two finger scroll much faster, lets
also add a Mac style proportional scrollbar only on scroll 4px wide in
the same gray, it should show progress from the top to the calculated
length (if new content in timeline)"; "it could be a target when visible
for 2s after lifting finger, it would have horizontal slop (another 4px on
each side)"; then "yes, pts is correct". This adds to the ribbon and
jump-line amendments. One-finger dragging and flinging are unchanged.

1. **Two-finger scroll.**
   - A drag with two fingers down moves the view 5× the fingers' distance
     (their mean movement).
   - Its fling starts at 5× the release speed, under the same cap.
   - Lifting one finger mid-drag goes back to 1× from that point.
2. **The scrollbar.**
   - **Shape:** a 4 pt rounded thumb at the right edge of the scrollback,
     in the placeholders' gray (the text's colour) at about 35% opacity.
   - **When:** it shows on any drag, fling, jump scroll or thumb drag. It
     stays fully visible for 2 s after the finger lifts or the motion
     stops, then fades out over 0.3 s.
   - **Length:** the window's share of the ribbon's calculated length,
     never shorter than 24 pt.
   - **Position:** from the session's top to the end of that length.
   - **The calculated length:** every slot of the outline at its laid-out
     or estimated height. While a new outline is held aside, its sections
     after the old ribbon's newest are counted too, so the thumb stops
     short of the bottom at the old end.
3. **The thumb is a target while it shows.**
   - **Hit area:** the thumb's length, and 4 pt either side of it
     horizontally (a 12 pt strip).
   - **Dragging:** moves the view in proportion to the calculated length,
     so top to bottom travels the whole session. Contents load where it
     stops. Letting go leaves the view there, with no fling.
   - **Visibility:** it stays visible while dragged, and the 2 s starts when
     it is let go. A touch on the track outside the thumb scrolls as
     before.

Tests:
- a two-finger drag moves 5× and its fling starts at 5×, and lifting one
  finger goes back to 1×;
- the thumb's length and position at the top, middle and end, and its
  minimum;
- the length counts a held-aside outline's newer sections;
- it shows on scrolling, stays 2 s after lifting, then fades;
- a touch within 4 pt of the thumb drags it proportionally, with no
  fling;
- a touch outside the strip scrolls as before;
- it is not a target once faded.

## Amendment — 2026-10-24 11:32 EDT: the scrollbar's band works even when invisible, and a visibility setting (living section L3)

The owner (2026-10-24): "touching the scrollbar band should work as a
scrollbar even when invisible, we should have a Scrollbar visibility with
the arrow-based option selector in libcontrols (left/right arrows select
options) Disabled, Hidden, Always"; then "yes". This replaces item 3's
"the thumb is a target while it shows" in the 11:09 EDT amendment. The
scrollback keeps a 12 pt margin at the right, the band's width, so no
content sits under it.

1. **The band.** Unless visibility is *Disabled*, the 12 pt strip at the
   right edge of the scrollback is always a scrollbar:
   - touching it shows the thumb and moves it to the finger, centred;
   - dragging moves the view in proportion to the calculated length, with
     no fling;
   - the 2 s visibility starts when the finger lifts.
2. **Scrollbar visibility**, a setting (the settings plan, 20261024_02):
   - *Disabled*: no thumb and no band; the strip is plain scrollback;
   - *Hidden* (default): the thumb is drawn while scrolling and 2 s after;
     the band always works;
   - *Always*: the thumb is always drawn; the band always works.

   Until the settings screen exists, it is *Hidden*.

Tests:
- a touch in the band with the thumb faded moves the view there and
  drags;
- the thumb is centred under the finger;
- with *Disabled*, a touch in the band scrolls as content does;
- with *Always*, the thumb is drawn at rest.

## Amendment — 2026-10-24 11:40 EDT: the general agent's centre button in the agent view's controls (living section L3)

The owner (2026-10-24): the general agent button is "on all screens, only
replaced with X when it means cancel voice input" (plan 20261024_03, the
amendment of the same time). In the agent view's controls row:
- the centre is the general agent's button (plan 20261024_03 for its
  behaviour). Record (left) and the message or send button (right) keep
  their roles for this agent;
- while a message is dictated, the centre is an X that discards it. This
  replaces the dictation's own cancel button, and the dictation controls
  become send, X, record;
- with the general agent off (the settings plan), the centre is empty,
  except for the X while dictating.

## Amendment — 2026-10-24 11:54 EDT: full text in the scrollback; tool calls fold and expand on tap (living section L3)

The owner (2026-10-24): "if we're going to ellipsize sections they should
expand on tap like cards. generally I don't want to though, only spoken
messages should be shortened"; then "yes". This replaces the 500-character
cut of every entry the view shows (`ENTRY_SUMMARY_MAX`, applied by the
daemon to `Entry.summary`), and the app's fixed cuts of tool calls and
resolved cards.

1. **Full text.** Prompts, replies, notices and context lines show whole,
   never ellipsized.
   - **The daemon:** sections carry each entry's full redacted text, up to
     the reader's limit (`TEXT_MAX`, 32 KiB), in `Entry.text`. This is a
     new, appended, serde-defaulted field, so libhandlr_proto gets a patch
     release.
   - **`Entry.summary`:** it stays the 500-character form, for older
     clients, speech, the jump line and the overview.
   - **Old daemons:** the app shows `text` when present, and `summary`
     otherwise.
2. **Tool calls fold.**
   - **Folded:** a tool section shows its call and result at their first
     lines, marked "…" when there is more.
   - **Tap:** expands it in place to the whole call and result (the
     reader's `TOOL_TEXT_MAX`, 4,000 characters). Tapping again folds it.
   - **The view:** it does not move, because the anchor holds by section.
   - **Remembered:** an expanded section stays expanded while the view is
     open.
3. **Cards.** A resolved card in the scrollback folds and expands on tap,
   as the pinned card does.
4. **Speech is summarised; nothing else is.** Reading aloud, spoken
   summaries and the jump line keep their short forms.

Tests:
- a 2,000-character reply shows whole;
- `summary` is used when `text` is absent;
- a folded tool shows "…" and its first lines, a tap expands it to the
  whole, and a second tap folds it;
- expanding does not move the view;
- a resolved card expands and folds;
- on the daemon, `Entry.text` carries the full redacted text, and
  `summary` stays at 500 characters.

## Amendment — 2026-10-24 11:59 EDT: only tool calls and diffs fold by default (living section L3)

The owner (2026-10-24): "default folding only on tool calls and diffs".
This replaces item 3 of the previous amendment (resolved cards folding)
and narrows item 2:
- **Folded by default:** tool calls with their results, and diffs (an
  edit's patch), each at its first lines with "…", expanding and folding
  on tap as item 2 says.
- **Shown whole:** everything else, resolved cards included. The pinned
  card keeps its own fold, as before.

The previous amendment's tests change to match: a resolved card shows
whole, and a diff folds and expands as a tool call does.

## Amendment — 2026-10-24 12:03 EDT: summary cards in three forms: short, expanded with points, and spoken (living section L3)

The owner (2026-10-24): "two versions, like the current summary cards,
expanded has full context and point-by-point summary, third version incl
spoken version"; then "yes". This replaces "resolved cards included" in
the previous amendment's "Shown whole". Cards like the current summary
cards (Finished, and any card with a summary) have three forms:
1. **Short** (the default, folded): the title and the current short
   summary, as now.
2. **Expanded** (tap; tap again folds it):
   - the full context: the whole reply or question the card is about
     (`Entry.text`);
   - then a point-by-point summary, as a bulleted list, when the daemon
     gives one.
3. **Spoken:** at the end of the expanded card, under a small dim
   "spoken" label, the exact text read aloud or to be read. A small play
   button beside it replays it (not drawn while sound is off, per the
   settings plan).

The points come from `speech.summarize` (CHANGES.md, the same time).
Without them, the expanded card shows the full context and the spoken
text.

Tests:
- a Finished card folded shows its title and summary;
- expanded, it shows the full reply, the points, and the spoken text with
  its play button;
- with no points, there is no list;
- tapping again folds it;
- expanding does not move the view.

## Amendment — 2026-10-24 12:06 EDT: the summary card's points and spoken text come on the card (living section L3)

The owner (2026-10-24): the summary is a daemon option, under `[summary]`
(CHANGES.md, the same time). This replaces the previous amendment's "The
points come from `speech.summarize`": the expanded card's points are
`CardBody.points`, and its spoken text is `CardBody.spoken`, which the
daemon fills when `[summary] cards` is on and Gemini answers. When
`spoken` is absent, the spoken text is the reply's own read-aloud text as
before, and the points list is absent.

## Amendment — 2026-10-24 12:08 EDT: a summary card's short form never reads as cut off (living section L3)

The owner (2026-10-24), of a Finished card showing only "Written and
pushed:": "the last card says Written and pushed: implying there's more";
then "yes". This adds to the short form (item 1 of the three-forms
amendment):
1. **With a summary:** when the card carries the daemon's summary (`[summary]
   cards`, CHANGES.md), the short form is that summary, not the reply's
   first line.
2. **Without one** (no Gemini, or not yet arrived), the short form is the
   reply's opening, up to 3 lines:
   - a first line ending in ":" always takes the lines after it, so a list
     it introduces starts showing;
   - when the opening is cut, its last line ends with "…", and a small dim
     "more" sits at the card's bottom right.

Tests:
- a card with a summary shows the summary;
- "Written and pushed:" followed by a list shows the lead-in and the
  list's first lines, then "…" and "more";
- a short reply shows whole, with no "more".

## Amendment — 2026-10-24 12:12 EDT: closed views are kept in memory and reopen at once (living section L3)

The owner (2026-10-24): "if the session is cached do we need loading
session when opening it?"; then, adding a way to clear it (the settings
plan, the same time). Today closing the agent view drops its outline and
contents, so reopening shows "(loading session)".
1. **Kept on close.** The client keeps a closed view's outline, contents
   and place, for the 8 most recently opened panes, the least recent
   dropped first. Contents past 20 screens are already forgotten, so each
   stays small.
2. **Reopened from the cache.** A cached pane shows at once, with no
   "(loading session)" and no placeholders where contents are held. It
   subscribes with `after`, `from` and `epoch` to resume, and only what is
   newer comes over the link, going below.
3. **Where it opens:** at the bottom, following, with the jump line's
   state showing what happened while it was closed.
4. **Stale caches:** another epoch or a new session file merges by serial,
   as a reset does. "(loading session)" shows only for a pane not opened
   since the app started, or after the cache is cleared.
5. **Later:** plan 20261023_09's on-phone store keeps the same cache across
   app restarts, and this moves onto it then.

Tests:
- a closed and reopened view shows its sections with no loading state and
  resumes with `from`;
- a ninth pane drops the least recent;
- a new epoch merges;
- clearing the cache empties it.

## Amendment — 2026-10-24 12:15 EDT: closed views paged out to the phone's redb store, a few in memory, and a response to low memory (living section L3)

The owner (2026-10-24): "closed should be paged out to disk (data) with a
small number of recents in memory, as redb"; "it should also respond to
memory pressure if possible"; then "yes". This replaces item 1 of the
previous amendment ("the 8 most recently opened panes" in memory), and
item 5's "later".
1. **In memory:** the 3 most recently closed views (outline, contents,
   epoch and place).
2. **On disk:** older closed views are paged out to the phone's redb store
   (plan 20261023_09, decision 6, as amended at the same time). Reopening
   one reads it back at once, then resumes over the link with `from` and
   `epoch`. The cap is that plan's: 20 sessions and 50 MB, the least
   recently viewed dropped first.
3. **Low memory** (`MobileEvent::LowMemory`, once libmorph passes it to the
   app; until then, `Input::LowMemory`, tested and never fired):
   - every cached closed view is paged out, and none is kept in memory;
   - the open view keeps contents only within 1 screen of the window;
     the rest is read back from disk or fetched when scrolled to;
   - the layout caches of blocks off screen are freed.

   Nothing is lost: it is all on disk or on the daemon.
4. **Clear cache** (the settings plan) empties the memory cache and the
   redb store, and compacts the file.

Tests:
- a fourth closed view goes to disk;
- reopening one from disk shows it at once and resumes;
- low memory pages everything out and trims the open view to ±1 screen;
- Clear cache empties both.

## Amendment — 2026-10-24 12:21 EDT: memory pressure in grades, least recently used first, with a full drop on low memory (living section L3)

The owner (2026-10-24): "the low memory response should be oldest since
last used first depending on pressure, full drop on lowmemory to prevent
kill"; then "yes". This replaces item 3 of the previous amendment.
Within each grade, the least recently used views go first.
- **`UI_HIDDEN`** (libmorph's `on_trim_memory`, stage 2): closed views are
  paged out, keeping only the most recent one in memory. The open view
  keeps contents within ±5 screens.
- **`BACKGROUND`** (stage 2): every closed view is paged out. The open view
  keeps ±1 screen.
- **`LowMemory`** (libmorph's `on_low_memory`, stage 1): a full drop, so the
  app is not killed:
  - every cached view is paged out;
  - the open view keeps only what is on screen, and its outline;
  - every layout and shimmer cache of blocks off screen is freed.

  Anything needed again is read back from disk or fetched.

Until libmorph 0.4.0 delivers them, the three are tested behind
`Input::Memory(Pressure)`.

Tests:
- each grade pages out in least-recently-used order and keeps the stated
  range;
- the full drop leaves only the screen's blocks and the outline;
- scrolling after it reads back from disk.

## Amendment — 2026-10-24 12:22 EDT: pull past the end to refresh (living section L3)

The owner (2026-10-24): "deliberate scroll up past end should solicit
recent data from daemon, like pull to refresh"; then "yes".
1. **The gesture.** At the bottom, a finger dragging the content further
   up overscrolls with resistance (half the finger's movement), up to
   96 pt. A dim line shows below the last section:
   - `-- pull to refresh --`;
   - `-- release to refresh --` once past 64 pt.

   Letting go past 64 pt refreshes. Letting go before springs back. A
   fling reaching the end never triggers it.
2. **The refresh:**
   - a timed ping (`PING_WAIT_RESUME`, 3 s), reconnecting at once if the
     link is dead;
   - a resubscribe with `after`, `from` and `epoch`, so anything missed
     arrives;
   - a fetch of the newest screen's contents, whatever their revs.
3. **Feedback.** While it runs, the line reads `-- refreshing… --` and the
   view stays overscrolled. Then it reads `-- up to date --`, or
   `-- N new --` when sections arrived, for 1 s, and springs back to the
   bottom, following.

Tests:
- the resistance and the two texts by distance;
- release past 64 pt sends the ping, the resubscribe and the fetch;
- release before springs back with nothing sent;
- a fling to the end sends nothing;
- the result line, then the spring back.

## Amendment — 2026-10-24 12:23 EDT: the refresh's result line is the end's short summary (living section L3)

The owner (2026-10-24): "it shows the short summary as in the other plan,
not N new except as fallback". This replaces item 3's result line in the
previous amendment (`-- up to date --` or `-- N new --`). When sections
arrived, the line shows the end's state as the jump line does (the
amendment "the jump line says what is at the end"):
- the question, approval, broke or done line, with a summary card's short
  form (the daemon's summary when it has one);
- `-- N new --` only as the fallback, when none of those holds.

With nothing new, it reads `-- up to date --`.

## Amendment — 2026-10-24 12:27 EDT: section labels at the Finished cards, on the scrollbar's scale (living section L3)

The owner (2026-10-24): "if it makes sense I'd like to plan section labels
while scrolling with scrollbar based on the finished messages, which
become bookends"; "they would have the same basic gray variations as the
scroll and land in place of the finish messages on the same scale as the
scrollbar, like a label attached to each place but centered on the main
screen, alpha transparent overlay"; then "yes".
1. **Bookends.** Each Finished card closes a section of the session.
2. **When:** the labels show while the scrollbar is in use (a thumb or band
   drag), and fade with the thumb.
3. **Where:**
   - vertically, each label sits at its Finished card's place on the
     scrollbar's scale: a bookend 40% through the calculated length is 40%
     down the track;
   - horizontally, it is centred on the main screen.
4. **Look:** an alpha-transparent overlay in the scrollbar's grays.
   - **Each label:** a rounded pill, the text colour at about 10%, with its
     text at about 60%.
   - **The label nearest the thumb:** about 20% and 90%.

   The scrollback shows beneath.
5. **Text:** the bookend's short summary (`CardBody`'s summary when it has
   one) and its time, as "Landed the settings screen · 12:41", cut to one
   line. Before a summary arrives: "Done · 12:41".
6. **Crowding:** labels that would overlap give way to the one nearest the
   thumb, and show again as the thumb moves.
7. **Data:** the daemon marks Finished-card slots in the outline, so labels
   are placed before contents load. Their text comes with the cards.

Tests:
- labels show during a thumb drag and fade with it;
- a bookend's label sits at its share of the track, centred;
- the nearest is brighter;
- overlapping labels give way to the nearest;
- a label without a summary reads "Done · time".

## Amendment — 2026-10-24 13:45 EDT: Android's running trim levels mapped onto the memory grades (living section L3)

libmorph 0.4.1's `on_trim_memory(TrimLevel)` reports that, while a foreground
service runs, Android counts the process as foreground and sends only the
running levels (`RunningModerate`, `RunningLow`, `RunningCritical`), never
`UiHidden` or `Background`. On the S23, only `on_trim_memory` fires. The
owner (2026-10-24): "Yes, write the mapping and start that build". This adds
to the grades of the 12:2x EDT amendment ("memory pressure in grades"):

| Android level | handlr grade |
|---|---|
| `RunningModerate`, `UiHidden` | keep the most recent closed view, the open view ±5 screens |
| `RunningLow`, `Background`, `Moderate` | page out every closed view, the open view ±1 screen |
| `RunningCritical`, `Complete`, `on_low_memory` | the full drop |

Tests: each level gives its grade.

## Amendment — 2026-10-24 13:46 EDT: a correction to the previous amendment's reference (living section L3)

The previous amendment names "the 12:2x EDT amendment". It is the amendment
of 2026-10-24 12:21 EDT, "memory pressure in grades, least recently used
first, with a full drop on low memory".

## Amendment — 2026-10-24 15:56 EDT: a colour for every session, and other sessions' questions as cards at the bottom (living section L3)

The owner (2026-10-24): "should we add in-app notifications when a session
we've interacted with in the last hour has a question?"; "why not at the bottom
under the scroll area but with a different card header (such as a color
assigned to each session, including ours, using 4 color and light/dark
variants that's about 16 colors)"; "we apply the color band on overview as
well".
1. **A colour per session,** this one included.
   - **The palette:** 16 colours, 8 hues with a light and a dark variant each,
     as theme roles `session_1` to `session_16` (plan 20261024_05; a theme that
     leaves them out takes its default's).
   - **The assignment:** each session gets one from a hash of its session id,
     kept for its life, so the same agent always has the same colour.
   - **Where it shows:** a band on the session's overview card, and in its
     agent view's header.
2. **Another session's question, at the bottom.** When a session you
   interacted with in the last hour asks a question or needs an approval,
   and you are anywhere in the app but its own view, a card appears at the
   bottom under the scroll area, where the pinned card sits.
   - "Interacted" means you opened its view, sent it a message, or answered
     one of its cards.
   - The card's header is in that session's colour and names it: "<agent> is
     asking", or "<agent> needs approval".
   - It sits above this view's own pinned card, if there is one.
3. **Using the card:** a tap opens that session's view at the question. A
   swipe down dismisses it, and the question stays on the overview and in
   that view's jump line. Several at once collapse to the newest, with "+N".
4. **Sound:** the ding plays when *Ding when an agent is done* and *All sound*
   are on. In the background, the system alert covers it instead
   (CHANGES.md, the alerts).
5. **The client** keeps each session's last interaction time, and tells the
   app which sessions have a live question.

Tests:
- a session's colour is stable across reopenings and themes;
- the band shows on the overview card and the view's header;
- a question from a session used within the hour shows a card in its
  colour, and one from a session not used doesn't;
- a tap opens that session, and a swipe dismisses it;
- several collapse to "+N".

## Amendment — 2026-10-24 16:00 EDT: other sessions' finished turns as one-line cards too (living section L3)

The owner (2026-10-24): "same for finished with one-line"; then "build the
session colours and question cards now". This adds to item 2 of the amendment
of 2026-10-24 15 ("a colour for every session"). When a session you interacted with in
the last hour finishes a turn, and you are anywhere in the app but its own
view, a card appears at the bottom in that session's colour, as for a
question, but one line high: "<agent> finished: <short form>", with the
daemon's summary when it has one. A tap opens that session, a swipe dismisses
it, and it hides itself after 10 s. A question from the same or another
session takes the place of a finished card. Several finished cards collapse
to the newest, with "+N".

Tests: a finished turn from a session used within the hour shows a one-line
card in its colour; it hides after 10 s; a question replaces it.

## Amendment — 2026-10-24 16:00 EDT: a correction to the previous amendment's reference (living section L3)

The previous amendment cites "the amendment of 2026-10-24 15", a time cut
short by a mistake in my command. It means the amendment of 2026-10-24 15:56 EDT, "a colour for
every session, and other sessions' questions as cards at the bottom".

## Amendment — 2026-10-24 16:01 EDT: only long-running turns give a finished card (living section L3)

The owner (2026-10-24): "one-line finished summaries from other sessions but
only for long running tasks". This narrows the amendment of 2026-10-24 16:00 EDT ("other
sessions' finished turns as one-line cards too"). A finished card appears
only for a turn that ran at least 3 minutes, from the user's prompt to the
Finished card, in the agent's own times. Shorter turns give no card, and
still show on the overview. Questions and approvals are unchanged, and
always give a card.

Tests: a turn of 3 minutes or more gives a finished card; a shorter one
gives none.

## Amendment — 2026-10-24 16:15 EDT: a keyboard glyph left of the record button (living section L3)

The owner (2026-10-24), after libmorph was asked for soft keyboard support:
"for us, we'll add a keyboard glyph to the left of record but not as a button
(no background) but same hit box".
1. **Where:** in the controls row, immediately left of the record button, on
   the overview and in the agent view.
2. **What:** a keyboard glyph alone, drawn as strokes (the face has no
   keyboard character), in the controls' text colour. There is no background,
   border or fill, but its touch target is the record button's size.
3. **What it does:** a tap opens the soft keyboard for a typed message. In the
   agent view the message goes to that agent; on the overview it goes to the
   agent herdr has in focus, as dictation does. The typed text shows in the
   dictation card's place, and its Send sends it.
4. **Until libmorph has text input:** the glyph is drawn dim and ignores taps.
5. **Microphone off** (the settings plan) does not dim it, since typing needs
   no microphone.

Tests: the glyph sits left of record with record's hit box and no fill; it is
dim and inert without text input; a tap asks for the keyboard when text input
exists.

## Amendment — 2026-10-25 17:53 EDT: keyboard prompt card styling and shaded input area (living section L3)

The owner approved (2026-10-25): in keyboard mode, the prompt card displays
"Type your message" and a shaded input area.
1. **Mode tracking:** `flankr_companion::Dictation` distinguishes `PromptMode`:
   `Voice` (started by dictation/record button) and `Keyboard` (started by the
   keyboard glyph).
2. **Placeholder:** When the buffer is empty, voice mode shows
   `"(say your message)"` and keyboard mode shows `"Type your message"`.
3. **Shaded input area:** In keyboard mode, the input text / placeholder sits
   inside a distinct shaded input box (`theme().input`) within the prompt card,
   inset below the target line and above the controls, on both the overview
   and agent view screens.

## Amendment — 2026-10-25 23:34 EDT: prompt card text area focus, IME lifecycle, and external keyboard (living section L3)

The owner directed (2026-10-25): "ime should be shown when text area focused unless dismissed or external kbd active".
1. **Focus & Tap:** Tapping the shaded input `TextArea` on the prompt card focuses it and positions the insertion cursor. If the IME was previously dismissed, tapping the text area reactivates it (unless an external physical keyboard is active).
2. **IME Dismissal:** On soft keyboard dismissal (`TextInputEvent::Dismissed`), the prompt card remains open in its current state instead of being cleared or destroyed. The IME stays hidden until the text area is tapped again.
3. **External Keyboard:** When an external keyboard is active (desktop by default, or upon receipt of physical keystrokes on Android):
   - Physical typing (`Key::Character`, `Key::Backspace`, etc.) edits the prompt card buffer directly.
   - The on-screen soft keyboard (IME) is suppressed while an external keyboard is active.

## Amendment — 2026-10-25 23:41 EDT: 3-line 12pt text area in keyboard prompt card (living section L3)

The owner directed (2026-10-25): "good but its still one line, should be 3, about 12pt font".
1. **Dimensions:** The keyboard prompt card's `TextArea` is sized for at least 3 lines of text (`line_count = input_rows.len().max(3)`), maintaining a 3-line minimum input box.
2. **Typography:** The prompt text uses ~12pt font (`12.0 * s` px) instead of the larger dictation heading font.

## Amendment — 2026-10-25 23:55 EDT: expanding message collapses pinned cards with state memory and user override (living section L3)

The owner directed (2026-10-25): "expand message on a sesdion can collapse pinned cards (but they should remember previous state and user can override and that should be remrmbered too)".
1. **Pinned Card Visibility During Message:** Taking a message on a session (not replying directly to the pinned card) retains the pinned card above the message prompt card rather than removing it.
2. **Automatic Collapse with Memory:** Opening a message collapses an expanded pinned card (`pin_expanded = false`) to conserve vertical space while preserving its prior expansion state (`pin_saved_expanded`).
3. **User Override:** The user may tap the pinned card to toggle its expansion while the message is active, marking it as user-overridden (`pin_overridden = true`).
4. **Restoration:** When message mode closes, the pinned card's previous expansion state is restored if not overridden; if the user overrode it, the user's override is preserved.


## Amendment — 2026-10-27 13:21 EDT: a plan question needs "approve" and "plan"; the plan named by its id as written (living section L3)

The owner (2026-10-27), after "Restart the daemon on the plan 05 build?"
was shown as a plan question and its approve sent "Approve the plan": "for
now, the text has to have \"approve\" and \"plan\" to get the approve
suggestion, and the plan id (as included in the text, such as 03) should be
included in the suggestion. this is a heuristic and shouldn't be tied
closely to the CRITICAL RULES/tpt planning format (those are a separate
concern from flankr itself)".

This replaces the 2026-10-23 21:57 EDT amendment's item 1, first
paragraph, on what makes a question `about_plan` and what `plan` holds:
1. A pending question is `about_plan` only when it has both the word
   "approve" and the word "plan" (whole words, any case). A
   `docs/design/…_PLAN.md` path, "amendment" or "draft" alone no longer
   count, and nothing reads the planning format's file names.
2. `plan` is the id the text gives the plan: the word right after "plan"
   when it holds a digit (`plan 03`, `plan 20261027_04`, `plan #5` read as
   `5`), sent as "plan <id>". With no such word, or two different ids, it
   is `None`.
3. The answers name it so: **approve** sends "Approve plan 03", **discuss**
   starts "Discuss plan 03:", **implement** sends "Approve plan 03 and start
   building it"; without an id, "the plan" as before.

The rest of the 21:57, 22:58 and 23:00 EDT amendments stands.

## Amendment — 2026-10-27 13:24 EDT: "approve" with "draft" is a plan question too (living section L3)

The owner (2026-10-27), right after the 13:21 EDT amendment: "draft doesn't
make it about a plan unless it includes \"approve\" and \"draft\" (as in,
approve the draft)". This adds to the 13:21 EDT amendment's item 1: a
question with both "approve" and "draft" (whole words) is `about_plan` too.
With no plan id in it, its `plan` is "the draft", so **approve** sends
"Approve the draft".

## Amendment — 2026-10-27 14:04 EDT: a draft's plan id, from the question or the reply (living section L3)

The owner (2026-10-27): "approve the draft should also get the number, as
generally the question is approve the draft with a new plan id associated".
This adds to the 13:21 and 13:24 EDT amendments:
1. A plan id may follow "draft" as well as "plan" ("Approve draft 07?").
2. When the question itself gives no id, the last id after "plan" or
   "draft" earlier in the same reply is used, so "Approve the draft?" after
   "I wrote plan 06 as a draft." names plan 06 and approve sends "Approve
   plan 06". With no id anywhere, it stays "the draft".

## Amendment — 2026-10-27 21:58 EDT: the terminal view: drag scrolls the pane like a wheel, a tap arms typing and raises the keyboard, Finished cards one line (living section L3)

The owner (2026-10-27), after trying plan 20261027_07's terminal view on
the S23: "drag in terminal should work like mouse wheel, touching shoukd
enable ime. finished cards should be reduce to one line in terminal"; asked
what the drag scrolls, "Wheel events to the pane"; asked what a touch does,
"Tap arms and raises IME".

1. **A vertical drag on the terminal scrolls the pane,** as a mouse wheel
   does: the app sends `terminal.scroll { terminal_id, lines }` (positive
   back into the scrollback) and the daemon sets herdr's `pane.scroll`
   offset, so the pane scrolls on the Mac too. It is input, gated as
   `terminal.input` is (`[terminal] input`, the agent's
   `terminal_input`, a device that may approve, and an arm); the drag arms
   as a tap does. Without input allowed, a vertical drag pans the frame as
   before. A horizontal drag still pans.
2. **A tap on the terminal arms typing for 60 s and raises the soft
   keyboard,** when input is allowed; this replaces plan 20261027_07
   decision 6's 1.5 s hold. Without input allowed a tap does nothing new.
3. **Finished cards are one line while the terminal is shown** (the reply's
   first line), so they cover little of it; in the transcript they are as
   before.

Plan 20261027_07 is frozen; these are recorded there as departures in
CHANGES.md.

## Amendment — 2026-10-28 00:47 EDT: shell panes, terminal only (living section L3)

The owner (2026-10-28), while testing the terminal on a pane running htop:
"shell/non-agent should be supported but only have terminal mode (no
switch)".

1. **A pane that runs no known agent** (its agent resolves to `shell`,
   `agents.list`'s `AgentInfo.agent`) is a session the app lists and
   opens, like an agent's.
2. **It has only the terminal view.** Opening it shows the terminal at
   once; there is no transcript, and the bottom bar's terminal/list glyph
   (plan 20261027_07 decision 7) is not drawn for it. The prompt card,
   message sends, suggestions and summaries do not apply to it.
3. **Access is the `shell` entry's,** as for any agent: the terminal opens
   only when `[[agents]]` has a `shell` entry the principal may reach,
   and typing, scroll and mouse need its `terminal_input` as well as
   `[terminal] input` and the arm. Without the entry the session is listed
   but its terminal is refused with the daemon's message.

## Amendment — 2026-10-28 10:42 EDT: the terminal's arm without a timer or frame; a keys row above the keyboard (living section L3)

The owner (2026-10-28): "remove the keyboard timer and frame. add a new keys
row just above the ime inset with ESC TAB CTRL ALT and collapsed dpad with
up arrow above left down right like 101 keyboard, the CTRL and ALT as
modifiers for ime button press".

1. **No timer.** Typing, once armed by a tap or a drag (the L3 amendment of
   2026-10-27 21:58 EDT), stays armed until the terminal is closed (the
   view is left, the session switched, the terminal closed by the daemon)
   or the link drops; it no longer ends after 60 s without a key. This
   replaces plan 20261027_07 decision 6's "it drops after 60 s without a
   key" and the daemon's 60 s window: the daemon holds the arm for the
   terminal's life on that connection. `terminal.input_arm`'s
   `expires_in_s` is answered as 0, meaning no expiry. The gates are
   unchanged: `[terminal] input`, the agent's `terminal_input`, a device
   that may approve, and the arm.
2. **No frame.** The armed terminal draws no coloured frame (and, as
   already removed, no pill).
3. **A keys row just above the soft keyboard,** shown while the keyboard
   is raised into the terminal, the full width of the view, one row high:
   **ESC**, **TAB**, **CTRL**, **ALT**, then a collapsed d-pad laid out
   like a 101-key keyboard's arrow block: **↑** above, **← ↓ →** below,
   the two rows sharing the row's height. ESC, TAB and the arrows send
   their keys at once (`escape`, `tab`, `up`, `down`, `left`,
   `right`).
4. **CTRL and ALT are modifiers** for the next key from the soft keyboard:
   tapping one latches it (drawn highlighted); the next character typed
   is sent with it (`ctrl+<key>`, `alt+<key>`, or both), and the latch
   clears. Tapping a latched modifier again clears it. A modifier also
   applies to the row's own keys (ctrl+arrow). The daemon accepts
   `alt+<printable>` and `ctrl+alt+<key>` as well as today's
   `ctrl+<a-z>`; `ctrl+c`, `ctrl+d`, `ctrl+z` and `ctrl+\` still need
   the agent's `terminal_signals`.

## Amendment — 2026-10-29 00:18 EDT: L3, the bottom bar as plan 20261028_06 has it (living section L3)

The owner approved docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md on
2026-10-28. Its decision 13 names the L3 entries it supersedes (the 16:02 EDT
play and skip bullets, the 11:40 EDT centre placement, the 16:15 EDT
keyboard-glyph amendment, the 00:47 EDT terminal-only keyboard glyph, and the
10:42 EDT amendment's item 3); for the agent view's bottom row, the text
card's row and the terminal keys row, that plan is the reference from here.
