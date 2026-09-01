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

## 3. Command Permissions & the Confirmation Gate

Commands are gated in two independent layers before they reach a pane.

### A. Per-Agent Permissions (`[[agents]]` in `config.toml`)
Each pane's agent is resolved from the `herdr` snapshot (`extract_agent_name_for_pane`) and matched against a `[[agents]]` entry. A pane whose agent has no entry can never be sent a command.

* `allowed_commands` — an exact allow list (`"continue"`, `"/compact"`, `"/usage"`); matched case-insensitively.
* `allow_arbitrary_commands` — permits free-form spoken instructions that are *not* on the allow list. Because these are typed verbatim into the agent's prompt, `commands::validate_arbitrary_command` restricts them to ASCII letters, digits, spaces and the punctuation `. , ' ? :` — rejecting `/` (slash commands, paths), `-` (CLI flags), shell metacharacters, control characters (which would inject a second line of input), and anything over 256 characters.
* `require_confirmation` — enables the mechanical gate below.
* A top-level `allow_arbitrary_commands` key, or `--allow-arbitrary-commands` / `--no-arbitrary-commands`, forces the setting for every agent at once.

### B. The Mechanical Confirmation Gate (`VoiceState::confirmation_gate`)
The gate is a state machine (`Idle -> Armed -> Confirmed`) that the model cannot drive on its own:

1. The model answers with a confirmation question. `commands::parse_confirmation_request` extracts the pane and command it named and **arms** the gate with them, flushing queued audio so the user's reply is captured cleanly.
2. The user's next utterance either confirms the gate (an affirmative such as "yes"/"go ahead") or resets it.
3. `HerdrToolExecutor` calls `verify_and_consume_confirmation`, which requires a `Confirmed` gate *and* a pane/command match against what was announced, then consumes it — so a confirmation authorizes exactly one execution of exactly the command the user heard.

---

## 4. Gemini System Prompt

The runtime system prompt used by `handlr` is maintained in `PROMPT.md`. It is loaded at session startup and injected into Gemini to ground it in the terminal multiplexer environment.