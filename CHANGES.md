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
