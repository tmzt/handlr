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
