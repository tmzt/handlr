---
title: Pluggable ReasoningEngine and ToolExecutor traits
date: 2026-08-31
status: In implementation
covers: the ReasoningEngine and ToolExecutor traits and their concrete engines
sources:
  - PLAN.md as committed in ecaaa09 (2026-08-31 04:27 -0400)
related:
  - docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md
  - docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md
---

# Pluggable ReasoningEngine and ToolExecutor traits

Split from `handlr` Architecture & Implementation Plan. This document outlines the detailed design and architectural phase for `handlr`, the voice-native command line interface and LLM tool-calling bridge for the `herdr` terminal multiplexer.

---

## 2. Pluggable `ReasoningEngine` Trait Design

To allow clean swapping and layering (e.g., mock engine for testing, local offline models, or complex multi-agent orchestrators), the reasoning layer is decoupled using the `ReasoningEngine` trait.

### A. Trait Definition

```rust
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait ReasoningEngine: Send + Sync {
    /// Accepts a natural language transcript, executes any required tool calls
    /// via the provided executor, and returns a natural language voice response.
    async fn process_transcript(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with arguments, returning the output as JSON.
    async fn execute_tool(&self, name: &str, args: &Value) -> Result<Value, String>;
}
```

### B. Concrete Implementations

1.  **`GeminiReasoningEngine` (Default):** Wraps `libgenai::GeminiAgent`. It injects the system prompt from [`AGENTS.md`](file:///Users/tmeade/src/handlr/AGENTS.md), declares the tool schema (`list_panes`, `inspect_pane`, etc.), and processes results via Gemini's function execution flow.
2.  **`MockReasoningEngine` (Testing):** Bypasses LLM network requests. Simulates tool execution outputs or returns deterministic responses to facilitate rapid test automation.
3.  **`RoutingReasoningEngine` (Layered/Fallback):** Routes conversational phrases locally (e.g. "stop", "cancel") while forwarding complex orchestration queries to Gemini.

---

## Implementation Phases

*   **Phase 1 (Abstractions & Traits):** Implement `ReasoningEngine` and `ToolExecutor` traits. Build mock adapters.


## Amendment — 2026-10-22 10:47 EDT: retroactive conversion from PLAN.md

This document was split out of the single `PLAN.md` (commit `ecaaa09`,
2026-08-31) when the repository adopted the libcontrols critical rules. The
body above is the original text, unchanged apart from the split; it predates
Rule 1.2 and has no **Done when:** criteria. The date is the original one.

Implementation of the traits and engines (Phase 1) started on 2026-08-31 in `a53c2a2`, so the
status is set to `In implementation` and the document is now immutable.

## Amendment — 2026-10-22 10:53 EDT: implementation record

Implementation records only; the body above is not changed by this amendment.

**Phase 1 (traits and mocks).** Traits, `GeminiReasoningEngine` and
`MockReasoningEngine` first landed in `src/reasoning.rs` in `a53c2a2`. They
were then moved to the new `libinfer` crate: traits and JSONL
`SessionManager` in `2b85d0a`, Gemini engine in `1d3dd24`, mocks in
`53efd11`, `EngineType` and `ReasoningEngineBuilder` in `89e11e6`.
Credentials resolve through libgenai's service-account/OAuth handling
(`febf29c` to `e3465a1`).

**Departures from the body:**

- The traits live in `crates/libinfer`, re-exported by `src/reasoning.rs`,
  not in handlr itself.
- Tool calling as described (`list_panes`, `inspect_pane`, …) was replaced by
  `HerdrContextContributor`, which injects the herdr snapshot and the recent
  50 lines of active, working or focused panes into each turn (`5af8931`,
  `379776e`, `27ed8a0`). Declared tools are now `focus`, `run_command` and
  `watch_pane`.
- The system prompt comes from `PROMPT.md` (`3d0ef99`), not `AGENTS.md` (B.1).
- `RoutingReasoningEngine` (B.3) has not been built.

**Learned.** Reading panes through tool calls cost a round trip per turn;
putting the pane state in the turn context answers most questions in one
request. Gemini returns opaque `thought_signature`s that must be kept and
replayed in the session (`a5546a2`).
