---
title: libconversate speech pipeline integration
date: 2026-08-31
status: In implementation
covers: building the speech-to-speech pipeline, processor vs STT callbacks, pipeline testing
sources:
  - PLAN.md as committed in ecaaa09 (2026-08-31 04:27 -0400)
related:
  - docs/design/20260831_01_ARCHITECTURE_AND_TRANSPORT_PLAN.md
  - docs/design/20260831_02_REASONING_ENGINE_PLAN.md
---

# libconversate speech pipeline integration

Split from `handlr` Architecture & Implementation Plan. This document outlines the detailed design and architectural phase for `handlr`, the voice-native command line interface and LLM tool-calling bridge for the `herdr` terminal multiplexer.

---

## 3. Alignment with `libconversate` Builder API

`libconversate` automatically configures platforms defaults (like CPAL-based audio output device factories via `CpalAudioSink` and no-op callbacks) when not explicitly provided. Consequently, `handlr` only needs to provide the `SttConfig`, `TtsConfig`, and the core semantic `.with_processor(...)` closure to build a fully-functional pipeline.

### A. Pipeline Instantiation

```rust
let stt_config = SttConfig { ... };
let tts_config = TtsConfig { ... };

let reasoning_engine: Arc<dyn ReasoningEngine> = Arc::new(GeminiReasoningEngine::new(api_key)?);
let tool_executor: Arc<dyn ToolExecutor> = Arc::new(HerdrExecutor::new(transport));

// Setup the S2S pipeline automatically using only `with_processor`
let pipeline = SpeechToSpeechBuilder::new(stt_config, tts_config)
    .with_processor(move |transcript| {
        // Bridge the synchronous processor callback to the async executor
        smol::block_on(async {
            match reasoning_engine.process_transcript(transcript, tool_executor.as_ref()).await {
                Ok(response) => response,
                Err(e) => {
                    log::error!("Reasoning error: {}", e);
                    Some("Sorry, I encountered an error processing that.".to_string())
                }
            }
        })
    })
    .build()?;
```

---

## 4. Coexistence of Processor & STT Callbacks

A common design question is whether to use the central **Processor Loop** or **STT Callbacks**. In `handlr`, they coexist cleanly through clear separation of concerns:

### A. Division of Labor
*   **The Processor Callback:** Owns semantic processing. It intercepts the finalized `on_transcript` string, executes business logic, and yields the final response text to feed to `TtsSpeaker`.
*   **STT Callbacks:** Optional. Own real-time UI/UX feedback and local hardware orchestration. They trigger on low-level pipeline events:
    *   `on_vad_speech(active)`: Triggers immediate UI updates (e.g. changing microphone colors/cues).
    *   `on_partial(text)`: Renders user speech draft on-screen as they speak.
    *   `on_mic_request(active)`: Activates or deactivates microphone physical capture hardware.

### B. Execution Flow & Pipeline Interaction
```
   [ Mic Capture ] -> PCM Audio
         |
         v
   [ SttSession (Whisper/Sherpa) ]
         |
         +---> on_vad_speech(true)  =======> Updates UI Voice Indicator (Optional)
         |
         +---> on_partial("focus")  =======> Renders draft text on terminal (Optional)
         |
         +---> on_transcript("focus on second project")
                     |
                     v
             [ Processor Callback ]
                     | (Bridged to Async Reasoning Engine)
                     v
             [ ReasoningEngine::process_transcript ]
                     |
                     +---> ToolExecutor::execute_tool("focus", target_id)
                     |
                     +---> Returns "Switched focus to second workspace."
                     |
                     v
             [ TtsSpeaker::speak ]
                     |
                     v
             [ Audio Output ]
```

---

## Implementation Phases

*   **Phase 3 (Pipeline Configuration):** Configure `SpeechToSpeechBuilder` integration using automatic sink/callback defaults and verify event propagation.
*   **Phase 4 (Testing):** Validate barge-in functionality and compile schema assertions.


## Amendment — 2026-10-22 10:47 EDT: retroactive conversion from PLAN.md

This document was split out of the single `PLAN.md` (commit `ecaaa09`,
2026-08-31) when the repository adopted the libcontrols critical rules. The
body above is the original text, unchanged apart from the split; it predates
Rule 1.2 and has no **Done when:** criteria. The date is the original one.

Implementation of the speech pipeline (Phase 3) started on 2026-08-31 in `a84d284`, so the
status is set to `In implementation` and the document is now immutable.

## Amendment — 2026-10-22 10:53 EDT: implementation record

Implementation records only; the body above is not changed by this amendment.

**Phase 3 (pipeline).** First built in `a84d284`, switched to
`with_async_processor` in `28e334f` and to the builder's defaults in
`f61030b`. Native TTS with `--tts-voice`/`--tts-speed` in `e5827b0`,
selectable TTS backends in `ea22c60`, Sherpa Kokoro auto-download in
`3005fea` and `cf1393a`, model symlinks in `03527b9`, sherpa-onnx rpath in
`d9b3953`.

**Phase 4 (testing).** Not done as written: there are no barge-in tests.
`tests/live_naming.rs` and unit tests in `src/reasoning.rs` cover naming and
tool declarations.

**Departures from the body:**

- The processor is `with_async_processor` (`28e334f`), not `with_processor`
  bridged through `smol::block_on` (§3.A), and the runtime is Tokio
  (`be9362c`).
- Capture is muted at the CPAL stream while reasoning and speaking
  (`1a3f87d`); the mute is released when a turn produces no reply (`2817db2`).
- `Arc<VoiceState>` is shared by the pipeline and capture (`2ebc2a4`); it
  carries the confirmation gate, which flushes queued audio when armed
  (`2e713c5`).
- Spoken notifications queue behind the current utterance instead of barging
  in (`a7769c9`).

**Learned.** Blocking on the async engine inside the processor callback had no
reactor to run on; an async processor on Tokio removes the bridge. Without the
hardware mute the assistant hears its own TTS.
