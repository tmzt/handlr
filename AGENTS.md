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

The runtime system prompt used by `handlr` is maintained in `PROMPT.md`. It is loaded at session startup and injected into Gemini to ground it in the terminal multiplexer environment.