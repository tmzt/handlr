---
title: The handlr.harness protocol, session peek and the MCP adapter
date: 2026-10-22
status: Draft
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
