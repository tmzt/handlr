---
title: Session subscriptions, pushed session entries
date: 2026-10-23
status: In implementation
covers:
  - "A subscription to an agent's session: the daemon tails the session
    file and pushes new, redacted entries to every subscriber as they are
    written, instead of clients polling `session.peek`"
  - "A subscription names one pane and stays on it. It does not follow
    herdr's focus on its own: the user may be checking another project
    while the terminal attached to herdr sits on a different pane. A
    Follow toggle in the companion may move the subscription with the
    focus; how it behaves is left open (section 13)"
  - "herdr's focus pushed to companions from herdr's own `pane.focused`
    and `tab.focused` events, not by a 30 s poll, so the companion knows
    the focus at once whether or not it follows it"
  - "Cursors, so a client that reconnects gets only what it missed, and
    resets when the session file changes under it"
  - "Proving the protocol end to end: over the daemon socket, over the
    Noise link with large messages, with the herdr mock and growing
    fixture files, and with a read-only command-line client against live
    herdr"
  - "Not covered: an off-screen or overview screen of several agents
    (later, its own plan); harness clients (plan 20261022_03 step 5);
    pushing pane scrollback for agents with no session reader"
sources:
  - "Owner (2026-10-23): \"is it only pulling that on demand? if the
    session is subscribed we should be pushing them\"; \"for now, just the
    focused session, we'll come up with some kind of off-screen/overview
    screen later. this should be a plan for SESSION_SUBSCRIPTIONS with a
    comprehensive design, not a one-off change. we also need to make sure
    the underlying protocol works\"; \"we don't want to necessarily follow
    herdr's subscription, we'll probably have a follow button of sorts to
    enable/disable that. I might be checking in on other projects while
    the terminal connected herdr is on a different pane. we'll have to
    figure that one out\""
  - "handlr at `5145e33`:
    - `src/sessions/mod.rs` (`SessionReader`, `read_tail`, `link_results`,
      `Sessions::peek`, `TAIL_CHUNK` 256 KiB, `TAIL_MAX` 2 MiB);
    - `src/sessions/{claude,codex,redact,pending}.rs`;
    - `src/server/conn.rs` (`session.peek`, `events.subscribe` with the
      topics `agent.status` and `confirm`, `forward_events`, each
      connection's unbounded `out` queue);
    - `src/server/companion.rs` (`focus.changed`, sent after
      `agents.focus` and every `VOCABULARY_INTERVAL`, 30 s);
    - `src/server/watcher.rs` (one herdr event connection; herdr closes a
      connection that receives a second `events.subscribe`);
    - `crates/libhandlr_proto` (`SessionPeekParams`, `SessionPeek`,
      `Entry`, `PEEK_ENTRIES_MAX` 200, `ENTRY_SUMMARY_MAX` 500; the link's
      `MAX_FRAME` 65535, `MAX_MESSAGE` 1 MiB, messages chunked into
      frames);
    - `crates/handlr-companion/src/client.rs` (`AgentView`,
      `peek_if_due`, `PEEK_INTERVAL` 5 s, `PEEK_IDLE_INTERVAL` 15 s,
      `VIEW_ENTRIES`) and `net.rs` (`MAX_LINE` 1 MiB);
    - `crates/libherdr/src/events.rs` (`Subscription` with two variants,
      `HerdrEvent::Other` for unmodelled events)"
  - "herdr's API schema (`herdr api schema --json`, 2026-10-23): the
    subscription types include `pane.focused`, `tab.focused`,
    `workspace.focused` and `pane.updated`, none scoped to a pane;
    `pane.report_agent` carries `agent_session_id` and
    `agent_session_path`"
  - "Measured on 2026-10-23 at 13:3x EDT: `session.peek` over the daemon
    socket found three live Claude Code sessions by id (`w2:pZ`, `w2:pV`,
    `w2:pR`) and returned entries; the S23 had connected but not opened
    the agent view, because nothing on the main screen says the focus
    line opens it"
related:
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (session readers, redaction, `session.peek`, the agent view; this plan replaces the view's polling)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (the companion protocol and the Noise link)
  - docs/design/20261022_04_COMPANION_APP_PLAN.md (the companion's states)
  - docs/design/20261023_02_BULK_INPUT_PLAN.md (herdr's focus in the status data, `focus.changed`)
---

# Session subscriptions, pushed session entries

## 1. Goal

The agent view shows an agent's session as terminal scrollback (plan
20261022_03, decision 9). Today the companion pulls it: `session.peek` when
the view opens, then again every 5 s while the agent works and every 15 s
while it is idle, each time re-reading up to 50 entries. New output takes up
to 15 s to appear, and every poll re-sends what the phone already has.

This plan turns that around. A client **subscribes** to a session; the
daemon tails the session file and **pushes** each new entry once, redacted,
to every subscriber, within a second of the agent writing it. A
subscription names **one pane** and stays on it. For now the companion has
one subscription, to the agent whose view is open, which starts as herdr's
focused agent. It does not move when herdr's focus moves, unless a Follow
toggle says so (section 13). The design takes any number of subscriptions
per connection and per pane, so an overview of several agents can be added
later without changing the protocol.

It also makes the protocol prove itself: integration tests over the daemon
socket and over the Noise link, with growing session files and the herdr
mock, and a read-only command-line client that follows live sessions.

## 2. Where things stand

* **Session reading.** `Sessions::locate` finds a pane's file: by the
  session id herdr reports (`agent_session`), else the newest file for the
  pane's cwd. `read_tail` reads back from the end in 256 KiB chunks, up to
  2 MiB, and parses whole lines. `link_results` gives each tool result its
  call's tool name, and replaces a result whose call names a `.private/` or
  `.env` path with `[redacted]`. `Item::entry` redacts and cuts to 500
  characters. Nothing keeps state between reads.
* **Pushes that exist.** `events.subscribe` takes the topics `agent.status`
  and `confirm`. `forward_events` fans the daemon's broadcast channels out to
  each connection, which writes through an unbounded queue.
* **Focus.** The daemon tells a companion about herdr's focus with
  `focus.changed`, after `agents.focus` and on a 30 s check. herdr can push
  focus itself (`pane.focused`, `tab.focused`), but the daemon's one herdr
  event connection subscribes only to `pane.agent_status_changed` for
  watched panes.
* **Limits on the wire.** The daemon socket is newline-delimited JSON. The
  link carries one JSON-RPC object per message, chunked into Noise frames of
  at most 65535 bytes, up to 1 MiB a message. The companion refuses lines
  over 1 MiB.

## 3. Decisions

1. **The daemon pushes; clients do not poll.** A subscription delivers a
   backfill once, then only new entries. `session.peek` stays, for one-off
   reads and for clients that do not subscribe.
2. **A subscription is to a pane, and stays there.** It names a pane
   (`{"pane_id": "w2:pZ"}`) and follows that pane's agent, including a new
   session file in the same pane. It never moves to another pane by itself.
   herdr's focus is the terminal's focus, not the phone's: the user may be
   checking another project from the phone while the attached terminal is
   on a different pane (owner, 2026-10-23). Following the focus is a client
   choice: with a Follow toggle on, the client moves its subscription when
   `focus.changed` arrives, by unsubscribing and subscribing again. The
   daemon has no focus-following subscription, so the toggle's behaviour
   can change without changing the protocol. Whether the toggle exists,
   where it sits and its default are open (section 13).
3. **One tailer per session file, shared.** The daemon keeps a session hub:
   one tailer for each file that at least one subscription needs, however
   many subscribers it has. A tailer reads only the bytes appended since its
   last read, parses whole lines, links and redacts them once, and hands the
   same batch to every subscriber. A tailer with no subscribers is dropped.
4. **Change is found by checking size, not by file events.** Each tailer
   checks its file's length and modification time every 250 ms, and every
   2 s it checks whether the pane's session has changed. That is one `stat`
   per file every 250 ms, for one or two files. It needs no new dependency
   and behaves the same on macOS and Linux. File events (`notify`) can
   replace the check later without changing the protocol.
5. **Cursors are positions in the file.** Each entry is numbered by the
   byte offset where its line starts (`seq`), and a delivery's cursor is
   the session id plus the offset just after the last whole line read. Both
   only grow within a file, need no stored state, and survive a daemon
   restart. The cursor is opaque to clients: they keep it and give it back.
6. **A reset rather than a gap.** When the client's cursor cannot be
   continued, the daemon sends a reset with a fresh backfill. That happens
   when the session id changed (a new file, as after `/clear`), when the
   file is now shorter than the cursor (replaced or truncated), when the
   focus moved, or when more was written than one catch-up may carry. A
   client never shows a gap without being told.
7. **Redaction is unchanged and holds across batches.** Everything goes
   through `Item::entry`. The secret-path rule in `link_results` needs the
   call that a result answers, and with incremental reads the call can be in
   an earlier batch. So each tailer keeps the call ids of its last 512 tool
   calls, with each call's tool name and whether it names a secret path. A
   result whose call is not known, because it is older than that or before
   the backfill, keeps its text only if its own text does not name a secret
   path. This is the same check, applied to the result.
8. **Pending questions and status travel with the entries.** A delivery
   carries the pane's agent status and its pending question whenever either
   changed since the last delivery to that subscriber. The view needs no
   second stream.
9. **Back-pressure is coalescing, not queuing.** Each subscription has at
   most one delivery waiting to be written. While one is waiting, newer
   entries are added to it, up to the batch limits (decision 11). Past them,
   the waiting delivery becomes a reset that carries the newest entries. A
   slow phone therefore costs bounded memory, and the connection's
   unbounded queue never grows without limit from session traffic.
10. **Permissions match `session.peek`.** Subscriptions are served to
    companions and on the daemon socket, and the principal must be allowed
    to reach the pane's agent. The check is made at subscribe, and again
    when the pane's agent changes (a different agent started in the pane);
    when it fails then, the delivery says so (`session: null`, `reason:
    "not_permitted"`) and no entries follow until it passes. Harness
    clients get subscriptions with plan 20261022_03 step 5.
11. **Numbers.**

    | What | Value |
    |---|---|
    | File check | every 250 ms |
    | Session re-check | every 2 s, and at once on a herdr `pane.updated` or `pane.closed` event |
    | Pushes per subscription | at most 4 a second (a 250 ms gap) |
    | Backfill | `entries`, 1 to `PEEK_ENTRIES_MAX` (200), 50 by default |
    | One catch-up delivery | at most 200 entries and 256 KiB of JSON |
    | Subscriptions per connection | 8 |
    | Remembered tool calls per tailer | 512 |
    | Latency, write to phone | under 1 s on the tailnet when idle (250 ms check, plus 250 ms gap, plus transit) |

12. **herdr focus is pushed to companions.** The daemon's herdr event
    connection always subscribes to `pane.focused`, `tab.focused`,
    `pane.updated` and `pane.closed`, as well as the status of watched
    panes. A focus event sends `focus.changed` to companions straight away;
    the 30 s check stays as a backstop. The companion then knows the focus
    at once, to show it and, if it follows, to move its subscription.
    Because herdr closes a connection on a second `events.subscribe`, these
    join the watch set's one subscription, and the connection stays open
    even when no pane is watched.

## 4. The protocol

These are added to `libhandlr_proto::common`, so they are shared by the
companion protocol and the daemon socket.

### 4.1 `session.subscribe`

Request:

```json
{"jsonrpc":"2.0","id":7,"method":"session.subscribe","params":{
  "pane_id": "w2:pZ",
  "entries": 50,
  "after": "c1:992c3f44-18e7-4cf8-814f-e437b90209b6:1048576"
}}
```

* `pane_id`: a pane id or a name. A name resolves as `session.peek`
  resolves it, once, when the subscription is made; the subscription then
  holds the pane id.
* `entries`: the backfill size, 1 to 200, 50 by default.
* `after`: optional, a cursor from an earlier delivery. When it is still
  valid for the target's session, the backfill is only what came after it.
  Otherwise the reply is a reset.

Result, which is also the first delivery:

```json
{"subscription_id": 3,
 "delivery": { …a SessionDelivery, see 4.2… }}
```

Errors:

| Error | When |
|---|---|
| `INVALID_PARAMS` | `entries` out of range; a cursor that does not parse |
| `UNKNOWN_AGENT` | `pane_id` names no pane |
| `NOT_PERMITTED` | a client that is neither a companion nor local; a pane the principal may not reach |
| `TOO_MANY_SUBSCRIPTIONS` (new, -32012) | the connection already has 8 |

### 4.2 `session.delivery` (notification)

```json
{"jsonrpc":"2.0","method":"session.delivery","params":{
  "subscription_id": 3,
  "pane_id": "w2:pZ",
  "agent": "claude",
  "reset": false,
  "session": {"session_id": "992c…", "matched": "id"},
  "reason": null,
  "entries": [{"seq": 1049003, "ts": "…", "role": "assistant",
               "kind": "text", "summary": "…"}],
  "cursor": "c1:992c3f44-18e7-4cf8-814f-e437b90209b6:1050112",
  "status": "working",
  "pending_question": null
}}
```

`SessionDelivery`:

| Field | Type | Meaning |
|---|---|---|
| `subscription_id` | u64 | |
| `pane_id` | string | The subscription's pane |
| `agent` | string | The pane's agent |
| `reset` | bool | Drop what was shown for this subscription; `entries` is a fresh backfill |
| `session` | object or null | `session_id` and `matched`, as in `session.peek`; null when the pane has no session |
| `reason` | string or null | Why `session` is null: `no_reader` (an agent without a session reader), `no_session` (no file found), `not_permitted`, or `pane_closed` (the pane is gone; no more deliveries follow, and the client unsubscribes) |
| `entries` | `[Entry]` | Oldest first; may be empty when only `status` or `pending_question` changed |
| `cursor` | string or null | Give it back as `after` |
| `status` | string or omitted | Present when the status changed since the last delivery, or on a reset |
| `pending_question` | object or null or omitted | Omitted when unchanged; null when it was cleared |

`Entry` gains `seq` (u64, optional, omitted by `session.peek`). The field is
additive and defaulted, so `session.peek` replies are unchanged.

The cursor is `c1:<session_id>:<offset>`, where `c1` is the format's
version. A cursor for a cwd-matched session with no id uses the file name's
stem. Clients treat it as opaque.

### 4.3 `session.unsubscribe`

`{"subscription_id": 3}`, answered with `{}`. An unknown id gets
`SUBSCRIPTION_NOT_FOUND` (new, -32013). Closing the connection ends all its
subscriptions.

### 4.4 Ordering guarantees

* Within a subscription, deliveries arrive in order. Entries are strictly
  increasing in `seq`, except across a reset.
* The `session.subscribe` result is written before any `session.delivery`
  for that subscription.
* Deliveries carry their `subscription_id`, so a client that re-subscribes
  on a focus move can tell old deliveries still in flight from new ones.

## 5. The daemon

### 5.1 The session hub (`src/sessions/hub.rs`)

```rust
pub struct SessionHub {
    sessions: Arc<Sessions>,
    tailers: Mutex<HashMap<PathBuf, Arc<Tailer>>>,
}

struct Tailer {
    path: PathBuf,
    reader_kind: &'static str,
    located: Located,
    state: Mutex<TailState>,              // offset, len, mtime, recent calls
    batches: broadcast::Sender<Arc<Batch>>,
}

pub struct Batch {
    pub items: Vec<(u64 /*seq*/, Entry)>,  // already redacted
    pub end: u64,                          // offset after the last whole line
    pub truncated: bool,                   // the file shrank: subscribers reset
}
```

* `SessionHub::backfill(located, n, after)` reads with `read_tail`, keeps
  each line's start offset, so `parse_window` returns `(seq, Item)`, and
  returns the last `n` entries with `seq > after` and the end offset.
* `SessionHub::tailer(located)` gives the shared tailer, starting its task
  if it is new. The task checks every 250 ms. When the length grew, it reads
  from the offset to the end. It keeps only bytes up to the last `\n` (a
  partial line waits for its newline), parses, links against the recent
  calls, redacts, and broadcasts the batch. When the length shrank, it
  broadcasts `truncated`. When nobody is subscribed, which it checks with
  `receiver_count() == 0`, it stops and removes itself.
* The task reads at most 1 MiB in one check, and the rest on the next.

### 5.2 Subscriptions (`src/server/subscriptions.rs`)

Each subscription runs a task per connection:

1. Resolve the pane from the latest snapshot.
2. Locate the session. Reply with the backfill, or with `reason`.
3. Loop, using `select!` over:
   * the tailer's batches, which are added to the waiting delivery;
   * a 2 s timer, and the daemon's pane events for this pane, which
     re-locate the session; a different session id sends a reset, and a
     closed pane sends `pane_closed` and ends the task;
   * `CoreEvent::AgentStatus` for the current pane, which recomputes the
     pending question and sets `status`;
   * the send gate, which writes the waiting delivery when 250 ms have
     passed since the last one;
   * cancellation, from unsubscribe or the connection closing.

The delivery waiting to be written is a `Pending { entries, bytes, reset,
status, pending_question }`. When `entries` would pass 200 or `bytes`
256 KiB, it is replaced by a reset carrying the last 50.

### 5.3 Focus from herdr

* libherdr gains `Subscription::PaneFocused` and `Subscription::TabFocused`,
  serialised as `pane.focused` and `tab.focused` with no fields. Their
  events stay `HerdrEvent::Other` until their payload is recorded from live
  herdr (step 1). They are then modelled as `HerdrEvent::PaneFocused {
  pane_id }`, or left as `Other` with the pane read from the snapshot if the
  payload carries no pane.
* libherdr also gains `Subscription::PaneUpdated` and `PaneClosed`
  (`pane.updated`, `pane.closed`), recorded and modelled the same way.
* `watcher::run` always includes these four, and the watch set's status
  subscriptions when there are any. A focus event sends
  `CoreEvent::Focus { pane_id }`; a pane event sends
  `CoreEvent::Pane { pane_id, closed }`.
* `CoreEvent::Focus` sends `focus.changed` to companions through the
  existing comparison with `conn.focus`. `CoreEvent::Pane` wakes that
  pane's subscriptions.

### 5.4 Pending questions

`pending::pending_question` takes the items and the status. A tailer keeps
the last 20 items (unredacted text stays inside the daemon, as now), so a
subscription can recompute the question on a status change without reading
the file.

## 6. The companion client (`crates/handlr-companion`)

* `Input::OpenAgent(pane)` subscribes to that pane (`entries: 200`), and
  `Input::CloseAgent` unsubscribes. The subscription is the source of
  truth for the agent view and its cards. For now the only way into the
  view is the focus row, so the pane is the focused one when the view
  opens; it then stays on that pane.
* On reconnect, an open view subscribes again with `after` set to its last
  cursor.
* **Follow, behind a setting for now:** when `follow` is on and
  `focus.changed` names another agent pane, the client unsubscribes and
  subscribes to the new pane, and the view shows it. When it is off,
  `focus.changed` only updates the focus row. Default off, pending section
  13.
* `AgentView` keeps up to 200 entries. A delivery with `reset` replaces
  them; otherwise its entries are appended and the oldest dropped past 200.
  Status and pending question are applied as they come.
* `peek_if_due`, `PEEK_INTERVAL`, `PEEK_IDLE_INTERVAL` and `Sent::Peek` are
  removed.
* After a disconnect, the view keeps what it showed, as now. On reconnect,
  the `after` cursor gets only what was missed, or a reset.
* `Event::AgentView` is still given only when the view changed (the card
  rules in `view.rs` are unchanged).

## 7. The app (`apps/companion`)

* The focus line on the main screen becomes a visible control: a row with
  a chevron (`›`) and the agent, project and tab. Tapping it opens the
  agent view on that pane. This is the one main-screen change. The
  overview of several agents, which is how other projects will be checked
  on, is for later.
* The agent view's header names the pane it is on. When herdr's focus is
  elsewhere, it shows a dim "terminal on: <name>" line beneath, so it is
  clear the view is not following.
* The agent view does not change. It already follows the bottom, and
  scrolling back holds its place as entries arrive (L3-1), which a reset
  also respects by keeping the entry at the top when it is still there.

## 8. The command-line client

`handlr session tail [--pane <id or name>] [--follow] [--entries N]`
connects to the daemon socket, says `hello` as `handlr.companion` with the
client name `tail`, subscribes, and prints each delivery as it comes.
Without `--pane` it starts on herdr's focus. `--follow` moves with
`focus.changed` exactly as the companion's Follow does, so the client-side
follow is exercised from the command line too. It
prints one line per entry (`seq`, role, kind, the summary's first line), a
`-- reset (<why>) --` line for resets, and status and question changes. It
only reads: it sends nothing to herdr or to any pane. It is how the
protocol is checked against live sessions without the phone (step 6).

## 9. Failure cases

| Case | What happens |
|---|---|
| The agent writes half a line | Held until its newline; never parsed half |
| `/clear` starts a new session file | herdr's `agent_session` changes; within 2 s (or at once on `pane.updated`) the subscription resets onto the new file |
| The file is truncated or replaced | `truncated` from the tailer; the subscribers reset |
| herdr's focus moves elsewhere | Nothing changes for the subscription; companions get `focus.changed` |
| The pane closes | A delivery with `reason: "pane_closed"`; the subscription ends |
| An agent with no session reader (agy) | `reason: "no_reader"`; scrollback for those is not covered |
| The phone is slow or stalled | Coalescing, then a reset with the newest 50 (decision 9) |
| The daemon restarts | Clients reconnect with their cursor; offsets in the file are still valid, so they get only what they missed |
| herdr restarts or the event stream drops | The 2 s re-check carries on from snapshots; the watcher reconnects as now |
| A burst larger than 1 MiB of file in one check | Read over several checks; a subscriber that falls past the batch limits is reset |
| A tool result whose call is older than the remembered 512 | Its own text is checked for secret paths (decision 7) |

## 10. Proving the protocol

"Make sure the underlying protocol works" (owner) is met by tests at each
layer, all against mocks and fixtures. Nothing types into or sends to a
live pane.

1. **Types** (`libhandlr_proto`): round trips for the three methods,
   `SessionDelivery` with every optional field present and absent, `Entry`
   with and without `seq`, and an old `session.peek` reply parsing
   unchanged.
2. **The hub** (unit tests with temporary files):
   * a growing file delivers each line once, in order, with the right
     `seq`;
   * a half-written line waits for its newline;
   * truncation resets;
   * two subscribers share one tailer, and it stops when both leave;
   * a tool call and its secret-path result in different batches comes
     out `[redacted]`;
   * a 3 MiB burst arrives complete, over several checks.
3. **The daemon socket** (`tests/daemon.rs`, the herdr mock and a fixture
   `$HOME`):
   * subscribe by pane, append to the fixture file, and the delivery
     arrives within 1 s;
   * `after` gives only the missed entries;
   * a changed `agent_session` in the mock snapshot resets;
   * a mock `pane.focused` event sends `focus.changed` at once and leaves
     the subscription on its pane;
   * a mock `pane.closed` ends the subscription with `pane_closed`;
   * unsubscribe stops deliveries;
   * the ninth subscription is refused;
   * a principal without reach gets `not_permitted`.
4. **The link** (`tests/link.rs`, a paired device over Noise):
   * the same subscribe-and-append check;
   * a backfill of 200 entries of 500 characters each, about 130 KiB and
     so spanning several 64 KiB frames, arrives whole;
   * a client that stops reading while 5 MiB is appended gets a reset,
     not 5 MiB, once it reads again;
   * reconnecting with the cursor gives only what was missed.
5. **The companion client** (`handlr-companion` unit tests, with scripted
   deliveries): append, reset, the 200 cap, reconnect with `after`;
   `focus.changed` with follow off leaves the subscription alone, and with
   follow on unsubscribes and subscribes to the new pane; no
   `session.peek` sent any more.
6. **Live, read-only:** `handlr session tail --pane <id>` against the
   running daemon and live herdr, on this session's pane and another
   agent's. Entries appear within a second of being written; moving
   herdr's focus by hand does not move the subscription, and with
   `--follow` it does. Logged under
   `out/sessions/YYYYMMDD_NN_subscriptions_live/`.
7. **On the S23:** the agent view fills when opened and gains new entries
   within a second while an agent works, with no polling in the log.

## 11. Steps

1. **libherdr: focus and pane subscriptions.** Add
   `Subscription::PaneFocused`, `TabFocused`, `PaneUpdated` and
   `PaneClosed`. This adds variants to a public enum without
   `#[non_exhaustive]`, so it is a minor release, 0.3.0, under Rule 5.4.
   Record a live `pane.focused` and `tab.focused` event read-only (a
   subscription, with focus moved by hand) as a fixture, and model them
   from it, with `pane.updated` and `pane.closed` likewise. Tag
   `libherdr-v0.3.0`.
   **Done when:** a test decodes the recorded events, and a subscription
   with all four types is accepted by live herdr.
2. **libhandlr_proto: the protocol.** The three methods,
   `SessionSubscribeParams` and `Result`, `SessionDelivery`,
   `SessionUnsubscribeParams`, `Entry.seq`, and the codes -32012 and
   -32013. The crate is still 0.1.0, so this is its first version change:
   it becomes 0.2.0, with a VERSIONS.md entry.
   **Done when:** the round trips in section 10.1 pass.
3. **The daemon: the hub.** `src/sessions/hub.rs`, with `parse_window`
   returning offsets, and the redaction across batches.
   **Done when:** the tests in section 10.2 pass, and `session.peek` gives
   the same entries as before.
4. **The daemon: subscriptions and focus.** `src/server/subscriptions.rs`,
   the three methods in `conn.rs`, the watcher's focus and pane
   subscriptions, `CoreEvent::Focus` and `CoreEvent::Pane`, and
   `focus.changed` sent on focus events.
   **Done when:** the tests in sections 10.3 and 10.4 pass.
5. **`handlr session tail`.**
   **Done when:** the live check in section 10.6 passes and is logged.
6. **The companion client.** Subscribe on opening the view, deliveries
   into `AgentView`, Follow behind a setting, and the peek polling
   removed.
   **Done when:** the tests in section 10.5 pass.
7. **The app.** The focus row as a control, and the view's header naming
   its pane and where the terminal is.
   **Done when:** the S23 check in section 10.7 passes.

Each step is one crate, one commit (Rule 3.1). Steps 1 and 2 can go in
either order; 3 needs 2; 4 needs 1 and 3; 5 and 6 need 4; 7 needs 6.

## 12. Later, not in this plan

* **The overview screen:** several pane subscriptions, one per agent of
  interest, each showing its last lines and live cards. The protocol
  already allows it: up to 8 subscriptions a connection, by pane.
* File events in place of the 250 ms check.
* Subscriptions for harness clients (plan 20261022_03 step 5) and through
  `handlr mcp`.
* Pushing pane scrollback (`pane.read`) for agents without a session
  reader, possibly through herdr's `pane.output_matched` or
  `pane.scroll_changed`.

## 13. Open questions for the owner

1. **Follow.** The owner: "we'll probably have a follow button of sorts
   to enable/disable that … we'll have to figure that one out". This plan
   leaves it client-side, behind a setting, off by default. To decide
   later:
   * where the button sits (the view's header, the main screen, or both);
   * whether it is on by default;
   * whether opening a view of a pane other than the focused one turns it
     off;
   * whether the companion should also be able to *move* herdr's focus
     (`agents.focus`) from the view, the other direction of the same link.
   None of these changes the daemon or the protocol.
2. **How other panes are reached.** For now the view opens only from the
   focus row, so its first pane is always the focused one. Checking in on
   other projects needs a way to pick a pane: the overview screen, or a
   list. That is for its own plan, and the protocol already allows it.
3. **Subscribe while the view is closed?** This plan subscribes only
   while the view is open. The alternative is to subscribe from connect,
   so the main screen could show the newest line and the view would open
   already full, at the cost of traffic while nobody is looking.
4. **The cursor across a daemon restart.** Offsets survive, so this plan
   continues from them. If you would rather reset after a restart, the
   cursor gains the daemon's start time.

## Amendment — 2026-10-23 14:54 EDT: sections, serials and scrolling back through the session

Agreed with the owner on 2026-10-23: "if I scrollback, it should
scrollback in the session and rebuild the missing cards/sections";
"sections is all of those, blocks of text output, actual cards, turns,
etc. they should all have a serial number associated with them
monotically"; "one serial number, not a different one per type"; "it
could be derived from the jsonl line if that makes sense, with the
filename hashed in"; "maybe the filename hash is the prefix?"; "that
works, as long as we can store and compare them, and use them for
backfill"; "the UI should always show them in order".

This amendment replaces, where they disagree: decision 5 (cursors) and
the cursor format in 4.2; `Entry.seq`; the `entries` fields of 4.1 and
4.2 (deliveries now carry sections); the ordering rules of 4.4; the
client's card handling in section 6; the app's "the agent view does not
change" in section 7; and the numbered steps 2, 3, 4, 5, 6 and 7, which
gain the work below. Everything else stands.

### A1. Sections

A **section** is one unit of the agent view. There are four kinds:

| `kind` | What it is |
|---|---|
| `turn` | A user prompt: the start of a turn |
| `text` | A block of the agent's output: consecutive assistant text entries with no tool call between them |
| `tool` | A tool call and its result, once the result arrives |
| `card` | A priority card: Approval, Question, Something broke, Finished |

Sections are flat and ordered: a turn's text, tools and cards follow it
in serial order until the next `turn`. Entries of other kinds (thinking,
system lines the readers already drop) form no section.

### A2. The serial

Every section has one serial, from one sequence shared by all four kinds.
It is derived from the session file, not counted:

* **file**: a 32-bit FNV-1a hash of the session file's name (for example
  `992c3f44-18e7-4cf8-814f-e437b90209b6.jsonl`), not its path;
* **line**: the 1-based number of the JSONL line the section starts on.
  Every line counts, including lines the readers skip or do not parse,
  so a later reader that understands more line types gives the same
  numbers;
* **slot**: the section's place among those starting on that line,
  0 to 255: the line's own blocks first, in order (a Claude line is one
  message, which can hold text and several tool calls), then the cards
  anchored after it, highest priority first. A line with more than 256
  sections puts the rest into slot 255's section.

**On the wire** it is a string with the hash as its prefix:
`"3fa9c21e:412.2"`, that is `<file hash, 8 lower-case hex digits>:<line,
decimal>.<slot, decimal>`. A full u64 would exceed 2^53, which
JavaScript and many JSON readers cannot hold exactly.

**In memory** it is `libhandlr_proto::common::Serial`:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Serial { pub file: u32, pub line: u32, pub slot: u8 }

impl Serial {
    /// The packed form: file in bits 63–32, line in 31–8, slot in 7–0.
    pub fn to_u64(self) -> u64;
    pub fn from_u64(v: u64) -> Self;
    pub fn same_file(self, other: Serial) -> bool;
}
impl PartialOrd for Serial { /* None across files; else by (line, slot) */ }
impl FromStr for Serial { /* "3fa9c21e:412.2" */ }
impl Display for Serial { /* the same form */ }
// Serialize and Deserialize as the string.
```

* **Store:** it round-trips exactly through the string, the u64 and
  serde, so a client can keep it in memory, in a file or in a database.
  `line` must fit 24 bits (16,777,215 lines); a longer file is a parse
  error for serials past that line, and the daemon sends a reset with
  `reason: "too_long"`.
* **Compare:** within one file, by line then slot. Serials of two files
  have no order (`partial_cmp` returns `None`); a different prefix means
  a different session.
* **Backfill:** it replaces the cursor. `session.subscribe`'s `after`
  and `session.history`'s `before` are serials. The `c1:` cursor format,
  `SessionDelivery.cursor` and `Entry.seq` are dropped; a delivery's
  newest serial is what the client gives back.
* **Stable:** the same line gives the same serial on every read, after
  a reconnect, after a daemon restart, and for every client. It does not
  depend on how the daemon groups sections, so changing the card rules
  later does not renumber anything already shown.

After `/clear` the pane's agent writes a new file, so serials get a new
prefix and the subscription sends a reset, as in decision 6.

### A3. The daemon builds sections and runs the card rules

The four card rules move from `crates/handlr-companion/src/view.rs` into
the daemon, as `src/sessions/sections.rs`, run by the hub once per
tailer and shared by all its subscribers, as redaction is (decision 7).
The rules are unchanged in what they find. They become a pure function
over a span of entries plus the pane's status, pending question and
confirmations, so the same function finds live cards at the newest end
and resolved cards in history.

```rust
pub struct Section {
    pub serial: Serial,
    pub kind: SectionKind,          // Turn, Text, Tool, Card
    pub entries: Vec<Entry>,        // redacted, oldest first; empty for some cards
    pub card: Option<CardBody>,     // for kind Card
}
pub struct CardBody {
    pub card: CardKind,             // Approval, Question, Broke, Finished
    pub live: bool,
    pub title: String,
    pub text: String,
    pub confirm_id: Option<u64>,
}
```

`Section`, `SectionKind`, `CardBody` and `CardKind` are in
`libhandlr_proto::common`. A section that changes is sent again with the
same serial: a text block that grows, a tool group that gets its result,
a card that goes from live to resolved. The client replaces it.

The tailer counts lines as it reads and keeps each line's start offset
(8 bytes a line; a 100,000-line session costs 800 KB), built by one scan
of the file when the tailer starts. History pages are found from it.

### A4. The approval journal

Approval cards come from handlr's confirmations, not the session file.
To rebuild them in history and after a restart, the daemon appends each
confirmation that reaches a session's pane to
`~/.local/state/handlr/sessions/<session_id>.jsonl` (directory 0700, file
0600), one line each:

```json
{"ts":"2026-10-23T18:40:02Z","line":412,"slot":2,"confirm_id":17,
 "command":"/compact","outcome":"approved"}
```

`line` is the session file's newest line when the confirmation was
made, and `slot` the card's slot after that line; `outcome` is written
when it resolves (`approved`, `declined`, `expired`), as a second line
for the same `confirm_id`. The command is the redacted text the card
showed. The serial does not depend on the journal: a missing or deleted
journal loses only the Approval cards, never shifts a number.

### A5. The protocol, as amended

`session.subscribe` params: `pane_id`, `sections` (the backfill size, 1
to 200, 50 by default; replaces `entries`), `after` (a serial, optional).

`SessionDelivery`: `subscription_id`, `pane_id`, `agent`, `reset`,
`session`, `reason` (adds `too_long`), `sections` (replaces `entries`),
`status`, `pending_question`. A delivery may resend sections at or below
the newest serial already delivered (changes); new sections are always
above it.

New method, **`session.history`**:

```json
{"jsonrpc":"2.0","id":9,"method":"session.history","params":{
  "subscription_id": 3, "before": "3fa9c21e:412.0", "sections": 100}}
```

* `before`: a serial; the page is the sections just below it, from the
  subscription's current session file.
* `sections`: 1 to 200, 100 by default.
* Result: `{"sections": [ … oldest first … ], "start": true}`. `start`
  is true when the page reaches the file's first line.
* The file is read backwards from the line's offset in 256 KiB chunks;
  a line split across chunks is joined; a line longer than `TAIL_MAX` is
  handled as `read_tail` handles it now.
* Cards in the page are found by the same rules, as resolved, and the
  journal's Approval cards for the page's lines are merged in.
* Redaction in an older page: a tool result whose call is on a line not
  in the page keeps its text only if its own text names no secret path
  (decision 7's rule for an unknown call). When the page holds both, the
  full rule applies.
* One history request in flight per subscription; a second waits for
  the first.

| Error | When |
|---|---|
| `SUBSCRIPTION_NOT_FOUND` (-32013) | an unknown `subscription_id` |
| `SESSION_CHANGED` (new, -32014) | `before`'s prefix is not the current session file's; the client waits for the reset delivery |
| `INVALID_PARAMS` | a serial that does not parse; `sections` out of range |

Back-pressure (decision 9) counts sections: one catch-up delivery holds at
most 200 sections and 256 KiB of JSON; past that it becomes a reset
carrying the newest 50 sections.

### A6. Ordering, in the protocol and the UI

* Within a subscription, new sections arrive in increasing serial order;
  resent sections may repeat an earlier serial.
* **The UI always shows sections in serial order**, whatever order they
  arrive in: a history page that lands after live deliveries, a resent
  section, a rebuilt card, a reconnect's backfill. The client keeps its
  sections sorted by serial and inserts by binary search; a serial
  already held replaces that section in place, never adds a copy.
  Sections whose prefix is not the view's current one are ignored unless
  they come with a reset.

### A7. The companion client and the app

* `AgentView` holds `Vec<Section>` sorted by serial instead of entries
  and cards. The card rules, `Cards`, `Facts` and `live_cards` leave
  `view.rs`; the client applies sections and finds the pinned card (the
  highest live one) among them.
* On reconnect, an open view subscribes with `after` set to its newest
  serial.
* **Scrolling back:** when the top of the flow comes within one screen
  of the view's top, the client asks `session.history` with `before` set
  to its oldest serial. A dim "loading earlier…" row shows at the top
  while it is in flight, and a dim "start of session" row once a page
  says `start`.
* Prepending a page keeps the section under the user's thumb at the same
  place on screen (the L3-1 rule).
* Up to 1000 sections are held while scrolled back. When the view is
  pinned to the bottom again, it trims to the newest 200.
* The app lays out sections: `turn` as a prompt line, `text` and `tool`
  as terminal rows as now, `card` as a card block (live raised, resolved
  faded).

### A8. Testing, added to section 10

1. **Serial:** string, u64 and serde round trips; ordering within a file
   and `None` across files; the 24-bit line limit; parsing errors.
2. **Sections:** recorded Claude and Codex transcripts give the expected
   sections and serials; a line with several tool calls gets slots in
   order; the same file read twice, and read after appending, gives the
   same serials for the same lines; adding a skipped line type does not
   renumber.
3. **The backward reader:** lines straddling a 256 KiB chunk; a line
   longer than `TAIL_MAX`; pages that walk the whole file back to
   `start: true` and together equal one forward read.
4. **Redaction in history:** a page whose oldest result's call is in the
   next older page shows no text from a secret path.
5. **Rebuilt cards:** the cards a forward, live run found and resolved
   equal those a history read finds, by serial and kind; the journal's
   Approval cards land at their serials; a deleted journal loses only
   them.
6. **The UI order:** the same sections delivered shuffled, duplicated and
   split between live deliveries and history pages give the same sorted
   view; a resent section replaces in place.
7. **Over the Noise link:** a 200-section history page spanning frames.
8. **Live, read-only:** `handlr session tail --pane <id> --history N`
   pages back N sections and prints serials, logged with the live check
   in `out/sessions/YYYYMMDD_NN_subscriptions_live/`.
9. **On the S23:** scrolling back to the start of a long session loads
   pages and shows past cards, faded, in order.

### A9. The steps, as amended

* Step 2 (libhandlr_proto) also adds `Serial`, `Section`, `SectionKind`,
  `CardBody`, `CardKind`, `session.history`'s params and result, and
  `SESSION_CHANGED` (-32014); it drops `Entry.seq` and the cursor.
  **Done when:** A8.1 and the round trips pass.
* Step 3 (the hub) also builds the line index, sections and serials
  (`src/sessions/sections.rs`, with the card rules), and the backward
  reader. **Done when:** A8.2 to A8.5 pass.
* Step 4 (the daemon) also adds `session.history` and the approval
  journal. **Done when:** A8.7 passes as well.
* Step 5 (the CLI) also adds `--history N`. **Done when:** A8.8 passes.
* Step 6 (the client) holds sorted sections, removes its card rules, and
  pages back. **Done when:** A8.6 passes.
* Step 7 (the app) lays out sections and adds the scroll trigger and the
  marker rows. **Done when:** A8.9 passes.

### A10. Left for later

Scrolling back past the current session file into the pane's earlier
sessions (before a `/clear`). The prefix already tells sessions apart;
reaching an earlier file needs a way to find it and is not in this plan.

## Amendment — 2026-10-23 22:55 EDT: sections kept in memory for full scrollback; what changed since the draft

Agreed with the owner on 2026-10-23: "should we use redb?", then "right,
if in memory but we should still store sections by serial and have a full
scrollback", then "Approve the plan". This replaces the 14:54 EDT
amendment's A7 bullet "Up to 1000 sections are held while scrolled back.
When the view is pinned to the bottom again, it trims to the newest 200",
and brings the plan in line with what was built since it was drafted.

1. **The phone keeps every section it has loaded,** in memory, for the
   life of the agent view: a map sorted by serial (`BTreeMap<u64,
   Section>` on the packed serial), with no cap and no trimming at the
   bottom. Scrolling back pages in with `session.history` until a page
   says `start`, so the whole session file is reachable. Nothing persists
   across app restarts (no redb or other store); reopening backfills from
   the daemon.
2. **Memory guard:** sections far from the window (more than five screens
   away) keep their data but drop their laid-out rows; they are laid out
   again when scrolled near.
3. **The interim merge goes.** The client's `merge_entries` and
   `VIEW_KEEP` (the 22:53 EDT CHANGES.md entry) are replaced by the
   section map when step 6 lands.
4. **What exists since the draft, which the steps build on:**
   - entries carry `links` and `origin` (libhandlr_proto 0.3.3, 0.3.4);
     a section keeps its entries' links, and a context entry (with an
     origin) is its own section of kind `context`, never part of a turn
     and never a card;
   - the overview's `Turn` and the pending question (with `about_plan`)
     are computed above the readers; the hub reuses them;
   - the reader's read budget skips bookkeeping lines
     (`SessionReader::is_bookkeeping`); the line index counts every line,
     bookkeeping included, so serials do not depend on what is skipped;
   - the link's sleep (`client.options` `background`, `ping {active}`,
     the 60 s inference): a sleeping connection gets no
     `session.delivery`; on waking, a subscription catches up with `after`
     set to its newest serial;
   - the session readers plan (20261023_06) will make the trait record
     based; this plan's hub reads through the trait as it is, and moves
     with it.
5. **Versions,** by the owner's rulings ("appending to a struct is
   additive"; "same with open enums, not closed"): new methods, new
   structs and fields appended to structs are patch releases of
   libhandlr_proto; a variant added to a closed enum is a minor release.
   libherdr's `Subscription` is closed, so step 1 is libherdr 0.3.0, as
   planned.

## Amendment — 2026-10-23 22:55 EDT: a correction to the previous amendment's reference

Item 3 of the previous amendment cites "the 22:53 EDT CHANGES.md entry";
that entry, "the agent view keeps what it showed", is at 22:51 EDT.

## Amendment — 2026-10-23 23:02 EDT: implementation has started; the document is now immutable

The owner approved building the plan on 2026-10-23: "Approve the plan".
Implementation starts with step 1 (libherdr 0.3.0, the focus and pane
subscriptions). From this commit the document is frozen; departures are
recorded in CHANGES.md (Rule 1.7).

## Amendment — 2026-10-30 15:20 EDT: the last view cached, shown at start and refreshed on connect

On the owner's instruction (Rule 1.5), 2026-10-30: "Last session should also return to that remote if available or to the most recent remote and session if not. Ideally this will be loaded from cache so the user is sent back to a working screen with the updates loading in the background (like pull to refresh)"; agreed as
proposed. For the settings screen plan's *At startup* (its amendment of
the same time):

1. **What is kept.** When the agent view closes, or the app goes to the
   background with it open, the client keeps that session's view: its
   outline window and the sections in it (the last screenful and the
   windows either side, at most 200 sections or 512 KiB of section JSON,
   whichever is less), with the pane, the session id and the time, in the
   remote's `state.redb` (a `view` table, one entry per remote,
   replaced each time).
2. **Shown at once.** flankr-companion, additive: `State::restore_view`
   gives the kept view as the usual `Event::AgentView` events, marked as
   cached, before the connection is up, so the agent view opens usable:
   scrolling, reading and tapping links work.
3. **Refreshed on connect.** When the link is up, the subscription opens
   as it does now and its deliveries merge into the cached view by
   section serial; the agent view shows the pull-to-refresh indicator
   until the first delivery has arrived.
4. **Gone.** If the session is no longer listed, or the subscription says
   there is no session, the view says *This session has ended* with a
   button back to the overview, instead of an empty view.
5. **Versions.** flankr-companion takes the next patch (additive); the
   app's version is moved by the settings plan's amendment.
