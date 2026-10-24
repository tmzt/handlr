# CHANGES

Planned new work, feature plans and architectural changes, appended in order
(AGENTS.md, Rule 0). Entries are never edited.

## 2026-10-22 10:55 EDT — Adopting the planning system

Plans: [System architecture and herdr transport](docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md),
[Pluggable ReasoningEngine and ToolExecutor traits](docs/design/20260831_02_REASONING_ENGINE_PLAN.md),
[libconversate speech pipeline integration](docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md)
(all `In implementation`).

- handlr predates the libcontrols critical rules. AGENTS.md adopts them
  (`0fc5967`), and the original `PLAN.md` (`ecaaa09`, 2026-08-31) is split
  by topic into the three plans above with its original date and text
  (`0c99db7`). Their implementation records are appended as amendments
  (`7298df7`).
- The departures below happened before the rules were adopted. They are
  recorded here after the fact, as Rule 1.7 requires. Where handlr has to
  diverge further, that work gets new dated plans.

## 2026-10-22 10:55 EDT — Departures from the 2026-08-31 plans

Plan: [System architecture and herdr transport](docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md).

- §5.1: the control loop runs on Tokio (`#[tokio::main]`, `be9362c`), not
  `smol`/`async-std`, because libgenai's async processor needs a Tokio
  reactor.
- Phase 2: the transport is the `libherdr` crate, speaking herdr's JSON-RPC
  2.0 methods (`aaa7e05`). The executor that reasoning calls is
  `HerdrToolExecutor` in `src/transport.rs`, an adapter, not a
  `HerdrExecutor` (`aa7ff67`). This keeps `libherdr` and `libinfer`
  independent.
- Not in the plan: per-agent command permissions and the mechanical
  confirmation gate (`3c5087c` to `be095ab`, AGENTS.md §3), and pane
  watching with spoken agent notifications (`888c64f`, `a7769c9`,
  `a421b2f`).

Plan: [Pluggable ReasoningEngine and ToolExecutor traits](docs/design/20260831_02_REASONING_ENGINE_PLAN.md).

- Phase 1: the traits and engines live in the `libinfer` crate
  (`2b85d0a` to `89e11e6`), and handlr re-exports them.
- §2.B.1: pane-reading tools gave way to `HerdrContextContributor`, which
  adds the herdr snapshot and recent pane output to every turn (`5af8931`).
  The model's tools are now `focus`, `run_command` and `watch_pane`. The
  system prompt is loaded from `PROMPT.md` (`3d0ef99`).
- §2.B.3: `RoutingReasoningEngine` has not been built.

Plan: [libconversate speech pipeline integration](docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md).

- §3.A: the pipeline uses `with_async_processor` (`28e334f`), not
  `with_processor` bridged through `smol::block_on`.
- Not in the plan: capture is muted at the CPAL stream while reasoning and
  speaking (`1a3f87d`, `2817db2`), and notifications queue behind speech
  instead of barging in (`a7769c9`).
- Phase 4: the barge-in tests have not been written.

## 2026-10-22 11:36 EDT — handlr as an agent orchestration daemon

Plans: [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md),
[handlr link pairing and the companion protocol](docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md),
[The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md)
(all `Draft`).

- handlr changes direction from a voice front end to an agent
  orchestration daemon. The phone (a voicexn-based app) owns the voice
  side, and talks to handlr in text turns.
- Plan 01 replaces the process model and client side of
  [System architecture and herdr transport](docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md)
  (Rule 1.7). `libherdr` is kept.
- The confirmation gate moves from libconversate's `VoiceState` into a
  broker in handlr's policy core, used by every client.
- A new crate, `libhandlr_proto`, holds the wire types of the
  `handlr.harness` and `handlr.companion` protocols and of handlr link. It
  starts in the workspace and is to move out.
- Agent supervision (plan inference, a local Qwen model, loop detection,
  plan status by voice) is to be a later plan, once it is settled which
  parts T3 Code owns.

## 2026-10-22 16:01 EDT — The companion app in this repository

Plan: [The companion app, on handlr-companion and libvoicexn](docs/design/20261022_04_COMPANION_APP_PLAN.md)
(`Draft`).

- The phone and desktop voice client is built here, on libvoicexn, as
  logic in `crates/handlr-companion` with front ends in
  `apps/companion-tty` and `apps/companion`.
- `apps/` is its own workspace, excluded from the root, so a missing
  libvoicexn checkout or the Android tooling cannot break handlr's build.
- Agent supervision, listed as a later plan in the 11:36 entry, takes a
  later number than 04.

## 2026-10-22 17:11 EDT — Unblocking the build: the libinfer interim fix, rustfmt, and a retired plan

Plans: [Pluggable ReasoningEngine and ToolExecutor traits](docs/design/20260831_02_REASONING_ENGINE_PLAN.md)
(`In implementation`), [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md)
(now `In implementation`), [libconversate speech pipeline integration](docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md)
(now `Retired`).

- **20260831_02, departure.** libinfer was written against a libgenai with
  a `tokio` feature and `GeminiAgent::client()`, `system_prompt()` and
  `tools()`. The only libgenai that exists (`tmzt/libgenai`, `57fb65d`) has
  none of these. With libconversate gone, this was the next thing stopping
  the workspace from building. `GeminiReasoningEngine` now holds its own
  `gemini_rust::Gemini` client, system prompt and tools. `from_agent()` and
  `agent()` are removed, and `system_prompt()` and `tools()` are on the
  engine. libgenai is pinned by git revision (`f892246`). libinfer has
  never been released on its own (VERSIONS.md), so its version is
  unchanged. This is interim: a libinfer provider split (a neutral core,
  then `libinfer-gemini` and later `libinfer-openai`, under
  `crates/libinfer/`) is to be plan 05, and libgenai is to be retired by it.
- **rustfmt.** libinfer and handlr had never been formatted, so Rule 3.4's
  `cargo fmt --check` could not pass. Each got a formatting-only commit
  first (`7ef43e8`, `9cc4fec`). libherdr is still unformatted, and gets the
  same when it is next changed.
- **20261022_01, step 2.** The speech pipeline is removed (`5ce0b71`), and
  the plan is `In implementation`. Departure from the step's **Done when:**
  `cargo tree -p handlr` still shows `reqwest`, but only through
  gemini-rust (Gemini is HTTP). handlr's own dependency on it, for the
  model downloads, is gone, which is what the step meant. clippy reports
  two warnings in handlr (`src/transport.rs`, `HerdrContextContributor`)
  and three in libherdr. All five are in code this work did not touch, and
  none is new.
- **20260831_03, retired.** A new plan status, `Retired (by <plan>)`
  (AGENTS.md, `27346b9`), is for a plan whose code was implemented and
  then removed. Its amendment names the last commit with the code, so it
  can be recovered. The speech pipeline is `Retired (by
  20261022_01_ORCHESTRATION_DAEMON_PLAN)`, removed in `5ce0b71`. The last
  commit with the code is `f89224618e114a4b54e0761a7417e7f118b90eb8`.

## 2026-10-22 17:12 EDT — Correction to the 17:11 entry

In the 17:11 entry, "libgenai is pinned by git revision (`f892246`)" should
read: libgenai is pinned at revision `57fb65dd9269b7341346a8241d9f10d7a9565266`
of `tmzt/libgenai`, in commit `f892246`.

## 2026-10-22 17:20 EDT — Confirmations: handle, token and nonce

Plan: [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md)
(`In implementation`), decision 8 as amended at 12:06 and 16:23 EDT; step 3.
Agreed with the owner before implementation (Rule 1.7).

- **Names.** The requester's descriptive label is the *confirmation handle*.
  The secret three-word phrase is the *confirmation token*. Wire names:
  `handle` and `token`. The format and rules of each are unchanged.
- **The token never crosses the wire.** An approver that must prove it
  knows the token sends a *nonce* instead:
  `hex(BLAKE2s-256-MAC(key = token, "handlr.confirm/1" || confirm_id as
  8 bytes big-endian || handle))`. The handle is empty when there is none.
  The daemon computes the same value and compares it in constant time. The
  nonce binds the approval to one confirmation and its handle, so it cannot
  be replayed against another. The token itself is never sent in a request
  and never logged.
- `confirm.reply` is `{confirm_id, approve, nonce?}` in place of
  `{confirm_id, approve, phrase?}`. A wrong nonce gets `-32008`, and five
  wrong nonces deny the confirmation. A paired principal with `approve`
  may omit it, as before.
- `confirm.requested` carries `token` only to principals that can
  approve. The broker enforces this, not the transport.

## 2026-10-22 17:34 EDT — Plan 20261022_01, steps 1, 3, 4 and 5, and departures

Plan: [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md)
(`In implementation`). Step 1 is in `4e61db7`, step 3 in `05f59bd`, step 4
in `6e9d876` and step 5 in `75df096`. Step 2 (`5ce0b71`) is recorded
above, and step 6 was dropped by the 12:06 EDT amendment.

Departures, all small and needed for the plan to work:

- **Step 5, `ctl --client <name>`.** The step's **Done when:** sends and
  approves with `handlr ctl`, but a requester can never approve its own
  command, and every `ctl` was `local:ctl`. `--client` sets the label
  (default `ctl`), so the check runs as `ctl --client mcp agents.send …`
  followed by `ctl confirm.reply …`. A label is not a credential (decision 4).
- **Step 5, `--show-tokens`.** This is `--show-phrases` of the 16:23 EDT
  amendment, renamed with the phrase (now the token).
- **Step 5, `ctl confirm.reply` takes `token` and `handle`** and sends
  the nonce computed from them. The token stays off the socket.
- **Step 5, checked against the mock.** `agents.list` was run against the
  live herdr (21 panes). Sending and confirming were checked with `handlr
  serve --mock` and `tests/daemon.rs`, so that nothing was typed into a
  live agent session. Refusing another uid is unit-tested
  (`server::conn::peer_allowed`); no second user was available.
- **Step 5, closing a connection** cancels the confirmations opened on that
  connection, not every confirmation of its principal. Several connections
  can share a principal id (`local:ctl`), and one closing must not cancel
  another's requests.
- **Step 5, only `handlr.harness` is served for now.** `hello` with
  `handlr.companion` gets `-32011` until plan 20261022_02 adds it.
- **Step 5, `confirm.requested.project`** is left empty. The broker does
  not look up projects. It is filled when a client needs it.
- **Step 3, `Outcome::Approved`.** It is the state while an approved
  command is being sent, so a second approval cannot race the first.

## 2026-10-22 17:59 EDT — Naming agents gets its own plan

Plan: [Naming agents aloud and on screen](docs/design/20261022_05_SPOKEN_NAMES_PLAN.md)
(`Draft`).

- The naming rules first written as two amendments to plan 20261022_02 (17:52
  and 17:55 EDT) move to their own plan, with a `[naming]` section in
  `config.toml` for spelling, letter names and aliases. Plan 02 points to it.
- The libinfer provider split, earlier called "plan 05", takes the next free
  number when it is written.

## 2026-10-22 18:24 EDT — Plan 20261022_02, step 1, and rules for step 3

Plan: [handlr link pairing and the companion protocol](docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md)
(now `In implementation`). Step 1 is in `3fb935d`.

Departures and decisions the plan left open:
- **The pairing id.** With `XXpsk3` the PSK enters late, and the daemon may
  have several pending pairings. After the mode byte `0x01` the client
  therefore sends `BLAKE2s-256(secret)[..4]` in the clear, and the server
  finds the secret by it. The id is derived from the secret, not carried in
  the pairing string. A wrong id fails like a wrong secret.
- **A first message when pairing.** `pair` may carry the first message in
  the third handshake message, after the PSK, as `IK` carries it when
  reconnecting.
- **Split halves.** `LinkStream::into_split` has a reader and a writer, on
  snow's stateless transport with a nonce counter each, so the daemon can
  read and write at the same time.
- **No built-in timeouts or expiry check.** Callers wrap handshakes in a
  timeout and check `PairingPayload::is_expired`.

Rules for step 3 (the daemon side), found in review:
- **The first message is hello only.** An `IK` first message can be
  replayed by anyone who captured it. They cannot finish the session, but
  the server reads the payload. The daemon therefore accepts only `hello`
  as a handshake's first message, never a request with effects, and treats
  anything else as a protocol error.
- **Handshakes time out.** The daemon wraps `accept` in a 10 s timeout, so
  an idle or slow peer cannot hold a connection open.
- The daemon burns a pairing secret on first use and checks its expiry
  before looking it up.

## 2026-10-22 18:26 EDT — Plans 20261022_05 (steps 1–2) and 20261022_04 (step 1), and departures

Plans: [Naming agents aloud and on screen](docs/design/20261022_05_SPOKEN_NAMES_PLAN.md)
and [The companion app](docs/design/20261022_04_COMPANION_APP_PLAN.md), both
now `In implementation`. Naming is in `8dce2a5`, and the companion crate in
`d838873`.

**Plan 05, departures and decisions it left open:**
- **A label that is only a number** is spoken "tab <label>", not "tab
  <number>". The live snapshot has a tab labelled `4` whose number is 7,
  and "tab 7" would contradict the tab bar. The number is used only when the
  label is empty.
- **Uniqueness compares spoken forms**, so labels that sound alike (`17-hr`
  and `hr`) fall to the tab number.
- **Past rung 5**, two agents in one tab end ", number K", their order in
  the snapshot. Names never fall back to a pane id.
- **Shells** are named too, ranked among shells ("the regraph shell"), so
  a shell does not lengthen an agent's name. Resolution prefers agents.
- **Rung 2 is lowercase** ("the handlr agent"), so `notify::announcement`
  capitalises the start of its sentence.
- **Words are split on spaces, `-` and `_`** for spelling.
- **Known issue, raised with the owner.** The 18:15 EDT amendment's "every
  word of at most `spell_max_len` letters" puts spelled forms of ordinary
  words and ids into the vocabulary. Examples are `CLAUDE` / "see el ay you
  dee ee", "bee you bee bee el ee" for `bubble`, and `1EDF0E1E987E`. That
  is literal to the amendment, but noise for an STT booster. Narrowing it
  needs the owner's decision.

**Plan 04, step 1, departures:**
- **Confirming takes precedence** over a running turn. A confirmation can
  arrive mid-turn, and the user must be able to answer the question just
  spoken. A denying utterance during a turn is held until the turn ends,
  to avoid `-32020` (busy).
- **Only the running, untapped turn speaks.** A `turn.say` for a turn the
  user interrupted, or one that has ended, is dropped (the lead's fix in
  review).
- **`session_id` is in memory only** until the link (step 2) stores it in
  `ServerRecord`. `Input::Pair` answers "pairing needs the link (step 2)".
- **`FileKeyStore`** refuses any group or other access (mode & 0o077),
  which is stricter than "readable". It also uses the `base64` and `toml`
  crates.
- **`wire::ConfirmRequested`** accepts both `handle`/`token` (the
  CHANGES.md names) and `label`/`phrase` (plan 02's text).

## 2026-10-22 18:34 EDT — Plan 20261022_04, step 2, client side

Plan: [The companion app](docs/design/20261022_04_COMPANION_APP_PLAN.md)
(`In implementation`). The client side of step 2 is in `548447b`. The step's
**Done when:** is checked once the daemon's side of the link (plan 02, step 3)
is in.

Decisions the plan left open:
- **Failing over** during pairing moves to the next address only if the TCP
  connect fails. A handshake failure at a reachable address ends the attempt:
  the secret is single use, and a key mismatch is a security signal.
  Reconnecting tries the next address on any failure.
- **Pairing while connected** closes the current connection first. If
  pairing fails, the client returns to its previous server, or to the local
  socket, and to the previous session.
- **Timeouts** are 10 s for the TCP connect and 10 s for the handshake,
  matching the daemon's handshake timeout.
- **Link messages are whole JSON objects**, one per Noise message, with no
  trailing newline. The Unix socket keeps newline-delimited lines.
- **There is no `Unpaired` state of its own.** Without a server record the
  client uses the local socket.

## 2026-10-22 18:44 EDT — Plan 20261022_02 steps 2–3, plan 20261022_04 step 3 (crate), and departures

Plans: [handlr link pairing and the companion protocol](docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md)
and [The companion app](docs/design/20261022_04_COMPANION_APP_PLAN.md), both
`In implementation`. The link's daemon side is in `0fc9886` (handlr) and
`219fb35` (libhandlr_proto: `link.pair_*` types and the `link_client`
example). `handlr-audio` is in `2fd5256`.

**Plan 02, steps 2–3, not yet checked:** pairing from a second machine on the
tailnet, which step 3's **Done when:** requires. It needs the `tailscale
serve` forward, which handlr never sets up itself. Everything else was
checked end to end over the mock, and by hand with `serve --mock`, `handlr
pair` and `link_client`: pairing, refusing a reused secret, reconnecting,
and revocation closing the connection.

**Plan 02, departures and decisions:**
- **`hello` after a pairing** returns `paired: true` as an extra field.
  `HelloResult` is unchanged.
- **`handlr pair` waits** for a `link.pair_done` notification on its own
  connection. If it exits, its pending pairing is withdrawn. A name can have
  one pending pairing at a time.
- **Defaults:** `--protocols` is companion; the TTL is 1–3600 s; names are
  1–64 of `[A-Za-z0-9._-]`.
- **`approve` defaults by the protocol of the connection's `hello`.**
- **Error codes.** A `hello` protocol not in the entry gets `-32001`;
  `link.pair_begin` over the link, or with the link off, gets `-32001`;
  `pair_begin` refusals get `-32602`.
- **`auth.toml` is stricter than the plan.** Group or other write access, a
  repeated name or key, or a malformed key authorizes nobody. A failed
  periodic read closes every link connection. A key already paired under
  another name is refused.
- **`--replace`** keeps the old entry's `agents` and `approve`, so
  re-pairing never widens what a device may do. It rewrites the file
  atomically, which drops comments.
- **`[link]`:** `listen = []` turns the link off, and `revoke_check_secs`
  sets the interval.
- **A handshake that fails after the secret is taken** still burns it.

**Plan 04, step 3 (the crate), not yet checked:** `companion-tty` and the
spoken **Done when:** wait for the companion protocol (plan 02, step 4). The
Mac this is built on, a Mac mini, has no microphone. CoreAudio has no input
device, and libvoicexn's `voicexn-tty` fails the same way. The echo example
was therefore checked with `--input-wav`. With that, both models loaded, a
WAV was heard as an `Utterance`, `Say` gave `Speaking`, and a stop
silenced playback within 22–26 ms. STT stepped in 61.9 ms of 80. A live
microphone check needs the S23, or a Mac that has a microphone.

**Plan 04, departures and decisions:**
- **A playback flush of its own.** libvoicexn flushes only when `Stop`
  arrives while synthesis is still running. A finished reply can leave about
  2 s queued, so `handlr-audio` drains the ring itself on `StopSpeaking`.
- **`Speaking`** lasts until the ring has run dry, not just until synthesis
  finishes, so notices wait for the end of the audio.
- **`Listening`** runs from `Armed` to `Idle`, as libvoicexn's
  `Gate::listening` does.
- **The companion's `vocab.txt`** (next to its keys) wins over
  `VOICEXN_VOCAB` when both exist. It takes effect at the next start, until
  libvoicexn's runtime vocabulary (its plan 07) is released.
- **`morph`** compiles, wired through libvoicexn's adapters, but has not
  been run.

## 2026-10-22 18:55 EDT — Plan 20261022_05, step 3, and departures

Plan: [Naming agents aloud and on screen](docs/design/20261022_05_SPOKEN_NAMES_PLAN.md)
(`In implementation`). Step 3 is in `a6c9d93` (libhandlr_proto) and
`c0b483e` (handlr). All three steps are now implemented. The read-only check
against the live herdr named all 21 panes, none by pane id. "hr" read the
handlr pane, and "highbay" returned both candidates.

Departures and decisions:
- **The tests use a fake herdr socket.** It answers with a herdr-shaped
  snapshot, because the herdr mock's typed `EnvironmentSnapshot` cannot
  hold top-level tabs.
- **`agents.focus`** passes a workspace or tab id through to herdr
  unresolved, so focusing a workspace still works. Anything else is
  resolved by name.
- **`AgentInfo.project` is unchanged** (the workspace label, else the cwd
  basename). Decision 3's project appears only inside `name`.
  `ConfirmRequested.project` now carries the same value as
  `AgentInfo.project`.
- **`handlr ctl` still prints JSON.** It now carries the names, so scripts
  keep working.
- **An ambiguous target's `-32002`** also lists its candidates in
  `data.candidates`.
- **Each connection takes its own snapshot** to name a `confirm.requested`
  it forwards. That is one herdr read per subscriber per request, which is
  acceptable for now.

## 2026-10-22 19:14 EDT — Plan 20261022_02, step 4, and departures

Plan: [handlr link pairing and the companion protocol](docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md)
(`In implementation`). Step 4 is in `5ef1101` (libhandlr_proto), `382e951`
(handlr-companion) and `7c4d650` (handlr). It was checked over the herdr mock
with a scripted engine. Live over the link, pairing and connecting worked,
and each turn ended in a clean spoken error, because no Gemini key is
configured on this machine.

Departures and decisions:
- **A turn waits for its confirmations.** The tool returns
  `awaiting_user_approval` and tells the model not to ask in words. The
  reply is said first, then `confirm.requested`, so the question is heard
  last. The turn ends once its confirmations resolve, which is why `ran` can
  list an approved command. The forwarder skips a companion's own
  voice-turn requests, which it carries itself. `ConfirmView` gained
  `lifetime`, and the broker gained `pending` and `has_pending`.
- **A `turn.start` while busy** first ends the session's unanswered
  voice-turn questions. If the running turn was only waiting on them, the
  new turn waits up to 2 s for it to end; otherwise it gets `-32020`.
- **Sessions follow the principal**: `companion:<name>` for a paired
  device, `companion:local:<client>` for a local client. A `session_id`
  sent by the client is ignored.
- **History.** A finished turn records the text and the reply. An
  interrupted turn also records `[interrupted by user]`. A failed turn
  records nothing.
- **Errors.** Gemini is built on first use, from `[gemini] api_key` or
  `GEMINI_API_KEY`. The engine call times out after 120 s.
  `[gemini] model` is not used yet, because libinfer has no setting for it.
- **The current agent** is the last pane a tool call addressed. It is not
  taken from names in the text.
- **Interrupting** sends keys within about 400 ms in all.
  `agents.interrupt` returns `-32001` for an agent that is not
  configured or out of reach, and `{interrupted: false}` when its key is
  empty.
- **The interrupt key** defaults to `esc`, herdr's canonical name for the
  Escape key (`herdr pane send-keys --help`), not `escape`. At load it must
  be a named key or one printable character, optionally after `ctrl+`,
  `alt+` or `shift+`.
- **The vocabulary** is checked after each turn and every 30 s.
- **Names reach the model.** The tool executor resolves targets by name,
  and the herdr context lists agents by name. Outside a companion turn, a
  confirmation-required command is still refused.

**Open, raised with the owner.** `PROMPT.md` still tells the model to ask
for vocal confirmation before calling `run_command`. With real Gemini that
means asking twice: once in words, and then again through the daemon. The
prompt needs its rules rewritten for daemon-side approval.

## 2026-10-22 21:22 EDT — the link's local key is a `StaticKey` (plans 20261022_02 and 20261022_04)

**Departure from plan 02, decision 1, and plan 04, decision 5; agreed with
the owner.** The released libmorph key store (rev `568cf953`, plan
`~/src/libmorph/docs/design/20261022_06_KEY_STORE_PLAN.md`, section 7)
keeps the companion's Noise static key in the Android Keystore or the
Keychain, and does X25519 itself. It does not hand the private key out.
The handshakes took the raw private key, so they could not use it.

- `libhandlr_proto::link` adds the `StaticKey` trait (`public`, and `dh`
  with a peer key). `pair`, `reconnect` and `accept` take a `&K` where
  `K: StaticKey + Clone`, instead of `&[u8; 32]`. `StaticKeypair`
  implements it, in memory, for the daemon, the tests and the example.
- The handshakes give snow a resolver of their own. Its static-key DH
  calls `StaticKey::dh`, so snow never holds the static private key.
  Ephemeral keys, the cipher, the hash and the random source are still
  snow's defaults.
- `handlr_companion::KeyStore` replaces `secret` and `set_secret` with
  `static_key` and `generate_static_key`, which return an
  `Arc<dyn StaticKey>`. `FileKeyStore` keeps its 0600 file, for the
  desktop.
- The wire is unchanged.

A locked key store fails the handshake with `LinkError::Noise`. Telling
"locked, retry" apart from "re-pair needed" is left to the store, which
plan 04 step 4 implements on libmorph's `Vault` with flags 0.

## 2026-10-22 22:40 EDT — the Gemini key comes from `.private` (plan 20261022_01)

**On the owner's instruction.** The Gemini key is never kept in code, in git
history, in shell history or in an LLM session.

- `[gemini] api_key` is removed. `[gemini] api_key_file` replaces it. It
  defaults to `.private/gemini_api_key`, relative to the daemon's working
  directory, and `.private` is gitignored.
- The file holds the key alone. Surrounding whitespace is ignored.
- A file with any group or other permission is refused, like the link key.
- The turn service reads the file on the first turn that needs Gemini, so a
  key added later needs no restart.
- Errors name the file but never its contents, and are spoken as "the
  server can't read its Gemini key".
- With no file, or an empty one, libinfer still falls back to
  `GEMINI_API_KEY`.

## 2026-10-22 22:59 EDT — plan 20261022_06: qrcodeurl and pairing by a scanned URL

New plan, agreed with the owner:
`docs/design/20261022_06_QRCODEURL_PLAN.md`, with the amendments of 22:43
and 22:55 EDT. Steps 1 to 3 are done:

- `crates/qrcodeurl` (`7c39d51`);
- the pairing URL in `libhandlr_proto` (`e5771f0`);
- `handlr pair` (this batch).

`handlr pair` now prints `handlr://<host>:<port>/pair#handlr1…` and the
QR code of that URL as a `data:` URL. The host is the first address that
isn't loopback, which is the Tailscale name when `tailscale serve` is
detected.

Step 4, the phone's scan and deep link, is plan 04's. The deep link was
requested from libmorph, for its own plan.

## 2026-10-22 23:21 EDT — plan 20261022_07: libinfer providers (Draft)

New plan, written on the owner's instruction:
`docs/design/20261022_07_LIBINFER_PROVIDERS_PLAN.md`. It replaces the
libinfer provider split that was earlier called "plan 05", and builds plan
20260831_02's missing routing engine and the `[gemini] model` setting.
OpenAI support is left to its own plan. libgenai turned out to supply only
`gemini-rust` re-exports to libinfer, so `libinfer-gemini` depends on
`gemini-rust` directly.

## 2026-10-22 23:21 EDT — pairing by public keys (plans 20261022_02 and 20261022_06)

Implements both amendments of 23:2x EDT. Pairing now moves only public keys.

- **The pairing URL** carries the server's public key and its addresses.
- **The device** shows its code (`handlrkey1…`), and the user pastes it
  into `handlr pair`.
- **The handshake.** Every connection, the first one included, uses `IK`.
- **Removed:** `XXpsk3`, pairing ids, pairing secrets and their expiry.

Choices the amendments left open:

- **When the first connection counts.** `paired: true` and
  `link.pair_done` are given when a `hello` is accepted, not at the
  handshake. A `hello` refused for its protocol doesn't use them up.
- **Devices not yet heard from** are kept in memory only, so a daemon
  restart forgets them. The `auth.toml` entry stays.
- **Name lengths.** A `--name` override follows the existing `auth.toml`
  rule of up to 64 characters. A device's own name in its code is at most
  32.
- **The device's default name** is the first label of its hostname,
  cleaned and lowercased, or "companion".
- **The client's retries** come every 2 s for 10 minutes after any failed
  connect.
- **Data URLs.** The client's reader also accepts RGB and RGBA PNGs, such
  as screenshots, and refuses data URLs over 4 MiB.

Still to check by hand: pairing from another machine on the tailnet.

## 2026-10-22 23:26 EDT — plan 20261022_08: PROMPT.md and daemon-side approval (Draft); a correction

**New plan.** `docs/design/20261022_08_PROMPT_CONFIRMATION_PLAN.md`, on the
owner's instruction. It takes up the open item in the plan 02 step 4 entry,
and changes only the conflicting block of the Gemini-written prompt.

**Correction.** The commits `0cb5898` and `a6ee077`, and the "pairing by
public keys" entry above, cite plan 02's amendment as "23:2x EDT". Its
timestamp is **23:07 EDT**.

## 2026-10-22 23:46 EDT — plan 20261022_07, steps 1 to 3

**What changed:**
- The libinfer split is done: `libinfer` is the provider-neutral core,
  and `libinfer-gemini` is built on `gemini-rust`. libgenai is gone.
- `[gemini] model` is used, defaulting to `gemini-2.5-flash`.
- handlr answers "never mind", "cancel", "forget it" and "nothing" with
  "Okay." without asking Gemini.
- This ends the interim libinfer fix of 17:11 EDT.

**How it was committed.** The subagent that built it committed only under
`crates/libinfer/`, as the owner asked. So `83d0747` and `98a1ac2` don't
build on their own: they need the workspace commit that follows them,
which moves the manifest's paths.

**Still to check live** (step 3), with the owner's Gemini key.

## 2026-10-22 23:53 EDT — plan 20261022_04, step 4 begun: apps/companion on the S23

`apps/companion` is built from libvoicexn's `examples/voicexn` UI, as the
owner asked. It's a separate workspace, so the root build skips it.

**What it uses:**
- `handlr-companion` for the connection;
- `handlr-audio` for speech: cpal on the desktop, libmorph on Android;
- libmorph's key store for the Noise key, under the name `noise.static`
  with flags 0, which is StrongBox on the S23;
- its secure store for the server record.

**What was checked on the S23:**
- it installs and starts;
- it makes a key in the key store and keeps it across restarts;
- it shows and logs its device code.

**Departures from plan 04:**
- **handlr-audio's `morph` feature** uses its own libmorph Sink/Source
  adapter, copied from libvoicexn v0.2.0. It no longer turns on
  `libvoicexn/morph`, which pinned libmorph 0.3.0. The app then has one
  libmorph, 0.3.1, which has the key store.
- **Typing the pairing input.** libmorph has no text input on Android,
  and no paste or Backspace on the desktop. The input therefore comes from
  `pair.txt` in the app's files directory (read once, then deleted),
  `--pair` on the desktop, or the on-screen field for typed characters.
- **No Copy button.** There is no clipboard API in libmorph, libmobile or
  libcontrols. The device code is on screen and in logcat; it is a public
  key.
- **`handlr://` links can't open the app yet.** libmobile already passes
  the launch Intent on, but libmorph 0.3.1's Android runner drops it.
  `App::open_url` is ready for libmorph's URL-schemes plan (libmorph
  20261022_07, Draft).
- **X25519 in the key store** uses curve25519-dalek 4.1.3, already in the
  lock, rather than snow's resolver, whose key array is never cleared.
- **The device name** on Android is `ro.product.model` (`sm-s918u`).

**Not yet checked:**
- **Desktop:** the Mac's screen was locked and it has no microphone.
- **S23:** a pairing with a real daemon, a turn, an approval and an
  interrupt. The phone holds a throwaway server record (`127.0.0.1:9`)
  from the key test, which a real pairing replaces.

**Google Lens** reads `https` QR codes but ignored `handlr://` ones, and
also a plain-text one. The owner chose to wait for the scheme
registration.

## 2026-10-23 00:28 EDT — the Gemini key file refuses JSON; service-account auth was lost

**The key file now refuses anything that isn't an API key.** An API key is
one token with no spaces or line breaks, at most 256 characters.
`.private/gemini_api_key` had been made a symlink to a service-account
JSON, which would have been read as an API key. The JSON is now refused
before any request is built, and the error never quotes it.

**A regression found while checking this.** handlr once authenticated to
Gemini with a service account: `febf29c` and `4892d5d` used `gcp_auth`,
then `e3465a1` moved that into libgenai. The interim libinfer fix of 17:11
EDT, and plan 20261022_07's move to `gemini-rust` 1.7, dropped it, because
that library only sends an API key.

The owner's credential is a service account
(`.private/service_credentials.json`). Restoring service-account auth
changes plan 07, so it waits on the owner. First, a check run by the owner
must show which endpoint the credential works with: Vertex AI, or the
Gemini API over OAuth.

## 2026-10-23 00:54 EDT — plan 20261022_07: the default model is gemini-3.8-flash; the first real turns

**A departure from decision 3, needed to make it work.** Google answered
`gemini-2.5-flash` with a 404 for the owner's key: "no longer available to
new users … use models/gemini-3.8-flash". `libinfer-gemini`'s default is
now `gemini-3.8-flash`, which the key lists, and it is sent as
`Model::Custom`, since gemini-rust 1.7 doesn't name it. `src/config.rs`
still says "gemini-2.5-flash" in its doc comment; that is fixed in the next
handlr commit. The owner's `~/.config/handlr/config.toml` sets
`[gemini] model = "gemini-3.8-flash"` meanwhile.

**The first real turns.** With the owner's API key in
`.private/gemini_api_key` (mode 0600), `handlr serve --mock` answered a
typed turn from `handlr-companion`'s text example from the mock pane. A
spoken turn from the S23 over the tailnet reached Gemini with no error.

**Found along the way:**
- The key had an IP restriction, which the owner widened. It is now
  bound to the service account `handlrofflinesa`.
- The text example drops a line typed before it has connected.

## 2026-10-23 01:08 EDT — plan 20261022_08, step 1: the prompt leaves approval to handlr

Gemini no longer asks "Should I send…?" itself. It calls `run_command`
with a handle drawn from the user's words, and handlr asks a single
approval question. Only the gate block of `PROMPT.md` changed. The item 1
wording the owner approved is recorded in the plan.

Step 2's live turn uses the daemon on the owner's live herdr, without
`--mock`, by the owner's choice. Nothing reaches a pane without approval
on the phone.

## 2026-10-23 01:48 EDT — plan 20261023_01: confirmation handles, steps 1, 2, 3, 5 and 6

**What changed:**
- Gemini's handle is trusted within the turn, so a misheard "sin" no
  longer replaces "send".
- Handles that come back later are matched by entropy.
- PROMPT.md's three must-fix lines are applied: 11, 19 and 35.
- The companion app shows a confirmation card.

**Breaking:** `handlr-companion`'s `Event` changed. The crate is used
only inside this repository.

**Still to do:**
- `libhandlr_proto` constants for `-32009` and the new `confirm.status`
  fields;
- step 4, which waits on its open question;
- step 7, the live S23 check.

## 2026-10-23 02:15 EDT — plan 20261023_02, steps 1 to 4: dictated messages

The work is built, with its tests. The live S23 check is next.

**Departures, Rule 1.7:**
- **Step 1, the protocol.** `focus.changed` is a companion-protocol
  notification. `agents.send`'s `command` field also reads as `text`,
  so the companion sends `{pane_id, text, kind: "message", handle}` as the
  plan writes it. Adding fields to `AgentsListResult` and
  `AgentsSendParams` breaks callers that build them as struct literals.
  `libhandlr_proto` stays at 0.1.0, since it is used only inside this
  repository.
- **Step 2, the confirmation's lifetime.** A companion's message opens a
  `VoiceTurn` confirmation, because only a voice turn lets the requester
  approve its own send. So `agents.send` sends the approval question
  itself, and the confirmation expires after 60 s, or at the session's next
  turn. On a harness connection it is `Standing`.
- **Step 3, the silence is longer than planned.** The client re-arms the
  voice gate after each dictated utterance, so dictation is hands-free. The
  armed gate counts as listening for its 6 s arm time-out, so the silence
  that submits is about 14 s (6 + 8), not 8. This is raised with the owner.
- **Step 4, the Android setting.** The app has no settings screen, so the
  silence time on Android comes from `dictation_silence.txt` in its data
  directory.

**Also:** `config.default.toml` does not yet mention `allow_messages`.

## 2026-10-23 02:23 EDT — plan 20261023_02: the silence counts from the end of voice

**Fixed:** the effective silence of about 14 s that the step 3 entry
recorded. `handlr-audio` now reports when the gate opens on speech, as
`Input::Voice(bool)`. An armed gate waiting for speech is not voice. Once
a front end reports voice, the companion counts the dictation silence from
the end of voice, so a message submits 8 s after the speaker stops, as the
owner asked.

Front ends without a VAD, such as the text example, still count from the
gate going idle.

## 2026-10-23 02:52 EDT — plan 20261022_03, steps 1 to 3: agents' sessions reach Gemini

**What it does:**
- Gemini's turn context now carries the agents' own transcripts, as clean
  text and redacted, found by herdr's `agent_session` id.
- An agent's pending question is detected, and becomes a spoken notice
  when a watched agent stops on one.
- Terminal tails are read with `recent_unwrapped`, 80 lines.

**Versions:**
- libherdr is 0.2.0, tagged `libherdr-v0.2.0`, its first separate
  version. Before it, a rustfmt-only commit cleaned up its formatting.
- `libhandlr_proto`'s additions are additive. It stays at 0.1.0 while it
  is used only inside this repository, as before.

**Also:** `pane.read` used to send `count`, which herdr ignores. It now
sends `lines` as well, so the 50 lines Gemini was given were really
herdr's default all along.

**Still to check live:** step 2's question to the phone.

## 2026-10-23 03:11 EDT — plan 20261022_03, step 4: the agent view; a correction

**Step 4 is built:** `session.peek`, the agent's session on `agents.list`,
and the app's agent view.
- Terminal-style scrollback, with the app's own finger scrolling.
- Priority cards by the four rules.
- The pinned card, which shrinks to a bar while scrolled back.

How it behaves is recorded in plan 03's L3. `AgentInfo` gained a field,
which breaks callers that build it as a struct literal. `libhandlr_proto`
stays at 0.1.0, as it is used only inside this repository.

**Correction to the entry for steps 1 to 3.** It said the 50 lines Gemini
was given "were really herdr's default all along". That isn't verified.
`pane.read` sent `count`, which herdr's schema does not define, so herdr
**probably** used its own default. The read now sends `lines` too.

## 2026-10-23 14:54 EDT — plan 20261023_03: sections and serials; the card rules move to the daemon

Plan 20261023_03 (session subscriptions, Draft) gains an amendment of the
same time: the agent view is made of sections (turns, text blocks, tool
groups and cards), each with one serial derived from the session file
(`<file-name hash>:<line>.<slot>`), used to store, compare and backfill
them; `session.history` pages back through the session; resolved cards
are rebuilt in history; an approval journal per session keeps Approval
cards across restarts; and the UI always shows sections in serial order.

This departs from plan 20261022_03 (In implementation), decision 9 as
amended at 02:34 EDT and its step 4: the four card rules there run in the
companion client (`crates/handlr-companion/src/view.rs`). Under plan
20261023_03 they move, unchanged in what they find, to the daemon
(`src/sessions/sections.rs`), and the client receives cards as sections.
Discussed with and agreed by the owner on 2026-10-23 (Rule 2). Plan
20261022_03 is not edited.

## 2026-10-23 15:01 EDT — plan 20261022_03, step 4: the agent view's controls and suggested responses

Asked by the owner on 2026-10-23: "move the back to a glyph on the title
row, add a \"suggested response\" row with yes, no, continue, compact (if
needed), etc."; "just above the bottom buttons, plain text in a different
color (green/red/yellow)"; "suggestions that belong in a card can go
there (like if Claude is asking a question)"; "replace take a message
with the mic glyph but keep the button shape, always in the bottom
right"; "(if we don't have mic, us a centered \"record\" dot but note it
for later)".

This departs from the agent view of plan 20261022_03 (In implementation),
step 4, whose bottom row was Back and Take a message:

* **Back** is a `‹` glyph (U+2039, from the terminal face; the bundled
  Roboto is ASCII only) at the left of the title row; the left 44 pt of
  the header is its tap area.
* **Take a message** is a button at the bottom right, 96 pt wide, with a
  centred red record dot in place of a microphone glyph, which no bundled
  atlas has yet. **For later:** a microphone icon. While dictating,
  Submit, Scratch and Clear share the rest of the row; the button stays.
* **Suggested responses** (`handlr_companion::Suggestion`: `yes`, `no`,
  `cont`, `cmpct`): plain text just above the bottom row, yes green, no
  red, the commands yellow. Those that belong to a card are in it: a
  Question card's actions are Yes, No and the message button. The row
  shows `cont` when the pinned live card is Something broke or Finished,
  and nothing while working, dictating, or when the card has its own
  buttons. A tap is `Input::Respond`, one `agents.send` to the view's
  pane: `yes` and `no` as messages, `continue` and `/compact` as
  commands. The daemon confirms it as any other send; the tap is not the
  confirmation, and a refusal is said.
* **`cmpct` is not shown yet:** "if needed" needs the context's use,
  which the daemon does not report. It waits for that data (to be
  proposed for plan 20261023_03). **For later:** Claude's numbered
  question options (`1.`, `2.`, …) detected and made tappable.

## 2026-10-23 15:11 EDT — plans 20261022_03 step 4 and 20261023_02: scroll hold, Submit waits, read aloud, the play and record buttons

Asked by the owner on 2026-10-23, after trying the 15:01 build: "if I'm
scrolled back, don't force the scroll"; "it got it but said the message
was empty, that's why I repeated it"; "change the record button to white
on blue, and add a play button (dull/bright) on the left, this will play
or not the incoming white text boxes as voice using Gradium. it won't
attempt to speak the terminal output from Claude (bash tool etc.)".

* **Scrolled back, the view holds** (plan 20261022_03, step 4, L3-1).
  When the entry at the window's top leaves the 200-entry window, the
  view now stays on the next entry still there, instead of keeping its
  old offset into the new content, which moved it. Entries are matched
  by time, role and kind, so a summary that changes between peeks (a
  result linked to its call) no longer loses the place.
* **A Submit waits for speech still in STT** (plan 20261023_02). A
  Submit while the voice is on, or less than 3 s (`HEARD_WAIT`) after it
  ended with no utterance since, waits for the utterance and then
  submits, instead of saying "The message is empty." while the sentence
  was still being transcribed (the sentence then started a Gemini turn
  of its own). With nothing heard by then, it is refused as before. Only
  front ends that report voice (a VAD) wait.
* **Read aloud** (`Input::ReadAloud`): the play button, at the bottom
  left, dull when off and bright when on, reads the agent view's new
  replies with the app's TTS (Gradium's Phonon, on the device). Only
  replies (assistant text) are read, from when it is turned on; never
  tool calls, their output, or code blocks within a reply, and Markdown
  marks are dropped. They are said as notices are, after speaking,
  listening and confirmations. The setting is kept while the view is
  closed, not across restarts.
* **The record button** is a white dot on blue, at the bottom right. While
  dictating, Submit, Scratch and Clear sit between the two buttons.

## 2026-10-23 15:26 EDT — plans 20261022_03 step 4 and 20261023_02: the audio controls, the message card, own approvals

Asked by the owner on 2026-10-23: "the new UI is confusing. it should have
the bottom row to control the audio, with the left having a square play
button, the right a square record button, the play button should be the
submit button (android send button or similar) with an X button in the
middle to cancel. the entered text should be on a pinned card with the
scrolling pane pushed up and out of the way. the same applied to an inline
message, it expands the card being responded to with the same buttons in
place, hiding the global ones"; "a separate approval isn't needed for UI
entered buttons or when text has been buffered for review first"; "one
control per file with the restructuring".

This replaces the 15:01 and 15:11 entries' layout of the agent view:

* **The bottom row controls the audio**, all square: play (`▶`, dull or
  bright) at the left and record (a white dot on blue) at the right.
  While a message is taken, play becomes send (`➤`, on green), a cancel
  (`✕`) sits in the middle, and the record dot turns red. Scratch has no
  button any more; "scratch that" still works by voice.
* **The message being taken is a pinned card** above the row; the agent's
  pinned card makes way for it and the flow ends above it.
* **Replying to a card**: a Question card's record button (`AgentHit::Reply`)
  takes a message in reply to it. The card is pinned above the bottom
  edge, expanded with the message under its title and text, with send,
  cancel and record inside it; the screen's row is hidden.
* **Own approvals** (`handlr_companion`, departing from plan 20261023_02's
  amendment on button submits, under which "a silence submit still
  asks"): every submit of dictated text, by the button or the silence, is
  its own approval, since the text was shown for review; and a tapped
  suggestion is its own approval. The daemon still makes the
  confirmation; the phone approves it without asking. A message denied by
  another client still returns to the dictation with its text kept.
* **One control per file**: the new controls live in
  `apps/companion/src/agent/` (`control_row.rs`, `dictation_card.rs`,
  `square_button.rs`). The rest of `agent.rs` moves the same way as it
  is next touched, not all at once.

## 2026-10-23 15:29 EDT — a correction to the previous entry: record becomes send

The owner, on the build of the previous entry: "play doesn't become send,
record becomes send"; "the record/send action should always be in the
bottom right per Fitz". While a message is taken, the row is now play at
the left, cancel in the middle and send in record's place at the bottom
right, in the screen's row and inside a card being replied to. This
replaces the previous entry's "play becomes send … and the record dot turns
red": record is not shown while a message is taken.

## 2026-10-23 15:39 EDT — read aloud: paths and code as their English words

The owner, on a reply read aloud: "abbreviate a path like the one in the
last spoken message (I only need the English parts of it)", "(such as \"the
overview screen plan\")". `handlr_companion::speakable` now reads a link
as its text, a path as "the" and the words of its last part
(`docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md` is "the overview screen
plan", without the date, sequence or extension), inline code as its words
(`AgentInfo.asking` is "agent info asking"), a number or version as it is,
and a commit hash not at all. Part of the read aloud of the 15:11 entry.

## 2026-10-23 15:49 EDT — read aloud: skip while speaking

The owner: "tapping the play while it's speaking should stop the current
playback (skip). we should probably switch the glyph to the >> glyph while
speaking"; "or |> if we have it". With read aloud on, while something is
being said the play button shows `▶│` (the terminal face has no ⏭ or ⏩)
and a tap is `Input::Skip`: the client stops the speech
(`Event::StopSpeaking`) and the next held reply follows. When nothing is
being said, a tap turns read aloud on or off as before.

## 2026-10-23 15:55 EDT — plan 20261023_05, step 3: the summary's temperature is not applied yet

Plan 20261023_05, decision 8, gives `spoken_summary` the member
`- [abc] temperature: 0.2`. libinfer-gemini's `GeminiReasoningEngine` has
no generation settings, so `speech.summarize` cannot pass a temperature
without changing libinfer-gemini, which the plan does not cover. The
member is read and kept in the prompt's settings, as decision 7 keeps any
member the daemon does not use, and Gemini runs at its default
temperature. Passing it on needs a generation-settings option in
libinfer-gemini, left to a later change.

## 2026-10-23 16:00 EDT — plan 20261022_03, decision 8: a question at the end of a task is pending when the pane is done

The owner: "you're install question didn't get a card. if the question stops
as a task, it should be a pinned card". Decision 8 counted a reply's final
question as pending only while herdr says the pane is `idle` or
`blocked`. herdr says `done` when an agent has finished a turn the user
has not yet looked at, which is exactly when a question at the end of a
task waits, so no Question card (and no Yes or No) appeared. `done` now
counts too (`src/sessions/pending.rs`). The client's rules are unchanged:
the Question card is the highest live card, so it is the pinned one.

## 2026-10-23 16:04 EDT — plan 20261022_03, L3: the pinned card is shown once

The owner: "got two cards instead of one with the same content". The
pinned card was drawn in full above the bottom row and again at its place
in the flow, which at the bottom of the view sit one above the other. It
is now drawn only at the bottom; the flow keeps its place (no height), and
tapping its bar while scrolled back returns to the bottom instead of
jumping into the flow. This replaces the 03:11 EDT amendment's "Tapping
the bar puts the card a third of the way down the view".

## 2026-10-23 16:38 EDT — plan 20261023_05, step 8: only Gemini's summaries are kept, and a failed summary is still an error

Two small departures in the daemon's `speech.summarize`, needed by the
16:28 EDT amendment:

* **The cache keeps only Gemini's summaries.** The 15:45 EDT amendment's
  64-entry cache "keeps both answers". Under A2 the rules' text reads clock
  times relative to when it is asked for, and under A4 the answer for the
  same reply changes with the connection's `spoken_output`, so a kept
  rules answer would go stale ("just now" said an hour later). The rules
  are cheap and are applied afresh on every call; a summary is kept and
  answered from the cache only while spoken output is on.
* **A summary that fails is still `UNAVAILABLE`.** A3 has the client read
  its own copy of the rules only when the daemon cannot be reached or does
  not answer in time. When spoken output is on and the reply is complex
  but Gemini has no key, the prompt did not load, or the call fails or
  times out, the daemon still answers `UNAVAILABLE` (decision 4 and step
  3's tests), and the client reads the reply through its copy of the
  rules, which is the same `handlr_speech::read_aloud` (at the phone's
  local time rather than the daemon's).

## 2026-10-23 17:33 EDT — plan 20261023_06, step 1: the ignored test of the example waits for the reader

Step 1, as amended, says the example is "read by an ignored test". Reading
it needs the agy reader of step 4 and a SQLite dependency, so that test is
written in step 4, as `#[ignore]` unless `HANDLR_AGY_EXAMPLE` names the
copy. Step 1's mapping was checked instead with a throwaway decoder outside
the repository, against the example and the synthetic fixtures alike.

## 2026-10-23 17:31 EDT — plan 20261023_04, step 2: the pending panes and the Gemini turns as events

Two small departures in `handlr-companion`, needed because the app sees
the client's `State` only through its events (the `net` thread owns it):

* **`Event::PendingPanes(Vec<String>)`.** Section 4 gives
  `State::pending_panes()`, which is there, but the app cannot call it.
  The same list is also given as an event whenever it changes, for the
  cards' approval outline (A3).
* **`Event::Conversation { speaker, text }`.** A5 moves the main screen's
  conversation to the overview's lower half as a log. The main screen
  kept only the last utterance heard and the last thing said, and
  `Event::Say` also carries notices and replies read aloud. The client now
  gives the user's words as a turn starts and each `turn.say` of the
  running turn as `Conversation` lines (Gemini's are still given as
  `Say`), so the log holds the Gemini turns only.

Decision 6's refreshes (on `focus.changed`, and every 15 s) are asked only
while no agent view is open, as step 2's **Done when:** says, and closing
the view asks for one; `Event::Agents` is given on the first answer of
each connection, then when the list changes.

## 2026-10-23 17:34 EDT — plan 20261023_04, A4: the agent view's turn is listed while herdr says working

A4 has the agent view give the Finished card for a stopped turn when herdr
still says working. The turn comes with `agents.list`, which the view asks
only when it opens, so a turn that stops while the view is open would not
be seen. The client therefore also asks `agents.list` when a peek of the
open view ends on a reply while herdr says working and the turn it knows
is not stopped, at most once in 5 s and never with a list on its way.
The card itself needs the reply to be the view's newest entry, so a turn
the view has seen go on (a tool call after the reply) is not finished on
an older list. This is the only list asked while a view is open besides
the one on opening it.

## 2026-10-23 17:43 EDT — plan 20261023_04, step 4: the scrolling as a common control

A1 has both halves of the overview scroll on their own "with the agent
view's drag, fling and slop". That scrolling lived inside `AgentScreen`, so
two more windows would have copied it. It is now a component of its own,
`Scroll` in `apps/companion/src/controls/scroll.rs` (a file A6 does not
list), using the agent view's slop, decay and least fling speed; `CardGrid`
and `TurnLog` each keep one. `AgentScreen` keeps its own scrolling for now,
as decision 8 moves code into the layout as it is changed, not all at once.

## 2026-10-23 17:48 EDT — plan 20261023_04, step 5: what the overview takes from the main screen

Small departures in `apps/companion`, needed to put the overview in the
main screen's place:

* **The overview draws the whole home screen.** The main screen was one
  libcontrols `Router` whose draw clears the list, so the overview could
  not draw beside it. The overview draws the title, the status lines (and
  a pairing note or an error under them), the two halves, the
  confirmation and dictation cards and the bottom row itself. `ui.rs`'s
  `Screen` is now the pairing screen only: it shows while the phone is
  neither paired nor connected, or shows its device code, and it loses the
  conversation, the focus line and its Approve, Deny, Take a message,
  Submit, Scratch and Clear row, which the overview replaces.
* **The confirmation card's buttons are in the card.** Approve and Deny
  sit under its lines and answer its own id (`Input::Answer`), as the
  agent view's do. While a confirmation asks, the dictation card makes
  way for it, as the main screen's did.
* **Scratch is a word, not a button, on the home screen.** The bottom row
  is the agent view's (decision 5): record, then send and cancel. "Scratch
  that" still removes the last utterance.
* **`OverviewHit::Tap`.** Section 4's `OverviewHit` has `Agent` and
  `Bottom`; a tap on no control is also a hit, the main screen's tap
  (`Input::Tap`), so the overview returns it too.
* **What Gemini says is the turn log.** The main screen showed the last
  `Event::Say`, which also carries notices and replies read aloud; the
  lower half shows `Event::Conversation` (see the 17:31 EDT entry), with
  what STT is hearing as its last, dim line.

## 2026-10-23 17:58 EDT — plan 20261022_02, liveness: the phone pings a quiet link

The owner (2026-10-23), after the phone sat on "(loading session)" over a
half-open link (the daemon had closed it at 17:36:27; the phone never
noticed, so the view's peek never came back): "ping 30s is fine, but
should only be sent if nothing received for that time". Plan 20261022_02
left liveness to `SO_KEEPALIVE`, whose Android timers run for hours, and
said "not `ping`". Now the companion client sends `ping` after 30 s
(`PING_IDLE`) with nothing at all from the daemon, and closes the link to
make it again when the ping goes unanswered for 10 s (`PING_WAIT`). Any
message from the daemon counts as heard, so a busy link is never pinged.
The daemon already answers `ping`; nothing else changes on its side.

## 2026-10-23 18:00 EDT — plan 20261022_05, decision 3: the project is the directory by default

The owner (2026-10-23): "the herdr session name isn't useful compared to
the agent/dir name, it could be an option for others that use them
properly but I just use the tabs". Decision 3 named an agent's project by
herdr's workspace label, falling back to the cwd's last part only when
agents shared a label. Now `[naming] project_from` chooses: `"directory"`
(the default) names it by the cwd's last part, using the label only when
there is no cwd; `"workspace"` keeps decision 3 as it was.

## 2026-10-23 19:22 EDT — plan 20261023_04: the overview remembers where it was scrolled

The owner: "The scroll should remember where it is on the overview page".
The common `Scroll` (`apps/companion/src/controls/scroll.rs`) kept only a
clamped offset, so any moment when the window was shorter or the content
smaller (a confirmation or message card appearing, the strip, a list
answer while cards were re-laid out) moved the place up for good. It now
keeps where the user left it, and the offset returns there as soon as
there is room again. The agent view's own scrolling is unchanged.

## 2026-10-23 21:29 EDT — plans 20261022_02 and 20261023_04: the link sleeps in the background

The owner: "in any case it should be a single sleeping socket". When the
app leaves the screen (libmorph's `on_suspend`) it sends
`Input::Background(true)`, and the companion client stops everything it
does on a timer: the agent view's peeks (5 s working, 15 s idle), the
overview's `agents.list` refresh (15 s, plan 20261023_04 decision 6), and
so the read-aloud summaries that follow peeks. The daemon still pushes
confirmations, notices and focus over the same socket. The ping (plan
20261022_02, as departed from at 17:58 EDT) waits 270 s of quiet
(`PING_IDLE_BACKGROUND`), under the usual NAT and carrier idle timeouts,
instead of 30 s. Back on screen (`on_resume`), one catch-up (a peek of the
open view and a list), then the usual timers. Keeping the process and its
socket alive while asleep is the foreground service's job (libmobile).

## 2026-10-23 21:34 EDT — plan 20261022_05: the listed project follows the naming too

The owner: "we haven't fixed the agent names on the overview screen". The
16:58 EDT change named projects by directory in each agent's `name`, but
`agents.list`'s separate `project` field still carried herdr's raw
workspace label ("cc-highbay"), and the overview's cards show that field.
It now carries the project as named (`[naming] project_from`, the
directory by default), falling back to herdr's label only with no name.

## 2026-10-23 21:34 EDT — a correction to the previous entry

The previous entry cites "the 16:58 EDT change"; the change it means,
"the project is the directory by default", is at 18:00 EDT.

## 2026-10-23 21:40 EDT — plan 20261022_02: the daemon sends a sleeping phone only what needs the user

The owner: "only blockages or done messages should be sent on a sleeping
conn". The phone's sleep (the 21:29 EDT entry) now tells the daemon too:
`client.options` gains `background`, sent when the app is suspended and
resumed. While it is set, the daemon's periodic vocabulary and focus
pushes (`vocabulary.changed`, `focus.changed`) are held, to go at the
first check after waking; notices were already only for an agent blocked,
done or asking, and confirmations always go. libhandlr_proto 0.3.1, a
patch by the owner's ruling: "I would consider it additive, just patch
level", "appending to a struct is additive", "same with open enums, not
closed".

## 2026-10-23 21:42 EDT — plan 20261022_02: ping says whether the app is in front; a silent phone is taken as asleep

The owner: "do we consider a daemon not getting a ping for 60s enough to
background the link?", and "ping should have active flag (assuming it's
json), and send it when user resumes app".
- `ping` takes `{active}` (libhandlr_proto 0.3.2, `PingParams`): the
  companion's background ping says `active: false`, and resuming the app
  sends `ping {active: true}` at once. The daemon takes the flag as the
  phone's state, as `client.options` `background` does.
- A companion that has asked nothing but inactive pings for 60 s
  (`ASLEEP_AFTER`) is taken as asleep even without being told, since an
  awake phone always asks within about 30 s; any other request, or an
  active ping, wakes it.

## 2026-10-23 22:20 EDT — plan 20261022_03, L3: the setting is `[urls] repowiki_url`

The owner: "make the section urls then option repowiki_url". The 21:57 EDT
amendment's `[repowiki] url` is `[urls] repowiki_url` instead, so other
addresses can join it under `[urls]`. Nothing else changes.

## 2026-10-23 22:31 EDT — plan 20261022_03: the Claude Code reader shows everything the terminal shows

The owner: "I'm comparing the terminal and our output, why are there so
many missing messages?", and "right, this is more for active sessions but
our history infill should get them also, the ui feels sparse like I'm
missing something." The reader (`src/sessions/claude.rs`) read only
`user` and `assistant` lines, so it missed:
- what the user types while the agent works, which Claude Code records
  as an `attachment` of type `queued_command` (human origin): now a user
  text entry like any other;
- task and background-command notifications (the queued form, and user
  lines whose `origin.kind` is `task-notification`, which were dropped),
  cross-session messages and subagent hand-backs (`origin.kind: peer`);
- `system` lines: `away_summary` recaps and `compact_boundary` /
  `informational` compaction notices.

These come back as user text entries marked by a new `Entry::origin`
(libhandlr_proto 0.3.4, a patch by the owner's ruling, "appending to a
struct is additive"): `"task"` (the notification's `<summary>`),
`"peer:<name>"`, `"agent:<id>"`, `"recap"`, `"compact"`, each cut to 300
characters. An entry with an origin is context, never the user speaking:
it does not answer a pending question, end a turn in the overview, or
resolve a Question or Finished card, and the app draws it as one dim
line with an amber dot.

`read_tail` read back at most 2 MiB, but most of a busy session's tail is
lines with no conversation in them (`api-request-blob`, `api-request`,
`file-history-snapshot`, `queue-operation`, other attachments, `mode`,
`permission-mode`, `last-prompt`, `ai-title`), so a peek of 200 entries
returned 100. Those lines are now recognised from their first bytes and
not counted: the budget is 2 MiB of conversation lines, with at most
32 MiB read per call.

## 2026-10-23 22:45 EDT — plan 20261022_03: details of the 22:31 EDT entry

- Peer messages delivered as user lines (not queued) are `isMeta` with
  `origin.kind: peer`; they are read too, before `isMeta` lines are
  skipped.
- An `informational` system line is a compaction entry only when it
  speaks of compaction (one in the session read did not); the app shows
  a compaction once when the boundary and its notice come together.
- Gemini's session context labels a context line by its origin (`- task
  [ts]: …`, `- peer:<name> [ts]: …`) instead of `user`, and its heading
  says these are not the user speaking.

## 2026-10-23 22:51 EDT — plan 20261022_03, L3: the agent view keeps what it showed

The owner, on entries vanishing while scrolled back: "it might be the
scroll back, it shows and then disappears when it lands". Each peek
replaced the view's entries with the newest 200, so as new entries arrived
the oldest left the window, under a reader scrolled back to them. The
client now lays each peek over what it had: entries older than the
peek's first stay, the peek replaces the rest (so a changed entry is
updated), up to 1000 kept (`VIEW_KEEP`); a different session file, or a
peek that does not overlap, starts fresh. History paging (plan
20261023_03) replaces this when it is built.

## 2026-10-23 23:16 EDT — plan 20261022_03, L3: after a gap the view starts fresh

Checking the scrollback over adb (the owner: "scrolling down (finger drag
down) not scrolling back"), the view's newest entries went backwards
after the app had been in the background and its link reconnected (at
11:08 local the bottom was the 03:07 UTC reply, at 11:14 the 03:02 one).
The 22:51 EDT merge kept what the view held across the gap. After a
disconnect, or the app coming back to the front, the next peek now
replaces the entries instead of merging onto them, so the newest are
certain; and each peek's result is logged (the count and the newest
entry's time, no text) to find the cause if it recurs.

## 2026-10-23 23:19 EDT — plan 20261022_03: a huge line counts at most 32 KiB toward the read budget

After the previous entry the phone's log showed the view getting 27
entries, not 200. The newest part of the owner's session holds screenshot
results, `user` lines of 750 to 825 KiB each (base64 images), and the
22:31 EDT reader change counted every conversation line's bytes whole
toward the 2 MiB budget, so five of them filled it. Each line now counts at
most 32 KiB (`LINE_BUDGET_MAX`): the reader keeps only a few thousand
characters of a tool result anyway. The 32 MiB cap on what one read may
cover stays. This is also the likely cause of the view going backwards:
as screenshots arrived, a peek's window no longer overlapped what the
phone held.

## 2026-10-23 23:03 EDT — plan 20261023_03, step 1: the focus and pane events are modelled from herdr's schema, not a recording

Step 1 says to record live `pane.focused`, `tab.focused`, `pane.updated`
and `pane.closed` events read-only and model them from the recording.
None was recorded: the fixture,
`crates/libherdr/tests/fixtures/lifecycle_events_synthetic.jsonl`, is
synthetic, written from herdr's own API schema (`herdr api schema
--json`, protocol 19, schema_version 1, read on 2026-10-23), whose
`EventData` gives `pane_focused {pane_id, workspace_id}`, `tab_focused
{tab_id, workspace_id}`, `pane_updated {pane}` and `pane_closed
{pane_id, workspace_id}`. The events are modelled as
`HerdrEvent::PaneFocused`, `TabFocused`, `PaneUpdated` and `PaneClosed`
from it. Step 1's other half of **Done when**, a subscription with all
four types accepted by live herdr, is left to the owner's live check
with step 5.

## 2026-10-23 23:24 EDT — plan 20261023_03, step 3: how the hub numbers, groups and redacts sections

Small departures needed to make A2 and A3 work, built in `src/sessions/`
(`sections.rs`, `hub.rs`, `journal.rs`):
- **Slots in fixed ranges.** A2 gives a line's own entries the first
  slots and the cards after it the next ones, so a card's slot would
  depend on how many entries the reader finds on the line. Instead:
  0–63 the line's entries (more go into slot 63's section), 64–127
  Approval cards (the journal's next free slot), 128 Question, 129 the
  Broke card of a blocked spell, 130–193 a Broke card at a failing
  result (130 plus the result's slot), 194 Finished. Within a line the
  order is still entries, then cards by priority.
- **Grouping looks back at most 256 lines (`REACH`).** A text entry joins
  the open text block, and a result its call's section, only within 256
  lines; and a reply's turn counts as ended (a Finished or resolved
  Question card) only when the next entry, but for context, is the
  user's within 256 lines. That makes a section's grouping a function of
  the 256 lines before it, so a history page read from 256 lines before
  its span (and 256 past it) gives the same sections and serials as the
  live tail and a read from the file's start.
- **Cards that no longer stand are removed, not kept faded.** A Finished
  card for a reply the agent then carried on from (no user entry
  followed), a live Question card whose question the agent left, and the
  Broke card of a blocked spell (which no line of the file records) are
  not cards that history can rebuild, so when they stop being live the
  tailer sends their serials in `SessionDelivery::removed`
  (libhandlr_proto 0.3.5) and the client drops them. Every card left
  showing is one a history read rebuilds (A8.5).
- **Secret calls are found file-wide.** Decision 7 remembers the last 512
  calls, and A5 lets an old result whose call is not on the page keep
  its text if its own text names no secret path. That would show a
  `.env` file's contents in history whenever its call was more than a
  page back. Instead the tailer's one scan of the file (for the line
  index) also notes the lines holding `.env` or `.private`, parses only
  those, and keeps the ids of the calls among them that name a secret
  path; every builder, live or history, redacts their results wherever
  the call is. The result's own text is still checked for a call no
  scan found.
- **The journal also keeps the question's text** (`text`, redacted),
  which the Approval card shows, beside A4's `command`.

## 2026-10-23 23:35 EDT — plan 20261023_03, step 4: subscriptions on a sleeping link, coalescing, and where the tests are

Small departures needed to make step 4 work (`src/server/subscriptions.rs`,
`src/server/conn.rs`):
- **A connection with a session subscription is not taken as asleep for
  being quiet.** `Conn::asleep` (the 21:42 EDT entry) took a companion
  that had asked nothing but pings for 60 s as asleep, since an awake
  phone polled within 30 s. With the view pushed to, an awake phone may
  ask nothing for minutes, and the phone pings only after 30 s with
  nothing heard, so a busy subscription would have stalled every
  minute. The inference now applies only to a companion holding no
  subscription; `client.options` `background` and `ping {active:
  false}` still put any link to sleep.
- **The daemon catches a woken subscription up itself.** The 22:55 EDT
  amendment's item 4 has the subscription catch up with `after` on
  waking. Instead, while the link sleeps the subscription keeps adding
  to its one waiting delivery (decision 9, so a long sleep becomes a
  reset with the newest 50) and writes it at the first check after
  waking; the client sends nothing for it. `after` is still how a client
  that reconnects catches up.
- **"Waiting to be written" is counted at the writer.** Each connection's
  writer counts the lines it has written; a delivery is the waiting one
  until the count passes its place in the queue, so a phone that stops
  reading holds one delivery in the queue and the rest coalesce.
- **A catch-up resends the 256 lines before `after`.** A section at or
  just below the client's newest serial may have changed (a text block
  that grew, a tool that got its result, a card resolved), so `after`
  gives the sections from 256 lines before it on, not only those above
  it.
- **The tests are in `tests/subscriptions.rs`** (sections 10.3 and 10.4
  name `tests/daemon.rs` and `tests/link.rs`), with a stand-in herdr
  event socket for `pane_focused`, `pane_updated` and `pane_closed`, and
  link clients paired by writing `auth.toml` (an entry's `agents` narrows
  its reach). `tests/daemon.rs` and `tests/link.rs` now read sessions
  under a temporary home, so the journal is never written to this
  machine's `~/.local/state`.
- **A closed pane's subscription ends by itself**: after the
  `pane_closed` delivery its id is unknown (`SUBSCRIPTION_NOT_FOUND`).

## 2026-10-23 23:40 EDT — plan 20261023_03, steps 5 to 7: the tail's sizes, the focus row, and where the view's cards and status come from

Small departures in `src/tail.rs`, `crates/handlr-companion` and
`apps/companion`:
- **Step 5: `handlr session tail --entries N` counts sections**, as A5
  made the backfill's size (`sections`); each printed line starts with
  its section's serial, as A2 replaced `seq`. Only the CLI is built; the
  live check of section 10.6 and A8.8 against herdr is the owner's.
- **Step 7: the focus row is the focused agent's card.** Section 7 makes
  the main screen's focus line a control with a chevron. Since plan
  20261023_04 the main screen is the overview, which has no focus line:
  its focused agent's card, which a tap already opened, is the control,
  and its heading now ends in `›`.
- **Step 6: Approval cards come only from the daemon.** The client no
  longer makes an Approval card from `confirm.requested`; the daemon's
  card sections carry them (A3, A4), so the view never shows one twice.
  A pane with no session here has no sections, and so no Approval card
  in the view; its confirmation is still asked as before.
- **Step 6: the view's status is the subscription's** once it has
  answered; until then it is `agents.list`'s.

## 2026-10-24 00:13 EDT — plan 20261022_03 L3, the ribbon and jump-line amendments: readings in implementing them

Living section L3, the 23:54 EDT (ribbon) and 00:05 EDT (jump line)
amendments, built in `420c8f9` (handlr-companion) and `cc5ba56` (companion).
- **Index from the client for now.** `ViewSection.index` is the section's
  position among the sections held (`view::section_index`). It shifts when
  a page lands above, until plan 20261024_01 sends the daemon's index. The
  place is held by serial, so the shift never moves the view.
- **Jump-line text, item 2 read literally:** `-- jump to end --` with nothing
  new, and `-- N new: <newest first line> --` otherwise, `-- 1 new: … --` for
  one. "New" means a serial newer than the newest held when the view left
  the bottom, so history pages and replaced sections don't count.
- **Jump-line place:** its 28 pt is always reserved directly below the
  scrollback's window, above the pinned card, the suggestions and the
  controls. It is blank while following.
- **Cards' `at` is 0 in sections,** recomputed when flattened, so a page
  landing doesn't mark every later card as changed.
- **Positions move down by the marker row,** which is now always there.
- **The assembled flow stays:** cached blocks are put together into one
  `Flow` for the kept span, so drawing, hits and links are unchanged.
- **Running heights are summed again in full** on each change (O(n)
  additions), not only from the changed slot on.
- **A block is also laid out again** when its pin changes, or when the
  neighbour of a leading compaction line changes (`NoticeLine::repeats` now
  takes a "previous entry is a compaction" flag).

## 2026-10-24 00:30 EDT — plan 20261024_01, steps 1–3: departures

Built in `e971a80` (libhandlr_proto 0.3.6), `e971a80` (handlr-companion, compile fix
only) and `4185076` (handlr, steps 2 and 3 in one commit, because the crate does not
build against 0.3.6 with the hub alone).
1. **Step 1:** `SESSION_CHANGED` (-32014) already existed in 0.3.5, so it
   was not added. `session.content` uses it, and only its doc comment
   changed.
2. **Step 2, decision 1, "stable after a restart": not fully met.**
   Indexes are the same on every read of the same file in the same live
   state. But a section that appeared and was removed before a fresh read
   is not numbered by that read. And a card that appears behind sections
   already numbered (a Finished card after context lines) takes the next
   index, not its serial position. So after a daemon restart, indexes can
   differ from the ones a client holds. Making them match would mean
   persisting the numbering (plan 20261023_09's daemon cache). Raised with
   the owner.
3. **Step 2, "never reused":** a removed section whose own serial comes
   back (a live Finished card that returns) gets its old index back, with
   `rev` raised. No other section ever takes that index.
4. **Step 3, decision 3:** a resumed outline (`after` plus `from`) also
   resends the slots within 256 lines before `after`, and their contents
   when they are among the newest 8, as the existing catch-up does.
5. **Step 3, decision 3:** outline deliveries carry the old `removed`
   serials as well as `removed_index`. `first` is always 0.
6. **Step 3, decision 4:** `from` greater than `to` is invalid params. On
   a `to`-only or no-bound request, `more` means there are more sections
   before the first one returned. `limit` is capped at 200, including for
   `slots_only`.

## 2026-10-24 00:31 EDT — correction: the libhandlr_proto 0.3.6 commit and tag

This corrects the previous entry's first line. The libhandlr_proto 0.3.6
commit is `6baa3ee`, not `e971a80`; `e971a80` is handlr-companion's
compile fix, the commit right after it. The tag `libhandlr_proto-v0.3.6` was
made, and pushed, at `e971a80` by the same mistake: a commit search matched
"0.3.6" in the wrong message. Rule 5.1 means a pushed tag is never moved, so
it stays there. Its tree holds libhandlr_proto 0.3.6 exactly as `6baa3ee`
does, plus three lines of test and struct fields in handlr-companion.

## 2026-10-24 00:36 EDT — plan 20261024_01, steps 3–5: a daemon epoch, outline-only resets merged by serial, and the old ribbon kept while scrolled back

Agreed with the owner (2026-10-24): "just the outline, it should merge the
new indexes received. it should also keep the old one while scrolled back,
showing the new content jump label which swaps and jumps to the end"; then
"yes". This replaces my question about resetting a client that reconnects
to a restarted daemon (the previous entry's departure 2), and departs from
decision 3's reset in outline mode and decisions 6 and 7.
1. **Step 3, the epoch.** Each daemon run picks a random `epoch: u64`, sent
   in every outline delivery (`SessionDelivery.epoch`) and given back by a
   resuming client (`SessionSubscribeParams.epoch`). If the client's epoch
   isn't this run's, the outline starts again from index 0 with `reset`.
   In outline mode, `reset` drops only the outline, never the contents.
   Both fields are appended, so this is libhandlr_proto 0.3.7, a patch.
2. **Step 4, merged by serial.** Contents stay keyed by serial. A new
   outline replaces only the index-to-serial map. Each new slot is matched
   to the contents held for its serial, and fetched again only when its
   `rev` is higher. Contents with no slot in the new outline are dropped.
3. **Step 5, the old ribbon kept while scrolled back.** A new outline that
   arrives while the view is scrolled back is held aside, and the view
   keeps the old ribbon unchanged. The jump line reads `-- new content --`.
   Tapping it swaps in the new ribbon and jumps to the end. While following,
   the swap is at once, at the bottom.
4. **The same for a new session file** (`/clear`): while scrolled back, the
   old session stays until the tap.

## 2026-10-24 01:02 EDT — plan 20261024_01, steps 4–5: departures

Built in `f9c9e91` (handlr-companion, step 4), `3672a86` (apps/Cargo.lock) and
`63b90fb` (companion, step 5), with the departure entry of 00:36 EDT.
1. **Step 4, holes:** a hole is asked for only after the outline's first
   hole has stayed unfilled for 1 s (`GAP_WAIT`). Read literally, decision
   3's rule always holds while an outline is still arriving in chunks, and
   a new slot at the end can arrive before them.
2. **Step 4, rev:** contents are fetched again when the held rev *differs*
   from the slot's, not only when it is higher. Revs count from 0 again
   after a daemon restart, so "higher" would keep stale contents.
3. **Step 4:** contents are recorded at the rev their slot had when they
   were asked for, so a slot bumped while the request is in flight is
   fetched again.
4. **Step 4, old daemons:** a daemon that sends no `epoch` gets the old
   view (sections, history pages, the client's positional index), so
   "without outline, as before" holds. `view::section_index` stays for
   that case only.
5. **Step 4, while a new outline is held aside:** nothing is fetched for the
   old ribbon, because `session.content` answers by the daemon's new
   indexes. Live contents go to the outline held aside, and the old ribbon
   stays unchanged. Contents are not fetched until the outline shown is
   complete.
6. **Step 4, never forgotten:** the contents of the newest `LIVE_INLINE`
   indexes and of live cards, which the jump line and the pinned card
   need.
7. **Step 5:** contents filling a placeholder, or being forgotten, do not
   reset the "continue" suggestion's 30 s quiet timer.

Open: while scrolled back with a new outline held aside, replies that
arrive are not read aloud until it is applied. Window reports during a
drag are sent only when the window changed, with no other throttle.

A correction to `f14c2e4`'s message, "apps: lock libhandlr_proto 0.3.7":
it locked 0.3.6. `3672a86` locks 0.3.7.

## 2026-10-24 01:06 EDT — plan 20261024_01, step 5: the jump line's tap adds new content below and scrolls, in place of the swap

Agreed with the owner (2026-10-24); see the L3 amendment of the same
time in plan 20261022_03. This replaces item 3 of the 00:36 EDT entry
("tapping it swaps in the new ribbon and jumps to the end"). With a new
outline held aside, the tap adds the new outline's sections that come
after the old ribbon's newest serial below its end, and scrolls there in
2 s on an ease-in-out curve. The rest of the new outline is merged once
the view is at the bottom and following. Holding the outline aside while
scrolled back (item 3's first half, and item 4) stands.

## 2026-10-24 01:19 EDT — plan 20261022_03 L3, the 01:06 and 01:07 EDT amendments: readings in implementing them

Built in `efcf983` (handlr-companion) and `587550d` (companion).
- **Indexes of sections added below:** they keep the new outline's indexes
  when the first comes after the old ribbon's last. Otherwise each becomes
  the old last index + 1 + its own index, so order holds and stays stable
  as more arrive.
- **A new session file:** every slot of the new file counts as newer
  (serials of two files can't be compared), so the new session goes below
  the old one until the merge.
- **Window reports:** none is sent during the scroll. Landing reports
  `scrolled_back: false`, which triggers the merge.
- **The full 2 s:** the scroll follows only at the end of the 2 s, never
  earlier, when within 1 px of the end.
- **Content landing above during the scroll:** the start shifts by the
  same amount, so the curve stays on the same content.
- **State text:**
  - New sections with none of states 1–5 holding (idle, no live card) show
    `-- jump to end --`.
  - "done" holds only while herdr's status is not `working`.
  - "approval: …" shows the Approval card's first line; the card has no
    separate command field.
- **New event field:** `Event::AgentView.held_end`, the held-aside outline's
  live cards and newest section, so the line can state the end it covers.
  handlr-companion's version is not bumped, as with `new_content` before.

## 2026-10-24 11:14 EDT — plan 20261022_02 liveness: the resume ping is timed, 3 s

Found on the S23. The owner said: "app didn't update at all when returning
to it". Samsung's Freecess had frozen the app in the background. Back in
front, the client sent its `ping {active: true}` but didn't time it, so
the dead socket was only noticed 13 s later, and the view sat stale until
then. The resume ping now sets `ping_sent` with its own wait,
`PING_WAIT_RESUME` = 3 s. Unanswered, the link is closed and made again,
and the subscription resumes (`after`, `from`, `epoch`). The idle ping
keeps `PING_WAIT` (10 s). This adds to the earlier departure, in which
the idle ping replaced `SO_KEEPALIVE`.

## 2026-10-24 11:24 EDT — plan 20261022_03 L3, the two-finger scroll and scrollbar amendment: readings in implementing it

Built in the handlr-companion commit (`Event::AgentView.held_newer`, the
held-aside outline's newer slots, counted in the calculated length; the
new `view::newer_slots`) and the companion commit after it.
- **The 5× fling** is 5 × the one-finger fling (after the force curve),
  then any carried fling is added, under `MAX_FLING_OUT`. It applies when
  the second-to-last finger lifted within 100 ms of the last: fingers
  lifted together never leave Android at the same instant.
- **Three or more fingers** also scroll at 5×. Two fingers down is never a
  tap.
- **The thumb** sits 4 pt in from the right edge, so its 12 pt strip ends
  at the screen's edge. Its length is at most the viewport's height.
- **While the thumb is dragged:** no window is reported, so contents load
  where it stops. The pinned card stays in its bar form, and other fingers
  are ignored.
- **Multi-touch:** libmorph's Android runner already sends each pointer
  with its own id, so nothing was needed upstream.
- handlr-companion's version is not bumped for the new event field, as
  with the earlier ones.

## 2026-10-24 11:40 EDT — plan 20261023_04 (the overview), departure: the general agent's centre button and the live transcript

Agreed with the owner (2026-10-24), from plan 20261024_03 (the general
agent as a realtime chat) and its amendment of the same time. The
overview's plan is frozen, so its part is recorded here.
- **The centre button.** The overview's controls row gets the general
  agent's centre button between record and the right-hand button. It is
  an X while voice input can be cancelled. It is hidden when the general
  agent is off (the settings plan), except for the X.
- **The live transcript.** The Gemini strip shows the chat's latest line
  as it is heard or spoken, and the overlay (A5) shows the whole
  transcript live: partial words dim, then final; the reply as it
  streams; action lines in the accent.

## 2026-10-24 12:03 EDT — plan 20261023_05 (spoken summaries), departure: points for the expanded card

Agreed with the owner (2026-10-24); see plan 20261022_03's L3 amendment of
the same time. `speech.summarize` also asks Gemini for `points`: at most 6
short lines summarising the reply point by point, returned in an appended
`SpeechSummarizeResult.points: Vec<String>` (serde-defaulted; a
libhandlr_proto patch release by the owner's rulings). Without Gemini, or
when it gives none, `points` is empty. `read`, `summary` and `complex` are
unchanged.

## 2026-10-24 12:06 EDT — plan 20261023_05 (spoken summaries), departure: the daemon summarises Finished cards itself, under `[summary]`

Agreed with the owner (2026-10-24): "should be daemon option, under
summary tag"; then "yes". This replaces the entry "points for the expanded
card" (2026-10-24 11:59 EDT). The points no longer come back from
`speech.summarize`.
- **The setting,** in `config.toml`: `[summary] cards = true` (the
  default). On, the daemon summarises every new Finished card once with
  Gemini, whether or not a client reads aloud. Off, it summarises only
  while a client reads aloud, as plan 20261023_05's A4 says. There is no
  phone toggle for it.
- **Two fields on the card,** `CardBody.points: Vec<String>` (at most 6
  short lines) and `CardBody.spoken: Option<String>` (the text to read
  aloud). Both are appended and serde-defaulted, so libhandlr_proto gets a
  patch release by the owner's rulings. When they are ready, the card's
  section is replaced, so every client gets them, the outline included.
- **Read aloud** uses `CardBody.spoken` when the card has it, and does not
  ask again. `speech.summarize` stays for replies without a card, and for
  `cards = false`.
- **Without Gemini in the daemon** (no key, unreachable, or an error), both
  fields stay empty. This is not the general agent toggle, which only
  stops the user's own Gemini turns.

## 2026-10-24 12:06 EDT — correction: the entry the previous one replaces

The previous entry says it replaces "points for the expanded card
(2026-10-24 11:59 EDT)". That entry is dated 2026-10-24 12:03 EDT.

## 2026-10-24 12:26 EDT — plan 20261024_02, steps 1–5: departures

The steps are built in the commits on main, from `7716930` to the companion commit
after the handlr-audio one.
1. **An extra handlr-audio commit for the chime.** The speaker's ring
   buffer belongs to the TTS worker, so `Event::Ding` becomes
   `Action::Chime`, a 0.35 s two-tone chime mixed over the speaker output.
2. **"A Finished card" on the overview:** the overview has no cards, so it
   dings when an agent's turn in `agents.list` changes to stopped. In the
   agent view, it dings on a new live Finished card after the agent has
   worked. Both are deduplicated per pane.
3. **Microphone off doesn't stop capture:** libvoicexn's STT takes only
   `Arm` and `Close`. The gate is never armed, utterances are ignored, and
   the status line reads "microphone off".
4. **Also disabled with the microphone off:** the dictation controls, and
   *discuss*, which starts a dictation.
5. **The desktop data directory** is `~/.config/handlr-companion`, until
   libmorph 0.4.0's `data_dir()` on every host.
6. **No version bump** for handlr-companion's new `Input::Settings`,
   `Input::Unpair`, `Event::Ding`, `Event::Unpaired`, `Output::Forget`,
   `Settings` and `KeyStore::forget`, or handlr-audio's `Action::Chime`, as
   with earlier entries.
7. **Two intermediate commits don't build** on their own, because of
   exhaustive struct literals and matches. The protocol commit needs the
   handlr-companion commit, and that one needs the companion commit.
8. **`VaultKeyStore::forget` has no unit test:** libmorph's in-memory
   `SecureStore` constructor is crate-private. `FileKeyStore::forget` is
   tested.
9. **The plan's amendments are in time order.** The immutable marker
   (11:49 EDT, written in the worktree) sits before the Clear cache
   amendment (12:12 EDT), which was written on main meanwhile.

Open: the S23 checks of step 5. The battery rows wait on libmorph 0.4.0's
battery wrapper, behind the `Unreleased` stub.

## 2026-10-24 12:29 EDT — plan 20261024_02 and plan 20261022_03 L3: a smaller gear, and "On Scroll"

The owner (2026-10-24): "smaller gear, Hidden should be On Scroll".
- **Plan 20261024_02, decision 1:** the gear is drawn at about a third of its
  touch target (16% of the side as its wheel's radius, from 25%). The
  target stays 44 pt.
- **Plan 20261022_03 L3** (the band amendment) and plan 20261024_02's Display
  amendment: the middle visibility, *Hidden*, is shown as *On Scroll*. It is
  still `hidden` in `settings.toml`, and `ScrollbarVisibility::Hidden` in
  code, so saved settings read as before.

## 2026-10-24 12:31 EDT — plan 20261024_02 and plan 20261022_03 L3: "None"

The owner (2026-10-24): "change Disabled to None". The first visibility,
*Disabled*, is shown as *None*. It is still `disabled` in `settings.toml`,
and `ScrollbarVisibility::Disabled` in code.

## 2026-10-24 12:33 EDT — plan 20261024_02, decision 3: confirming modals close at once

The owner (2026-10-24): "closing a confirm modal shouldn't have a delay,
no fade out". The settings screen's modal stack uses libcontrols'
`ModalTransition::Timed { open_secs: 0.18, close_secs: 0.0 }`, in place of
the default 0.12 s close. A zero close is instant, so the choice acts in
the same frame and nothing fades. Opening keeps its 0.18 s.

## 2026-10-24 12:35 EDT — plan 20261024_02, decision 3: a Notifications section; notifications asked for only from it

Agreed with the owner (2026-10-24): "we should explain notifications and
only request them when requested in settings, same as termux/connectbot";
then "Approve the plan". This adds to decision 3, between Display and
Battery:
- **Notifications**
  - **Explanation,** in dim text: "handlr shows one ongoing notification
    while it's connected, which lets it stay connected in the background.
    It can also alert you when an agent finishes or needs an answer.
    Notifications stay off until you turn them on here."
  - ***Notifications*** (a toggle, off):
    - turning it on asks for `POST_NOTIFICATIONS` (Android 13+), through
      libmorph's `request_permissions`;
    - when Android won't ask again, it opens the app's notification
      settings instead;
    - the toggle shows the real permission each time the screen opens or
      is returned to.
- **Never asked for otherwise:** not at launch, and not on connecting.
- **Without the permission,** the foreground service still runs (libmorph
  0.4.0), and its notification shows only in the running-apps list
  (libmobile's `NotificationHidden`). The section then reads "Off: handlr
  stays connected, but its notification shows only in the running apps
  list."
- **Until libmorph 0.4.0 is wired in,** the section is built, and turning
  the toggle on says it needs an app update, as the battery rows do.

Tests:
- the section's place and text;
- the toggle stays off until turned on;
- turning it on asks once, through the battery-like stub;
- a refusal that won't be asked again opens the settings page;
- the toggle follows the real permission;
- nothing is asked at start.

## 2026-10-24 12:38 EDT — plan 20261024_02, decision 3: Background and Notifications as two sections, with ⓘ modals for long explanations

Agreed with the owner (2026-10-24): "the two are separate concerns,
background and notify when in background"; "add i tooltip/modals for long
explanations"; then "yes". This replaces the entry "a Notifications
section; notifications asked for only from it" (2026-10-24 12:5x EDT, the
entry just before this one).
- **Background** (between Display and Battery):
  - ***Stay connected in the background*** (a toggle, on): on, the app starts
    the foreground service when it connects, so the link survives in the
    background. Off, the link sleeps when the app is left, as now. It asks
    for no permission.
  - Its ⓘ: "Android only lets handlr stay connected in the background while
    it shows an ongoing notification. Without notification permission, that
    notification appears only in the running apps list."
- **Notifications** (after Background):
  - ***Alert when an agent needs you*** (a toggle, off): on, while the app is
    in the background, a question, an approval or a finished turn posts a
    notification, and tapping it opens that agent.
    - Turning it on is the only place `POST_NOTIFICATIONS` is asked for.
      When Android won't ask again, it opens the app's notification
      settings.
    - The toggle shows the real permission.
    - With *Stay connected in the background* off, the row is dim, with the
      line "Needs Stay connected in the background".
  - Its ⓘ: "handlr can tell you when an agent finishes or needs an answer
    while you're in another app."
- **ⓘ modals:**
  - **The button:** a row with a long explanation has a small ⓘ after its
    label, a 44 pt target in the dim colour.
  - **The modal:** a tap opens libcontrols' `ModalDialog` with the row's name
    as its title, the explanation, and *OK*. It closes at once, as the
    confirm modals do. A tap outside, or Back, also closes it.
  - **Which rows:** the two above, *Battery optimization*, *Samsung battery
    (app sleeping)* and *General agent*, each with a sentence or two.
  - **Inline:** short state lines stay under their rows ("Exempt", "Off:
    notification shows only in running apps").
- **Never asked otherwise,** and until libmorph 0.4.0 is wired in both
  sections say they need an app update when used.

Tests:
- both sections' place and defaults;
- the alerts row is dim with the background off;
- turning alerts on asks once;
- a refusal that won't be asked again opens the settings;
- each ⓘ opens its modal, which closes at once;
- nothing is asked at start.

## 2026-10-24 12:38 EDT — correction: the time of the entry replaced above

The previous entry names the entry it replaces as "2026-10-24 12:5x EDT".
That entry is dated 2026-10-24 12:35 EDT.

## 2026-10-24 12:46 EDT — plan 20261022_03 L3 (full text and summary cards) and plan 20261023_05: the protocol and daemon half, departures

Built in libhandlr_proto 0.3.9 (`Entry.text`, `CardBody.points` and `spoken`,
`Slot.bookend`, `CARD_POINTS_MAX`), the handlr-companion literals, the daemon
commit, and the companion literals commit after it.
1. **`Entry.text` is sent only when `summary` was cut,** so the same text
   isn't sent twice. The client shows `text` when present, else `summary`.
2. **Links and sizes come from the full text.** `Entry.links` are found in
   the full text. A text slot's `lines` and `chars` count the full text.
   Tool slots still count the summary, since they fold.
3. **The points request is in ADDITIONAL_PROMPTS.md.** The `spoken_summary`
   prompt gains `- [#] max_points: 6` and a `points` block, appended after
   the user message. The answer is split at a `POINTS:` line, list marks
   are stripped, and each point is at most 160 characters. A prompts file
   without `points` means no card summaries.
4. **Which Finished cards are new:**
   - on a tailer's first look, only the newest; after that, each card not
     yet sent;
   - answers are kept by serial and card-text hash (up to 1,024), and are
     not retried after an error.
5. **Before a subscription names the pane,** the prompt uses the agent's own
   name ("claude").
6. **Integration tests set `[summary] cards = false`** unless a test gives a
   mock, so tests never call real Gemini.
7. **Not yet done:**
   - read aloud reusing `CardBody.spoken`, and `speech.summarize` checking
     the card store (both go to the client half);
   - the summary prompt's temperature, still not applied, as before.
8. **The bookend mark** is `Slot.bookend: bool`, so `Slot` stays `Copy`.

## 2026-10-24 12:59 EDT — plan 20261024_02: the settings copy in SETTINGS_COPY.md, the ⓘ texts, and the notification's content

Agreed with the owner (2026-10-24): "remember this is an app for coders and
power users but even we have trouble understanding Google/Samsung
policies"; "libmorph should allow naming the notification/app title, and 1
line sent to ip is meaningless, estimated data usage would be better or the
state of the current session (finish summary or what it's working on)";
"move that copy to a buildtime libtypedmd file SETTINGS_COPY.md"; then "yes
plan and impl" (plan 20261024_04, the Markdown renderer). This adds to the
entry "Background and Notifications as two sections, with ⓘ modals".
1. **The copy is in `apps/companion/SETTINGS_COPY.md`,** compiled in at build
   time (plan 20261024_04, decision 6).
2. **The ⓘ texts are precise about the mechanisms.** Each says what Android
   or One UI does, what the setting changes, and what is still not
   guaranteed:
   - *Stay connected in the background:* the cached-app freezer, Doze, App
     Standby; a `specialUse` foreground service; its required notification
     appears only in the running apps list without `POST_NOTIFICATIONS`; Doze
     can still delay the network unless battery optimization is off.
   - *Alert when an agent needs you:* a question, an approval or a finished
     turn, while handlr isn't on screen; needs `POST_NOTIFICATIONS`, asked
     only here; relies on the background connection.
   - *Battery optimization:* lifts App Standby's limits and some of Doze's
     network limits; doesn't stop Doze; doesn't override "Restricted".
   - *Samsung battery (app sleeping):* One UI's sleeping and deep sleeping
     apps and its freezer; add handlr to "Never sleeping apps"; Android's
     exemption doesn't cover this list.
   - *General agent:* what you ask Gemini (voice turns, later the chat
     button); the daemon's spoken summaries are separate, under `[summary]`.
3. **The notification shows the session's state** (with libmorph 0.4.0's
   `NotificationContent` title and text; the app's label stays "handlr"):
   - **Title:** the agent that matters most (needs you, then working, then
     done), or a count with several ("handlr · 1 needs you, 2 working").
   - **Text:** that agent's state, as the jump line gives it: `question: …`,
     `approval: …`, `working: <newest line>`, `done: <summary>`.
   - **All idle:** "All agents idle · <n> MB this session", the client's
     count of bytes over the link since connecting.
   - **Updates:** at most every 2 s, by `update_text`, only on a change.
   - **Tap:** opens that agent.

## 2026-10-24 13:02 EDT — plan 20261024_02 and plan 20261024_04: vendor additions to the settings copy

Agreed with the owner (2026-10-24): "settings should have additional vendor
subkey, appended to copy not replacing, needs libmorph support, does not
expose private identifiers just vendor, make and model"; then "no, just
vendor, I was describing the libmorph feature".
- **Vendor subkeys** in `SETTINGS_COPY.md`: `# [Copy] <key>.<vendor>` (vendor
  lower-cased: `samsung`, `google`, `xiaomi`). On a phone of that vendor,
  its text is appended after the base key's text, never replacing it.
  There are no model subkeys.
- **`build.rs`** checks that each subkey has a base key, and that its slots
  are declared, as for base keys.
- **Where the vendor comes from:** libmorph's device identity (asked for in
  a libmorph plan): vendor, make and model, from `Build.MANUFACTURER`,
  `Build.BRAND` and `Build.MODEL` on Android, `None` elsewhere. It has no
  serial, Android ID, IMEI or other private identifier. Only the vendor is
  used here. Until libmorph has it, only base text shows.

## 2026-10-24 13:04 EDT — the vendor's source is named `brand_info`

The previous entry, "vendor additions to the settings copy", calls the
vendor's source libmorph's "device identity". The owner named it
(2026-10-24): "brand_info", which libmorph's own plan
(`docs/design/20261024_02_DEVICE_INFO_PLAN.md`, in libmorph, awaiting
approval) is to use: `ctx.brand_info()`, with `vendor`, `make` and `model`.
handlr uses its `vendor`.

## 2026-10-24 13:08 EDT — the vendor's source is `build_info`, not `brand_info`

This corrects the previous entry's name. The owner settled the name in
libmorph's session: `ctx.build_info() -> Option<&BuildInfo { vendor, make,
model }>`, the same name as libmobile's `build_info()`, recorded in libmorph
`0a725b4`. It ships in libmorph 0.4.0, after libmobile 0.2.4. It returns
`None` on desktop, the web and scripted runs. handlr uses its `vendor`.

## 2026-10-24 13:20 EDT — plan 20261024_04, steps 1–2, with plan 20261024_02's copy and sections: departures

Built in the handlr-markdown commit and the companion commit after it.
1. **`fill` is a method:** each key is a `CopyText` const with `label()`,
   `body(vendor)` and `fill(vendor, &[(name, value)])`. There is no free
   `fill(key, …)`.
2. **Finding a missing key:** `build.rs` scans `src/**/*.rs` for `copy::<KEY>`.
   A missing one would also fail to compile.
3. **Slots** are declared as the object's members (`- [abc] daemon: …`).
   Every object has a `text`. A row with an ⓘ uses `title` as its label and
   `text` as the explanation.
4. **`build.rs` has no `rerun-if-changed`,** so libmorph's Gradle step still
   reruns on any package change, as before.
5. **The Background section also shows on the desktop.** Battery stays
   Android-only.
6. **"Needs an app update"** shows whenever a toggle that needs the service
   is on and the service isn't there, not only on a tap.
7. **All notes are drawn as Markdown,** the battery notes and confirm texts
   included. `auth.toml` and **Restricted** are formatted, and the unused
   "Vendor battery" label is gone.
8. **Link taps** are logged and kept as `tapped_link()` until `open_link` is
   wired in. `SettingsHit` is unchanged.
9. **The Markdown faces keep copies of the atlas metrics,** without pixel
   data, because libcontrols' `DrawContext` carries one face.
10. **`apps/Cargo.lock`** also moves libhandlr_proto to 0.3.9, catching it
    up.

Open:
- The two extra atlases must fit the app's atlas set on Android, or start-up
  panics. This needs a check on the S23.
- The "refusal that won't be asked again" test waits for the real
  permission (libmorph 0.4.0, released, to be wired in).

## 2026-10-24 13:29 EDT — plan 20261022_03 L3 (full text, folding, summary cards, cache, memory, refresh, labels) and plan 20261024_02's Clear cache: the client and app half, departures

Built in the handlr-companion commit, the apps lock commit and the companion
commit before this entry. When merging with plan 20261024_04's settings
work, the *Clear cache (n MB)* label moved into `SETTINGS_COPY.md`
(`clear_cache`, with a `size` slot).
1. **No spoken part without `CardBody.spoken`.** The expanded card shows only
   the full context, with no spoken section and no replay. The app doesn't
   have the speech rules, and what read aloud would say depends on the time
   it is said.
2. **Memory grades always keep live cards,** because the pin and the jump
   line need them. Under pressure, the newest-8 exception doesn't apply.
3. **Resolved summary cards** read `-- Finished: <short form>` in dim text.
   Expanded in the scrollback, they repeat the reply just above.
4. **Folded sizes:** a folded diff shows 4 lines (12 before), and a folded
   error 4. Notices join their whole text onto one line, then wrap.
5. **Labels and the jump line:** a bookend label's summary is the first line
   of `spoken`, and its time comes from the reply on the card's line. The
   jump line's "done" also prefers `spoken`'s first line.
6. **Reopened views** open at the bottom, following, so the jump line doesn't
   show on reopening.
7. **Refresh** fetches from the window's top to the outline's end, and gives
   up after 5 s (`REFRESH_WAIT`).
8. **A settings modal test** taps the modal's centre (kept from the settings
   work), not a row under it.

Open:
- paging closed views to redb (plan 20261023_09, a TODO in the client);
- wiring libmorph's memory events (0.4.0 has `on_low_memory`; the trim
  levels come in 0.4.1);
- the daemon's `speech.summarize` checking the card store.

## 2026-10-24 13:57 EDT — the companion's launcher icon: the magenta circuit dog

The owner (2026-10-24): "use icon from ~/Projects/Handlr … adapt to magenta
color scheme from the blue (just bump the hue number) and encode at proper
Android resolutions". The icon and its generator are in `apps/companion/icon/`.
- **The art:** the source's left icon, with its line art turned +110° in hue
  (blue to magenta, green to violet). The background is `#0F0F26`.
- **The sizes:** legacy and round mipmaps at 48–192 px; adaptive foregrounds
  at 108–432 px, the dog inside the 66 dp circle, with a colour background
  and a monochrome layer; masters at 512 and 1024.
- **Regenerating:** `make_icon.py` rebuilds it all from the source image, which
  stays in `~/Projects/Handlr`.
- **Getting it into the APK:** libmobile's `Project::extra_script` applies
  `icon/android/icon.gradle`. It adds the res directory, and points the debug
  and release builds at a manifest overlay setting `android:icon` and
  `android:roundIcon`. aapt shows `mipmap/ic_launcher`.
- **Temporary:** this hook goes away once libmobile and libmorph's icon plans
  (Drafts, awaiting the owner) give `Project::icon(..)`.

## 2026-10-24 14:10 EDT — libmorph 0.4.1 wired into the companion (plan 20261024_02's Background, Notifications, battery and copy; plan 20261022_03 L3's memory grades): departures

Built in the handlr-companion, handlr-audio and companion commits before this
entry, merged with the launcher icon (`build.rs` keeps both the icon's
`extra_script` and the service and battery declarations).
1. **handlr-audio's libmorph pin moved too,** with the root lock (libdevice 0.1.1
   to 0.1.2), since the graph holds one libmorph.
2. **The notification follows the client from threads.** libmorph's Android
   runner draws no frames while suspended, so the notification's content and
   the alerts are driven from the `net` thread (`Companion::spawn_tapped`) and
   a `notify` thread.
3. **A title change uses `update`,** because `update_text` can't change the
   title.
4. **"Done"** means a turn went from working to stopped since handlr was last
   on screen. Returning to the screen clears it. The approval line uses the
   asked confirmation's command, because `Event::Confirm` names no pane.
5. **"n MB this session"** counts each message's plaintext from the client's
   start, across reconnections. The handshake's `hello` isn't counted.
6. **A `notifications_asked` marker** in the data directory tells "never asked"
   from "won't ask again". Delete data removes it.
7. **Background reports ready on every host,** so "needs an app update" no
   longer shows on the desktop. Starting the service there gives
   `Unsupported`, which is logged.
8. **On the desktop, only `settings.toml` moves** to the `APP_ID` data directory:
   once, never over an existing file, and never in a scripted run.
9. **The vendor battery row** shows on Samsung, because libmorph can't tell
   whether a vendor page exists. It falls back to the app's page.
10. **`NotificationHidden` is only logged:** there is no inline note for it yet.

Not possible yet in libmorph 0.4.1:
- **A separate alert notification:** alerts are detected and logged, not
  posted.
- **Knowing what was tapped:** a tap on the notification opens the app but
  doesn't say what was tapped, so it can't open that agent.

## 2026-10-24 14:14 EDT — plan 20261024_02: settings dialogs are opaque, opening and closing at once

The owner (2026-10-24), from the S23: "no transparency on settings dialogs".
The screenshot showed the ⓘ dialog's panel half see-through, with the rows
behind it showing. The modal stack faded in over 0.18 s, and the screen stopped
redrawing before the fade finished, so the panel stayed part-transparent. The
settings screen's modal stack now uses `ModalTransition::Instant`: dialogs open
and close in one frame at full opacity, in place of the 2026-10-24 entry
"confirming modals close at once" (Timed, 0.18 s in, 0 out).

## 2026-10-24 14:20 EDT — plan 20261024_02: an Enable button for battery optimization, status lines from `enabled_text`/`disabled_text`, and `build.rs` reruns on copy changes

The owner (2026-10-24): "battery options needing OS approval should have small
0.8% text Enable button on right"; "what does Optimized mean?"; "it should have
a short summary of the feature status taken from enabled_text key for the
option".
- **Enable.** While handlr isn't exempt, the *Battery optimization* row has a
  small *Enable* button on its right in place of the "›", its text at 0.8 of
  the label's size (`LinkRow.action`, `ACTION_PART` = `ControlPart::Value`,
  since `Custom` parts are a Markdown note's links). The button asks the system
  (Android's Allow dialog). The row itself opens the list of every app's
  setting. Once exempt, the row shows the "›" again.
- **Status lines.** A copy object may carry `enabled_text` and `disabled_text`,
  a row's one-line status by whether its feature is on (`CopyText::status`).
  The battery row's subtitle uses them in place of the bare "Exempt" and
  "Optimized" (which meant Android's battery optimization is on for handlr,
  so Doze and App Standby may defer its network):
  - "Exempt: Android doesn't hold back handlr's network";
  - "Optimized: Android may hold back handlr's network when idle".

  The `battery_exempt` and `battery_optimized` keys are gone.
- **A fix to the copy build:** libmorph-build prints `rerun-if-changed` lines of
  its own, which turned off cargo's "rerun on any change in the package" rule
  that the plan 20261024_04 departures entry relied on (item 4). Edits to
  `SETTINGS_COPY.md` were not picked up. `build.rs` now names the copy file,
  `src`, `build`, `build.rs`, `Cargo.toml` and `icon`.

## 2026-10-24 14:21 EDT — plan 20261024_04 and the settings copy: each ⓘ's link is "Android documentation", drawn last

The owner (2026-10-24): "the links should say Android documentation and be the
last thing in the info text". A copy object may carry `link` (a Markdown link
as a literal block). `CopyText::body` draws it last, after the text and any
vendor addition. The ⓘ texts' links to developer.android.com moved out of their
texts into `link`, labelled "Android documentation". The Samsung page's link to
dontkillmyapp.com/samsung, which isn't Android's documentation, keeps its own
label and is drawn last too.

## 2026-10-24 14:35 EDT — the launcher icon through libmorph 0.4.2's `Icon`

libmorph 0.4.2 and libmorph-build 0.2.5 (rev `4e5b75c`, libmobile 0.2.5) make
the launcher icon from a master PNG. `build.rs` now calls
`.icon(Icon::png("icon/artwork-1024.png").background_color([0x0F, 0x0F, 0x26])
.monochrome_png("icon/artwork-1024.png"))`. The `extra_script` hook and its
manifest overlay (`icon/android/icon.gradle`, `AndroidManifest.xml`) are gone.
- **The master:** `make_icon.py` writes `artwork-1024.png`, the dog centred on a
  transparent 1024 square with its farthest pixel just inside the square's
  inscribed circle. libmobile scales the square into the centre 66 dp, where
  launchers keep only that circle.
- **Its other outputs** (the masters, the contact sheet, and `icon/android/res`)
  stay as `docs/APP_ICON.md`'s previews. The build no longer reads the res
  directory.
- **The pins:** handlr-audio's libmorph pin moved too, with the root lock.
- **Checked:** aapt shows `icon='res/mipmap-anydpi-v26/ic_launcher.xml'`.

## 2026-10-24 14:36 EDT — plan 20261024_02: a row's status wraps short of its button

The owner (2026-10-24): "wide labels should wrap in libcontrols, or explicit \n
if easier, optimized info should be capped in width to leave room for button".
A link row's subtitle now breaks at each "\n", and wraps at spaces to the room
left of its "›" or its *Enable* button (widths estimated at 0.5 em a character,
since layout runs without a font). The row grows by a line for each line past
the first. The *Battery optimization* row's status ("Optimized: Android may
hold back handlr's network when idle") wraps beside *Enable*, no longer
running under it. This is done in the app's own row control, not in
libcontrols. Toggle rows' labels don't wrap yet.

## 2026-10-24 14:38 EDT — plan 20261024_02: the battery row sees the exemption after *Enable*

The owner (2026-10-24): "can the app detect optimized status? it should check
after pressing enable upon returning to app (but I got a system dialog
requesting exemption)". On the S23, the Allow worked (`dumpsys deviceidle
whitelist` lists `dev.tmzt.handlr`), but the row didn't change. Android's
Allow dialog sits over the app without taking its surface, so libmorph's
`on_resume`, where the state was read again, never fires. After *Enable*, the
settings screen now reads the state once a second for up to 60 s, and stops
once handlr is exempt (`BATTERY_WATCH`, `BATTERY_LOOK`). `on_resume` still reads it
after the battery pages, which do take the surface.

## 2026-10-24 14:44 EDT — plan 20261024_02: nothing on the right of a granted row, and the "›" drawn as strokes

The owner (2026-10-24): "remove the Enable button if enabled, don't leave the
empty box". The "box" was the "›" (U+203A), which the settings face doesn't
have. A link row's chevron is now drawn as two short strokes. Once handlr is
exempt, the *Battery optimization* row shows nothing on its right: no button
and no chevron (`LinkRow.chevron`). The row still opens the list of every
app's setting.
