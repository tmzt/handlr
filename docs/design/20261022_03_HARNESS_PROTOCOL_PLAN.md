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
