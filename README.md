# handlr

`handlr` is the voice-native command line interface and LLM tool-calling
bridge for the `herdr` terminal multiplexer. See [AGENTS.md](AGENTS.md) for
the architecture and [PROMPT.md](PROMPT.md) for the runtime system prompt.

## Architectural decisions

| Document | Date | Status | Decision |
| --- | --- | --- | --- |
| [System architecture and herdr transport](docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md) | 2026-08-31 | In implementation | Splits handlr into libconversate audio, a reasoning loop and a herdr UDS/SSH transport; transport built in `libherdr`. |
| [Pluggable ReasoningEngine and ToolExecutor traits](docs/design/20260831_02_REASONING_ENGINE_PLAN.md) | 2026-08-31 | In implementation | Decouples reasoning behind `ReasoningEngine`/`ToolExecutor` so Gemini, mock and routing engines can be swapped; routing engine not yet built. |
| [libconversate speech pipeline integration](docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md) | 2026-08-31 | Retired (by [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md)) | Built the desktop speech pipeline on libconversate; removed when handlr became a daemon and voice moved to the companion apps. Recoverable from `f892246`. |
| [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md) | 2026-10-22 | In implementation | handlr becomes a daemon serving several clients over a socket, with permissions and a confirmation broker under every client; started by removing the libconversate speech pipeline. |
| [handlr link pairing and the companion protocol](docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md) | 2026-10-22 | Draft | Phones pair by QR code over Noise (`XXpsk3`, then `IK`) with keys in `auth.toml`, and talk to handlr in text turns with an Esc-like interrupt; the phone owns all audio. Proposed. |
| [The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md) | 2026-10-22 | Draft | Programs such as a T3 Code plugin or MCP read agent state and the last entries of each agent's own transcript, found from the pane's cwd, and request sends that the phone approves. Proposed. |
| [The companion app, on handlr-companion and libvoicexn](docs/design/20261022_04_COMPANION_APP_PLAN.md) | 2026-10-22 | Draft | The companion client's logic is a crate with no audio or UI, tested in handlr's build. Terminal and libmorph front ends join it to libvoicexn in a separate `apps/` workspace, so the ML stack stays out of handlr's build. Proposed. |
