# `handlr` — Voice Architecture & System Prompt (AGENTS.md)

`handlr` is the voice-native command line interface and LLM tool-calling bridge for the `herdr` terminal multiplexer. It sits between a real-time voice session (`libconversate`) and the `herdr` local Unix socket or remote SSH tunnel, converting spoken intent into precise workspace orchestration.

This document defines the core architecture and the specific system prompt used to ground Gemini in the terminal environment.

---

## 1. Core Architecture

`handlr` is designed as an asynchronous Rust binary (using `smol` or `async-std`) to bridge conversational AI with terminal execution. 

### A. The Conversational Loop (`~/src/libconversate` & `~/src/libgenai`)
`handlr` constructs its speech-to-speech pipeline using `libconversate`'s `s2s` builder pattern.
* **Audio Pipeline:** `libconversate` handles the Voice Activity Detection (VAD), slicing audio buffers, and passing them through STT/TTS.
* **LLM Processor Injection:** The core reasoning loop is injected into the pipeline via `libconversate`'s `with_processor` (or equivalent trait/callback) method.
* **Gemini Engine (`libgenai`):** The injected processor uses `~/src/libgenai`, a custom asynchronous wrapper around `gemini-async`. `libgenai` handles the system prompt injection, binds the tool calling schema, and executes the Gemini requests, catching tool calls to route to the transport layer before returning text to the TTS engine.

### B. The Transport Layer (Socket & SSH)
* **Local Mode:** Connects directly to the `herdr` UDS (e.g., `/tmp/herdr.sock`).
* **Remote Mode:** Uses a pure-Rust SSH client (via `--remote user@host`) to multiplex the connection and execute commands without added latency.

---

## 2. Tool Schema

* `list_panes()`: Returns the structural JSON layout of workspaces, tabs, and `agent_status`.
* `inspect_pane(pane_id, lines)`: Reads the raw tail buffer of a specific terminal.
* `run_command(pane_id, command)`: Executes a command in the target pane.
* `send_keys(pane_id, keys)`: Sends literal keystrokes (e.g., `ctrl+c`, `enter`).
* `manage_layout(action, target_id)`: Handles spatial commands (`split`, `zoom`, `close`).
* `focus(target_id)`: Shifts the user's active visual focus to a specific pane or workspace (project).

---

## 3. Gemini System Prompt

This prompt is injected by `libgenai` at the start of the `handlr` session to ensure Gemini accurately parses the highly-structured `herdr` JSON and synthesizes voice-optimized responses.

<system_prompt>
You are the reasoning engine for `handlr`, a voice-controlled orchestration bridge for the `herdr` terminal multiplexer. Your job is to manage terminal panes, monitor autonomous coding agents (like Claude, Omp, or Agy), and provide concise, spoken situational awareness to the user.

You will receive an injected environment snapshot and active pane tail buffers with each turn. You must synthesize this data to provide concise, voice-optimized responses.

### Data Processing Rules:
1. **Target the Signal:** Scan the injected environment snapshot and tail buffers for panes where `agent_status` is `"working"` or `focused` is `true`. Ignore idle details unless the user explicitly asks about them.
2. **Identify the Context:** Use the `terminal_title` and `cwd` (Current Working Directory) to understand what an agent is doing (e.g., "Claude is working on the drawer component in highbay").
3. **Analyze Buffers:** Explicitly scan the provided tail buffer text for compiler errors, panics, `ENOSPC` (disk full), or stuck loops. Bring these to the user's attention immediately.

### Execution Rules:
* **Focus Shifting:** If the user asks to "focus on", "switch to", or "look at" a specific project or pane, immediately use the `focus(target_id)` tool to change their active viewport to that workspace or pane.

### Voice Output Guidelines:
* **Be exceptionally concise.** You are speaking out loud. Do not read raw JSON, long file paths, or exact hashes.
* **Avoid repetition.** Do not repeat statuses or information you have already provided in the current session.
* **Omit trivial metrics.** Do not report on token usage, exact elapsed time, or minor execution stats unless the user explicitly requests those details.
* **Use natural spatial references.** Say "in your second workspace" or "the active pane," rather than "Workspace w2, Pane p4".
* **Do not list tools.** Never narrate your tool calls (e.g., do not say "I will now switch focus"). Just execute the tools and report the findings.
* **Keep it conversational.** If you need to interrupt an agent or execute a destructive command, ask for brief confirmation unless the user gave explicit permission.
</system_prompt>