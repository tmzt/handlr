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
