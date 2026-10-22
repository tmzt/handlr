# handlr

`handlr` is the voice-native command line interface and LLM tool-calling
bridge for the `herdr` terminal multiplexer. See [AGENTS.md](AGENTS.md) for
the architecture and [PROMPT.md](PROMPT.md) for the runtime system prompt.

## Architectural decisions

| Document | Date | Status | Decision |
| --- | --- | --- | --- |
| [System architecture and herdr transport](docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md) | 2026-08-31 | In implementation | Splits handlr into libconversate audio, a reasoning loop and a herdr UDS/SSH transport; transport built in `libherdr`. |
| [Pluggable ReasoningEngine and ToolExecutor traits](docs/design/20260831_02_REASONING_ENGINE_PLAN.md) | 2026-08-31 | In implementation | Decouples reasoning behind `ReasoningEngine`/`ToolExecutor` so Gemini, mock and routing engines can be swapped; routing engine not yet built. |
| [libconversate speech pipeline integration](docs/design/20260831_03_SPEECH_PIPELINE_PLAN.md) | 2026-08-31 | In implementation | Builds the pipeline from `SpeechToSpeechBuilder` defaults plus a processor, with STT callbacks only for UI feedback; barge-in testing still open. |
| [handlr as an agent orchestration daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md) | 2026-10-22 | Draft | handlr becomes a daemon serving several clients over a socket, with permissions and a confirmation broker under every client, replacing the voice-only gate; proposed, not yet agreed for implementation. |
