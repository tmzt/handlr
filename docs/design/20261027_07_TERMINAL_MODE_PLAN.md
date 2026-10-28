---
title: A terminal mode for a session, tunnelled over the link
date: 2026-10-27
status: In implementation
covers:
  - A per-session terminal view in the companion app, beside the transcript
    view, switched by a swapped terminal/list glyph in the bottom bar
  - A new crate, `libflankr_term`, with the screen-diff state machine
    (mosh-like: the daemon keeps the last frame the phone acknowledged and
    sends diffs from it) shared by the daemon and the app
  - The `terminal.*` methods and the binary frame kind that carries diffs
    over the existing Noise link
  - Keyboard input into the terminal, behind an explicit per-agent setting,
    an approving principal and a per-session toggle; read-only first
  - Not covered: herdr changes (a cell grid with attributes and a cursor is
    requested of herdr as an open question); local echo prediction; a
    terminal for panes that are not agent sessions
sources:
  - The owner (2026-10-27), verbatim: "adding a terminal mode, tunneled via
    the same protocol but using a mosh-like update with the same binary
    framing protocol as the main flankr protocol (different message ids),
    bumping the base version if needed, it would be it's own crate,
    switching between terminal and transcript would be via a swapped
    terminal/list icon in the second slot on the bottom bar, per session.
    if this is not feasible in the same app, we should say so."
  - `crates/libflankr_proto/src/link/mod.rs` (the link's framing: 2-byte
    length, Noise message, a flag byte `0x00`/`0x01` then a chunk; 1 MiB
    messages; prologue `flankr.link/1`), `link/stream.rs` (`send_message`,
    `recv_message`, `into_split`), `link/handshake.rs`
  - `crates/libflankr_proto/src/lib.rs`, `envelope.rs`, `common.rs`,
    `companion.rs` (JSON-RPC 2.0 over the link; `hello` with `protocol`
    and `version`; `companion::VERSION = 1`; the method families)
  - `src/server/conn.rs:468-532` (the exact-version check in `hello`,
    `UNSUPPORTED_PROTOCOL`), `src/server/link/mod.rs` (link connections
    carry the same JSON-RPC lines)
  - `crates/libherdr/src/protocol.rs` (`HerdrRequest::InspectPane { lines,
    source }`, `ReadSource::{Visible, Recent, RecentUnwrapped, Detection}`,
    `RunCommand`, `SendKeys`), `events.rs` (`pane.scroll_changed`,
    `pane_updated`, `pane_focused`, `pane_closed`), `client.rs`,
    `transport.rs:644` and `lib.rs:56` (the read's result is `{"buffer":
    "<text>"}`)
  - `src/core/policy.rs:101` (`check(principal, agents, agent, pane_id,
    command)`), `src/core/principal.rs` (`Principal { id, agents,
    can_approve }`), `src/core/confirm.rs`, AGENTS.md section 3
  - `apps/companion/src/agent.rs:236-262` (`AgentHit`), `:5660-5676` (the
    bottom bar's four buttons and their order), `:5574` (the transcript's
    `NOTO_SANS_MONO_ATLAS` and `TERMINAL_SYMBOLS_ATLAS`),
    `apps/companion/src/prompt/keyboard.rs` (`KeyboardRoute`, the one
    place the soft keyboard is raised), `apps/companion/src/prompt/bars.rs`
  - `VERSIONS.md` (libflankr_proto 0.3.12; the owner's ruling that an
    appended, serde-defaulted field is additive)
  - `docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md`, decisions 1-2
    and the amendment of 23:07 EDT (the link)
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (principals,
    policy, the JSON-RPC envelope)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (the link this
    channel rides on)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (its living section
    L3: the agent view, the bottom bar, the keyboard glyph)
  - docs/design/20261027_03_UNIFIED_PROMPT_CARD_PLAN.md (the keyboard route)
---

# A terminal mode for a session, tunnelled over the link

## 1. Goal

From the agent view of a session, the owner can flip to the pane's terminal
as herdr shows it, live, and back to the transcript, with one tap on a glyph
in the bottom bar. The terminal arrives over the paired Noise link the app
already holds, as small diffs against the last frame the phone acknowledged,
so a slow or lossy phone network shows a slightly stale screen rather than a
backlog. Typing into the terminal is a second, later step, gated as strictly
as sends are today.

## 2. Findings

### What exists

- **One link, JSON-RPC inside.** Every flankr protocol is JSON-RPC 2.0, one
  object per message (`crates/libflankr_proto/src/lib.rs:5-10`). The link
  wraps each message in Noise frames: a 2-byte big-endian length, a Noise
  message of at most 65535 bytes whose plaintext is a flag byte (`0x00`
  last, `0x01` more) and a chunk; a message reassembles to at most 1 MiB
  (`link/mod.rs:14-17`, `MAX_FRAME`, `MAX_CHUNK`, `MAX_MESSAGE` at
  `link/mod.rs:60-65`). `LinkStream::send_message`/`recv_message`
  (`link/stream.rs:43-73`) move whole messages; `into_split` gives a reader
  and a writer for separate tasks (`link/stream.rs:84`). A frame whose flag
  byte is not `0x00`/`0x01` is `LinkError::BadFrame` and closes the
  connection (`link/mod.rs:99-100`).
- **There is no binary message-id layer.** The "ids" of the main protocol
  are JSON-RPC method strings (`common::methods`, `companion::methods`) and
  per-request `Id`s (`envelope.rs:32`). The only binary bytes the link
  defines are the handshake mode byte (`MODE_RECONNECT = 0x02`) and the
  per-frame flag byte. So "the same binary framing protocol with different
  message ids" maps to: the same Noise frames, with a new value of the flag
  byte marking a terminal payload (decision 3).
- **Versions.** A connection names `protocol` and `version` in `hello`
  (`common.rs:83-86`); the daemon serves exactly `COMPANION_VERSION`
  (`companion::VERSION = 1`, checked with `==` at
  `src/server/conn.rs:468-469`, else `UNSUPPORTED_PROTOCOL`). The link's
  prologue is `flankr.link/1` (`link/mod.rs:54`). The crate is
  `libflankr_proto` 0.3.12; by the owner's ruling an appended,
  serde-defaulted field and a new method are additive (patch bump).
- **Herdr gives text, not cells.** `libherdr` reads a pane with
  `HerdrRequest::InspectPane { pane_id, lines, source }`
  (`protocol.rs:232-240`), where `ReadSource::Visible` is "the visible
  frame only" and `Recent`/`RecentUnwrapped` are tails
  (`protocol.rs:92-104`). The result is `{"buffer": "<text>"}`
  (`transport.rs:644`, `lib.rs:56`), rows separated by newlines. There is
  no cell grid, no attributes (colour, bold, inverse), no cursor position,
  no reported width and height, and no byte stream of the pane's output.
  Herdr's own source is not checked out under `~/src/herdr`, so this plan
  takes libherdr's protocol as the record.
- **Herdr tells us when to look.** The subscription `pane.scroll_changed
  { pane_id }` fires when the pane produces output (`events.rs:43-45`),
  with `pane_updated`, `pane_focused` and `pane_closed` beside it
  (`events.rs:127-150`). The daemon already keeps one herdr event
  subscription for agent status.
- **Input is `send_keys` and `run_command`** (`protocol.rs:243-256`,
  `client.rs:125-135`). Every send today passes `policy::check(principal,
  agents, agent, pane_id, command)` (`src/core/policy.rs:101`) and, where
  the agent requires it, a confirmation (`src/core/confirm.rs`, AGENTS.md
  section 3). Arbitrary text is restricted to letters, digits, spaces and
  `. , ' ? :`; `/`, `-`, control characters and anything over 256
  characters are refused (AGENTS.md 3.A). A `Principal` is `{ id, agents,
  can_approve }` (`src/core/principal.rs:5-13`); paired devices may be
  allowed to approve, local clients never are.
- **The app's bottom bar has four buttons** in the agent view: play
  (`ReadAloud`) at the far left, the general agent in the centre, the
  keyboard glyph just left of the microphone, the microphone (`Message`)
  at the far right (`apps/companion/src/agent.rs:5660-5676`). The left
  half has one button where the right half has two: the place between
  play and the centre is empty. The transcript is drawn with
  `NOTO_SANS_MONO_ATLAS` for text and `TERMINAL_SYMBOLS_ATLAS` for box and
  block glyphs (`agent.rs:5574-5576`), so a fixed-advance row renderer
  already has its fonts. The soft keyboard is raised in one place,
  `KeyboardRoute::sync` (`prompt/keyboard.rs:1-10`), keyed by a
  `FocusedPrompt`; a physical keyboard types directly.
- **Per-session state** lives in the agent view keyed by the session's
  pane id (`BlockState` and the slot machinery, `agent.rs:347`,
  `:2164-2270`); the view is rebuilt per session with its own scroll and
  cards, so a per-session "terminal or transcript" flag has a home.

### What is missing

- A screen model richer than text: no cursor, no attributes, no size. The
  daemon cannot run its own terminal emulator (a `vt100`/`alacritty_terminal`
  state fed by the pty bytes) because herdr exposes no output byte stream,
  only snapshots on request. Mosh's model needs the server to own the
  screen state; here herdr owns it and flankr can only poll a text frame.
- A binary payload kind on the link; a `terminal.*` method family; a
  place in `HelloResult`/`server.capabilities` to say the daemon has it.
- A keystroke path that is not a "command": `send_keys` exists in herdr,
  but flankr's policy has no notion of raw keys, only commands and
  messages.
- A grid renderer with horizontal pan and pinch zoom in the app; a
  terminal/list glyph in the bundled atlases (the keyboard glyph exists;
  a "list" glyph and a "terminal" glyph are to be confirmed in
  `TERMINAL_SYMBOLS_ATLAS`, else added to libmsdf).

## 3. Feasibility verdict

**Feasible in the same app, with a reduced model.** The link, the per-session
agent view, the monospace atlases and the keyboard route are all in place,
and a line-diff channel adds one frame kind and five methods to the existing
protocol with a patch bump. What is *not* feasible today is a mosh-grade
terminal: herdr gives flankr a plain-text visible frame on request and a
"something changed" event, not a cell grid, a cursor or a byte stream. So
the first terminal mode is a **live, read-mostly text mirror** of the pane's
visible frame (no colours, no cursor, no local echo), refreshed at most a
few times a second from herdr and sent to the phone as row diffs against
the last acknowledged frame. That is enough to watch a coding agent's TUI
and to catch what the transcript misses (spinners, prompts, errors on the
screen). Colours and a cursor arrive only if herdr grows a cell-grid read
(open question 1); the channel below is designed so that is an additive
change on top.

## 4. Decisions

1. **Crate.** `crates/libflankr_term` (`libflankr_term`), version 0.1.0,
   depends on `serde` and `libflankr_proto` only. It holds: the frame types
   (`Frame`, `RowDiff`, `Ack`), the server-side `Mirror` (last sent frame,
   last acknowledged frame, the diff from the acknowledged one), the
   client-side `Screen` (applies frames in any order by `seq`, keeps the
   newest), and the text-to-rows normaliser (tabs expanded, trailing spaces
   cut, rows padded or cut to `cols`). Both the daemon and
   `flankr-companion` depend on it. No herdr or link dependency: it is pure
   state.
2. **Methods** (in `libflankr_proto::common::methods`, served to the
   companion protocol only at first):
   - `terminal.open { session, cols_hint?, rows_hint? }` ->
     `{ terminal_id: u32, cols, rows }`: start mirroring the pane of
     `session` for this connection. One terminal per session per
     connection; a second `open` returns the same id. Hints are what the
     phone could show; the daemon reports the pane's size as herdr gives it
     (today: the longest row and the row count of the visible frame).
   - `terminal.close { terminal_id }` -> `{}`.
   - `terminal.ack { terminal_id, seq }` -> notification, phone to daemon:
     the newest frame the phone holds. The daemon diffs from it thereafter.
   - `terminal.input { terminal_id, keys }` -> `{}` (step 6 only; before
     that, `NOT_PERMITTED`). `keys` is herdr's `send_keys` syntax, at most
     64 bytes per call.
   - `terminal.closed { terminal_id, reason }` -> notification, daemon to
     phone: the pane closed, the agent changed, or input was revoked.
   Each takes the same `NOT_PERMITTED`/`UNKNOWN_AGENT`/`HERDR_UNAVAILABLE`
   codes as `agents.read`; no new codes. An older daemon answers
   `METHOD_NOT_FOUND`, which the app treats as "no terminal mode".
3. **Framing: one new frame kind, JSON stays the control plane.** The flag
   byte gains a kind bit: `0x00`/`0x01` remain a JSON-RPC message
   (last/more), `0x02`/`0x03` are a terminal frame (last/more). A terminal
   frame's plaintext after the flag byte is: `u32 terminal_id`, `u32 seq`,
   `u32 base_seq` (0 = a full frame), `u16 cols`, `u16 rows`, then the
   diff: `u16 count` rows, each `u16 row`, `u16 len`, `len` bytes UTF-8.
   All big-endian. A frame is one message, so it may span chunks like any
   other; a full 200x60 frame is about 12 KB, a diff usually under 1 KB.
   `LinkStream` gains `recv() -> Result<Message, LinkError>` with
   `Message::{Json(Vec<u8>), Terminal(Vec<u8>)}` and `send_terminal(&[u8])`;
   `recv_message` is unchanged and still returns `BadFrame` on a terminal
   frame, so an older peer is never sent one: the daemon sends terminal
   frames only to a connection that called `terminal.open`, which proves
   it is new. Additive, so a patch bump.
4. **Version impact.** No bump of the base version: `companion::VERSION`
   stays 1, the prologue stays `flankr.link/1`, the handshake is unchanged.
   `HelloResult` gains an appended `terminal: Option<TerminalInfo>` and
   `ServerCapabilities` the same field, `None` from an older daemon. The
   `libflankr_proto` release is 0.3.13 (additive by the owner's ruling);
   a later change that makes the kind bit mandatory or changes `recv_message`
   would be a minor bump and is not planned. If the owner wants explicit
   negotiation instead, `companion::VERSION = 2` served beside 1 is the
   alternative (open question 2); it costs a minor bump and a change at
   `conn.rs:468`.
5. **The sync model (what of mosh fits).** The daemon's `Mirror` is the
   mosh server's half without the emulator: it keeps the last frame it sent
   and the last the phone acknowledged, and each new frame is a diff from
   the *acknowledged* one, so a lost or reordered frame costs nothing and
   a frame applies idempotently. Frames are rate-limited to 8 per second
   per terminal and sent only when the frame changed. A frame is read from
   herdr with `inspect_pane_from(pane_id, rows, ReadSource::Visible)` on
   `pane.scroll_changed` for that pane (coalesced to the rate limit), and
   every 2 s regardless while a terminal is open, since a TUI can redraw
   without scrolling. If the diff is larger than the full frame, the full
   frame is sent (`base_seq = 0`). On reconnect the phone's `open` gets a
   full frame. No prediction or local echo: the daemon does not own the
   pty, and the round trip to herdr makes echo guesses wrong often enough
   to be worse than none.
6. **Input and security.** Read-only ships first (steps 1-5) and is
   gated like `agents.read`: `principal.may_reach(agent)` and the agent
   configured in `[[agents]]`. Keystrokes (step 6) need *all* of:
   - `[[agents]] terminal_input = true` on that agent (off when left out)
     and `[terminal] input = true` in the daemon config (off when left
     out), so a fresh install cannot type into any pane;
   - a principal with `can_approve` (a paired device the owner trusts to
     approve sends; local clients never type);
   - a per-session toggle in the app, off each time the terminal view
     opens, armed by a 1.5 s hold on the terminal (not a tap) and shown as
     a coloured frame with "Typing into <agent>"; it drops after 60 s
     without a key, on leaving the view, and when the pane's agent changes
     (`SESSION_CHANGED`);
   - at the daemon, `terminal.input` is refused unless the terminal was
     opened on this connection and the toggle's `terminal.input_arm
     { terminal_id }` request succeeded within the last 60 s; each batch is
     at most 64 bytes; control sequences other than the named keys herdr
     accepts (`enter`, `escape`, `tab`, `backspace`, arrows, `ctrl+<a-z>`)
     are refused; and `ctrl+c`, `ctrl+d`, `ctrl+z` and `ctrl+\` are refused
     unless the agent also sets `terminal_signals = true`.
   Every input batch is logged as a `confirm`-style notice (`notice`,
   kind `terminal_input`, the agent and the key count, never the keys), so
   the voice path and the transcript show that the terminal was typed
   into. The existing confirmation gate is not used for keys: it protects
   commands the model composes; here the owner types, and the gate is the
   explicit arm plus the per-agent setting.
7. **The app UI.** A fifth bottom-bar button takes the empty second slot,
   between play and the centre button, mirroring the keyboard glyph right
   of centre: it shows a **terminal glyph** while the transcript is shown
   and a **list glyph** while the terminal is shown, and a tap swaps the
   views. The choice is per session and kept while the app runs (not
   persisted; open question 4). Without a daemon `terminal` capability the
   button is drawn disabled. In terminal mode the view below the header is
   a cell grid drawn with `NOTO_SANS_MONO_ATLAS` and
   `TERMINAL_SYMBOLS_ATLAS`, one `draw_text` per row at a fixed advance,
   starting at 11 pt (about 60 columns across a 412 dp portrait screen),
   pinch to zoom between 7 pt and 16 pt, one-finger pan in both axes,
   double tap to fit the frame's width. Reflow is not attempted: a TUI's
   rows are kept as rows. The keyboard glyph in terminal mode raises the
   soft keyboard through `KeyboardRoute` with a new `FocusedPrompt` kind
   `Terminal` only while input is armed; otherwise it opens the message
   card as today. The prompt card, bars and pending question stay as they
   are over the terminal.
8. **Daemon placement.** `src/server/terminal.rs`: the per-connection
   terminal table (`terminal_id -> Mirror, pane_id, agent, armed_until`),
   the herdr poller, and the `terminal.*` handlers; `conn.rs` dispatches to
   it. The herdr event subscription gains `pane.scroll_changed` per open
   terminal and drops it on close.

## 5. Steps

1. **The crate.** Add `crates/libflankr_term` with `Frame`, `RowDiff`,
   `Mirror` and `Screen`, the byte encoding of decision 3 and its decoder,
   and tests: a diff from the acknowledged frame applies to the old screen
   and yields the new one; a reordered pair of frames ends at the newer;
   a frame larger than its full frame is sent full; the normaliser pads
   and cuts rows. Add it to the workspace.
   **Done when:** `cargo test -p libflankr_term` passes, and a round trip
   of 1000 random frame sequences through `Mirror` and `Screen` ends with
   the screens equal.
2. **The protocol.** In `libflankr_proto`: the methods and params of
   decision 2, `TerminalInfo` and the appended `terminal` fields of
   decision 4, and `LinkStream::recv`/`send_terminal` with the `0x02`/`0x03`
   kinds. `recv_message` unchanged. Tests: a terminal frame through
   `recv_message` is `BadFrame`; through `recv` it is `Message::Terminal`
   with the same bytes; a JSON message through `recv` is `Message::Json`.
   Release 0.3.13 with its `VERSIONS.md` entry and tag.
   **Done when:** the existing link tests pass unchanged and the new ones
   pass; `cargo clippy -p libflankr_proto --all-targets` is clean.
3. **The daemon, read-only.** `src/server/terminal.rs` per decision 8:
   `terminal.open`/`close`/`ack`, the poller on `pane.scroll_changed` and
   the 2 s tick at 8 frames per second at most, frames sent with
   `send_terminal`, `terminal.closed` on `pane_closed` and on an agent
   change. `hello` and `server.capabilities` report `terminal`.
   `terminal.input` answers `NOT_PERMITTED`. Tests with the mock herdr:
   opening sends a full frame; a buffer change sends a diff from the
   acknowledged frame; an unknown pane is `UNKNOWN_AGENT`; a principal that
   may not reach the agent is `NOT_PERMITTED`.
   **Done when:** `flankr ctl` (or a test client) opens a terminal on a mock
   pane and receives frames that reassemble to the mock's buffer.
4. **The companion crate.** `flankr-companion` learns `terminal.open`,
   `ack`, `close`, keeps a `Screen` per open terminal, reads with `recv`
   and exposes `terminal_screen(session) -> Option<&Screen>` and a change
   counter to the app. Reconnect reopens any terminal that was open.
   **Done when:** the cross-platform test in `cross.rs` drives a mock link
   and the `Screen` matches the daemon's mock buffer after a diff.
5. **The app, read-only.** The fifth button of decision 7, the per-session
   mode flag, the grid renderer with pan and zoom, the glyphs (added to
   libmsdf's atlas if missing), the disabled state without the capability.
   Frames are drawn as they arrive, acked after each draw.
   **Done when:** on the S23 a session's terminal shows Claude Code's TUI
   live, pans and zooms, and the glyph swaps back to the transcript with
   the scroll position kept.
6. **Input.** `[terminal] input`, `[[agents]] terminal_input` and
   `terminal_signals` in the config; `terminal.input_arm` and
   `terminal.input` with the checks of decision 6; the hold-to-arm toggle,
   its coloured frame and timeout; the `FocusedPrompt::Terminal` keyboard
   route; the `terminal_input` notice. Tests: input without the arm, from
   a principal without `can_approve`, on an agent without the setting, over
   64 bytes, or a signal key without `terminal_signals` are each refused;
   an armed input reaches the mock's `send_keys` with the same keys.
   **Done when:** with both settings on, a paired device that may approve
   arms the terminal with a hold and types `ls` and Enter into a mock pane,
   and the same from a device that may not approve is refused with
   `NOT_PERMITTED`.

## 6. Risks

- **Polling cost.** Each frame is a herdr round trip (a UDS call, or an SSH
  exec in remote mode, `libherdr/src/transport.rs`). At 8 per second per
  open terminal this is fine locally and heavy over SSH; the rate limit
  and the "only when changed" rule bound it, and remote mode may need a
  lower cap (2 per second) from a config key.
- **A text frame hides state.** Without a cursor the user cannot see where
  typing goes; without attributes a TUI's highlighted selection is
  invisible. This is why input is a separate step and off by default.
- **Width.** Herdr panes are often 150-250 columns; at 11 pt a phone shows
  60. Pan is unavoidable and the first frame should land at the frame's
  right edge, where a prompt usually is, or at the fit-width zoom.
- **The kind bit** is a link-layer change in a crate that `snow` peers
  depend on; a bug there closes every connection. The tests of step 2
  guard `recv_message`'s behaviour exactly.
- **Input is a new way to reach a pane** that the confirmation gate does
  not cover; the three independent switches of decision 6 are the control.

## 7. Open questions for the owner

1. Should herdr be asked for a cell-grid read (`ReadSource::Cells`: per
   row, runs of text with fg/bg/bold/inverse, plus `cursor: (col, row)` and
   `cols`/`rows`)? With it the frame's row payload becomes runs and colours
   and the cursor are drawn; without it the mode stays plain text.
2. Capability discovery through `hello`'s appended `terminal` field (no
   version bump) or an explicit `companion::VERSION = 2`?
3. The bottom bar: the plan puts the terminal/list glyph in the empty place
   between play and the centre button (a fifth button). If "second slot"
   meant replacing the centre general-agent button in terminal mode, say
   so.
4. Should the per-session terminal/transcript choice persist across app
   runs (in the app's settings), or reset to the transcript each launch?
5. Is keyboard input wanted at all for the first release, or should step 6
   wait until herdr can report a cursor?
6. Should the companion's `terminal.input` also be offered on the harness
   protocol (a local `flankr ctl term`), or stay phone-only?

## Amendment — 2026-10-27 14:23 EDT: herdr's source and protocol; a split flag byte

The owner's request (verbatim): "tell it to checkout
https://github.com/herdrdev/herdr and continue the exploration, including the
protocol herdr uses over ssh. also investigate splitting the current byte into
two with the flags staying in one nibble."

herdr was cloned read-only to `~/src/herdr` (commit `2713feb`, 2026-10-27,
crate version 0.9.5). Nothing was run and no socket was touched. This
amendment replaces the "Herdr gives text, not cells" finding and the sentence
"Herdr's own source is not checked out" in section 2, the second half of the
verdict in section 3, decision 3's framing (the whole-byte `0x02`/`0x03`
kind), and open question 1. Where it disagrees with the body, this amendment
wins.

### Findings in herdr

1. **herdr owns a full terminal emulator.** Every pane is a Ghostty VT
   (`crates/ghostty-vt`; `src/pane/terminal.rs`), with `visible_text()`,
   `visible_ansi()` and `cursor_state() -> Option<TerminalCursorState>`
   (`src/pane/terminal.rs:2002`, `:2161-2175`). The cell grid, attributes
   and cursor exist in process; only the JSON API hides them.
2. **The JSON API already has an attributed read.** `pane.read`
   (`src/app/api/panes.rs:1512`, `src/app/api_helpers.rs:109-139`) takes
   `source` (`visible`, `recent`, `recent-unwrapped`, `detection`) **and**
   `format`: `text` (default) or `ansi` (`src/api/schema/common.rs:112-121`).
   `{source: "visible", format: "ansi"}` returns `terminal.visible_ansi()`:
   the visible frame with SGR colour and attribute escapes. The result is
   `PaneReadResult { pane_id, workspace_id, tab_id, source, format, text,
   revision, truncated }` (`src/api/schema/panes.rs:729-738`); `revision` is
   hard-wired to 0 (`panes.rs:1546`), `lines` is capped at 1000. No cursor,
   no `cols`/`rows`. libherdr's `InspectPane` sends `lines` and `source`
   only (`crates/libherdr/src/protocol.rs:232-240`), so `format` is a
   one-field, serde-defaulted, additive change on flankr's side.
3. **herdr's own clients get a mosh-like cell sync today.** The binary
   thin-client protocol (`src/protocol/wire.rs`, `PROTOCOL_VERSION = 22`,
   bincode frames behind a 4-byte little-endian length, at most 2 MiB) runs
   on the *client* socket (`src/server/socket_paths.rs:23`, distinct from
   the API socket). It carries `CellData { symbol, fg: u32, bg: u32,
   modifier: u16, skip, hyperlink }` (`wire.rs:697`), `CursorState { x, y,
   visible, shape }` (`wire.rs:749`), `PaneSurfaceFrame { boot_id,
   projection_revision, surface_revision, frame, panes, splits, popup,
   graphics }` (`wire.rs:1268`), `PaneSurfacePatchRow { x, y, cells }`
   (`wire.rs:1289`) and the `surface_delta` capability
   (`src/protocol/surface_delta.rs`: kind `endpoint.surface-delta.v1`,
   `base_projection_revision`, `base_surface_revision`,
   `GridUpdate::{Patch, Replace}`, at most 4096 spans). The server keeps a
   per-client baseline (`src/server/render_stream.rs:15-30`,
   `ClientRenderState::Semantic { last_surface, surface_revision, .. }` or
   `TerminalAnsi { blit_encoder, seq, .. }`) and diffs from it: the same
   model as decision 5's `Mirror`. A "stable endpoint contract" for
   client-owned shells (`src/protocol/endpoint.rs`,
   `ENDPOINT_PROTOCOL_GENERATION = 1`, `endpoint.hello.v1`/`welcome.v1`,
   codecs `shell.snapshot.v1`, `shell.surface.v1`,
   `shell.input.semantic.v1`) is documented as a floor "for Local, SSH, and
   Cloud shell endpoints" that "must remain available indefinitely". But it
   is the whole composed shell surface (all panes, splits, popups, sized by
   `EndpointClientHello.surface_size {cols, rows}`), not one pane, and its
   encoding is bincode of private Rust types with no published document; a
   third party speaking it would track herdr's source. The API has
   `client_shell.surface.set` (`docs/next/api/herdr-api.schema.json:4895`)
   only to size such a shell.
4. **The protocol over SSH.** herdr's remote mode is not RPC over ssh: it is
   "a remote thin-client launcher over SSH command stdio"
   (`src/remote/attach.rs:1`). `herdr --remote <target>` runs
   `ssh <target> exec herdr --session <name> remote-client-bridge
   [--idle-timeout-v1]` (`attach.rs:417-433`, dispatched at
   `src/main.rs:565`). On the host, `run_remote_client_bridge`
   (`src/remote/host.rs:6-37`) starts the server if needed, connects to the
   remote client socket and pumps ssh's stdin/stdout to it
   (`platform::forward_remote_bridge_stdio`). Locally the ssh child's stdio
   is served on a per-process Unix socket,
   `herdr-remote-<pid>-<target>-<session>.sock` (`attach.rs:3633-3642`),
   and the local thin client connects to that as if to a local server. So
   the bytes over ssh are the binary client protocol (version 22, endpoint
   generation 1), one ssh session per attached client, under a managed
   ssh_config with `ControlMaster=auto` and a herdr-owned `ControlPath`
   (`attach.rs:1430-1460`); the authentication probe uses `BatchMode=no`,
   `StrictHostKeyChecking=yes`, `-T <target> exit` (`attach.rs:796-812`).
   libherdr's transport (`crates/libherdr/src/transport.rs:34-45`,
   `:495-556`) is a different channel to a different socket: one `ssh` exec
   per JSON request (`ControlMaster=auto`, `ControlPath`, `ControlPersist`),
   whose remote command is `nc -U <api socket>` (`SocketPipe`, the default),
   `herdr cli --socket <api socket>` (`HerdrCli`) or a template, against the
   *API* socket `~/.config/herdr/herdr.sock` (`transport.rs:21-29`),
   newline-delimited JSON. The two never meet. For the terminal poller this
   confirms risk 1: in remote mode each frame is one ssh exec over the
   multiplexed master unless the poller shares libherdr's long-lived
   subscription process (open question 8).
5. **Events and input.** Besides `pane.scroll_changed`, the API has
   `pane.updated`, `pane.wait_for_output`, `pane.output_matched`,
   `pane.resize`, `pane.scroll`, `pane.send_text`, `pane.send_input`,
   `pane.send_keys` and `pane.selection.read` (schema.json). There is no
   per-pane "screen changed" or revision event.

### Revised verdict (replaces the second half of section 3)

Feasible in the same app, and richer than the body says: with
`format: "ansi"` the visible frame carries colours, bold, inverse and the rest
of SGR, so the first terminal mode is a **live, attributed mirror**, not a
monochrome one. What is still missing for a cursor and an exact size is small
and sits in herdr's JSON API, not its emulator (addition (a) below). A
mosh-grade sync fed by a true cell grid is possible from state herdr already
holds and already ships to herdr's own clients (`surface_delta`); it is not
reachable from the JSON API today and is not needed for the first release.

Consequences for the body: decision 1's text-to-rows normaliser becomes an
ANSI-to-runs normaliser (SGR parsed into runs of `{ text, fg, bg, flags }`
per row; other escapes dropped), and `RowDiff` carries runs; decision 5's
read is `InspectPane { source: Visible, format: Ansi, lines: rows }`;
decision 8's poller skips a frame whose ANSI string is unchanged before
parsing it; steps 1 and 3 hold with runs instead of plain rows; risk 2's
"without attributes" clause no longer applies, its cursor clause stands until
(a). The libherdr change is additive (`#[serde(default)] format: ReadFormat`
on `InspectPane`, `revision` and `truncated` surfaced from the result): a
patch bump of libherdr.

**Open question 1 is replaced.** It asked herdr for a cell-grid read. The ask
is now one of two smaller, additive herdr changes, either enough for a cursor:
(a) `pane.read` gains optional result fields `cursor: { row, col, visible }`
and `size: { cols, rows }` (herdr has both: `cursor_state()` and the
emulator's size) and a non-zero `revision`, so a poller can skip unchanged
frames; (b) a per-pane subscription to the existing
`PaneSurfacePatch`/`surface_delta` stream over the API socket (say
`pane.surface.subscribe`), which would feed `Mirror` with herdr's own diffs
and make the mode mosh-grade. (a) is a few lines in `handle_pane_read`; (b)
is a design change in herdr.

### The flag byte, split in two nibbles (replaces decision 3's framing)

Today each frame's plaintext starts with one byte, `0x00` last or `0x01` more
(`crates/libflankr_proto/src/link/mod.rs:72-73`); `read_message` matches the
whole byte and returns `BadFrame` on anything else (`link/stream.rs:206-214`),
which finishes the connection (`mod.rs:99-100`). Both peers reassemble with
`recv_message` (`src/server/conn.rs:292`; `crates/flankr-companion/src/net.rs:516`).

- **An old peer and a new value.** Any byte other than `0x00`/`0x01` is
  `BadFrame` and drops the connection, under the nibble layout exactly as
  under the whole-byte one. So, as decision 3 already requires, a new value
  is sent only to a peer that has proved it is new by calling
  `terminal.open`. An old peer never emits a new value.
- **`0x00` and `0x01` stay valid as kind 0.** With the flags in the low
  nibble and the kind in the high nibble, `0x00` is kind 0 (JSON-RPC), last,
  and `0x01` is kind 0, more: existing traffic is bit-for-bit unchanged.
- **Which nibble holds which.** Flags low (bits 0-3), kind high (bits 4-7).
  The two existing values are then the kind-0 values with no renumbering;
  `byte & 0x0F` is the flag the reassembler already switches on and
  `byte >> 4` the kind. Flag nibble: bit 0 = more (0 last, 1 more); bits
  1-3 reserved, must be zero and `BadFrame` when set, so a later use of them
  is refused cleanly by a peer of this generation. Kind nibble: 0 = JSON-RPC,
  1 = terminal frame, 2-14 reserved, 15 = extended (the kind continues in
  the next byte), which keeps the layout if sixteen kinds ever run out. All
  frames of one message carry the same kind; a change mid-message is
  `BadFrame`. The layout is recorded in `link/mod.rs`'s header beside the
  existing frame description.
- **Version under Rule 5.** No bump of the prologue (`flankr.link/1`),
  `companion::VERSION` or the handshake. In `libflankr_proto` the change is
  additive: `recv_message` keeps returning `BadFrame` for every non-kind-0
  frame exactly as today, and `recv() -> Message` and
  `send_kind(kind, &[u8])` are new functions, with `Message`
  `#[non_exhaustive]` so later kinds are additive too. That is 0.3.12 to
  0.3.13 (patch), with its `VERSIONS.md` entry, as decision 4 says. A later
  change that makes `recv_message` accept kind 1 would be observable
  behaviour and a minor bump; it is not planned. Decision 3's byte layout of
  the terminal payload after the flag byte is unchanged.

### New open questions

7. Ask herdr for (a) cursor, size and revision on `pane.read`, (b) a per-pane
   surface subscription, or both? The recommendation is (a) first.
8. In remote mode, should the terminal poller share libherdr's long-lived
   subscription process (one `nc`/`herdr cli` per connection, requests and
   events on one channel) rather than pay one ssh exec per frame? libherdr's
   subscription transport does not take requests today.
9. An ANSI visible frame of a colourful 200x60 TUI is several times its text
   size. Is the 8 Hz cap with the unchanged-string skip enough, or should
   remote mode default to 2 Hz as risk 1 suggests?

## Amendment — 2026-10-27 20:44 EDT: approved: local mode only, a minor bump of the protocol crate, the glyph between play and the centre

The owner (2026-10-27): "impl the terminal plan on a branch main-terminal,
bunp the proto crate a minor as a special case, use subagents as needed (up
to 3 opus) with path exclusion. yes, the icon goes between the play and
center buttons. only impl local mode, not ssh between the flankr daemon and
herdr".

The plan is approved with its 14:23 EDT amendment and this one, and is built
on the branch `main-terminal`. Where this amendment disagrees with the body or
the 14:23 amendment, it wins.

1. **The bottom bar** (open question 3): the terminal/list glyph is a fifth
   button between play and the centre button, as decision 7 has it.
2. **Local mode only** (open question 8; risk 1's remote clause; the 14:23
   amendment's finding 4 and question 9's remote half). The terminal is
   built only for a daemon that reaches herdr over its local socket. When
   herdr is reached over SSH (`--remote`), `hello` and
   `server.capabilities` report `terminal: None` and `terminal.open` is
   refused with `HERDR_UNAVAILABLE`. No remote-mode rate cap is added.
3. **`libflankr_proto` takes a minor bump, 0.3.13 to 0.4.0**, by the owner's
   ruling for this change only, although its changes are additive. This
   replaces "0.3.13" in decision 4, step 2 and the 14:23 amendment's
   "Version under Rule 5" (0.3.13 went to plan 20261027_06). The tag is
   `libflankr_proto-v0.4.0`. `companion::VERSION` stays 1 and the prologue
   `flankr.link/1`, so capability discovery is through the appended
   `terminal` field (open question 2, as decision 4 has it). The split flag
   byte of the 14:23 amendment stands.
4. **The open questions not answered keep the body's answer:**
   - herdr is not changed (questions 1 and 7): the frame is
     `pane.read { source: "visible", format: "ansi" }` through libherdr's
     `InspectPane`, which gains the serde-defaulted `format` field (libherdr
     0.3.0 to 0.3.1, a patch); the size is the longest row and the row
     count, and there is no cursor;
   - the per-session choice is not persisted (question 4);
   - keyboard input is built as step 6, off unless both settings are on
     (question 5);
   - `terminal.*` is served to the companion protocol only (question 6);
   - the cap is 8 frames a second with the unchanged-string skip
     (question 9).
5. **Related:** `docs/design/20261022_04_COMPANION_APP_PLAN.md` (the agent
   view and its bottom bar) is also related; the front matter is not edited.

The status becomes `In implementation` with step 1's commit.

## Amendment — 2026-10-27 20:53 EDT: implementation has started; the document is now immutable

Implementation has started with step 2 (`libflankr_proto` 0.4.0), which
lands before step 1; the 20:44 EDT amendment's "with step 1's commit" is
corrected to this commit. Steps 1 and 2 are independent. The status is
`In implementation`, and the document is now immutable: it changes only on
the owner's explicit instruction.
