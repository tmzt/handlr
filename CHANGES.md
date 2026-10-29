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

## 2026-10-24 14:58 EDT — plan 20261024_02: a Voice row in the Sound section

Agreed with the owner (2026-10-24): "does libvoicexn expose supported voices,
can we use arrow label to select in settings?"; then "yes". libvoicexn reports
Phonon's voices on load (`TtsEvent::Loaded { voices }`), and a speech request
can name one. The S23 has 30 or more voices in `phonon-31m-en/voices/`.
- ***Voice*** (Sound section, after *Ding when an agent is done*): libcontrols'
  `ArrowLabel`, stepping through *Default* (Phonon's own) and then the voices in
  alphabetical order, wrapping at both ends. It is disabled until Phonon has
  loaded, and while *All sound* is off.
- **Preview:** stepping says "This is <voice>." in that voice. A further step
  cuts the preview off.
- **Saved** as `voice` in `settings.toml`. A saved voice that is no longer
  installed falls back to *Default*.
- **handlr-audio:** `AudioEvent::TtsLoaded` carries the voice names, and an input
  sets the voice while the app runs. Both are additions.
- **The copy** is in `SETTINGS_COPY.md`. An ⓘ says the voices are Phonon's
  checkpoints, pushed by `scripts/push-model.sh`.

## 2026-10-24 15:06 EDT — the Voice row: departures

Built in the handlr-audio commit and the companion commit before this entry.
1. **`Settings` is no longer `Copy`:** it holds the voice as a `String`. Its
   thread-local is a `RefCell`, and `current()` returns a clone.
2. **handlr-audio adds `TtsLoaded.names`** beside `voices: usize`, rather than
   changing `voices`. New `AudioSession::set_voice` and `voice`. The "last
   `Voice`" in `pump.rs` was the gate's `Input::Voice(bool)`, not a voice.
3. **The preview for *Default*** says "This is Default.", as the spec's wording
   gives.
4. **A preview stops whatever is being said,** a reply read aloud included, and
   not only an earlier preview.
5. **A saved voice that is no longer installed** is shown and spoken as
   *Default*, but stays in `settings.toml` until another one is chosen.
6. **On the desktop,** `--voice` still applies when no voice is saved. The row
   then shows *Default*.
7. **`OptionRow` gains `enabled`:** drawn dim, with its arrows ignored. Its ⓘ
   still works while dim.

## 2026-10-24 15:13 EDT — the Voice row: *Default*'s preview says "This is the default voice."

The owner (2026-10-24), of item 3 in the previous entry: "queue that change for
next release". The *Default* preview reads the new copy key
`voice_preview_default`, "This is the default voice.", instead of filling
`voice_preview` with "Default". It is in the next build, not the one just
installed.

## 2026-10-24 15:19 EDT — the Voice row: long names drawn smaller, and *Default* listed once

The owner (2026-10-24): "adjust font size if too long, and there's two default
labels".
- **Long values:** an option row's value is drawn smaller to fit between its
  arrows, down to 9 pt (estimated at 0.5 em a character). This is for voices
  such as "Captain Silas Blackwater".
- **One *Default*:** Phonon can name its own voice "default". That name is left
  out of the list (case-insensitively), so *Default* appears once, first.
  Not seen on screen: the phone was in another app. The second "Default" is
  taken to be Phonon's own.

## 2026-10-24 15:36 EDT — plan 20261024_05, decision 6: the system bars follow the theme's colours

The owner (2026-10-24), on Android's status and navigation bars: "right, they
should follow our theme background and titlebar colors". This replaces
decision 6's "follow the theme's `dark` flag (light or dark icons), if
libmorph offers a way":
- the status bar's background is the theme's `header`, and the navigation
  bar's is its `bg`;
- each bar's icons are dark or light, whichever contrasts with its colour
  (by relative luminance), and are set again on every theme change.

libmorph is asked to include this with its dark mode plans rather than later.
Until it has the call, handlr builds the colours behind a seam that does
nothing, with a TODO naming that request.

## 2026-10-24 15:37 EDT — plan 20261024_05: a theme's missing roles come from the light or dark default

The owner (2026-10-24): "a specific theme should fall back to light/dark
defaults for missing keys". This replaces, in decision 2 and the amendment's
item 1, "every role is present" for every theme:
- **The defaults:** the first `dark = true` theme in the shipped file
  (*Dark (gray)*) and the first `dark = false` one (*Light (black text)*). These
  two must define every role, and `build.rs` checks that.
- **Every other theme,** shipped or in the user's `themes.toml`, may leave roles
  out. Each missing role takes the value from the default of its own kind, by
  its `dark` flag. A theme without `dark` is an error: the build fails for a
  shipped one, and a user one is skipped and logged.
- **The build still fails** for a value that isn't a colour, an unknown role
  name, or a repeated id.

## 2026-10-24 15:44 EDT — plan 20261022_03 L3: an expanded pinned card stays expanded while scrolling

The owner (2026-10-24): "scrolling with finish card expanded shouldn't collapse
it". The pinned card used to fold to its bar whenever a finger scrolled or the
view was scrolled back (`AgentScreen::compact`). It now folds only when it
isn't expanded. An expanded card (a Finished card's reply, points and spoken
text, or a cut question) stays as it is while the view scrolls behind it, until
it is tapped to fold.

## 2026-10-24 15:55 EDT — plan 20261024_05, steps 1–3: departures

Built in the three companion commits before this entry, rebased over the
pinned-card change.
1. **Three extra roles: `fg`, `question` and `toggle`.** The old `FG` (not `TEXT`),
   the question accent and the toggle's on colour had no exact role.
   `scrollbar` is libcontrols' indicator colour in settings; the agent view's
   thumb stays `fg` at its opacity.
2. **Colours may be `[r, g, b, a]` arrays as well as hex,** because most of
   today's values aren't n/255 and *Dark (gray)* must match them exactly.
3. **`theme()` returns a `Colors` copy,** not a `&'static Theme`, since themes are
   loaded at run time.
4. **The pairing field is themed** (`input`, with a `band` border). Under *Dark
   (gray)* it changes slightly from libcontrols' default.
5. **Not themed:** libcontrols' modal backdrop (black at 0.45) and its brief
   pressed-button colours.
6. **A theme change rebuilds the settings router,** as a new layout does, so
   an open modal closes.
7. **`apps/Cargo.lock` is in step 1's commit** (`toml` with `preserve_order`).
8. ***Appearance*'s ⓘ says System is dark** until the app can read the phone's
   setting. That line goes once libmorph's dark mode is wired in.
9. **The system bars** (CHANGES.md, earlier today) are computed by
   `SystemBars::of`, with icons chosen by WCAG luminance, and handed to
   `apply_system_bars`. It does nothing until libmorph's call is released.
   libmorph wants linear RGBA, so the theme's sRGB values will need converting
   then.

## 2026-10-24 15:56 EDT — plan 20261023_04 (the overview) and plan 20261024_05 (themes): session colours

From the owner (2026-10-24); see plan 20261022_03's L3 amendment of the same
time.
- **The overview** (frozen): each agent's card has a band in its session's
  colour.
- **Themes** (in implementation): new optional roles `session_1` to `session_16`,
  8 hues in a light and a dark variant each, defined in both defaults and
  inherited by any theme that leaves them out.

## 2026-10-24 16:11 EDT — plan 20261022_03 L3: a pull to refresh leaves the pinned card whole

The owner (2026-10-24): "pull to refresh shouldn't collapse pinned cards
either". The pinned card folded to its bar as soon as a finger moved, a pull
at the bottom included. It now folds only while the view is scrolled back or
the scrollbar is held (`AgentScreen::compact`), so a pull past the end, which
keeps the view at the bottom, leaves it whole. An expanded card still never
folds (the entry before).

## 2026-10-24 16:19 EDT — session colours and other sessions' cards (plan 20261022_03 L3): departures

Built in the handlr-companion and companion commits before this entry,
rebased over the pull-to-refresh change.
1. **Turn length:** `agents.list`'s `Turn` has no times. When a used, unviewed
   session stops, the client peeks it (200 entries) and measures from the
   user's last prompt to the last entry. With no prompt among those entries,
   it measures from the oldest, a lower bound.
2. **The list is polled while an agent view is open,** every 15 s, but only
   while another session used within the hour is watched. This departs from
   plan 20261023_04's decision 6, which listed only on the overview.
3. **A ding fix:** a list taken while a view is open doesn't ding that view's
   own agent, which dings its own Finished card.
4. **Finished cards use the overview's turn line** (`Turn::Stopped.line`), not
   the daemon's summary, which isn't sent for a session the phone isn't
   subscribed to.
5. **"Approval"** is a handlr confirmation waiting for that pane, and its text
   is the command.
6. **Opening a session clears its cards.** A swiped question or approval stays
   away until what is asked changes.
7. **In Settings,** the card sits over the bottom of the screen; the content
   doesn't make room for it.
8. **Interactions are keyed by pane.** Another session id in the pane doesn't
   count as used. A shell with no session takes its colour from its pane id.
9. **No version bump** for handlr-companion's new `Event::SessionCards` and
   `Input::DismissCard`, as with earlier additions.
10. **Session colours:** an FNV-1a hash of the session id picks one of 16
    colours, 8 hues each with a light and a dark variant. Each theme tunes its
    own, and band text is black or white by contrast (tested at 3:1 or more).


## 2026-10-25 10:47 EDT — Plan 20261023_06, step 5: tailing and subscriptions for agy sessions

Plan: [Session readers for agy and other agents, behind one agnostic trait](docs/design/20261023_06_SESSION_READERS_PLAN.md)
(`Implemented`). Step 5 requires the S23 companion app to display the agent view
for an agy session.

Departures:
- **`Tailer` and `Source` support record-based sessions alongside line-based ones.**
  `session.subscribe` and `session.content` rely on `Tailer` and `Source` in
  `src/sessions/hub.rs`. `Tailer` originally required `LineReader` and a text
  `LineIndex`. For agy's SQLite database, `AgyReader` exposes `max_idx` and
  `read_range`, allowing `Tailer` and `Source` to feed `Builder` and construct
  the session outline and sections without a `LineIndex`.
- **Change detection for SQLite.** `Tailer::check` checks for database and WAL
  modification timestamps and compares `SELECT MAX(idx) FROM steps` to detect
  new records, broadcasting batches to subscribers as steps occur.

## 2026-10-25 12:20 EDT — Rename project to flankr

Plan: [Rename project to flankr across CLI, protocols, crates and apps](docs/design/20261025_01_RENAME_PROJECT_TO_FLANKR_PLAN.md)
(`In implementation`).

- The project changes name from `handlr` to `flankr`.
- Workspace crates rename to `crates/libflankr_proto`, `crates/flankr-speech`,
  `crates/flankr-markdown`, `crates/flankr-companion`, `crates/flankr-audio`,
  and the root binary target is `flankr`.
- Wire protocols, Noise Link framing prefixes, URL schemes, and pairing codes
  cut over to `flankr.harness`, `flankr.companion`, `server: "flankr"`,
  `flankr.link/1`, `flankr1…`, `flankr://`, and `flankrkey1…`.
- The companion mobile application ID changes to `dev.flankr.companion`, and
  desktop macOS secure storage namespace changes to `flankr-companion`
  (`libmorph.flankr-companion`).
- Default filesystem locations move to `~/.config/flankr/`,
  `~/.local/state/flankr/`, and `~/Library/Application Support/dev.flankr.companion/`.
- Historical design documents, historical entries in `CHANGES.md`/`VERSIONS.md`,
  and past evidence logs remain untouched under their original names.

## 2026-10-25 15:27 EDT — Navy overview theme, saturated session palette and custom panel shader

Plan: [Navy overview screen theme, saturated session palette and custom panel shader](docs/design/20261025_03_NAVY_OVERVIEW_THEME_PLAN.md)
(`In implementation`).

- The companion app adopts `Dark (navy)` as its default theme, placed first in
  `apps/companion/themes.toml` with the palette extracted from the design mockup.
- Session colors `session_1..16` in `Dark (navy)` and dark themes are upgraded
  to saturated, vibrant hues matching the design example, retained as colored
  bars on overview cards and under the agent view header.
- The overview screen adapts to the design mockup: top-left `flankr` title,
  centered metadata subtitle, horizontal header divider, `OVERVIEW · 2-COLUMN FIXED TAB ORDER`
  section label, right-aligned status labels, interactive question choice pills,
  monospace command pill containers, and footer metadata lines.
- A custom WGSL shader (`PanelShader`) is implemented in `apps/companion` to
  render anti-aliased navy background panels with signed distance fields,
  composited beneath `libmsdf`'s overlay pass.

## 2026-10-25 17:53 EDT — Keyboard prompt card styling and shaded input area

Plan: [The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md)
(living section L3).

- `flankr_companion::Dictation` distinguishes `PromptMode`: `Voice` and `Keyboard`.
- The prompt card displays "Type your message" placeholder in keyboard mode instead of "(say your message)".
- In keyboard mode, the input text / placeholder renders inside a shaded input area (`theme().input`), distinct from the card container and buttons.

## 2026-10-25 23:34 EDT — Prompt card text area focus, IME lifecycle, and external keyboard

Plan: [The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md)
(living section L3).

- The prompt card's `TextArea` is registered in `AgentHit::TextArea` on overview and agent view screens.
- Tapping the `TextArea` sets the cursor from the tap position and requests the soft keyboard if not dismissed and external keyboard is inactive.
- Soft keyboard dismissal (`TextInputEvent::Dismissed`) retains the prompt card without clearing it.
- External physical keyboard keystrokes edit the prompt card directly and suppress the soft keyboard.

## 2026-10-25 23:41 EDT — 3-line 12pt text area in keyboard prompt card

Plan: [The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md)
(living section L3).

- The prompt card's `TextArea` is sized for at least 3 lines of text (`input_rows.len().max(3)`).
- The keyboard prompt uses ~12pt font (`12.0 * s` px) for both the placeholder and typed text.

## 2026-10-25 23:55 EDT — Expanding message collapses pinned cards with state memory and user override

Plan: [The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md)
(living section L3).

- Taking a message in an agent session retains the pinned card above the message prompt card rather than removing it.
- Opening a message collapses an expanded pinned card, recording its previous state in `pin_saved_expanded`.
- Tapping to toggle expansion while the message is open records a user override (`pin_overridden`).
- Closing the message restores the previous expansion state if not overridden, or preserves the user override if modified.

## 2026-10-26 00:30 EDT — General agent multi-turn chat as pinned card and overlay, realtime voice with short VAD silence, and centre button on every screen

Plan: [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- Centre button in `ControlRow` placed on every screen (overview and agent view): displays 'X' (`AgentHit::Clear`) when prompt input is active, and Gemini accent button (`AgentHit::GeneralAgent`) when idle.
- Multi-turn general agent chat presented normally as a pinned card above the bottom controls row, expanding into `TurnOverlay` on tap.
- Voice input uses realtime streaming STT with short VAD silence (~1.0s) ending the turn to immediately trigger Gemini's streaming response and TTS playback.
- Multi-line text input supported via the 3-line 12pt `TextArea` prompt card directed to the general agent.

## 2026-10-26 00:52 EDT — Centre button replaces overview tap-to-talk; 2 s hold opens overlay

Plan: [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- Tapping empty background or grid space on the overview no longer triggers voice listening; the centre button alone triggers general agent talk.
- Holding the centre button for 2.0 seconds opens the `TurnOverlay` conversation sheet across screens.

## 2026-10-26 01:14 EDT — Text filter removes symbols with threshold; general agent pinned card & expanded input

Plans:
- [The bulk input and dictation mode](docs/design/20261023_02_BULK_INPUT_PLAN.md).
- [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- `validate_arbitrary_command` and `validate_message` remove unallowed symbols rather than rejecting input outright.
- Single symbols followed by a space (or at the end of the text) do not count toward the symbol rejection threshold.
- Rejection occurs only when threshold-triggering symbols exceed 3.
- General agent responses are rendered as a pinned card above the bottom controls row.
- `TurnOverlay` expanded sheet includes the text input area at the bottom with turns scrollable above it.
- Restored normal message input card display and lifecycle so taking an agent message is not blocked.

## 2026-10-26 01:40 EDT — Text filter applied to general agent and session input

Plans:
- [The bulk input and dictation mode](docs/design/20261023_02_BULK_INPUT_PLAN.md).
- [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- Text filter (stripping unallowed symbols, threshold enforcement for excessive triggering symbols, and length validation) applies uniformly to general agent turns (`turn.start`) as well as session messages (`validate_message`).
- Multi-line support: line breaks (`\n`, `\r\n`) and tabs are permitted and preserved in message and general agent input so multi-line prompt submissions are retained.

## 2026-10-26 11:32 EDT — Tool results with exit code 0 mapped to successful calls, not Broke

Plan: [Session readers for agy and other agents, behind one agnostic trait](docs/design/20261023_06_SESSION_READERS_PLAN.md).

- Tool execution results reporting `Command exited with code 0.` are treated as successful tool calls, matching Claude's behavior. They are never prefixed with `error:` or reported as `CardKind::Broke` ("Something broke").
- agy command output envelopes (`Created At: ... \n Completed At: ... \n Command exited with code 0. \n\n Command output:\n...`) are unwrapped to the command output body (or empty string when no output was produced).
- Non-zero exit codes (`Command exited with code <N>`) are marked with `error: ` and correctly report broken/failed cards.
- agy SQLite status 5 and 3 are recognized as completed steps; running status 2 and status 5 are not misclassified as errors.
- `src/sessions/sections.rs` guards against treating exit code 0 as an error.

## 2026-10-26 11:41 EDT — Centre button active voice recognition pulsing and glyph feedback

Plan: [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- When voice recognition is active for the general agent, the centre button provides continuous dynamic feedback: pulsing / growing-shrinking inner ring or glyph, or alternating with dingbat sparkle symbols.
- Active visual state is displayed whenever recognition is active regardless of whether the prompt/chat card or overlay is visible or collapsed.
- The outer button dimensions remain fixed at `BUTTON_H * s` to prevent layout movement.

## 2026-10-26 12:09 EDT — Centre button idle state in neutral background, activating only when pressed

Plan: [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- The centre button starts in an idle state using the neutral button background (`theme().back`) with the Gemini star `✦` drawn in the accent colour (`theme().accent`).
- The button activates into its accent background (`theme().accent`) with the inner breathing pulse ring and alternating dingbat stars (`✦`, `✧`, `✶`, `✻`) only when pressed / when voice recognition is actively listening for the general agent.
- Deactivating (submitting or clearing) returns the centre button to the neutral idle state.

## 2026-10-26 12:21 EDT — Cross-session General Agent UI and state unified across screens

Plan: [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- General agent conversation turns, partial STT speech, voice recognition listening state, and prompt input text are shared cross-session across all screens (overview and agent views).
- Invalidation on general agent updates (`Event::Conversation`, `Event::Dictation`, `AudioEvent::Partial`) triggers immediate redraw for both the overview and active agent view.
- Layout order fix: overview layout is settled before the active agent screen sets the pinned card (`TurnStrip`) and overlay (`TurnOverlay`) bounds, preventing overview grid coordinates from overwriting agent screen placement.
- `TurnOverlay::draw` uses logical width on agent view matching overview, and touch/drag events inside the sheet invalidate the view for smooth scrolling and dismiss interactions.
- Pinned response card tapping on agent screen opens the conversation overlay sheet with proper hit testing.

## 2026-10-26 12:35 EDT — Immediate voice activation, Gemini pinned card on press, unified pinned card layout, and turn reactivation

Plan: [The general agent as a realtime chat, from its own centre button](docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md).

- Centre button press immediately starts voice recognition, starts dynamic button animation, and displays the Gemini pinned card (`Listening…`) on both Overview and Agent screens on touch down without waiting for release.
- Overview pinned card layout matches Agent screen: Gemini card sits pinned directly above the bottom row / dictation panels, with other sessions' cards and grid cards above it.
- TurnStrip displays `Listening…` immediately when voice recognition is active before partial speech arrives.
- Turn completion, silence submission, and cancellation cleanly reset listening and dictation state, allowing immediate reactivation for subsequent turns when the button is pressed from idle.

## 2026-10-26 12:52 EDT — Plan: Markdown detection in agent output and automatic switch to rich rendering

Plan: [Markdown detection in agent output and automatic switch to rich rendering](docs/design/20261026_01_MARKDOWN_DETECTION_AND_RENDERING_PLAN.md).

- Added draft plan `docs/design/20261026_01_MARKDOWN_DETECTION_AND_RENDERING_PLAN.md` to detect Markdown formatting usage exceeding a percentage threshold in output text (e.g. 15%) and switch from plain monospace terminal row wrapping to rich CommonMark rendering (`flankr-markdown::lay_out`).
- Defined `MarkdownScore` metrics, structural vs inline weighting, false-positive guards against terminal commands/flags, and integration in `apps/companion` agent view and turn overlay.

## 2026-10-26 13:33 EDT — Markdown rendering for assistant summary blocks and replies

Plan: [Markdown detection in agent output and automatic switch to rich rendering](docs/design/20261026_01_MARKDOWN_DETECTION_AND_RENDERING_PLAN.md).

- Following the owner's instruction ("the last summary block should have been rendered in markdown"), assistant reply entries (`SectionKind::Text` / `lay_out_entry`) exceeding the Markdown percentage threshold (default 15%) automatically switch to rich CommonMark layout (`flankr-markdown::lay_out`).
- Prose renders in proportional typography, list items render with bullets and indentation, diff lines in fenced blocks render syntax-highlighted in green and red, code blocks render with shaded background panels in `theme().input`, and Markdown links are registered into interactive link spans.
- Assistant replies below the threshold remain in terminal monospace row layout.













## 2026-10-27 01:27 EDT — plan 20261027_02, steps 1–4: departures

Built in the flankr (step 1), libflankr_proto 0.3.10 (step 3), flankr (step 2),
flankr-companion and companion (step 4) commits before this entry.
1. **Clients are kept per backend, model and key, and per profile** (the
   general agent, or summaries). A Gemini client fixes its tools and system
   prompt when built, so the general agent and the summaries can't share
   one; the two summaries do. A changed key gets a new client.
2. **`server.capabilities`'s params are `{ "llm": LlmInfo }`,** not a bare
   `LlmInfo`, so other capabilities can be added later.
3. **Test stand-ins:** `TurnOptions::engine` and `SpeechOptions::engine` still
   replace the real clients. A use with one counts as available unless its
   own `enabled = false` turns it off.
4. **A backend without `kind`** is an error for that backend alone.
5. **Step 1 also touched server code** (`companion.rs`, `speech.rs`, `mod.rs`) to
   compile with the deprecated sections now optional.
6. **Commit order 1, 3, 2, 4:** step 2 needs the protocol. The flankr crate
   doesn't compile at the protocol commit alone, only once step 2 lands.
7. **An old `[gemini] model`** applies to every use, so the owner's config
   (`gemini-3.8-flash`) runs all three uses on Flash rather than Flash-Lite for
   the summaries. Moving to `[llm]` with no model gets the tier defaults:
   `gemini-3.8-flash` for the general agent, `gemini-3.8-flash-lite` for both
   summaries.

## 2026-10-27 02:05 EDT — plan 20261027_03, steps 1–4: departures

Built in the four companion commits before this entry.
1. **A tap on Gemini's bars while a message is open** opens Gemini expanded
   over the message, not in the state it was left in, because the message
   card has no hidden state to step aside to. Collapsing goes back to the
   bars. Raised with the owner.
2. **More ways to collapse:** a tap on the dimmed area above the sheet, and a
   swipe down on a history already at its top, both kept from `TurnOverlay`.
3. **Message history is cleared** when the pane's `session_id` changes in
   `agents.list`. The app can't see the outline epoch without a client change.
4. **A message counts as sent** when it was Submitting or Confirming and then
   ends, so one refused or denied while Confirming is recorded too.
5. **New dependency:** `libc = "0.2"`, for `localtime_r` to stamp sent messages
   with local time. The Android build compiles with it.
6. **While typing to Gemini,** the centre button is an X and send takes
   record's place, as for a message. It no longer collapses the overlay.
7. **Stacking:** the confirmation card sits nearest the controls, with the
   prompt cards above it. A message still steps aside while a confirmation
   asks.
8. **Voice input:** a voice message shows its last 6 lines (the overview showed
   8). A tap on the overlay's input during a voice message does nothing.
9. **Back,** in order: settings, an expanded card, a dictation, then the agent
   view.
10. **Other sessions' cards** aren't drawn and take no taps while an overlay
    shows.
11. **Smaller changes:**
    - Gemini's card comes back expanded when the general agent is switched
      back on, if it was left that way.
    - The overlay shows its header under the grab bar.
    - Keyboard routing is in `prompt/keyboard.rs`.

## 2026-10-27 11:50 EDT — plan 20261027_04 steps 1–6's departures

Plan: docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md. Steps 3
(`af5cb63`, `861c3ee`), 6 (`bcb0a54`, `a63b4be`, `ea20791`), 1 (`0a03725`),
2 (`f4c1543`), 3b (`7147d27`), 4 (`8b0e2c5`) and 5 (`4571d82`).

1. **Step 1, free text with `/` and `-` is still refused.** The plan routes
   Gemini's free text through `policy::check_message`, whose
   `validate_message` has refused every symbol beyond `. , ' ? : _` since
   `2fa3e43` (`SYMBOL_THRESHOLD = 0`), so step 1's **Done when:** "free text
   with `/` and `-` accepted" does not hold. Requests longer than 256
   characters are accepted; "open src/main.rs" is refused with "Message
   rejected". `validate_message` is unchanged, since the app's typed
   messages use it too; raised with the owner.
2. **Step 1, the confirmation:** free text opens a message confirmation
   (`request_message`), worded as for a typed message; without a handle it
   falls back to "message <first words>". herdr always receives
   `run_command` with `pane_id` and `command`.
3. **Step 1, the context's agent list** said "Allow Arbitrary Safe
   Commands" and forbade `/` and `-`; it now shows "Requests in words:
   <allow_messages>" with the message rules. `PROMPT.md` section 3.2 still
   describes the old rules; only the tool's names were changed there.
4. **Step 2, `Prompt.user` is optional,** since `planning` has none;
   `spoken_summary` checks for its own `user` when loaded.
5. **Step 2, `Use::Planning`** is not in `Use::ALL` (the uses with their own
   backend and model); it is resolved after the general agent, appears in
   the `[llm]` log lines, and `from_name("planning")` finds it.
6. **Step 2, the `read` slot** is in the `files` block, which the 11:22
   amendment split from `system`.
7. **Step 2, the client** is keyed on its target, profile and planning
   profile, so any change to `planning` builds a new one. The `planning`
   prompt is loaded once at start, from `--prompts` when given.
8. **Step 2, refused patterns** also include empty patterns, empty segments
   (`a//b`) and `.` or `..` segments.
9. **Step 2, the `planning` prompt's wording** was written in the build;
   for the owner's review.
10. **Step 3b, `Session.current_agent`** is new, shared by the executor and
    the context; `Session.current` (the last pane addressed, for interrupts)
    is unchanged. `focus_project` also records the pane as addressed. The
    context's "### Current agent" says "None: …" when there is none.
11. **Step 4, refusals:** `.private`, `.git`, `.env` and `.env.*` are refused
    at any depth. The canonical path is checked against the patterns and
    refusals as well, so a link inside the project to `.env` is refused.
    `list_files` lists regular files only. The file tools and
    `agents_prompt` are refused when herdr is remote over SSH, since the
    `cwd` is on another machine.
12. **Step 4, layout:** `src/project.rs` for the file tools,
    `src/llm/planning.rs` for the patterns; `walkdir = "2"` is a direct
    dependency.
13. **Step 5, `flankr prompt`** resolves `[llm]` with the process's key
    environment, so it shows what the daemon would build; without a key it
    prints "planning: off (…)" and the base prompt. `--agent` prints the
    instructions block even with `agents_prompt` off, noting turns don't get
    it. `--use` takes only `general_agent`.
14. **Step 6, *Tab follows focus*** has an ⓘ like the row above it, and is
    dim while the daemon offers no general agent as well as while the user's
    toggle is off. The phone sends the setting as set even while the general
    agent is off; the daemon gives it no effect then.
15. **Step 6, `apps/Cargo.lock`** was still on `libflankr_proto` 0.3.10 after
    `861c3ee`; it is bumped in its own commit, `a63b4be`.

## 2026-10-27 12:03 EDT — messages: only the first word may be a command; paths checked against the project

Plans: docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md (step 1,
decision 1) and docs/design/20261023_02_BULK_INPUT_PLAN.md (decision 6, as
amended). Commit `7d72b82`.

The owner (2026-10-27): the symbol ban "was added to prevent agent commands
from being sent until things were tested, then a specific allow list of
commands. since command is only the first word, that restriction should
probably be applied there"; "except paths should be resolved and checked to
see if they fall outside of the focused repo, at least for now"; "this also
means a heuristic to make sure pathological paths aren't being resolved,
such as limiting the number of .. pairs"; and "it looks like whitespace at
the beginning of the message is ignored and commands are still processed".

1. **`validate_message`** no longer refuses symbols. The first
   non-whitespace character may not be `/`, `!` or `#` (a slash command,
   Claude Code's shell mode, older Claude Code's memory); after it any
   printable text is kept. Control characters and newlines are still
   refused, and the 4000-character limit stands. This changes the check on
   messages typed in the app as well as the general agent's requests, and
   closes the 11:50 entry's item 1: free text with `/` and `-` is accepted.
2. **`check_paths`**, run by `policy::check_message` with the pane's `cwd`:
   each word holding a `/` or starting with `~` (URLs aside) is resolved
   against the agent's project folder and refused outside it. Bounds: 32
   paths per message, 512 characters, 32 parts and 4 `..` per path.
   Resolution is by the text; a path that exists is also resolved on disk.
   Without a known folder, a message holding a path is refused. A slash
   command later in the text (`run /compact later`) reads as a path outside
   the project and is refused.
3. **`validate_general_input`** (what the user says or types to the general
   agent) keeps symbols: it never reaches a pane.
4. **File tools:** a leading `/` is the project's root, and a refused or
   missing path's error says "Paths are relative to the project's root, e.g.
   docs/design/plan.md." Absolute filesystem paths are not accepted.
5. **Context:** the "Current agent" line names the project, not its
   absolute folder, and says file paths are relative to its root. The
   message rules in the context and `PROMPT.md` section 3, item 2, describe
   the first-word rule; `PROMPT.md` was changed only there and in item 3's
   wording.

## 2026-10-27 12:58 EDT — plan 20261027_05 steps 1–4 and 6's departures

Plan: docs/design/20261027_05_PLANNING_FOLLOWUPS_PLAN.md. Steps 2
(`2747243`), 1 (`a43a0bd`), 3 (`0f67677`), 4 (`a0bd4ff`) and 6 (`6c12d9c`).

1. **Step 2:** `LlmConfig.enabled` is `Option<bool>`. Without an `enabled`
   key, `[gemini]` and `[summary]` are merged under `[llm]`; a declared
   `[llm.backends.gemini]` and a use's explicit `enabled` win. A note is
   logged whenever `[llm]` exists without `enabled`. `config.default.toml`'s
   wording follows.
2. **Step 1:** the watcher is started lazily by `Daemon::file_watcher()`;
   `TurnOptions.file_watcher` lets tests stand in a failed one. The
   `revoke_interval` loop (60 s) always runs beside the watcher as the
   fallback; `[llm]` runs no interval while the watcher works. A path whose
   folder doesn't exist yet is watched from the nearest folder that does.
   `notify` 8.2 and `notify-debouncer-mini` 0.7 are new dependencies.
3. **Step 3:** `GIT_LITERAL_PATHSPECS=1`, and the path passed as `./<rel>`,
   so pathspec magic isn't read; a leading `/` is the project root; paths
   with whitespace or control characters are refused; the path goes through
   `check_paths` with a trailing `/`, since a final `.` is otherwise taken as
   punctuation. The record separator comes first (`%x1e%h%x1f…`) so
   `--name-only` paths fall in their record. `count` is clamped to 1..50; no
   commits answers "no commits yet"; a folder outside git returns no commits
   with a note; `src/git.rs` finds `git`.
4. **Step 4:** the marker hashes the instructions as read (up to
   `max_instructions_kb`). The block is re-sent unless the *latest* marker
   for the project matches, so a reverted file is sent again. The block is
   inserted as a `system` message before the user's turn and appended to
   the JSONL history right after `contribute`, so a failed turn still keeps
   it. An `InstructionsCache` per session re-reads only on a project or file
   change, or every turn without a watcher. `PROMPT.md` and
   `ADDITIONAL_PROMPTS.md` don't mention `recent_commits`; its declaration
   does.
5. **Step 6:** the prefix is kept in the broker
   (`Broker::request_message_typed`, a new parameter on `Broker::open`); the
   direct-send path prefixes too, though requests are always confirmed.
6. **The `via flankr:` rule line** (the 12:33 amendment, item 2) is not yet
   added: `~/src/rules` became the canonical home of the rules at 12:57, in
   another session, and its rules say changes are made there first and
   copied out. It waits for that.
7. **Versions:** the flankr crate gains public items (`Daemon::file_watcher`,
   `TurnOptions.file_watcher`, `ToolSet.recent_commits`) and changes
   `LlmConfig.enabled` to `Option<bool>`, but flankr is the daemon
   application with no `VERSIONS.md`; no version change.

## 2026-10-27 13:40 EDT — the confirmed flag's trust, and the plan-question heuristic's tests

Plans: docs/design/20261023_02_BULK_INPUT_PLAN.md (the 2026-10-27 13:25 EDT
amendment; commits `9ce89dd`, `519c8f0`, `a225e07`, `b652196`) and
docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (L3, the 13:21 and 13:24
EDT amendments; `f8b00df`, `2d84726`).

1. **`confirmed` is trusted as far as self-approval goes,** not by
   `can_approve` alone as item 2 of the 13:25 amendment says: from a
   companion it is honoured for a message (whose voice-turn confirmation
   the companion could approve itself) and for a command only when the
   companion may approve. A local companion's tapped `continue` therefore
   still asks; a paired phone's does not. Nothing is trusted that wasn't.
2. **A submit by silence** does not send `confirmed`: only the Submit
   button does. It keeps its own approval of the confirmation, as before.
3. **The plan-question fixture** (`tests/fixtures/sessions/…/-work-plans`)
   now asks "Approve plan 04 and build …", since a plan path alone no
   longer makes a plan question.

## 2026-10-27 14:35 EDT — plan 20261027_06 steps 1–4's departures

Plan: docs/design/20261027_06_SUGGESTION_RULES_PLAN.md. Steps 1
(`4a120c3`), 2 (`0e29874`, tag `libflankr_proto-v0.3.13`, with
`6d5e89a` for the new field's struct literals), 3 (`efcf3b6`) and 4
(`fce074f`, `b28e49e`, `4a6736f`). The merge of the user file over the
shipped one by object works with libtypedmd's AST, as the 14:13 amendment
asks; a user rule offering an undefined answer falls back to the shipped
rule of that name.

1. **Where answers are filled:** `pending::question_for(reply, asker)`
   fills them for `session.peek` and the session hub, with `agent` the
   capitalised agent name (or the hub's shown name) and `project` the
   agent's project (or the cwd's last part). The general agent's context,
   notices and resolved cards keep `question_in` without answers.
2. **`about_plan` and `plan`:** `about_plan` is "the rule `plan`
   matched", and `plan` is `detected_plan_id` whenever it is, so "Shall I
   approve the plan?" now carries "the plan" where it carried none.
3. **Rules in the daemon:** `Sessions.suggestions` (`SuggestionStore`,
   shipped rules only by default, so tests never read a real user file);
   `TurnOptions.suggestions_file` and `suggestions_user_file`; `serve`
   passes `--suggestions` and `~/.config/flankr/SUGGESTIONS.md`. Both are
   watched through `Daemon::file_watcher()`, falling back to a 5 s check;
   a reload re-merges only when a file's text changed, and the hub
   recomputes when the rules change. The hub's use has no test of its own.
4. **Answers reach the card** by the client copying
   `pending_question.answers` onto the live Question card whose text
   matches; `CardBody` is unchanged.
5. **The client:** an answer sending exactly "yes" or "no" is still
   expanded ("Yes, install it on the S23"); `Input::Suggested(Answer)`;
   `Sent::Respond` carries the label; handles are
   `reply-<agent>-<project>-<label>`; every answer is sent confirmed.
6. **The app:** `AgentHit::Answer { index, text, opens }`; `FlowButton.label`
   is a `Cow<'static, str>`; `approve` and `deny` styles are buttons,
   `green`, `link` and `yellow` coloured text, and an unknown style is
   drawn as `link`; a `starts` answer needs the microphone; the record
   button stays after the answers.
7. **The file:** the shipped yes and no answers are labelled "Yes" and
   "No". The parser also refuses a key given twice, `command: true` with
   `starts`, and a rule without `offers`; a `when` or `unless`
   alternative may be a phrase of consecutive words.

## 2026-10-27 21:25 EDT — plan 20261027_07 steps 1-6's departures (terminal mode, branch `main-terminal`)

Plan: docs/design/20261027_07_TERMINAL_MODE_PLAN.md. Steps 2 (`571f6c4`,
tag `libflankr_proto-v0.4.0`, with `1ef8733`), the libherdr read
(`b718b38`, tag `libherdr-v0.3.1`), 1 (`5e36e10`), 4 (`597e815`),
3 and 6's daemon half (`b9fb089`), and 5 and 6's app half (`9d4326b`).
Not yet tried on the S23.

1. **libflankr_term:** the run encoding after the header is, per row,
   `u16 row, u16 run_count`, then per run `u32 fg, u32 bg, u8 flags,
   u16 len` and the UTF-8 bytes; colours are `0` default,
   `0x0100_00NN` palette, `0x02RRGGBB` RGB. `Mirror::retransmit` is
   added (a lost last frame of an unchanged screen is resent on the 2 s
   tick); a diff is sent only from a base fewer than 16 frames old;
   `reset` also clears the sent history; trailing spaces that draw nothing
   are cut, whatever their foreground; `decode` also refuses `seq == 0`
   and a diff whose base is not older. Characters count one column each.
   The crate depends on `serde` only, not `libflankr_proto`.
2. **libflankr_proto:** `TerminalOpenParams` names the pane `pane_id`
   (alias `session`), as `agents.read` does; `send_kind` takes a typed
   `Kind`; `terminal` is appended to the companion `HelloResult` only,
   not the harness one; the arm returns `expires_in_s`.
3. **libherdr 0.3.1:** `revision` and `truncated` are not surfaced, since
   the read's result is returned as raw JSON.
4. **The daemon:** `terminal.*` is served on link companion connections
   only (a companion on the daemon socket is `NOT_PERMITTED`, a harness
   client `METHOD_NOT_FOUND`); herdr over SSH gives `terminal: None` and
   `HERDR_UNAVAILABLE`, tested on constructed transports, not end to end.
   herdr's `pane.scroll_changed` arrives as `HerdrEvent::Other` and is
   picked up by the watcher, which subscribes per terminal pane. The read
   is always 1000 lines of the visible frame; the size hints are accepted
   and unused. A terminal frame from the phone closes the link. Frames
   share the connection's unbounded queue (no replace-latest yet).
5. **Keys (step 6):** herdr takes one key per token, so the syntax is
   space-separated tokens: a named key (`enter`, `escape`/`esc`, `tab`,
   `backspace`, `space`, the arrows), `ctrl+<a-z>`, or text sent as one
   key per character. Bad keys and batches over 64 bytes are
   `INVALID_PARAMS`; signals without `terminal_signals` and every gate
   are `NOT_PERMITTED`. `InputRevoked` is never sent: `[terminal]` and
   `[[agents]]` are read only at start, and a revoked device's link is
   closed. The notice kind `terminal_input` is a daemon constant, not in
   `companion::notice_kinds`. `policy::reach` is a new helper sharing the
   send path's check.
6. **The client:** the ack is sent automatically when a frame is shown,
   not by the app after each draw (step 5's wording); the screen is read
   with `Terminals::screen` (a clone) or `with_screen`; the arm's time is
   `status().armed_for`; errors arrive as `Event::Error`. The tests are
   in `tests/terminal.rs`, not `cross.rs`.
7. **The app:** the terminal and list glyphs are drawn from lines and
   rounded rects, not added to libmsdf; `FocusedPrompt` is an enum
   (`Card` or `Terminal`); the first frame lands at the fit-width size
   (at least 7 pt) at the bottom rows, at the left edge when wider. Every
   typed character is its own token, so a typed word is never read as a
   named key. libmorph's `Key` has no arrows, so arrows are not sent yet.

## 2026-10-27 22:13 EDT — the terminal view's drag, tap and one-line Finished cards (plan 20261022_03 L3; departures from plan 20261027_07)

Plan: docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md, the L3 amendment on
the terminal view, which departs from frozen plan
docs/design/20261027_07_TERMINAL_MODE_PLAN.md decisions 6 and 7 on the
owner's instruction. `libflankr_proto` 0.4.1 (`bf70776`, tag
`libflankr_proto-v0.4.1`), the daemon (`b785e53`), the client
(`09e017e`) and the app (`d36e8c5`).

1. **`terminal.scroll`** sets herdr's `pane.scroll` offset through
   libherdr's `HerdrRequest::Custom`, so libherdr is unchanged. The offset
   starts from the snapshot's `scroll` when given, else 0; `lines` is
   clamped to ±1000; an accepted scroll extends the arm as keys do and
   wakes the poller for a frame within the 8/s cap. No notice per scroll.
2. **The client** sends one scroll at a time and sums the rest; it now
   extends its own arm on each accepted key batch or scroll, as the daemon
   does (before, the phone dropped the arm after 60 s while typing went
   on). `Input::TerminalScroll` is a new variant of the public `Input`;
   `flankr-companion` keeps 0.1.0.
3. **The app:** the 1.5 s hold is gone; a tap arms and raises the keyboard
   once the arm lands (given up after 5 s). Fit-to-width moved from a
   one-finger to a two-finger double tap, since a tap now acts at once.
   With input allowed, one finger no longer pans vertically (two fingers
   do). The scroll offset shows as "N lines back". Only this session's
   pinned Finished card needed folding; other sessions' were one line
   already.
4. **Config:** the owner's config now has `[terminal] input = true` and
   `terminal_input = true` for claude, omp and agy; listing `[[agents]]`
   replaces the built-in list, so all three are written out.

## 2026-10-28 00:39 EDT — plan 20261027_08 steps 1-9's departures (libherdr_term, branch `main-terminal`)

Plan: docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md. Steps 1
(`756450b`, `5f9ae10`), 2-4 (`3c4293c`), 5 (`4366e5d`, tag
`libflankr_term-v0.1.1`), 6 (`cb4c0ff`, tag `libflankr_proto-v0.4.2`,
with `7e0a933` and `1bbc896`), 7-8 (`7ce1315`, `61f93a3`) and 9
(`ee040e9`, `3d5aeeb`); `7cd7836` reads herdr's socket from
`HERDR_SOCKET_PATH`. Step 3's check was run against a separate herdr
0.9.5 server (the release binary, config and state under
`~/.cache/flankr-herdr-test`): welcome at generation 1, `pane.focus` ok,
the pane at 80x24 with its cursor. The owner's own herdr is 0.8.0, before
generation 1, and is refused as incompatible (`no_welcome`), so the poll
serves it.

1. **Typed characters are key events** (the owner: "ime keyboard input
   should be translated to keys"): every character, space included, goes
   as `Key` press and release of `Char(c)`, so herdr encodes it for the
   pane's keyboard mode; decision 8's `TextCommit` is not produced.
2. **One attachment for the daemon** (the 23:32 EDT amendment, over
   decision 4 and step 7's wording). It returns to attached frames when it
   reconnects, and the poll serves a terminal whose pane the attached
   surface does not show; those frames carry a hidden, stale cursor. A
   desktop change counts only once herdr has answered flankr's requests
   and drawn a matching surface. A successful `terminal.input_arm` is a
   touch that takes the tab back, since a keyboard-mode tap sends no input.
   `pane.focus` is still sent within herdr's active tab when
   `tab_follows_focus` is off. `server.capabilities` is not pushed when
   the attachment comes or goes. A herdr without `client_shell.surface.set`
   takes the tab for a moment at daemon start, before the daemon detaches.
3. **libherdr_term:** `AttachOptions` has `active` and
   `handshake_timeout`; `set_active`, `requested`, `supports_surface_set`,
   `Event::{Snapshot, ActiveAnswer, ServerError}` and `DesktopChange`
   are added; the surface is clamped in the library; mouse converts by
   `TryFrom` (the open `Unknown` kind has no herdr event); key tokens
   become several events (named keys are press and release). The
   extractor lives in `build/extract.rs`, force-added since `.gitignore`
   has `build/`; its failing-build test runs it as a library. The fake
   herdr server is `libherdr_term::fake` behind `fake-server`.
   `HERDR_PROTOCOL_VERSION` comes from the extracted constant.
4. **The link protocol:** `TerminalMouseKind` has struct variants
   (`Down { button }`) for flat JSON and an open `Unknown`; the button is
   `MouseButton`; `TerminalInfo.mouse` and `modifiers` are left out when
   unset. `terminal.resize` on a polled terminal is answered `{}` and
   ignored. Scroll offsets come from the first later surface whose offset
   changed, else the latest within 500 ms.
5. **The app:** the fit uses the layout's existing keyboard inset. In a
   TUI that reports the mouse, a horizontal drag is a mouse drag and does
   not pan; a click on an unarmed terminal waits for the arm. herdr's
   cursor `shape` is DECSCUSR: 3-4 underline, 5-6 bar, else block.

## 2026-10-28 01:00 EDT — shell panes, terminal only (plan 20261022_03 L3)

Plan: docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md, the L3 amendment
on shell panes. The daemon already listed shell panes; the app's overview
filtered them out, and no longer does. The built-in `shell` entry also
sets `interrupt_key = ""`, so an interrupt never sends esc into a shell,
and the global `allow_arbitrary_commands` override (`--allow-arbitrary-commands`)
no longer turns arbitrary commands on for `shell`, since typed text would
run as a command line; the shell's own entry can still allow it. In a
shell's view the keyboard glyph shows only while typing is armed; the
general agent's centre button stays.

## 2026-10-28 01:15 EDT — typing into a terminal is not spoken (plan 20261027_07, decision 6)

Plan: docs/design/20261027_07_TERMINAL_MODE_PLAN.md, decision 6 ("Every
input batch is logged as a confirm-style notice"). The owner, on hearing
the app read each batch aloud: "why is it speaking the keys?". The daemon
now sends one `terminal_input` notice per arm window for keys ("Typing
into <name>'s terminal."), as it already did for the mouse, and the
companion client logs a `terminal_input` notice and never says it. Each
batch is still logged in the daemon, without the keys.

## 2026-10-28 10:33 EDT — no "Typing into" pill over the armed terminal (plan 20261027_07, decision 6)

Plan: docs/design/20261027_07_TERMINAL_MODE_PLAN.md, decision 6 (armed
input "shown as a coloured frame with 'Typing into <agent>'"). The app had
drawn that label as a pill with the arm's seconds left. The owner: "what's
with the countdown? that wasn't asked for", then "just remove the pill,
that was missed in reviewing plan 07". The armed terminal now shows the
coloured frame only, and the app no longer redraws each second for the
countdown.

## 2026-10-28 11:02 EDT — the keys row, no arm timer or frame (plan 20261022_03 L3; plan 20261027_07, decision 6)

Plan: docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md, the L3 amendment on
the keys row, which replaces frozen plan
docs/design/20261027_07_TERMINAL_MODE_PLAN.md decision 6's 60 s arm window
on the owner's instruction. libherdr_term 0.2.0 (`b2bf0e2`, tag
`libherdr_term-v0.2.0`), the daemon (`184e678`), the client (`0aa0883`)
and the app (`90b94d8`).

1. **The arm** is a flag held until the terminal is dropped;
   `terminal.input_arm` answers `expires_in_s: 0`. `libflankr_proto`'s
   `TERMINAL_INPUT_ARM_SECS` (60) is now unused, and its doc and
   `expires_in_s`'s are out of date; the proto crate is unchanged. One
   `terminal_input` notice per arm.
2. **Keys:** herdr's `parse_key_combo` spelling. Accepted: `ctrl+<a-z>`,
   `alt+<printable>`, `ctrl+alt+<a-z>` (`alt+ctrl+` normalised), and
   `ctrl+`/`alt+` before the named keys. Still refused: `alt+space`,
   `ctrl+<non-letter>`, other modifiers, other named keys. A latched
   modifier on a key the daemon would refuse is dropped and the key sent
   plain.
3. **The row** is 44 pt high, seven equal columns, arrows drawn as strokes.
   It sits directly on the keyboard and the bottom bar moves up by its
   height, so the terminal shrinks by exactly that much and the fit follows.

## 2026-10-28 13:38 EDT — plan 20261028_01 steps 1-5's departures (several remotes)

Plan: docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md. Steps 1 (`a9cfbcc`,
tag `libflankr_proto-v0.4.3`; `90b20d1` for `flankr pair`), 2-4
(`e93f76e`, tag `flankr-companion-v0.2.0`; `d964025`) and 5 (the commit
before this one). Not yet tried on the S23.

1. **Identity:** `link::fingerprint` returns the raw hash, as decision 2
   names it; `remote_id` and `fingerprint_text` give the strings. The
   confirmation code hashes `flankr.pair.confirm/1` and the key and takes
   the first 20 bits modulo 1,000,000.
2. **Pairing:** with no active remote, pairing also goes through the probe
   (one extra handshake; `Paired` arrives before `Connected`); any
   answered `hello` counts as accepted, since a completed `IK` handshake
   means the daemon lists the key; a probe that gives up reports
   `PairFailed`.
3. **Events and inputs:** `Event::RemoteUpdated` is new (the "Already
   paired" note); `Input::Unpair` and `Event::Unpaired` stay, meaning
   "remove the active remote" and "the last remote is gone".
   `CompanionConfig` gains `data_dir` and `pairing_window`;
   `ServerRecord.signing_key` is a real field.
4. **Tests** of step 4 use the in-test link server, its authorized keys
   standing in for `auth.toml`.
5. **No `flankr` command removes a paired device;** the offline copy names
   the `[[client]]` entry in `~/.config/flankr/auth.toml`, which the
   daemon re-reads.
6. **The app:** the + and ⓘ are a small `RemotesTitle` control; the
   pairing state and the Delete data order are in `src/remotes.rs`
   (testable on the host); Cancel replaces the field on the pairing screen
   in both modes; Delete data removes the active remote last; copy was
   added beyond the plan's keys (removing, paired, the pairing screen's
   confirmation code).
7. **`apps/Cargo.lock`** took `libflankr_proto` 0.4.3 with a few
   Windows-only and `tempfile` entries moved by cargo.

## 2026-10-28 15:12 EDT — plan 20261028_01, the six-word pairing step and the review's changes: departures

Plan: docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md, the amendments from
13:59 to 14:44 EDT. Commits: `5568c2c`, `1bffe09` (the review's item 4),
`a8fabb6` (libflankr_proto 0.5.0), `b248bbf` (the daemon), `1699bd2`
(flankr-companion 0.3.0), `15a5bf1` (the app). Not yet tried on the S23.

1. **Protocol:** `PairRefusal::Malformed` added beside wrong offer,
   expired and busy; `LinkError::Pairing` carries message errors; the
   pairing result is one binary message on the NK session;
   `accept_incoming` lets one listener take both IK and NK. NK does not
   prove the phone holds its key's private half; the words bind the key,
   and the phone's first IK connection proves it. A payload without an
   offer is still written as version 2.
2. **Daemon:** the new local methods and notifications (`link.pair_code`,
   `link.pair_answer`, `link.pair_request`, `link.pair_ended`,
   `timeout_secs` and `expires_in_secs` on `link.pair_begin`) live in
   `src/server/link/pair_mode.rs`, to move into libflankr_proto later;
   proto's doc on `LINK_PAIR_BEGIN` ("nothing is held pending") is out of
   date. A second `link.pair_begin` replaces the running run (the old one
   is told `replaced`, a held phone `Refused`). `link.pair_add` (no
   words) stays on the local socket for tools; `flankr pair` no longer
   uses it. After the fallback's yes, `flankr pair` exits without waiting
   for the device's first connection. Anything a held phone sends is taken
   as a cancel.
3. **Phone:** `Event::PairingCode` became `Event::PairingWords`, with
   `replace` in `DeviceCodeFallback`. A daemon that closes NK without a
   reply is treated as unreachable and gets the device-code fallback (it
   cannot be told from an older daemon); an immediate refusal does not fall
   back. A held request gives up 10 s (`RESULT_GRACE`) after the offer's
   time. A new offer for the daemon already being paired keeps the key and
   asks again. With an offer, the fallback still probes for the full 10
   minutes.
4. **Review fixes:** a listed remote's new addresses count as reached if
   any address in the link answers (within 30 s, `CHECK_TIMEOUT`);
   `keys.rs` still chmods its key files after creating them (only
   `state.redb` was in scope).

## 2026-10-28 17:20 EDT — plan 20261028_02 (QR scanning in the pairing window): departures

Plan: docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md. `2768957` (tag
`flankr-audio-v0.2.0`) and `f2467f0`. The APK builds and its merged
manifest has CAMERA with the camera `uses-feature`s not required; not yet
tried on the S23.

1. The scripted runs are lib unit tests (`apps/companion/src/pairing_scan_tests.rs`),
   not `tests/pairing_scan.rs`: `--camera` and the probe are `cfg(test)`.
2. The scanner is generic over the scan id and reads libmorph's events
   through a `Cam` enum, since libmorph's `ScanId` and `QrCode` cannot be
   built outside libmorph.
3. The "unknown version" unit case uses a link with a bad checksum.
4. "→" is drawn as ">" in the pairing window (its Roboto face has no "→");
   the copy file is unchanged.
5. The camera lines' source line breaks are joined into one paragraph.
6. In the add mode the placeholder shows `remote_add`'s text under Tap to
   scan, with `pairing_camera_why` under the area;
   `pairing_scan_unreadable` stays until the next code or the scan's end;
   after `PairFailed` in the camera window, `pairing_retry` replaces the
   app's own "pairing failed" note; the tests skip audio devices.
7. The "pairing…" state between a scan and the six words is covered only
   by a unit test (the local test daemon answers in the same frame).

## 2026-10-28 17:38 EDT — plan 20261028_02: the placeholder's copy cut to one line

Plan: docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md (decision 21's copy).
The owner, on the S23: "the copy in the tap to scan is too long, it just
needs to tell the user to scan the new QR code to add an additional
remote", and "it already says Pair another remote". `remote_add`'s text is
now "Scan the QR code shown by flankr pair.", drawn under Tap to scan in
both the add mode and the first run; the idle window shows no status line
(`pairing_camera_why` is no longer drawn; the key stays in the copy file).

## 2026-10-28 18:03 EDT — plan 20261028_02: pairing feedback, the words in a confirm modal, Details

Plan: docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md. The owner, after
trying it: "it should give more feedback after the scan succeeded, same when
pairing is otherwise completed. currently it just closes and looks like it
failed"; "if we're using the six words to confirm the connection,
shouldn't we show them in a confirmation modal?"; "add the Details button
to show the full fingerprint and hostname"; "it should be confirmed on the
Mac only"; "monospace is available via a different atlas, but flankr pair
should be in bold, such as using markdown". Commit `f4651b4`.

1. An accepted scan shows "Code read" and "Connecting to <host>…" with a
   check glyph and Cancel until the words arrive.
2. The six words are in a confirm modal ("Confirm on your Mac": the words,
   the line to type yes on the Mac, Details, Cancel); the phone has no
   match button; a tap off the card does nothing; Back cancels.
3. Details shows "<server> at <host:port>", the six fingerprint words and
   the 64-hex BLAKE2s fingerprint in the mono face. The event carries no
   key, so the app computes the hex from the link it read and shows it
   only when the link's fingerprint words match the event's (TODO: add the
   key or hex to `Event::PairingWords`).
4. Pairing completes on "Paired with <name>" (or "Already paired with
   <name>", with "Its addresses were updated." when the first address
   changed) with Details and Done; it stays until Done.
5. Failures stay in the window with the client's reason and Tap to scan
   again; Cancel says "Pairing cancelled: nothing was added."
6. Pairing copy that names the command writes **flankr pair** and is drawn
   with the app's Markdown view; values inserted are escaped.
7. Test isolation: test builds point the daemon socket at a path that
   does not exist and give each scripted run its own key directory, since
   the Mac's running daemon answered the tests; `data_dir` in tests is
   still the host's.

## 2026-10-28 18:28 EDT — the settings follow-up: remote details, standard rows, the copy folder

Plan: docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (the living
section's 2026-10-28 entries). flankr-companion 0.4.0 (tag
`flankr-companion-v0.4.0`) and the app commit after it. The app's copy is
now in `apps/companion/copy/` (`SETTINGS_COPY.md`, `PAIRING_COPY.md`),
all files read by `build.rs` into one generated module that fails the build
on a key defined twice, a missing key or a bad slot.

1. Cancelling an add-mode pairing no longer leaves a note under Remotes
   (the note is gone); the first-run window still says it.
2. Remove in the details modal closes the details and opens the X's
   confirm modal; a draft nickname is not saved on Remove. The confirm
   modal stays open on Remove ("Removing <name>…"), closing when the daemon
   answers, else showing "removed on this phone only" with OK.
3. `Confirm::new` gained `action_closes`; `Confirm::notice` is new.
4. A nickname is trimmed, its control characters dropped, at most 40
   characters; empty means the default. Closing the details by Close, Back
   or a tap outside saves a changed nickname.
5. The default-name placeholder for a remote that has a nickname is the
   bare base hostname (no conflict port).
6. Tests take a temporary `--data` directory; the old settings migration is
   compiled out of test builds. The soft-keyboard path for the nickname has
   no host or device test yet.
7. `Event::PairingWords.fingerprint_hex` replaces the app's workaround of
   recomputing the hex from the scanned link.

## 2026-10-28 19:10 EDT — plan 20261028_03, steps 1-4: departures

Plan: docs/design/20261028_03_SESSION_DRAWER_PLAN.md. Steps 1 (`bc17150`,
tag `libflankr_proto-v0.5.1`), 2 (`55357a7`), 3-4 (flankr-companion 0.5.0,
tag `flankr-companion-v0.5.0`, and the app's mechanical commit after it).

1. **The summary** is listed only while the latest Finished card is at the
   session's newest reply, and only for sessions a tailer has read (an
   agent view opened them); a session never opened shows none yet.
2. **A missing or unreadable subagent meta file** counts as not finished,
   so the 60 s write rule alone decides.
3. **`Input::ToRemote { remote, input }`** is added, beyond the plan, so the
   app can answer a background remote's approval or question.
4. **`leave_foreground`** drops a prompt aimed at one of the old remote's
   panes; a general-agent turn in progress on the old daemon keeps running,
   its later events arriving as that remote's background events.
5. **`replay()`** always gives the agents, pending panes, focus, cards,
   dictation and cache, even empty, so the app clears the old remote's.
6. **Offline links' states are ticked** every tick; requests while offline
   are logged at debug and dropped.
7. **Not yet done:** `Remotes::display_names` does not add the
   fingerprint group for two remotes with the same host and port (the
   18:48 EDT amendment's item 5); the app does not yet handle
   `Event::Background` or the switch's replay (a wildcard arm drops them
   until steps 6-8).

## 2026-10-28 19:47 EDT — plan 20261028_03, steps 5-8: departures

Plan: docs/design/20261028_03_SESSION_DRAWER_PLAN.md. The app commit after
the keys commit (`52ae8ca`). Not yet tried on the S23.

1. Copy keys `drawer_session_approval` and `drawer_session_idle_now`
   (the plain names were taken in SETTINGS_COPY.md); `drawer_not_paired_here`,
   `drawer_menu` and `drawer_add_remote` are defined but unused.
2. A 2 s press on a card opens Settings with the remote's details over it,
   and Settings closes when the details close.
3. On a switch, a waiting confirmation card of the old remote is kept,
   labelled with its name and answerable through `ToRemote` (the 18:51
   EDT amendment), not dropped as section 3.7 item 28 has it.
4. The drawer theme is scaled to the app's pixels; the sheet is opaque;
   the layers are created on the first visible frame and dropped on
   suspend.
5. The spinner is drawn over the rows after the router renders (router
   controls cannot hold the terminal face).
6. Not done: the S23 checks; the 3 s refresh against a live daemon; a
   background remote's spoken notices are not played aloud.

## 2026-10-28 20:12 EDT — Plan 20261028_04 (plugins), steps 1-4's departures

Small departures needed to make the plan work, none changing its
interfaces' intent:

1. Step 1: libflankr_proto also adds `PLUGIN_NS_MAX_BYTES` (128),
   `PLUGIN_METHOD_MAX_BYTES` (64), `is_plugin_ns()` and
   `is_plugin_method()`.
2. Step 2: `PluginCtx` errors also use -32100 (project unavailable), -32102
   (command failed) and -32103 (read refused) beside the plan's -32101; the
   host passes `METHOD_NOT_FOUND` and `CANCELLED` through and sends other
   codes outside -32199..-32100 as `INTERNAL_ERROR`.
3. Step 2: the host reads `[llm.uses.planning] read` from the raw config,
   so the rule holds with `[llm]` off.
4. Step 2: a plugin's task starts only once its `plugin.call` answer is
   queued, so no notification or result can arrive before it.
5. Step 2: `PluginInfo.reason` is computed only for "Reading files is
   off: …"; per-connection reasons such as no open project are not.
6. Step 2: `plugin.set_enabled` follows a symlinked config to its target and
   creates a missing one at 0600 with its folders; a config that does not
   parse is not written.
7. Step 2: `config.default.toml` leaves out the `source` line (tpt stays
   undocumented until it is built) and keeps `# commands = false`.
8. Step 2: `HerdrToolExecutor::current_folder` now calls
   `project::check_local` and `project::agent_folder`; its messages and
   their order are unchanged.
9. Steps 2 and 3 are one commit (`08c56e6`): the built-in registry lists the
   planning plugin, so neither compiles alone.
10. Step 3: a file cut at 64 KiB always carries `error: "cut at 64 KiB"`, as
    amendments may follow the cut; -32110 is returned when no plan file is
    listed, since a missing folder and an empty one look the same; list
    items carry `readme.row` and `show` carries `extra`, as in tpt; the
    front-matter reader skips blank lines and lets plain scalars continue on
    indented lines; a plugin method named like `plugin.call` is a plugin-local
    name, not refused.
11. Step 4 (libcontrols 0.10.3, plan 20261028_06): the agreed variant of 3.6
    item 3 adds a one-time `Router::attach_popup_menu(id, menu)` before
    `PopupMenuState::open`; its theme has `shadow` with `shadow_offset`, an
    optional `heading_color`, and further size and colour fields
    (`border_thickness`, `item_padding`, separator size, `text_color`,
    `check_*`, `highlight`, `font_size`, `offset`, `margin`); and
    `PopupMenuOptions::dismiss` chooses which dismissals apply.

## 2026-10-28 20:43 EDT — Plan 20261028_04 (plugins), steps 5-7's departures

Small departures needed to make the plan work:

1. Step 5: `Event`'s `Eq` is written by hand, as libflankr_proto's
   `ErrorObject` is `PartialEq` only.
2. Step 5: `Event::Plugins` is also given after a toggle is answered or
   refused (a refusal is `Event::Error`, then the unchanged list) and last
   in `State::replay`; `hello` gives it before the first `Agents`.
3. Step 5: the client refuses a call at once when offline
   (`UNAVAILABLE`), for a bad ns or method (`INVALID_PARAMS`) or over
   256 KiB (`PLUGIN_TOO_LARGE`); waiting calls get `CANCELLED` on a
   disconnect, unmatched results are dropped, and plugin inputs for a
   remote not in use are refused (`NOT_PERMITTED` for a call,
   `Event::Error` for a toggle) through `client::plugin_refusal`.
4. Step 6: `DevicePlugin` adds `height(view, w, s)`, `draw` takes the
   scale, views are passed as `&ViewKind`, `tap` is in the view's own
   (scrolled) coordinates, `PluginOut::call` returns the id and `notify`
   sends a call with no answer.
5. Step 6: the plugin modal is drawn by the app in the settings style
   (`plugins/modal.rs`, scrolled with `controls::scroll::Scroll`), not as a
   libcontrols `ModalDialog`, since the device half draws its content;
   cards are drawn over grid slots reserved by `CardGrid::set_extra`, a
   hold of 2 s is measured at release, and Unpin is in the shared
   PopupMenu (`app/unpin/<index>`).
6. Step 6: `PluginInfo` says nothing of `can_approve`, so the toggle can't
   be dimmed in advance on a device that may not approve; the refusal is
   the notice.
7. Step 6: *Needs a newer flankr* only when the daemon's version is below
   the half's range; above it, a hash version or no half is *Needs a newer
   app*.
8. Step 6: all plugin, menu and planning copy is in `SETTINGS_COPY.md`,
   as a new file would break the test that lists the copy files.
9. Step 7: the menu's Up/Down/Enter keys are not wired from the app's key
   handling (Back and Escape close it); the settings ⋮ is always dim, as
   the plan gives Settings no items; the ⋮ is three drawn dots, as the
   terminal face has no U+22EE.
10. Step 7: the libcontrols pin moves from 0.9.1 to 0.10.3 across 0.10.0's
    Button change (`"XX - Name"` labels drawn whole); no app change was
    needed.
11. Known flake, not from this work: flankr-companion's
    `tests/link.rs` `removing_the_active_remote_moves_to_the_most_recently_paired`
    fails about 1 run in 6 on `main-terminal` as well ("no remotes list
    where b online within 5 s"), from flankr-companion 0.5.0.

## 2026-10-28 22:03 EDT — Planned: title-bar icons (plan 20261028_04, its 2026-10-28 22:03 EDT amendment)

New planned work on the owner's instruction: title-bar icons registered
by the app and plugins (the agent menu ⊙, the planning plugin's ☑,
others, Settings), at most four before the ⋮, which is always last;
past four, Settings and then the last plugin icons move into the ⋮ menu.
`PluginMenuItem.icon` (libflankr_proto patch), `opens: "menu"`, and the
planning plugin 0.1.1's popup of plans. The subagents plan's ⊙ is the
first of these icons (its 2026-10-28 22:03 EDT amendment).

## 2026-10-28 22:31 EDT — Plan 20261028_04 (plugins), the 22:03 EDT amendment's departures

The owner approved building the title-bar icons ("approve and build").
Small departures needed to make it work:

1. `DevicePlugin` also gains `menu_opened(item, out)` and
   `menu_chosen(item, id, out)` beside the planned `menu(item)`, so a half
   can send `plans.list` and act on a choice; its ids are prefixed
   `app/popup/<ns>/<item>/` in the shared menu.
2. `PopupItem` has one label and cannot dim a second word: a plan's status
   follows its title after an en space, the title cut with … at 36
   characters.
3. The overview's gear is now the title bar's Settings icon: the overview's
   own gear target and `OverviewHit::Settings` are gone; the gear is drawn
   in the text colour, not the muted one, and answers while a prompt
   overlay is open. The ⋮ is at the overview title's right edge, dim when
   its menu is empty (Settings in the bar and no plugin items).
4. A plugin icon moved into the ⋮ with `opens: "menu"` opens its popup
   anchored at the ⋮; a half that gives no popup items opens its modal, and
   an unknown `opens` is opened as a modal.
5. The agent view's name and status lines are cut with … short of the
   first title icon.
6. The daemon refuses icon names other than 1-32 of a-z, 0-9 and `_`,
   starting with a letter.
7. The agent menu ⊙ is wired as a glyph and a hook
   (`agent_menu_registered`, false) but not registered: the subagents plan
   is not built.
8. flankr-companion needed no change and is not bumped.
9. Version numbers: libflankr_proto takes 0.5.3 here, so plan 20261028_05
   (binary transfers, Draft) takes the next free numbers when built
   (libflankr_proto 0.5.4, flankr-companion 0.5.2).
