# agy (Antigravity CLI) conversation steps

Findings for plan 20261023_06 (session readers). Append-only: each
addition is a dated section; corrections are later sections (Rule 3.6).
Structure only, never a conversation's text.

## 2026-10-23 17:33 EDT — the steps table and its protobuf fields

**Where:** `~/.gemini/antigravity-cli/conversations/<session id>.db`,
SQLite in WAL mode. herdr's `agent_session` for an agy pane is
`{source: "herdr:antigravity_cli", kind: "id", value: <session id>}`.
Beside the conversations are agy's memory and state (`brain/`,
`annotations/`, `presence/`, `conversation_summaries.db`) and its OAuth
token: a reader opens none of them (owner, 2026-10-23: "It shouldn't
include the memory or anything like that. Just a session").

**Read from:** one example conversation (the repowiki pane's, 993 steps),
copied with `scripts/record-agy-example.sh` to the gitignored
`out/sessions/agy_example/`, and checked against the synthetic fixtures
`tests/fixtures/sessions/agy/{asking,stopped}.db` written by
`scripts/make-agy-fixtures.py`.

**The table:** `steps(idx, step_type, status, …, step_payload, …)`; one
row per step, `idx` from 0 in order. The payload is a protobuf message;
field numbers below are its fields, nested with dots.

**Common to every step:**

| Field | What |
|---|---|
| 1 | the step type again (varint) |
| 4 | status (varint; 3 in finished steps) |
| 5.1 | when: `{1: seconds, 2: nanos}` since the Unix epoch |
| 5.4 | in a tool result: the call it answers, `{1: call id, 2: tool name}` |
| 5.20.2 | the step's `idx` |
| 5.20.4 | the conversation's id |

**The step types**, with counts in the example (type, status: count):

| Type | Count | What | Read as |
|---|---|---|---|
| 14 | 13 (status 3) | the user's prompt: 19.2 is its text | `Role::User`, `Text` |
| 15 | 484 (3), 1 (8) | the agent's response: 20.1 (equal to 20.8) is a reply, in 16 steps; 20.3 is narration between tool calls, in 114; 20.7 (repeated) is a tool call `{1: call id, 2: tool name, 3: arguments as JSON}`, in 476; 20.6 is the model's id | 20.1 or 20.3: `Role::Assistant`, `Text`; each 20.7: `ToolUse`, with `call_id` and `tool` |
| 132 | 459 (3), 1 (2), 6 (6), 7 (10) | a tool's result: 140.2.1 is its output (a string in most, bytes or a message in some); 140.1 (repeated `{1: key, 2: value}`) its metadata. Every one of the 476 answers a call of a type 15 step by 5.4.1 | `Role::Tool`, `ToolResult`, with `call_id`; output text when 140.2.1 is a string |
| 101 | 12 (3) | a task notification (114: title at 2.1, kind at 3) | `EntryKind::Other` |
| 23 | 1 (3) | a large state snapshot (field 30, about 300 KB): memory-like | not read |

A response with a reply (20.1) has no tool calls; narration (20.3) comes
with them. Statuses other than 3 in type 132 (6 and 7) look like failed or
cancelled calls; which is which is left for the reader's step.
