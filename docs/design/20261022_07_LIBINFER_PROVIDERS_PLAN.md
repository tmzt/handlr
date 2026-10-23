---
title: libinfer providers, the routing engine and the Gemini model setting
date: 2026-10-22
status: Draft
covers:
  - "Splitting `crates/libinfer` into a provider-neutral core (`libinfer`)
    and a Gemini provider (`libinfer-gemini`), and dropping libgenai"
  - "The pieces of plan 20260831_02 that were never built:
    `RoutingReasoningEngine`, and the `[gemini] model` setting"
  - "A later step: provider-neutral tool declarations, with a formatter for
    each provider's syntax"
  - "Not covered: an OpenAI-compatible provider (`libinfer-openai`, for
    llama.cpp or Qwen), which will be its own plan"
sources:
  - "Owner's request (2026-10-22): write the plan with the missing parts,
    leave out OpenAI support, and copy what is needed from libgenai into
    `libinfer-gemini` for now, with the plan to switch to a generic version
    later"
  - "Earlier owner decisions (2026-10-22): no protocol logic piled into
    libinfer; a workspace structure for libinfer, to be broken out when
    finished; `system_prompt()` and `tools()` to become generic with a
    formatter per syntax in the long term"
  - "handlr at `b698503`: `crates/libinfer/src/lib.rs` (730 lines);
    `src/reasoning.rs` (`declare_gemini_tools`); `src/server/companion.rs`
    (`TurnService::engine`)"
  - "libgenai at `57fb65d` (the pinned revision): `src/lib.rs`, 240 lines"
  - "CHANGES.md, 2026-10-22 17:11 EDT (the interim libinfer fix) and the
    entry recording that `[gemini] model` is unused"
related:
  - docs/design/20260831_02_REASONING_ENGINE_PLAN.md (the traits and engines;
    this plan builds its missing B.3)
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (the turn service
    that uses the engine)
---

# libinfer providers, the routing engine and the Gemini model setting

## 1. Goal

`libinfer` holds handlr's reasoning traits (`ReasoningEngine`,
`ToolExecutor`, `TurnContributor`), the JSONL session store, the mock
engines, and the Gemini engine, all in one file that depends on libgenai.

This plan does four things:

- It splits the Gemini engine into its own crate, so the core has no
  provider in it.
- It drops libgenai.
- It builds the two pieces of plan 20260831_02 that never landed: the
  routing engine, and choosing the Gemini model.
- It plans the move to provider-neutral tool declarations, which a second
  provider will need.

## 2. What libgenai gives today

At the pinned revision (`57fb65d`), libgenai is 240 lines:
- re-exports of `gemini-rust` 1.7 types (`Content`, `FunctionCall`,
  `FunctionCallingMode`, `FunctionDeclaration`, `Gemini`, `Message`,
  `Model`, `Role`, `Tool`);
- `GeminiEmbedder`;
- `GeminiAgent`, with its `AgentResponse`.

libinfer uses **only the re-exports**. Since the interim fix, its
`GeminiReasoningEngine` builds requests on `Gemini` itself and never calls
`GeminiAgent` or `GeminiEmbedder`. So "copying libgenai's code" means
depending on `gemini-rust` directly. Nothing else from libgenai is needed.
If a later step needs `GeminiEmbedder` or anything else from libgenai, it
is copied into `libinfer-gemini` at that point, with its licence notice
(MIT OR Apache-2.0, the same as libgenai).

## 3. Decisions

1. **Layout.** Both crates are members of the root workspace, under one
   directory, so they can be moved out together later:

   | Path | Package | Contents |
   |---|---|---|
   | `crates/libinfer/libinfer/` | `libinfer` | traits, `Message`, `SessionManager`, `SessionBuilder`, `TurnContributor`, mocks, `RoutingReasoningEngine` |
   | `crates/libinfer/libinfer-gemini/` | `libinfer-gemini` | `GeminiReasoningEngine` and its builder, and the `gemini-rust` types it takes |

   - The core keeps the package name `libinfer`, so handlr's `use libinfer::…`
     for traits and mocks does not change.
   - The core depends on no provider, network client or async runtime
     beyond `async-trait`.
   - `libinfer-gemini` depends on `libinfer`, `gemini-rust` 1.7 and
     `async-compat` 0.2. It does not depend on libgenai.

2. **What moves where.**
   - **`EngineType` and `ReasoningEngineBuilder`** are removed from the core,
     because they name Gemini. Each provider builds its own engine:
     `libinfer_gemini::GeminiReasoningEngine::builder()` with
     `api_key(Option<String>)`, `model(…)`, `system_prompt(…)`,
     `tools(Vec<Tool>)` and `max_turns(…)`, then `build() ->
     Result<GeminiReasoningEngine, String>`.
     - With no key, it reads `GEMINI_API_KEY`, as now.
     - Errors mention `GEMINI_API_KEY`, which handlr's spoken errors match
       on.
   - **The mock engine and mock tool executor** stay in the core, where tests
     of any provider-independent code use them.
   - **handlr's `src/reasoning.rs`** re-exports the Gemini engine and types
     from `libinfer-gemini`. `declare_gemini_tools` stays in handlr until
     step 4.
   - **`src/server/companion.rs`** builds the engine with the Gemini builder.

3. **The Gemini model.**
   - `[gemini] model` in `config.toml` is passed to the builder.
   - The default is `gemini-2.5-flash`, the value the config already
     documents.
   - The builder takes the model as a string. A name `gemini-rust` does not
     know is passed through as `Model::Custom` (or that crate's equivalent),
     so a new model needs no code change.
   - An empty value means the default.

4. **`RoutingReasoningEngine`** (plan 20260831_02, B.3). It lives in the
   core and wraps any `Arc<dyn ReasoningEngine>`. It is configured with
   routes:

   ```rust
   pub struct Route { pub phrases: Vec<String>, pub reply: String }
   pub struct RoutingReasoningEngine { /* inner, routes */ }
   impl RoutingReasoningEngine {
       pub fn new(inner: Arc<dyn ReasoningEngine>, routes: Vec<Route>) -> Self;
   }
   ```

   - **Matching.** A turn whose latest user message, normalised, equals one
     of a route's phrases is answered with the route's `reply`. The inner
     engine is not called, no tool runs, and the reply is appended to the
     session as a model message. Every other turn goes to the inner engine
     unchanged.
   - **Normalising** means lowercasing, trimming, folding runs of
     whitespace to one space, and dropping `.`, `,`, `!` and `?`.
   - **Whole utterances only.** A phrase inside a longer sentence does not
     match. This is the same rule as the companion's "approve" (plan
     20261022_04).
   - **No tool calls.** Routes never call tools, so they can never send
     anything to an agent. Stopping an agent stays with `turn.interrupt`
     and the interrupt key (plan 20261022_02, 18:03 EDT).
   - **handlr's routes.** handlr wraps its Gemini engine with one route:
     "never mind", "cancel", "forget it" and "nothing" all reply "Okay."
     This saves a Gemini round trip on the most common non-request.
     Approving and denying are not routes, because the companion handles
     them itself (plan 20261022_02, 12:06 EDT).

5. **Later: provider-neutral tools.** Today handlr declares its tools as
   `gemini-rust` `FunctionDeclaration`s, and the engine is handed
   Gemini-specific types. When a second provider arrives:
   - **The core gains** `pub struct ToolSpec { name: String, description:
     String, parameters: serde_json::Value /* JSON Schema */ }`.
   - **Each provider** formats `ToolSpec`s and the system prompt into its own
     syntax: Gemini function declarations, OpenAI `tools`, or a local
     model's chat template.
   - **handlr** declares its tools once, as `ToolSpec`s.
   - **Until then,** `libinfer-gemini` takes `gemini-rust` types directly.

## 4. Steps

1. **`libinfer-gemini`.** Create `crates/libinfer/libinfer-gemini` and move
   `GeminiReasoningEngine` into it from `libinfer`, unchanged except for:
   - the builder of decision 2;
   - depending on `gemini-rust` directly.

   handlr switches its Gemini imports and `TurnService::engine` to the new
   crate. The crate is added to the workspace in its first commit (Rule
   3.1). handlr's change is its own commit.

   **Done when:**
   - fmt, clippy and test pass for `-p libinfer-gemini` and `-p handlr`;
   - `cargo tree -p libinfer-gemini` shows no libgenai;
   - a daemon turn without a key still answers "the server has no Gemini
     API key".

2. **The provider-neutral core.**
   - Remove the Gemini engine, `EngineType`, `ReasoningEngineBuilder`, the
     re-exports and the libgenai dependency from `libinfer`.
   - Move the crate to `crates/libinfer/libinfer` (the workspace manifest
     change goes in the same commit).
   - Update the comment in `Cargo.toml` that names "plan 05".

   **Done when:**
   - fmt, clippy and test pass for `-p libinfer`;
   - `cargo tree -p libinfer -e normal` lists no `gemini-rust`, `reqwest`
     or libgenai;
   - libgenai is gone from `Cargo.lock`.

3. **The model setting and the routing engine.** Decisions 3 and 4:
   - `with_model` in `libinfer-gemini`;
   - `RoutingReasoningEngine` and its tests in `libinfer`;
   - handlr passes `[gemini] model` and wraps its engine with its route.

   Tests:
   - each phrase in several spellings ("Never mind.", "  cancel ",
     "NOTHING!") is routed;
   - "never mind the build" and "cancel the build" are not routed;
   - a routed turn calls no tool and leaves the inner mock engine's record
     empty;
   - the reply is appended to the session;
   - the builder passes the configured model, and an unknown model name is
     passed through.

   **Done when:**
   - fmt, clippy and test pass for all three crates;
   - with a Gemini key set by the owner, "never mind" is answered without a
     request to Gemini (the daemon's log shows no Gemini call), and a real
     request still reaches Gemini with the configured model.

4. **Provider-neutral tools** (decision 5). Do this when the OpenAI plan
   starts, or before it if the owner asks:
   - add `ToolSpec` to the core;
   - add a `ToolSpec` → `FunctionDeclaration` formatter in
     `libinfer-gemini`;
   - handlr's `declare_gemini_tools` becomes `declare_tools() ->
     Vec<ToolSpec>`.

   **Done when:**
   - the Gemini engine sends the same declarations as before (a test
     compares the formatted `FunctionDeclaration`s with today's);
   - handlr has no `gemini-rust` type outside its engine construction.

## 5. Relation to other plans

- **Plan 20260831_02.** When step 3 is done, every engine it names exists.
  It can then be marked `Implemented` by an amendment listing this plan's
  commits.
- **CHANGES.md 17:11 EDT.** The interim libinfer fix ends with step 2.
- **The OpenAI-compatible provider** will add `crates/libinfer/libinfer-openai`.
  It starts from step 4's `ToolSpec`.

## 6. Open questions

1. Should handlr's routes be configurable in `config.toml` (for example
   `[[routes]] phrases = […], reply = "…"`), or stay fixed in code until
   there is a need? This plan keeps them in code.
2. Should step 4 come before the OpenAI plan, so that handlr stops naming
   Gemini types sooner? This plan leaves it until that plan starts.
