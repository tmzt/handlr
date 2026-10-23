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
| [handlr link pairing and the companion protocol](docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md) | 2026-10-22 | In implementation | Phones pair by a `handlr1…` string or QR code over Noise (`XXpsk3`, then `IK`), with keys in `auth.toml`, and talk to handlr in text turns. An interrupt also sends the coding agent its interrupt key. Started with the link layer. |
| [The handlr.harness protocol, session peek and the MCP adapter](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md) | 2026-10-22 | Draft | Programs such as a T3 Code plugin or MCP read agent state and the last entries of each agent's own transcript, found from the pane's cwd, and request sends that the phone approves. Proposed. |
| [The companion app, on handlr-companion and libvoicexn](docs/design/20261022_04_COMPANION_APP_PLAN.md) | 2026-10-22 | In implementation | `handlr-companion` holds the client's connection, sessions, keys and turn logic. `handlr-audio` holds libvoicexn audio (cpal on the Mac, libmorph on Android). The apps are thin front ends. Started with the client logic. |
| [Naming agents aloud and on screen](docs/design/20261022_05_SPOKEN_NAMES_PLAN.md) | 2026-10-22 | In implementation | Agents are named by agent, project (cwd when workspaces share a label) and tab label. Codes without vowels are spelled by letter name, with configurable aliases. Pane ids are never spoken or shown. Started with `core::naming`. |
| [qrcodeurl, and pairing by a scanned URL](docs/design/20261022_06_QRCODEURL_PLAN.md) | 2026-10-22 | In implementation | The pairing QR code carries `handlr://<tailscale host>:<port>/pair#handlr1…` so the phone connects and pairs from one scan; `handlr pair` prints that URL and the code as a `data:` URL from a small crate with a hand-written PNG. Started with the crate. |
| [libinfer providers, the routing engine and the Gemini model setting](docs/design/20261022_07_LIBINFER_PROVIDERS_PLAN.md) | 2026-10-22 | In implementation | Splits libinfer into a provider-neutral core and `libinfer-gemini` on `gemini-rust` (libgenai dropped), builds the missing routing engine and `[gemini] model`, and plans provider-neutral tool declarations; OpenAI support is left to its own plan. Steps 1 to 3 done; step 4 waits for the OpenAI plan. |
| [PROMPT.md and daemon-side approval](docs/design/20261022_08_PROMPT_CONFIRMATION_PLAN.md) | 2026-10-22 | Draft | Analyses the Gemini-written prompt against the daemon and changes only its Mechanical Confirmation Gate block and the `run_command` description, so the model calls the tool and handlr does the asking, instead of asking twice. Proposed. |
