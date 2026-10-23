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
