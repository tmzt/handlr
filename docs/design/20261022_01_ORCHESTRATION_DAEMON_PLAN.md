---
title: handlr as an agent orchestration daemon
date: 2026-10-22
status: Draft
covers:
  - handlr as a long-running daemon (`handlr serve`) that holds the herdr
    connection and serves clients over a Unix domain socket
  - A policy core under every client: agent resolution, per-agent
    permissions and a confirmation broker that replaces the voice-only
    mechanical gate
  - Principals (who is asking), the shared orchestration methods, events
    and cancellation
  - The `libhandlr_proto` crate for the wire types, in this workspace for
    now
  - The desktop voice loop as an in-process client behind a `local-voice`
    feature
  - Not covered: the link layer, pairing and the companion protocol (plan
    20261022_02); the harness protocol and MCP (plan 20261022_03); agent
    supervision (a later plan)
sources:
  - Owner's discussion (2026-10-22): handlr becomes an agent orchestration
    protocol rather than a voice protocol; the mobile app (voicexn-based)
    owns the voice side; an MCP server and meta-harnesses such as T3 Code
    become clients; the protocol crate starts in the workspace and moves out
    later, with specific names for each protocol.
  - handlr at `9ce1a5f`: the permission check and gate in
    `HerdrToolExecutor::execute_tool` (`src/transport.rs`), the gate state
    machine in libconversate's `VoiceState`, the affirmative list and gate
    handling in `main.rs`'s processor closure, `WatchRegistry`
    (`src/watch.rs`), `run_agent_watch` (`src/notify.rs`), `[[agents]]`
    (`src/config.rs`).
  - The libconversate path in `Cargo.toml` (`../libconversate`) does not
    exist on this machine, so handlr does not build (2026-10-22).
related:
  - docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md (replaced for
    the process model and client side, Rule 1.7)
  - docs/design/20260831_02_REASONING_ENGINE_PLAN.md
  - docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (depends on this)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (depends on this)
---

# handlr as an agent orchestration daemon

## 1. Goal

Today handlr is one process: a microphone, a speech pipeline, Gemini, and
herdr, with the permission and confirmation rules wired into the voice path.
After this plan, handlr is a daemon that owns the herdr connection and the
rules, and serves several clients at once: a phone, a desktop command-line
client, an MCP adapter, a T3 Code plugin, and the existing desktop voice loop.
Every client reaches agents only through the same policy core, so a command
needs the same permission and the same confirmation whoever asks for it.

This plan replaces the process model and client side of
`20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md` (Rule 1.7). The herdr
transport it built, `libherdr`, is kept as it is.

## 2. Decisions

1. **Binary and subcommands.** One `handlr` binary:
   - `handlr serve`: the daemon. Also what `handlr` with no subcommand
     runs.
   - `handlr ctl <method> [json-params]`: sends one JSON-RPC request to the
     daemon socket and prints the result and any notifications until the
     request completes. `--follow` keeps printing notifications. It is a
     debugging client, not a product.
   - `handlr voice` (feature `local-voice` only): `serve` plus the
     in-process voice client (decision 10).
   - `handlr pair` (plan 02) and `handlr mcp` (plan 03) are added by their
     plans.

2. **Crates.**
   - `crates/libhandlr_proto` (new): the JSON-RPC 2.0 envelope, the
     `hello` handshake, error codes, and the parameter and result types of
     every method in every handlr protocol, in modules `common`,
     `companion` (plan 02), `harness` (plan 03), and `link` (plan 02). It
     depends on `serde` and `serde_json` only (plan 02 adds `snow` behind a
     `link` feature). It has no dependency on handlr, libherdr or libinfer,
     so voicexn and a T3 Code plugin's native helper can depend on it alone.
     It moves out of the workspace later, and may then split per protocol.
   - `libherdr` and `libinfer`: unchanged by this plan.
   - The `handlr` crate gains `src/core/` (decisions 4–8) and `src/server/`
     (decision 9).

3. **Protocol names.** Each protocol is named and versioned on its own,
   and a connection declares the one it speaks in its first message:
   - `handlr.harness`: structured orchestration for programs (plan 03).
   - `handlr.companion`: conversational turns for the phone (plan 02).
   - `handlr link`: the encrypted transport that carries either of them
     between devices (plan 02). Locally, both run over the daemon socket
     without it.

   The methods in decision 7 are common to both, and are defined once in
   `libhandlr_proto::common`.

4. **Principals.** Every connection has a principal, fixed at `hello`:

   ```rust
   pub struct Principal {
       pub id: String,              // "local:<client>" or "paired:<name>"
       pub protocol: Protocol,      // Harness | Companion
       pub agents: Option<Vec<String>>, // narrows [[agents]]; never widens
       pub can_approve: bool,
   }
   ```

   - **Local** (daemon socket): the peer's uid, from `getpeereid` on macOS
     and `SO_PEERCRED` on Linux, must equal the daemon's, or the connection
     is closed before `hello`. `id` is `local:` plus the `client` name from
     `hello`, which is a label, not a credential. `can_approve` is
     `[server] local_approve` (default `false`).
   - **Paired** (link): from `auth.toml` (plan 02). Companion principals
     default to `can_approve = true`; harness principals to `false`.
   - **In-process voice** (decision 10): `local:voice`, `can_approve =
     true`.

5. **Agents.** `core::agents` turns a herdr snapshot into

   ```rust
   pub struct AgentRef {
       pub pane_id: String,     // canonical, as resolve_pane_id returns it
       pub agent: String,       // lowercased, as extract_agent_name_for_pane
       pub project: String,     // workspace label, else cwd basename
       pub cwd: Option<String>,
       pub workspace_id: String,
       pub tab_id: String,
       pub status: AgentStatus,
       pub focused: bool,
   }
   ```

   reusing `extract_agent_name_for_pane`, `resolve_pane_id` and the project
   naming in `notify::agent_identities`, which move into this module.

6. **Policy.** `core::policy::check(principal, agent_cfg, command) ->
   Result<Checked, Denied>` is the body of today's permission check in
   `HerdrToolExecutor::execute_tool`, unchanged in effect: the agent must
   have an `[[agents]]` entry (and be in `principal.agents` if that is
   set); the command must be on `allowed_commands` (case-insensitive) or
   pass `validate_arbitrary_command` with `allow_arbitrary_commands`.
   `Checked` carries the final command string and whether the agent's
   `require_confirmation` is set.

7. **Common methods** (JSON-RPC 2.0; names are the wire names):

   | Method | Params | Result |
   |---|---|---|
   | `hello` | `{protocol, version, client}` | `{server: "handlr", server_version, protocol_version}` |
   | `agents.list` | `{}` | `{agents: [AgentRef]}` |
   | `agents.read` | `{pane_id, lines}` (1–200, default 40) | `{text}` |
   | `agents.send` | `{pane_id, command}` | `{status: "sent"}` or `{status: "pending", confirm_id}` |
   | `agents.focus` | `{target_id}` | `{}` |
   | `watch.add` / `watch.remove` | `{pane_id}` | `{watched: [pane_id]}` |
   | `confirm.reply` | `{confirm_id, approve}` | `{outcome}` |
   | `confirm.cancel` | `{confirm_id}` | `{outcome}` |
   | `events.subscribe` | `{topics: [..]}` | `{}` |
   | `cancel` | `{id}` (a request id on this connection) | `{cancelled: bool}` |
   | `ping` | `{}` | `{}` |

   Topics for `events.subscribe` and their notifications:
   `agent.status` → `agent.status_changed {pane_id, agent, status}`;
   `confirm` → `confirm.requested {confirm_id, pane_id, agent, project,
   command, requested_by}` and `confirm.resolved {confirm_id, outcome}`.
   Approvers are subscribed to `confirm` without asking.

   `hello` must be the first request; anything else gets error `-32010`
   and the connection is closed. A `protocol` the daemon does not serve, or
   a `version` it does not support, gets `-32011`.

   Error codes: `-32001` not permitted (with the reason from `Denied`),
   `-32002` unknown agent or pane, `-32004` cancelled, `-32005` herdr
   unavailable, `-32006` confirmation not found or already resolved,
   `-32007` principal may not approve, `-32010` hello required, `-32011`
   unsupported protocol or version.

8. **The confirmation broker** (`core::confirm`) replaces the gate in
   libconversate's `VoiceState` for every client:
   - `agents.send` on an agent with `require_confirmation` creates
     `Confirmation { id: u64, pane_id, agent, command, requested_by,
     created }` and returns `pending` at once. The broker, not the
     requester, sends the command when it is approved, so what runs is
     exactly what was announced, once. No reply text is parsed to find it.
   - States: `Pending` → `Approved` (then `Sent` or `Failed`) | `Denied` |
     `Expired` (60 s) | `Cancelled`. Every change is a `confirm.resolved`
     to the requester and to every approver.
   - Only a principal with `can_approve` may approve. The first reply
     wins; later ones get `-32006`. A requester without `can_approve`
     cannot approve its own request, so an MCP agent cannot confirm its own
     command; a companion principal can approve what its own turn asked
     for, because the approval is the user's answer on that device.
   - The requester may `confirm.cancel` its own pending confirmation.
     Closing the connection cancels it as well.

9. **The daemon socket.** `~/.config/handlr/handlr.sock` (overridden by
   `[server] socket` in `config.toml`), created with mode 0600. At start, a
   stale socket (one that refuses connections) is removed; a live one means
   another daemon is running, and `serve` exits with an error. Framing is
   one JSON object per line (UTF-8, `\n`), at most 1 MiB per line. Each
   connection is a Tokio task; requests on one connection run concurrently,
   and `cancel` aborts the task of the named request id.

10. **The desktop voice loop becomes a client.** It moves behind the
    `local-voice` feature (`libconversate` and `audio_core` become optional
    dependencies), so the default build no longer needs libconversate. On
    the core:
    - `run_command` from Gemini calls `agents.send` in-process. A
      `pending` result goes back to Gemini as a tool result telling it to
      ask the user to confirm that command.
    - While a confirmation that `local:voice` asked for is pending, an
      affirmative utterance (the list now in `main.rs`) approves it and
      anything else denies it, then the utterance is handled as a new turn.
    - `parse_confirmation_request`, arming from the model's reply text and
      `verify_and_consume_confirmation` are no longer used. They stay in
      `commands.rs` until plan 02 has no use for them either.
    - The result of an approved send is spoken as a notice.

11. **Watches and notices.** `WatchRegistry` and `run_agent_watch` move
    under the core. The herdr event subscription feeds a
    `tokio::sync::broadcast` bus of core events, which the server maps to
    the notifications of decision 7. Spoken wording (`notify::announcement`,
    `speakable_name`) stays with the voice clients.

## 3. Steps

1. **Policy core, no behaviour change.** Create `src/core/` with `agents`
   and `policy` (decisions 5 and 6), moving the code out of
   `src/transport.rs` and `src/notify.rs`. `HerdrToolExecutor` calls
   `core::policy::check`.
   **Done when:** the moved unit tests and the existing ones pass
   unchanged, and `cargo clippy -p handlr --all-targets` adds no warnings.
   (Needs libconversate on disk until step 2.)

2. **`local-voice` feature.** Make `libconversate` and `audio_core`
   optional, gate `main.rs`'s pipeline code and the `VoiceState` uses
   behind `local-voice`, and add a `serve` stub that connects to herdr and
   logs the agent list.
   **Done when:** with no `../libconversate` on disk, `cargo build -p
   handlr` and `cargo test -p handlr` pass, and `cargo tree -p handlr`
   shows no libconversate.

3. **Confirmation broker.** `core::confirm` per decision 8, with unit
   tests under paused Tokio time: approve sends exactly once; deny and
   expiry at 60 s send nothing; a second reply gets `-32006`; a principal
   without `can_approve` gets `-32007`, including on its own request;
   cancel by the requester, and on disconnect.
   **Done when:** `cargo test -p handlr core::confirm` passes, and the
   herdr mock records one `pane.send_input` per approved confirmation.

4. **`libhandlr_proto`.** The crate with the envelope, `hello`, the error
   codes and the `common` types of decision 7, added to the workspace in
   its first commit. Serde round-trip tests for every type, and a test
   that every error code is distinct.
   **Done when:** `cargo test -p libhandlr_proto` passes, and `cargo tree
   -p libhandlr_proto -e normal` shows only `serde` and `serde_json`.

5. **The daemon.** `handlr serve` on the socket of decision 9, the
   principals of decision 4, the common methods, events, cancel and
   `handlr ctl`.
   **Done when:** with herdr running, `handlr ctl agents.list` lists the
   agents; `handlr ctl agents.send '{"pane_id":"…","command":"continue"}'`
   on a `claude` pane returns `pending`; a second `handlr ctl --follow
   events.subscribe '{"topics":["confirm"]}'` sees `confirm.requested`;
   with `local_approve = true`, `handlr ctl confirm.reply` sends
   `continue` once, and replaying the reply returns `-32006`; a process
   under another uid is refused.

6. **The voice loop on the core.** Decision 10, with unit tests mapping
   affirmative and other utterances onto `confirm.reply`.
   **Done when:** with `--features local-voice` and libconversate
   available, "send continue to claude" is asked back, "yes" sends it
   once, and "no" sends nothing; `handlr ctl --follow` shows the same
   `confirm.requested` and `confirm.resolved`. This step waits for
   libconversate to be available again.

## 4. Open questions

1. Should local clients be able to approve at all outside testing
   (`local_approve`), or only the phone and the in-process voice client?
2. Is 60 s the right lifetime for a pending confirmation, given that the
   phone may be in a pocket when an MCP agent asks?
3. Is the `local-voice` desktop loop kept long term, or retired once the
   phone works?

## Amendment — 2026-10-22 12:06 EDT: approval phrases, action labels, and no in-process voice

Agreed with the owner. This amendment replaces parts of decisions 4, 7, 8
and 10, steps 3, 5 and 6, and open questions 1–3. Where it disagrees with
the body, this amendment wins.

**Two keys on every confirmation (decision 8).**

1. **The approval phrase**, generated by handlr. It is three words drawn at
   random from a fixed list of 2048 short English words, joined by `-`
   (for example `copper-lantern-ninety`). It is sent only in
   `confirm.requested` to principals that can approve. The requester never
   receives it, and it never appears in `confirm.resolved`, in responses,
   or in the log.
2. **The action label**, written by the requester. It is an optional
   `label` on `agents.send`: 3–64 characters of `a-z`, `0-9` and `-`, which
   describes the action and its context (for example
   `compact-claude-after-tests-pass`). A label is unique among the
   requester's pending confirmations. It appears in `confirm.requested`
   and `confirm.resolved`, and it lets the requester find its
   confirmation again after its own context has moved on.

**Approving (decisions 4 and 8).** `confirm.reply` becomes `{confirm_id,
approve, phrase?}`.
- A paired principal with `can_approve` (from `auth.toml`) may omit the
  phrase.
- Any other principal must give the matching phrase. A wrong phrase gets
  `-32008`, and five wrong phrases deny the confirmation.
- The requester can never approve its own confirmation, with or without the
  phrase, except for a companion's own voice-turn confirmation (below).
- `[server] local_approve` is removed. Local principals have `can_approve =
  false`.

**Status by label (decision 7).** A new common method, `confirm.status
{confirm_id | label}`, returns `{confirm_id, label, outcome}`. A lookup by
label only searches the calling principal's own confirmations. Resolved
confirmations stay queryable for 24 h.

**Lifetimes (decision 8, replacing the single 60 s).**
- A confirmation asked for by a companion's own voice turn checks what was
  heard. It ends at that session's next `turn.start`, or after 60 s.
- Any other confirmation lives for `[server] confirm_ttl`, 3600 s by
  default. It is sent to approvers again once, with `reminder: true`, after
  10 minutes.
- `confirm.cancel` by the requester, and the requester disconnecting, still
  end a confirmation at once.

**No in-process voice (decision 10).** The desktop voice loop is to be
replaced by a desktop build of voicexn, which shares its code with the
mobile app. That build pairs over the link at `127.0.0.1:7347` like the
phone (plan 02), so it is a paired principal with a key. Therefore:
- `handlr voice` and the `local:voice` principal are dropped.
- Step 2 stands: the current loop goes behind `local-voice`, unchanged, so
  the default build needs no libconversate. A later commit, recorded in
  CHANGES.md, deletes the feature and its code once desktop voicexn works
  as a companion client.
- Step 6 is dropped.

**Steps.**
- Step 3's tests add these cases: the phrase is absent from everything the
  requester receives; a local principal approves with the right phrase,
  and is refused with a wrong one or none; five wrong phrases deny the
  confirmation; the requester is refused even with the phrase; the two
  lifetimes and the reminder, under paused Tokio time; and `confirm.status`
  by label, including that it cannot find another principal's label.
- Step 5's **Done when:** approves with `handlr ctl confirm.reply
  '{"confirm_id":…,"approve":true,"phrase":"…"}'`, with the phrase read
  from the `--follow` subscriber's `confirm.requested`. The approval only
  works when that subscriber is a paired principal; the CLI test pairs the
  plan 02 example client, which prints the phrase. The step no longer
  relies on `local_approve`.

**Open questions.** Question 1 (local approval) and question 2 (lifetime)
are settled above. Question 3 is settled: the desktop loop is retired in
favour of voicexn.

## Amendment — 2026-10-22 16:23 EDT: remove the speech pipeline first, phrases from BIP-39, phrases shown by serve

Agreed with the owner. This amendment replaces step 2, decision 10, and
the parts of the 12:06 EDT amendment about `local-voice` and step 5's
**Done when:**. It also fixes the word list for approval phrases. Where it
disagrees with earlier text, this amendment wins.

**Step 2, replaced, and done first.** An optional path dependency must
still exist on disk for Cargo to resolve the workspace. Gating
libconversate behind a feature would therefore not make handlr build
without `../libconversate`. Today nothing in the workspace builds:
`cargo check -p libinfer` fails on the missing `audio_core` manifest. The
desktop speech pipeline is replaced by the companion apps
(`20261022_04_COMPANION_APP_PLAN.md`), so it is removed, not gated, and
git history keeps it.

- `src/main.rs`: the `SpeechToSpeechBuilder` pipeline, the STT and TTS
  setup, model resolution and downloads, CPAL capture, the processor
  closure, and the CLI flags for these. What is left loads the config,
  connects to herdr and logs the agent list. Step 5 makes it `handlr
  serve`.
- `src/notify.rs`: `drain` and `run_agent_watch`. The `Announcer`, the
  wording and the naming functions stay, with their tests.
- `src/transport.rs`: the `VoiceState` field and the gate check in
  `HerdrToolExecutor`, and the tests that build a `VoiceState`. Until step
  3's broker is in place, a command for an agent with
  `require_confirmation` is refused with `confirmation unavailable`, never
  sent. The current code would otherwise send it unconfirmed, because it
  checks the gate only when a `VoiceState` is present.
- `src/config.rs` and `config.default.toml`: `[voice.tts]`, `[voice.stt]`
  and their tests.
- `Cargo.toml`: `libconversate`, `audio_core`, the `kokoro` and
  `sherpa-kokoro` features, `reqwest`, `tar` and `bzip2`.
- `build.rs`: removed. It only added the sherpa-onnx rpath.

**Done when:** with no `../libconversate` on disk, `cargo build`, `cargo
test -p handlr` and `cargo clippy -p handlr --all-targets` pass. `cargo
tree -p handlr` shows no libconversate, audio_core or reqwest. A test
shows that a `require_confirmation` agent's command is refused.

The order of the steps is now 2, 1, 3, 4, 5. Step 1 no longer needs
libconversate on disk.

**Decision 10, replaced.** There is no `local-voice` feature, no
in-process voice client and no `handlr voice`. The affirmative list goes
with `main.rs`. The voice answer is matched by the companion client (plan
02's 12:06 EDT amendment). `parse_confirmation_request`, the arming from
the model's reply text, and `confirmation_match_candidates` stay in
`commands.rs` until step 3's broker replaces their last use, and are then
removed.

**Approval phrases.** The word list is the BIP-39 English list (2048
words), embedded as `src/core/words.txt`. Three words give 33 bits.
Together with the limit of five wrong attempts, that is enough for a
phrase that lives at most an hour.

**Phrases shown by `serve`.** When a confirmation is created, `handlr
serve` prints `confirm <id> [<label>]: <phrase>` to its standard output,
never to the log. It does so only when standard output is a terminal, or
with `--show-phrases`. This lets you approve from a local client with the
phrase read off the daemon's terminal. Step 5's **Done when:** takes the
phrase from there instead of from a paired subscriber, so plan 01 is
tested without plan 02.
