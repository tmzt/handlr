# `handlr` Remote Audio Protocol — Design & Implementation Plan

This document specifies the remote audio plane for `handlr`: PCM captured on the
user's laptop (machine A) is streamed over ssh to a dev box (machine B) and fed
into the existing `libconversate` speech-to-speech pipeline **as if it had been
captured locally**; synthesized TTS PCM streams back for local playback. The
control-plane pattern (`handlr --remote`, `libherdr::HerdrTransport::RemoteSsh`)
is imitated exactly: a private Unix domain socket owned by a daemonized fork on
the remote side, reached by a `handlr` subcommand invoked over ssh that bridges
stdin/stdout to the socket. The wire is the same length-prefixed bincode frame
layout as `voice_proto::framed` — not JSON lines, for reasons given in §6.

Ground truth read for this plan:

* `~/src/libconversate/crates/voice_proto/src/{framed.rs,asr.rs,tts.rs,lib.rs}`
* `~/src/libconversate/crates/audio_core/src/{lib.rs,audio_ring.rs,cpal_capture.rs,cpal_playback.rs}`
* `~/src/libconversate/crates/libconversate/src/{s2s.rs,stt.rs,tts.rs}`
* `~/src/libconversate/daemons/chitin_asr_vad/src/main.rs`, `daemons/chitin_tts_kokoro/src/{main.rs,net.rs}`
* `~/src/handlr/src/{main.rs,transport.rs,config.rs}`, `crates/libherdr/src/{transport.rs,protocol.rs,events.rs}`

---

## 1. Motivation & Topology

Today `handlr` runs everything on one machine: `CpalCapture` resamples the mic
to 16 kHz mono f32 and pushes into the global `audio_core::AudioRingBuffer`;
`SttSession`'s `conversate-pull` thread drains the ring every 10 ms into
VAD + whisper; transcripts drive the Gemini reasoning loop; `TtsSpeaker`
synthesizes into a `CpalAudioSink`. In `--remote` mode only the **herdr control
plane** crosses ssh — the models, the reasoning, and the audio hardware all
stay on the laptop.

That is backwards for the common deployment: the dev box (B) has the GPU, the
models, the herdr instance being orchestrated, and the agent panes; the laptop
(A) has the mic and speakers and possibly no models at all. Remote audio flips
the split:

* **Machine A (laptop)** owns: microphone, speakers, `CpalCapture`,
  `CpalAudioSink`, the ssh client. Nothing else. No VAD, no whisper, no Kokoro,
  no Gemini key, no herdr connection.
* **Machine B (dev box)** owns: VAD, ASR, TTS synthesis, the reasoning engine,
  the `VoiceState` confirmation gate, and a **local** herdr socket (no more
  double-hop herdr-over-ssh — the brain moves to where herdr lives).

The hard requirement: the remote side must treat incoming frames as
indistinguishable from local capture. Concretely, the daemon on B decodes wire
frames to 16 kHz mono f32 and calls
`audio_core::ensure_audio_ring().push_samples(&samples)` — the exact call the
`CpalCapture` closure makes in `src/main.rs` today. Everything downstream
(`SttSession`'s pull thread, VAD hysteresis, mute checks, `skip_to_end()`
flushes on gate arming) runs unmodified.

### Local case (today)

```
┌───────────────────────────── laptop ─────────────────────────────┐
│ mic → CpalCapture → AudioRingBuffer → SttSession → processor     │
│                                          (VAD+whisper)  │        │
│ speaker ← CpalAudioSink ← TtsSpeaker ←───────────────────┘       │
│                              │                                   │
│              HerdrTransport ─┴─→ herdr.sock (or ssh → remote)    │
└──────────────────────────────────────────────────────────────────┘
```

### Remote-audio case (this plan)

```
┌──────────── machine A (laptop) ────────────┐      ┌───────────────── machine B (dev box) ─────────────────┐
│ mic → CpalCapture ─┐                       │      │  handlr audio-daemon (forked, private UDS)             │
│                    ▼                       │ ssh  │   ├─ wire f32 → ensure_audio_ring().push_samples()     │
│  handlr --remote-audio <target>            │======│   ├─ SttSession (silero VAD + whisper)  ── unmodified  │
│   RemoteAudioLink                          │stdio │   ├─ async processor → ReasoningEngine (Gemini)        │
│   ├─ writer: PcmFrame(mic) ──────────────► │pipe  │   │    └─ HerdrToolExecutor → herdr.sock (LOCAL to B)  │
│   ├─ reader: PcmFrame(tts) ─► CpalAudioSink│      │   ├─ VoiceState (mute + confirmation gate) — lives HERE│
│   └─ reader: AsrEvent/State mirrors        │      │   └─ TtsSpeaker → WireAudioSink ── PcmFrame(tts) ────► │
│ speaker ◄──────────────┘                   │      │                                                        │
└────────────────────────────────────────────┘      │  $HOME/.handlr/sockets/audio-<session>.sock (0600)     │
                                                    └────────────────────────────────────────────────────────┘

ssh leg:  ssh -T <target> handlr audio-bridge --socket <path> --autostart
          (stdin/stdout ⇄ UDS, byte-for-byte; stderr = remote logs)
```

Both directions ride **one** ssh stdio pipe and **one** UDS connection.

---

## 2. Where the Seam Is Cut

Three candidate boundaries, judged against "processed as if local":

| Seam | What crosses A→B | Verdict |
|---|---|---|
| **Raw PCM (pre-VAD)** | 16 kHz mono frames, continuously while unmuted | **Chosen.** The ring buffer and `AudioSink` are existing seams; zero pipeline changes on B; VAD, RMS gate, confirmation-mode short-utterance handling, and `skip_to_end()` flushes all behave identically. |
| Post-VAD utterance spans | Whole utterances after a local silero pass | Loses: A must run silero + carry `confirmation_mode` state (the VAD bypass for short "yes" answers is toggled by the gate on B — now a cross-machine race on the exact feature that must not race). Saves bandwidth we don't need to save. |
| Post-ASR transcripts | Text | Loses hardest: A needs whisper (the thing the laptop doesn't have). This is just `--remote` with extra steps. |

**Primary seam: raw PCM.** The two fallback/degraded modes remain reachable by
configuration, not by new protocol machinery:

* **Degraded mode 1 (transcript seam)** — a deployment where A *does* have
  models but B has the herdr instance is exactly today's `--remote` flag.
  Nothing new to build; document it as the fallback when the audio link is
  unusable.
* **Degraded mode 2 (local-VAD pre-gating)** — run silero on A purely as a
  *transmit gate* (stream only around speech) while B still runs the real
  VAD. Deferred (§11); it saves idle bandwidth but reintroduces the
  confirmation-mode sync problem for marginal benefit.

### Bandwidth, honestly

The pipeline is tuned for 16 kHz mono (`SttConfig::sample_rate = 16_000`;
`CpalCapture` already resamples/downmixes to 16 kHz mono f32 **before** the
ring). So the resample-before-transmit question is already answered by
existing code: A resamples exactly as it does today, and transmits the
post-resample stream.

* Mic, as f32: 16 000 × 4 = **64 KB/s**.
* Mic, as i16: 16 000 × 2 = **32 KB/s** ← chosen; quantizing 16 kHz speech to
  i16 is what every ASR corpus does; whisper sees no difference.
* TTS return (Kokoro is 24 kHz mono, `KOKORO_SAMPLE_RATE = 24_000`), as i16:
  **48 KB/s**, and only while speaking. The `AudioSink` trait already takes
  `&[i16]`, so this direction is i16 natively — no conversion at all.
* Worst case both directions active: ~80 KB/s ≈ 0.64 Mbit/s. Trivial for any
  link ssh is usable on. We do **not** compress (no Opus dependency); §11.

Never transmitted: 48 kHz stereo f32 (768 KB/s) — that only exists inside the
cpal callback on A and dies there, exactly as it does today.

---

## 3. The Wire Protocol

### 3.1 Placement: `voice_proto::audio` (in-tree in libconversate)

New submodule `crates/voice_proto/src/audio.rs` in the libconversate repo,
alongside `asr.rs` and `tts.rs`. Reasons over a handlr-side crate:

* The codec (`framed`), the reused vocabularies (`AsrEvent`, `AsrCommand`,
  `DaemonMessage`), and the append-only-variant convention all live in
  `voice_proto`; splitting the audio vocabulary out would fork that convention
  across repos.
* `handlr` already path-depends on `libconversate` and `audio_core`
  (`Cargo.toml` lines 7–9); both endpoints of the wire link `voice_proto`
  either way.
* Other `voice_proto` consumers (the chitin daemons, future Android host) get
  the audio vocabulary for free.

The handlr-side *behavior* (daemon, bridge, link) stays in handlr; only the
*types* go to `voice_proto`.

### 3.2 Types

All enums follow the `asr.rs` rule, restated here because it is load-bearing:
**new variants are appended at the end so bincode discriminants never shift.**
Same for struct fields: bincode has no field names — fields are append-only
too, and any non-append change requires a `AUDIO_PROTOCOL_VERSION` bump.
That is why `PcmFrame.utterance_id`, `ConfirmResponse`, `UtteranceStart`,
and `ConfirmResolved` sit at the ends of their structs/enums. Nothing has
shipped: everything below, confirmation challenge/response and utterance
framing included, is part of `AUDIO_PROTOCOL_VERSION` **1** — no bump.

```rust
//! Remote audio wire types — one submodule of `voice_proto`.
//!
//! One connection, both directions multiplexed: A→B carries
//! [`AudioToRemote`], B→A carries [`AudioToLocal`]. Same
//! length-prefixed bincode frames as the rest of voice_proto
//! (see [`crate::framed`]).

use serde::{Deserialize, Serialize};
use crate::asr::{AsrCommand, AsrEvent};
use crate::tts::DaemonMessage;

/// Bumped only for incompatible changes (removed/reordered variants
/// or fields). Appended variants/fields do NOT bump this.
pub const AUDIO_PROTOCOL_VERSION: u32 = 1;

/// Encoding of the samples inside a [`PcmFrame`] payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleFormat {
    /// Interleaved little-endian signed 16-bit.
    I16,
    /// Interleaved little-endian IEEE-754 32-bit float in [-1, 1].
    F32,
}

/// Static description of one PCM stream. Negotiated once in
/// Hello/HelloAck — NOT repeated per frame (a frame carries only a
/// stream id; format changes require a reconnect, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFormat {
    pub rate: u32,
    pub channels: u16,
    pub format: SampleFormat,
}

/// Stream ids. u8 on the wire so a future third stream (e.g. an
/// earcon channel) is an append, not a schema change.
pub const STREAM_MIC: u8 = 0; // A → B
pub const STREAM_TTS: u8 = 1; // B → A

/// One chunk of PCM. ~20 ms nominal (§3.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PcmFrame {
    /// STREAM_MIC or STREAM_TTS.
    pub stream: u8,
    /// Per-stream monotonically increasing counter, starting at 0 at
    /// each Hello. Gaps mean the sender dropped frames (§3.7).
    pub seq: u64,
    /// Sender-side monotonic clock, microseconds. Never compared
    /// across machines; used for sender pacing, receiver gap sizing,
    /// and latency accounting against Pong RTT.
    pub timestamp_us: u64,
    /// Raw interleaved samples in the stream's negotiated
    /// StreamFormat. Byte length must be a multiple of the sample
    /// size × channels; receivers drop malformed frames and count
    /// them as a gap.
    pub payload: Vec<u8>,
    /// Utterance epoch this frame belongs to. 0 for STREAM_MIC (A
    /// does not segment utterances). For STREAM_TTS, the u32 counter
    /// announced by the preceding UtteranceStart — any frame is
    /// attributable to its utterance (and, via the challenge on the
    /// UtteranceStart, to a confirmation prompt, §3.6) without
    /// repeating text per frame. Appended field: 4 bytes/frame, no
    /// version bump. Also exactly the identity a future barge-in
    /// flush needs to drop only the cancelled utterance's frames.
    pub utterance_id: u32,
}

/// First frame on the wire, A → B.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    /// e.g. "handlr/0.1.0 (macos-aarch64)". Diagnostic only.
    pub client: String,
    /// Stable per-invocation id (uuid). Lets the daemon log/refuse
    /// a second concurrent session cleanly (§5).
    pub session_id: String,
    /// Format of STREAM_MIC frames this client will send.
    pub mic_format: StreamFormat,
    /// Playback formats A can service, preference-ordered. The
    /// daemon picks the first it can synthesize into.
    pub playback_formats: Vec<StreamFormat>,
}

/// First frame back, B → A. Anything else first is a protocol error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol_version: u32,
    /// e.g. "handlr/0.1.0 (linux-x86_64)".
    pub server: String,
    /// Daemon-chosen epoch, unique per accepted session. Frames from
    /// a previous epoch are dead after reconnect (§8).
    pub epoch: u64,
    /// The entry from Hello.playback_formats the daemon selected for
    /// STREAM_TTS.
    pub playback_format: StreamFormat,
}

/// Advisory mirror of the remote VoiceState, B → A. See §3.6 — the
/// remote copy is the ONLY authority; this exists so A can gate its
/// capture callback (bandwidth) and drive local UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSync {
    pub muted: bool,
    pub confirmation_mode: bool,
    /// `VoiceState::is_confirmation_active()` on B — a confirmation
    /// exchange is in flight. A may use it to duck notifications.
    pub confirmation_active: bool,
}

/// The challenge half of the confirmation protocol (§3.6), B → A,
/// carried inside UtteranceStart. The nonce is a single-use
/// capability to answer ONE pending confirmation question; pane_id
/// and command are display-only copies for the local UI — A never
/// sends them back, only the nonce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmChallenge {
    /// 128-bit CSPRNG value minted on B, fresh per gate arming.
    pub nonce: u128,
    pub pane_id: Option<String>,
    pub command: Option<String>,
}

/// B → A, sent once before the first PcmFrame of each STREAM_TTS
/// utterance. Binds the compact per-frame utterance_id to the
/// utterance's full text and, for confirmation prompts, its
/// challenge — so text crosses the wire once, not per 20 ms frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtteranceStart {
    /// STREAM_TTS (a u8 so a future stream can reuse this).
    pub stream: u8,
    /// Per-epoch counter starting at 1 (0 means "no utterance").
    /// Stamped on every PcmFrame of the utterance.
    pub utterance_id: u32,
    /// The String utterance id from voice_proto::tts::SpeakRequest,
    /// so Tts(DaemonMessage) mirrors correlate. Sent once here —
    /// repeating it per frame would cost ~44 bytes and an allocation
    /// every 20 ms for what a Copy u32 does in 4.
    pub tts_utterance_id: String,
    /// Full text being synthesized — the local UI's display copy.
    pub text: String,
    /// Some(_) iff this utterance is a confirmation prompt (§3.6).
    pub challenge: Option<ConfirmChallenge>,
}

/// Everything machine A sends. Append-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioToRemote {
    Hello(Hello),
    Pcm(PcmFrame),
    /// Pass-through to the remote SttSession (StartCapture /
    /// StopCapture / Cancel / Ping) — same vocabulary as the daemon
    /// path today.
    Asr(AsrCommand),
    /// Playback watermark: A has played every STREAM_TTS frame with
    /// seq <= played_seq. Sent every 250 ms while the local sink is
    /// non-empty and once when it empties. Drives the remote
    /// WireAudioSink's `drain()` and its 2 s lookahead throttle.
    PlaybackProgress { stream: u8, played_seq: u64 },
    /// RTT probe / keepalive; B echoes as Pong untouched.
    Ping { nonce: u64, t0_us: u64 },
    /// Clean shutdown: A stops sending, half-closes after B's Bye.
    Bye,
    /// Answer to a ConfirmChallenge from a local UI (§3.6). The
    /// voice path (a spoken "yes" as PCM) remains valid and carries
    /// no nonce. accept = false resets the gate immediately — a UI
    /// "No" must not have to wait for a timeout. The only wire
    /// message that can mutate VoiceState, and it is inert without a
    /// fresh nonce B itself minted (§9).
    ConfirmResponse { nonce: u128, accept: bool },
}

/// Everything machine B sends. Append-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioToLocal {
    HelloAck(HelloAck),
    Pcm(PcmFrame),
    /// Mirror of every AsrEvent the remote SttCallbacks sees —
    /// CaptureStart/SpeechStart/Transcript/… — for local UI/logs.
    /// Advisory; A must not act on Transcript content.
    Asr(AsrEvent),
    /// Mirror of TTS lifecycle (Started/Finished/Cancelled/Error)
    /// keyed by the same client/session/utterance ids as
    /// voice_proto::tts. `Cancelled` is also A's cue to reset() the
    /// local sink (barge-in dumps queued audio immediately).
    Tts(DaemonMessage),
    State(StateSync),
    /// Drop all queued STREAM_TTS audio now (barge / Stop). Follows
    /// or replaces Tts(Cancelled) for belt-and-braces flushing.
    FlushPlayback { stream: u8 },
    /// Receiver-side gap diagnostic for the mic stream (§3.7).
    Gap { stream: u8, expected_seq: u64, got_seq: u64, concealed_ms: u32 },
    Pong { nonce: u64, t0_us: u64 },
    /// Session-fatal daemon error; connection closes after this.
    Error { message: String },
    Bye,
    /// Utterance framing for STREAM_TTS (§3.5, §3.6). Precedes the
    /// utterance's first PcmFrame on the same FIFO stream.
    UtteranceStart(UtteranceStart),
    /// The challenge carrying this nonce is settled — confirmed
    /// (either path), declined, reset, or invalidated — so A
    /// dismisses any UI for it. Advisory: the authoritative
    /// settlement already happened on B.
    ConfirmResolved { nonce: u128 },
}
```

### 3.3 Session establishment

1. A connects (spawns the ssh bridge, §6), sends `Hello` with
   `protocol_version = AUDIO_PROTOCOL_VERSION`,
   `mic_format = {16_000, 1, I16}` and
   `playback_formats = [{24_000, 1, I16}, {16_000, 1, I16}]`.
2. B validates the version (equal major = compatible in v1; see §6 for
   mismatch handling), checks no other session is active, replies `HelloAck`
   with a fresh `epoch` and the chosen `playback_format`, then starts its
   pipeline pull. Any other first frame from either side → close with
   `Error`.
3. Steady state: A streams `Pcm(STREAM_MIC)` whenever its (advisory) mute
   mirror is off; B streams `Pcm(STREAM_TTS)` while speaking — each
   utterance preceded by its `UtteranceStart` (§3.6) — plus `Asr`, `Tts`,
   and `State` mirrors as they happen.
4. Teardown: either side sends `Bye`, the peer replies `Bye`, both close.
   §3.9 for half-close details.

### 3.4 One connection, multiplexed envelope — not separate connections

The `AsrEvent` / `AsrCommand` / `DaemonMessage` vocabularies ride the same
connection inside the envelope enums above, rather than opening a second
"control" connection. Justification:

* **Ordering is semantic.** `FlushPlayback` must not race the PCM frames it
  flushes; `State { muted: true }` must not race the mic frames it silences.
  Concrete example from the confirmation flow: `UtteranceStart` must arrive
  before the `PcmFrame`s carrying its `utterance_id` (else A plays audio it
  cannot attribute), and `ConfirmResolved` must arrive after the challenge
  it settles (else A dismisses a dialog that then pops back up). One FIFO
  byte stream gives all of this for free; two connections would need
  cross-stream sequencing we'd have to invent.
* Each extra connection is an extra `ssh` child + UDS accept + reconnect state
  machine. `open_event_stream` already demonstrates the one-held-pipe shape.
* Head-of-line blocking is a non-issue at 80 KB/s against MAX_FRAME_BYTES-
  bounded frames of ~1 KB: worst-case a control frame waits ~1 frame time
  (<1 ms on loopback, negligible vs. speech timescales).

The herdr control plane (`HerdrTransport`) stays exactly as it is — but in
remote-audio mode it is instantiated **on B inside the daemon**, pointed at
B's local `herdr.sock`. It does not share this connection; it doesn't cross a
machine boundary at all anymore.

### 3.5 Frame size vs `MAX_FRAME_BYTES`

Chosen frame duration: **20 ms nominal** for the mic stream. The A-side writer
coalesces whatever the cpal callback delivers (typically ~10 ms chunks post-
resample) until ≥ 320 samples are pending, then emits one frame; a 100 ms
flush timer bounds latency if capture stalls. TTS frames are cut at 20 ms
(480 samples at 24 kHz) by `WireAudioSink::write`, which may receive multi-
second synthesis chunks and slices them.

Byte math per frame (bincode 1.x: enum tag u32, struct fields in order,
`Vec<u8>` as u64 len + bytes):

* Mic: 4 (length prefix) + 4 (AudioToRemote tag) + [1 + 8 + 8 + 8 + 640 + 4] =
  **677 bytes / 20 ms** → ~33.9 KB/s including overhead (6 % overhead).
* TTS: 4 + 4 + [1 + 8 + 8 + 8 + 960 + 4] = **997 bytes / 20 ms** → ~49.9 KB/s.

The trailing 4 bytes are `utterance_id` (u32). This is why utterance *text*
rides `UtteranceStart` once per utterance instead of per frame: reusing
`voice_proto::tts`'s `String` utterance id on every frame would cost 8 (len)
+ ~36 (uuid) bytes plus a heap allocation per 20 ms decode, for information
that never changes mid-utterance.

`MAX_FRAME_BYTES = 1 MiB` corresponds to **32.7 s** of 16 kHz mono i16 in a
single frame — three orders of magnitude above our 20 ms frames, and even a
pathological 48 kHz stereo f32 frame could carry 2.7 s. The cap is adequate;
it is not touched. The daemon additionally enforces a *policy* cap of 250 ms
of audio per `PcmFrame` (larger frames are accepted by the codec but rejected
with `Error` — nothing legitimate produces them and they defeat pacing).

### 3.6 `VoiceState` across the boundary — one authority, one capability token

This is the security-relevant part; the design is deliberately boring:

**The `VoiceState` instance — `muted`, `confirmation_mode`, and the
`confirmation_gate` — lives exclusively on machine B, inside the daemon. A
never holds gate state. While a confirmation is pending, A holds exactly one
opaque token — a 128-bit nonce — that it can do nothing with except echo
back. A nonce is a capability to answer *one specific pending question*; it
is not a grant of gate state.**

The confirmation exchange is an explicit challenge/response protocol.
Walking the flow (the same flow `src/main.rs` runs today, every participant
moved to B, plus the challenge leg):

1. The async processor (on B) gets a model reply,
   `parse_confirmation_request` fires, and `vs.arm_confirmation()` — which
   now also **mints a fresh nonce** (OS CSPRNG, 128 bits, stored as `u128`)
   inside the `Armed` state — plus `ensure_audio_ring().skip_to_end()`.
   Arming and flushing happen on B, against B's ring, which is the only
   ring the VAD reads.
2. The confirmation question is synthesized on B. Its `UtteranceStart`
   crosses to A carrying the prompt **text**, the per-frame `utterance_id`,
   and the **challenge** (`ConfirmChallenge { nonce, pane_id, command }`).
   A can therefore render a real dialog — question text, the command being
   confirmed, Accept/Decline — alongside (or instead of) playing the audio,
   and can tell that the audio it *is* playing is a confirmation prompt.
3. The user answers by either path:
   * **Voice** (unchanged): "yes" crosses as PCM, is transcribed on B, and
     B's processor calls `vs.confirm_gate()`; any non-affirmative
     transcript calls `vs.reset_confirmation_gate()`. No nonce involved.
   * **Local UI**: A sends `ConfirmResponse { nonce, accept }`. B calls
     `vs.answer_challenge(nonce, accept)`, which under the same
     `Mutex<GateState>` verifies the nonce against the currently-`Armed`
     gate, then advances `Armed → Confirmed` (accept) or resets to `Idle`
     (decline — a UI "No" takes effect immediately, not by timeout).
4. `HerdrToolExecutor` (on B) calls `verify_and_consume_confirmation()`
   under B's own `Mutex<GateState>` — the same single-process atomicity the
   local case has today, identical regardless of which path confirmed.
5. However the challenge settles — either answer path, reset, the §8
   outage timeout, session teardown — B sends `ConfirmResolved { nonce }`
   so A dismisses the dialog (this is what clears the UI when the user
   answers by voice).

Gate-side additions in `voice_proto::lib.rs` (process-local types, not wire
types — field-position rules don't apply, but shown for concreteness):

```rust
pub enum GateState {
    #[default] Idle,
    /// nonce: the outstanding challenge for THIS arming.
    Armed     { pane_id: Option<String>, command: Option<String>, nonce: u128 },
    Confirmed { pane_id: Option<String>, command: Option<String> },
}

impl VoiceState {
    /// As today (also sets confirmation_mode), but mints and
    /// returns the challenge nonce.
    pub fn arm_confirmation(&self, pane_id: Option<String>, command: Option<String>) -> u128;
    /// UI path: verify nonce against the Armed gate under the one
    /// mutex, then Armed -> Confirmed (accept) or -> Idle (decline).
    /// false = no armed gate with this nonce; caller logs and drops.
    pub fn answer_challenge(&self, nonce: u128, accept: bool) -> bool;
}
```

Nonce lifecycle, stated as invariants:

* **Fresh and unpredictable.** 128 bits from the OS CSPRNG per arming;
  never reused, never derived from anything observable on the wire.
* **Tightly bound.** The nonce lives *inside* `Armed { .. }` — valid only
  for that specific gate instance (its `pane_id` + `command`). Any
  transition out of `Armed` — confirmation by either path, reset on a
  non-affirmative transcript, the 60 s outage reset (§8), daemon shutdown —
  destroys it. Re-arming (even for the same command) mints a different
  nonce; a response quoting the superseded one fails.
* **Single-use, consumed atomically.** `answer_challenge` takes the gate
  mutex once: verify, transition, done. A second `ConfirmResponse` with
  the same nonce finds no matching `Armed { nonce }` and is refused.
* **Epoch-bound.** A nonce never survives a reconnect: session teardown
  invalidates the outstanding challenge, and if the gate is still armed
  when a new session (fresh `epoch`, §3.3/§8) attaches, B mints a *new*
  nonce and re-issues the challenge over the new connection. A response
  quoting a previous epoch's nonce cannot match anything.

Why replay of a captured `ConfirmResponse` fails, exactly: it can only be
delivered over the live, ssh-authenticated connection — there is no
store-and-forward surface (§9) — and by the time an attacker could resend
it, the nonce has been consumed by the first delivery, destroyed by a gate
transition, or orphaned by an epoch change. All three checks happen under
one mutex on B.

`StateSync` (B→A) is a **read-only mirror**, sent on every transition of
`muted` / `confirmation_mode` / gate activity. A uses it for two things only:

* Gating the capture callback (`if mirror.muted { return; }`) to save
  bandwidth during TTS playback — the same shape as today's
  `vs_for_capture.is_muted()` check. If the mirror is stale, extra frames
  arrive at B and are discarded by B's authoritative check in the pull thread
  (`inner.is_muted()` → `ring.skip_to_end()`), exactly as a hot mic is
  handled locally today. Staleness costs bandwidth, never correctness.
* Local UI (a future "listening / speaking / confirming" indicator; the
  confirmation dialog itself is driven by `ConfirmChallenge`, not this).

`ConfirmResponse` is the **only** A→B message that can mutate `VoiceState`,
and it is inert without a fresh nonce B itself minted for a question B
itself asked. Any other future state mutation (e.g. a local hardware mute
button, §11) will be an appended `AudioToRemote` variant that *requests* a
change B applies through `VoiceState` methods — the authority does not move.

### 3.7 Backpressure, stalls, and gaps

Audio is latency-sensitive; unbounded buffering converts a network stall into
permanently-lagged conversation. Policy, per direction:

* **Mic (A→B): drop-oldest.** The writer task owns a bounded deque of 32
  encoded frames (~640 ms). The capture callback never blocks (it runs on the
  cpal audio thread); it appends and, when full, pops the oldest and
  increments a dropped-frame counter. `seq` is stamped at *capture* time, so
  drops appear to B as sequence gaps. Rationale: stale audio is worse than a
  gap — the VAD would segment against a phantom timeline and every utterance
  after a stall would transcribe seconds late. If the deque stays full for
  > 5 s, the link is declared dead and reconnect begins (§8).
* **TTS (B→A): throttle at source, never drop.** Synthesis is faster than
  real time; dropping TTS frames garbles speech audibly. `WireAudioSink`
  tracks A's `PlaybackProgress` watermark and blocks its `write()` when more
  than 2 s of un-played audio is outstanding. This is safe because `write()`
  is called from the TTS manager's playback thread (the same thread that
  paces a local `CpalAudioSink`), and gives `drain()` an exact definition:
  block until `played_seq` reaches the last seq written (with a timeout of
  outstanding-audio-duration + 3 s, then `SinkError::Io`).
* **Gap handling at B (mic receiver):** on `seq` discontinuity, B feeds
  `min(gap_duration, 500 ms)` of zero samples into the ring before the new
  frame's payload — silence is what silero should see for a dropout, and it
  lets in-progress utterances close naturally instead of splicing two
  half-words together. Gap duration is computed from the `timestamp_us`
  delta, not from seq alone (seq gaps have unknown per-frame sizes after
  coalescing). B reports `Gap { concealed_ms }` so A can log/announce link
  quality. Gaps ≥ 500 ms also `skip_to_end()` after concealment — don't
  hallucinate long silences.
* **Gap handling at A (TTS receiver):** none needed — B never drops, and the
  local `CpalAudioSink` VecDeque naturally plays silence on underrun.

### 3.8 Sequence numbers & timestamps

`seq` is per-stream, per-epoch, starting at 0 after each `Hello`. It is
stamped by the sender at sample-acquisition time (before the send queue), so
queue drops are visible. `timestamp_us` is the sender's monotonic clock; the
two machines' clocks are never compared — RTT and one-way estimates come from
`Ping`/`Pong` (A stamps `t0_us`, B echoes, A measures; sent every 5 s as
keepalive, which also makes ssh hangs detectable within ~10 s).

### 3.9 Clean shutdown & half-close

Mirrors the `open_event_stream` lifetime rule: **A holds the ssh child's stdin
open for the life of the link** — dropping it makes the remote bridge see EOF
and hang up (the exact "remote `nc` sees EOF" gotcha called out in
`libherdr::transport::open_event_stream`).

Shutdown handshake: initiator sends `Bye` and stops sending frames but keeps
reading; the peer flushes anything queued (B: lets the current TTS utterance
finish or cancels per config; A: sends a final `PlaybackProgress`), replies
`Bye`, and closes its write side. The initiator then closes. Hard timeout 3 s
after sending `Bye` → just close. `read_frame_*` returning `Ok(None)` (clean
EOF before a header) after a `Bye` exchange is a clean close; EOF *without*
`Bye` is a link failure → §8 reconnect path on A, session teardown + return
to accept loop on B.

---

## 4. Extending `framed`

Current read paths (`read_frame_sync` / `read_frame_async`) allocate a fresh
`vec![0u8; len]` per frame, and decoding `PcmFrame` allocates its payload
`Vec<u8>`. At 50 mic + 50 TTS frames/s that is ~100 allocations of ≤ 1 KB per
second — **measurably irrelevant** (malloc at this size/rate is tens of
nanoseconds; the ssh crypto costs orders of magnitude more). No zero-copy
rearchitecture is warranted, and the wire format must stay byte-identical, so:

* Keep `encode` / `try_decode` / `read_frame_sync` / `read_frame_async` /
  `write_frame_*` untouched.
* Add one reusable-buffer reader as allocation hygiene for hot loops — same
  bytes, no format change:

```rust
/// Incremental frame reader that owns its scratch buffer. Feed bytes
/// from any source; frames come out. Wire-compatible with encode():
/// this is try_decode with the buffer management folded in.
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self { Self { buf: Vec::with_capacity(8192) } }

    /// Append raw bytes read off the transport.
    pub fn extend(&mut self, bytes: &[u8]) { self.buf.extend_from_slice(bytes); }

    /// Decode the next complete frame, if any. Drains consumed bytes.
    /// Errors are fatal to the stream (same contract as try_decode).
    pub fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>, io::Error> {
        match try_decode::<T>(&self.buf)? {
            Some((value, consumed)) => { self.buf.drain(..consumed); Ok(Some(value)) }
            None => Ok(None),
        }
    }
}
```

This is the shape `chitin_tts_kokoro::net::serve_one` hand-rolls today
(scratch array + `Vec` + `try_decode` + `drain`); lifting it into `framed`
lets the daemon, the bridge tests, and future consumers share it. If
profiling ever shows the `PcmFrame.payload` allocation mattering, the escape
hatch is a borrowed-payload twin struct (`PcmFrameRef<'a>` with
`#[serde(with = "serde_bytes")] &'a [u8]`) deserialized via
`bincode::deserialize` from the retained buffer — byte-identical wire, added
later, only if measured.

One real fix while we're here: `encode()` allocates the full frame and
`write_frame_async` copies it — fine; but the **audio writer should reuse an
encode buffer** via a matching `encode_into(&msg, &mut Vec<u8>)` helper
(clears and fills the caller's Vec). Trivial, same bytes.

---

## 5. The Remote Daemon — `handlr audio-daemon`

New subcommand (subcommands are new to handlr; `main.rs` grows a
`match std::env::args().nth(1)` dispatch before flag parsing — see §6 for the
full list).

**What it runs:** the entire §1 machine-B stack — it is, roughly, today's
`main()` with `CpalCapture` replaced by the wire feed and the sink factory
replaced by `WireAudioSink`. It loads config from `~/.config/handlr/config.toml`
on B (models, TTS engine, Gemini key, agents, herdr socket) exactly as the
interactive binary does.

**Socket path convention** (per-user, per-session):

```
$HOME/.handlr/sockets/                    # 0700, created with fs::DirBuilder + mode
$HOME/.handlr/sockets/audio-<session>.sock  # 0600 after bind
$HOME/.handlr/sockets/audio-<session>.pid   # daemon pid, 0600
```

`<session>` defaults to `default` and comes from `--session <name>` /
`[voice.remote] session` on A, forwarded in the bridge argv. Two concurrent
remote sessions (two laptops, or one laptop with two deliberate sessions) use
distinct session names → distinct sockets → distinct daemons; **no collision
by construction**. A second bridge connecting to an already-served socket is
refused with `Error { message: "session busy (client <session_id>)" }` and an
immediate close — one live audio session per daemon, because one daemon owns
one `VoiceState`/ring/herdr conversation. (Multi-client fan-in is explicitly
deferred, same stance as `chitin_tts_kokoro/net.rs`.)

**Daemonization / auto-start.** The daemon is *not* started by the user; the
bridge starts it on demand:

1. `handlr audio-bridge` tries `UnixStream::connect(socket)`.
2. `ECONNREFUSED` / `ENOENT` (+ `--autostart`): read pidfile; if the pid is
   alive, back off 200 ms × 10 (daemon may be booting). If dead/absent:
   unlink stale socket + pidfile, then spawn
   `handlr audio-daemon --session <name>` detached — double-fork + `setsid`,
   stdin/stdout → `/dev/null`, stderr → `$HOME/.handlr/logs/audio-daemon-<session>.log`
   (mirrors the fork-and-own-a-private-socket pattern the herdr daemon uses;
   plain `std::process::Command` with `pre_exec` setsid, no daemonize crate).
3. Poll-connect up to 30 s (model loading — whisper + Kokoro take seconds),
   emitting `bridge: waiting for daemon (models loading)…` heartbeats on
   stderr so the A-side user sees progress through the ssh stderr channel.

**Lifetime:** `--idle-timeout <secs>` (default 1800). The accept loop tracks
last-disconnect time; with no client for the timeout, the daemon unlinks its
socket + pidfile and exits — models reload on next use. `--idle-timeout 0`
pins it. SIGTERM = graceful (finish current utterance, `Bye` if connected,
unlink, exit).

**Stale-socket handling:** bind-time, the daemon itself does the same
pid-liveness check; a live pidfile aborts startup ("already running"), a dead
one is cleaned. This is the standard pidfile dance the chitin daemons imply
but the bridge-race (two bridges autostarting simultaneously) is closed by
`O_EXCL` pidfile creation — loser of the race just proceeds to poll-connect.

**Security property (stated, relied upon):** the socket directory is `0700`
and the socket `0600` under `$HOME` of the ssh-authenticated user. The only
principals who can connect are processes running as that user (or root) on B
— i.e. exactly whoever could already read the mic stream out of the daemon's
memory. No network listener exists on any machine.

---

## 6. The SSH Bridge — `handlr audio-bridge`

### Subcommand roster

| argv | runs on | role |
|---|---|---|
| `handlr audio-daemon --session <name> [--idle-timeout <s>] [--config <p>]` | B | forked pipeline daemon (§5) |
| `handlr audio-bridge --session <name> [--socket <p>] [--autostart]` | B (invoked via ssh from A) | stdin/stdout ⇄ UDS byte pump |
| `handlr --remote-audio <target-or-transport-name>` | A | thin client: capture + playback + link supervision |

The bridge is deliberately dumb: connect (autostarting per §5), then two
`std::io::copy`-shaped pumps (stdin→socket, socket→stdout) until either side
EOFs, propagating half-closes (`shutdown(Write)` on the socket when stdin
EOFs, exit when the socket EOFs). It never parses frames — version handshake
is end-to-end between A's link and the daemon. Exit codes: 0 clean, 2
connect-failed, 3 autostart-timeout.

### Exact local spawn

Built from the same `RemoteSshConfig` fields (`libherdr::transport`) the herdr
plane uses — ControlMaster multiplexing means the audio channel rides the
already-authenticated master connection with zero extra handshakes:

```
ssh -T \
    -o BatchMode=yes \
    -o ControlMaster=auto \
    -o ControlPath=/tmp/handlr_ssh_%C \
    -o ControlPersist=10m \
    [-p <port>] [-i <identity_file>] [<extra_options>…] \
    <user@host> \
    handlr audio-bridge --session default --autostart
```

Implementation: a `build_audio_ssh_command(&RemoteSshConfig, session: &str) ->
tokio::process::Command` in `src/remote_audio/link.rs`, copying
`HerdrTransport::build_ssh_command`'s flag emission (it is private to
libherdr; we mirror the ~20 lines rather than widening libherdr's API —
revisit if a third copy appears). `stdin/stdout/stderr = piped`,
`kill_on_drop(true)`, and — copied verbatim from `open_event_stream` — the
reader task holds `stdin` and the `Child` for the life of the link so the
remote end never sees a spurious EOF.

### Why the JSON-lines herdr protocol cannot carry audio (and the flags above)

The herdr plane is newline-delimited JSON: it *depends* on `\n` as a frame
delimiter and on payloads being UTF-8 text. PCM payloads are arbitrary bytes —
every 256th byte would be a fake delimiter, and base64-ing costs 33 %
bandwidth plus copies. Length-prefixed bincode has no in-band delimiters and
is 8-bit clean. That 8-bit cleanliness must be preserved end-to-end:

* **`-T`** — never allocate a TTY. A PTY would cook the stream (CR/LF
  translation, ^C/^Z interpretation, flow-control bytes eaten). `-T` +
  `BatchMode=yes` guarantees a raw pipe; ssh itself is 8-bit clean over exec
  channels.
* **stderr stays separate** — remote logs (bridge heartbeats, daemon spawn
  errors, "command not found") arrive on the ssh stderr channel and are
  surfaced line-by-line into A's log with a `remote:` prefix. They can never
  corrupt the frame stream because they are not on stdout. The bridge and
  daemon must never print to stdout.
* No shell quoting hazards: the remote command is a fixed argv with a
  `[A-Za-z0-9_-]`-validated session name.

### Remote binary discovery & version mismatch

* **Presence:** if `handlr` isn't on B's default PATH, ssh exits 127 with
  "command not found" on stderr → A reports "handlr not installed on
  <target>" and (config-gated) falls back per §8. A `remote_command` config
  override (`[voice.remote] remote_command = "/opt/handlr/bin/handlr"`)
  handles non-PATH installs, same spirit as `SshInvocationMethod::CustomCommand`.
* **Version:** carried in `Hello`/`HelloAck` (`protocol_version` + free-text
  build string). v1 policy: `protocol_version` must be equal; on mismatch the
  daemon still sends its `HelloAck` (so *both* sides can log both versions)
  and then `Error { "protocol v{a} vs v{b}: update handlr on <side>" }` and
  closes. Because variants/fields are append-only, most future evolution
  won't bump the version at all — an old peer simply never receives the new
  variants it doesn't know. A peer that *receives* an unknown variant gets a
  bincode decode error and treats it as a protocol error (bincode enums are
  not skippable) — which is exactly why senders must gate new variants on the
  peer's advertised version before using them.

---

## 7. Local and Remote Integration

### New handlr files

```
src/remote_audio/mod.rs       — module root, shared consts (session-name validation)
src/remote_audio/link.rs      — RemoteAudioLink (A-side), build_audio_ssh_command
src/remote_audio/local.rs     — A-side capture/playback glue, WireSttMirror
src/remote_audio/daemon.rs    — run_audio_daemon() (B-side accept loop + pipeline)
src/remote_audio/bridge.rs    — run_audio_bridge() (byte pump + autostart)
src/remote_audio/sink.rs      — WireAudioSink (B-side AudioSink impl)
src/remote_audio/loopback.rs  — #[cfg(test)] in-process UnixStream::pair harness
```

### A-side (`src/remote_audio/link.rs`, `local.rs`)

```rust
/// Owns the ssh child and the framed connection to the remote daemon.
pub struct RemoteAudioLink {
    to_wire: tokio::sync::mpsc::Sender<AudioToRemote>, // control-plane sends
    mic_queue: Arc<MicQueue>,       // bounded drop-oldest deque, fed by capture cb
    state_mirror: Arc<StateMirror>, // last StateSync (atomics)
    // writer task, reader task, held Child + ChildStdin, reconnect supervisor
}
```

* **Capture:** the existing `CpalCapture::start_with_processor` closure
  changes from `ring.push_samples(samples)` to
  `if !mirror.muted() { mic_queue.push(samples) }` — f32→i16 conversion and
  20 ms coalescing happen in the writer task, off the audio thread.
* **Playback:** the reader task feeds `Pcm(STREAM_TTS)` payloads into a
  locally-constructed `audio_core::CpalAudioSink::new(rate, ch)` (from the
  negotiated `playback_format`) via its existing `AudioSink::write(&[i16])`;
  `FlushPlayback`/`Tts(Cancelled)` call `reset()`. A watermark task sends
  `PlaybackProgress` every 250 ms derived from bytes-written minus
  queue-depth (queue depth needs a small `len()` accessor added to
  `CpalAudioSink` — one method, additive).
* **Confirmation UI:** the reader task handles `UtteranceStart` by
  recording the live `utterance_id → text` mapping — so "is this TTS frame
  part of a confirmation prompt?" is a field lookup, not a heuristic — and,
  when `challenge` is `Some`, handing it to a `ConfirmUi` hook owned by
  `RemoteAudioLink`. v1's `ConfirmUi` impl lives in `local.rs`: it prints
  the prompt text and command to the terminal (`confirm? [y/N]: <command>`)
  and reads a single keypress; the trait seam is the point — a tray dialog
  or TUI overlay slots in later without touching the protocol. Accept or
  decline sends `AudioToRemote::ConfirmResponse { nonce, accept }`;
  `ConfirmResolved` dismisses the prompt, which covers the user answering
  by voice while the dialog is up.
* **Mirrors:** `WireSttMirror` implements nothing today beyond logging
  `AsrEvent`s at info level; it is the future seam for local UI and satisfies
  "a remote `SttCallbacks` implementation on the local side" without
  pretending A has an `SttSession` (in the primary seam it doesn't — there is
  deliberately no `SttBackend::Remote` in v1; that construct belongs to
  degraded mode 2 and is deferred, §11).

### B-side (`src/remote_audio/daemon.rs`, `sink.rs`)

`run_audio_daemon()` reuses `main.rs`'s existing setup path (extract the
pipeline-construction block into `handlr::app::build_pipeline(config, opts)`
so both entry points share it — the largest refactor in this plan and worth
it), with two substitutions:

```rust
/// AudioSink that frames i16 PCM onto the wire instead of a device.
/// Constructed by the sink factory: (rate, channels) come from the
/// engine, and must equal the negotiated playback_format (else the
/// daemon renegotiates by erroring the Hello — v1 keeps it strict).
pub struct WireAudioSink {
    conn: WireHandle,           // shared framed writer (Mutex<UnixStream> clone)
    seq: AtomicU64,
    peer_played: Arc<AtomicU64>, // updated from PlaybackProgress
    rate: u32,
    channels: u16,
    utterance: Arc<AtomicU32>,  // current utterance epoch, stamped on every frame
}
impl AudioSink for WireAudioSink {
    fn write(&self, samples: &[i16]) -> Result<(), SinkError>  { /* slice to 20 ms frames, throttle at 2 s lookahead */ }
    fn drain(&self) -> Result<(), SinkError>  { /* block until peer_played == last seq, timeout audio_len + 3 s */ }
    fn reset(&self) -> Result<(), SinkError>  { /* send FlushPlayback, jump watermark */ }
    fn sample_rate(&self) -> u32 { self.rate }
    fn channels(&self) -> u16 { self.channels }
}
```

* **Capture source:** the connection reader converts `Pcm(STREAM_MIC)`
  payloads i16→f32 and calls `ensure_audio_ring().push_samples(&f32)` — the
  ring, pull thread, VAD, mute logic, and `skip_to_end()` flushes are used
  *unmodified*, which is the whole point of the seam.
* **Pipeline wiring:** `SpeechToSpeechBuilder` exactly as `main.rs`, plus
  `.with_sink_factory(move |rate, ch| Arc::new(WireAudioSink::new(conn, rate, ch)))`
  and `.with_stt_callbacks(WireMirrorCallbacks { conn })` (forwards each
  `AsrEvent` as `AudioToLocal::Asr`); a `ManagerCallbacks` impl forwards
  Started/Finished/Cancelled as `AudioToLocal::Tts`, and on each Started it
  bumps the utterance epoch, stamps it into `WireAudioSink.utterance`, and
  emits `AudioToLocal::UtteranceStart` with the `SpeakRequest`'s text and
  String utterance id — attaching the pending challenge (below) if one is
  stashed. `StateSync` is emitted from a small wrapper the daemon installs
  around its `VoiceState` mutations plus a 250 ms poll of the atomics
  (cheap, catches mutations made deep in libconversate like TTS-driven
  mute release).
* **Challenge wiring:** the arm site is the same one `src/main.rs:523–529`
  has today (shared via `build_pipeline`): when `parse_confirmation_request`
  fires, `vs.arm_confirmation(req.pane_id, req.command)` returns the minted
  nonce, and the daemon stashes `ConfirmChallenge { nonce, pane_id,
  command }` as pending for the next STREAM_TTS utterance — which is the
  synthesized confirmation question, since the reply is spoken immediately
  after arming. The connection reader handles `ConfirmResponse` by calling
  `vs.answer_challenge(nonce, accept)` and, on any settlement (either
  answer path, reset, timeout), sends `ConfirmResolved`. The
  `HerdrToolExecutor` consumption path (`src/transport.rs:272–279`,
  `verify_and_consume_confirmation`) is untouched — it sees `Confirmed`
  the same way whichever path produced it.
* **Herdr:** `HerdrTransport::local(default_local_socket_path())` on B (or
  B's own config transport) — remote-audio mode and `--remote` herdr mode are
  mutually exclusive by construction.

### CLI / config surface

CLI (A): `--remote-audio <target>` where `<target>` is a `user@host` string
*or* the name of a `[[transports]]` entry (matching `find_transport`'s
name-or-target lookup). `--remote-audio` and `--remote` are mutually
exclusive (error out; the daemon owns herdr).

Config, following the `[[transports]]` array-of-tables idiom — audio settings
extend the transport entry they ride on, since they share the ssh identity:

```toml
[[transports]]
name = "mini"
mode = "remote"
remote = "tmeade@timothys-mac-mini-1"
# ── remote audio plane (new; all optional) ──
audio = true                  # `handlr -t mini` implies --remote-audio
audio_session = "default"     # remote daemon session name
audio_socket = "~/.handlr/sockets/audio-default.sock"  # override remote path
audio_idle_timeout = 1800     # forwarded to autostarted daemon
# remote_command = "/opt/handlr/bin/handlr"
```

`TransportConfig` gains `audio: Option<bool>`, `audio_session: Option<String>`,
`audio_socket: Option<String>`, `audio_idle_timeout: Option<u64>`,
`remote_command: Option<String>` — all `Option` so existing configs parse
unchanged; `config.default.toml` documents them commented-out, as it does
today's optional keys.

---

## 8. Reconnection & Failure Modes

* **ssh drops mid-utterance.** A's reader sees EOF/`Err` without `Bye` →
  link-down. A: stop draining `mic_queue` (it keeps rolling, drop-oldest, so
  up to 640 ms of pre-drop speech survives a fast reconnect), `reset()` the
  local sink (don't finish playing a sentence from a dead brain), and
  supervise reconnect with backoff 500 ms → 10 s (jittered), each attempt a
  fresh bridge spawn. B: on read error, feed 1 s of zeros into the ring (lets
  the VAD close any in-flight utterance so the *transcript still processes* —
  a command spoken as the link died is honored), then `ring.skip_to_end()`,
  cancel current TTS via the manager, and return to the accept loop. The
  armed/confirmed gate state is **kept** — it's B-local and a reconnecting
  user should be able to say "yes" to the question they already heard;
  `reset_confirmation_gate()` fires only if the disconnect outlasts 60 s.
  The outstanding *challenge* is not kept: the nonce dies with the session
  (§3.6 — it must not survive an epoch change), and when the reconnected
  session's `HelloAck` lands with a still-armed gate, B mints a fresh nonce
  and re-issues the challenge (same text, new nonce) over the new epoch, so
  the local dialog reappears instead of holding a token from a dead
  connection. A `ConfirmResponse` quoting the pre-drop nonce is refused.
* **Remote daemon restart.** Bridge exits (socket EOF) → same reconnect path;
  `--autostart` respawns the daemon; models reload (tens of seconds) while
  the bridge heartbeats on stderr. Session history: the daemon's
  `SessionManager` persists as it does for the local binary, so conversation
  context survives daemon restarts for free.
* **Format renegotiation on reconnect.** Every reconnect is a fresh
  `Hello`/`HelloAck` with a fresh epoch — formats are renegotiated by
  construction, never mid-stream. If B's config changed under us (e.g. TTS
  engine swapped to a different rate) the new `playback_format` simply
  differs and A builds a new `CpalAudioSink`.
* **What the user hears.** Link-down: a short local earcon (two descending
  sine blips, generated in `local.rs` — A has no TTS) and log line;
  reconnect: two ascending blips. During the outage the mic is effectively
  dead and playback is silent — no fake "still listening" cues. Sustained
  `Gap` reports (> 5 % concealed over 30 s) log a warning; B may optionally
  speak "the audio link is degraded" (config `announce_link_quality`,
  default off — it interrupts).
* **Fallback to local processing:** **not automatic.** Silently moving the
  reasoning engine — and therefore *which herdr instance executes commands* —
  because a link blipped is a surprise with security texture. If A has local
  models, the user restarts without `--remote-audio` (or with `-t local`).
  The failure mode is explicit silence + earcon, not a quiet brain
  transplant.

---

## 9. Security Considerations

* **Authentication is ssh's, entirely and explicitly.** The protocol adds no
  authentication of its own. What ssh gives us: the peer on B is a process
  running as the ssh-authenticated user; transport confidentiality and
  integrity; replay protection at the transport layer. What it does *not*
  cover: anything already running as that user on either machine (a
  same-user process on B can connect to the UDS and become the "session" —
  it could equally ptrace the daemon; UDS `0600` + dir `0700` is the correct
  and sufficient posture, same as `herdr.sock`), and it does not authorize
  *which* remote hosts may be brains — that's the user's config/argv choice.
* **Confirmation-gate trust boundary.** Restating §3.6 as checkable
  properties: `GateState` itself never crosses the wire. B tells A *that* a
  question is pending (`ConfirmChallenge`), and the only thing A can say
  back is *yes or no to that question* (`ConfirmResponse`) by quoting a
  128-bit CSPRNG nonce that B minted for this arming, verifies and consumes
  under its own `Mutex<GateState>`, and destroys on any gate transition or
  epoch change. Compare the two answer routes honestly:
  * The **voice route** accepts *any audio that transcribes to an
    affirmative* — "yes", "yeah", "confirm", "do it", "proceed", … per the
    list in `src/main.rs:476–486`. No secret is involved; every sound
    source near the mic (or any process feeding the mic stream) already
    has this capability.
  * The **nonce route** requires possession of a fresh, single-use secret
    that only ever traverses the ssh-authenticated channel and B's 0600
    UDS. Guessing is hopeless (2⁻¹²⁸ per armed window); replay fails per
    §3.6 — consumed on first use, bound to the specific armed gate, bound
    to the epoch, and there is no store-and-forward surface to hold a
    response for later.
  A nonce-carrying UI response is therefore *at least as* trustworthy as
  the voice route it supplements. What is newly possible, stated plainly:
  a compromised machine A can now confirm **without producing audio**,
  where before it would have had to inject PCM of a spoken "yes". That is
  acceptable because A already fully controls the microphone — a process
  positioned to send `ConfirmResponse` can equally send synthesized "yes"
  audio and pass transcription. The nonce widens *no one's* ability to
  confirm; it removes an audio-shaped speed bump for an attacker who had
  already won A. The audit rule for reviewers of future protocol changes:
  `ConfirmResponse` is the **only** state-mutating wire message, it is
  inert without a live challenge B minted, and any newly appended variant
  that mutates `VoiceState` must be either capability-gated the same way
  or strictly fail-safe (e.g. mute), with the same scrutiny as a new
  `run_command` path.
* **Microphone audio leaves machine A.** New for handlr and worth friction:
  remote audio activates only via explicit `--remote-audio` or a transport
  with `audio = true` — never by default, never by fallback. First activation
  per target logs a prominent "streaming microphone to <target>" line. Frames
  exist transiently (ssh buffers, B's ring — trimmed at 5 s by
  `TRIM_THRESHOLD`); the daemon writes no audio to disk. B-side operators
  should be assumed able to hear everything the mic hears while unmuted —
  that is the product, stated plainly.
* **Injection surface.** Wire payloads reach: the ring (bounded, numeric), the
  VAD/whisper (adversarial audio → adversarial *transcript*, which then faces
  the same allow-list + arbitrary-command validation + confirmation gate as
  local speech — the §3/AGENTS.md gates are the defense, unchanged), and the
  frame decoder (bincode with `MAX_FRAME_BYTES` cap + the 250 ms policy cap;
  malformed frames close the session), and the gate — but only through
  `ConfirmResponse`, which is inert without the live nonce (bullet above).
  The daemon refuses concurrent
  sessions, so a hostile same-user connection contends visibly rather than
  eavesdropping silently.

---

## 10. Phased Implementation Plan

Each phase compiles, passes tests, and is independently demoable.

**Phase 1 — wire vocabulary** (libconversate; ~350 lines)
* `crates/voice_proto/src/audio.rs`: every type in §3.2, `pub mod audio` in
  `lib.rs`; plus the §3.6 `GateState`/`VoiceState` additions in `lib.rs`
  (`Armed.nonce`, `arm_confirmation` returning the nonce,
  `answer_challenge`), with unit tests for consume-once, decline-resets,
  and stale-nonce-after-rearm.
* Tests in-module, matching `asr.rs`'s style: bincode round-trip per variant
  (`round_trip_hello_preserves_formats`, `round_trip_pcm_frame_payload`
  — asserting `utterance_id` survives —
  `round_trip_utterance_start_with_challenge`,
  `round_trip_confirm_response_and_resolved`, `gap/state/progress` field
  checks), `try_decode` short-input `None`, an oversize-header rejection,
  and a **discriminant-pinning test** (encode each variant, assert the
  leading u32 tag bytes) so an accidental reorder fails loudly.
* Demo: `cargo test -p voice_proto`.

**Phase 2 — framed hygiene** (libconversate; ~90 lines)
* `FrameReader` + `encode_into` in `framed.rs`; byte-identity test
  (`FrameReader` over a concatenation of `encode()` outputs, split at every
  possible boundary, yields the same frames).

**Phase 3 — daemon + loopback** (handlr; ~650 lines)
* `src/remote_audio/{mod,daemon,sink}.rs`; extract
  `handlr::app::build_pipeline` from `main.rs`; subcommand dispatch for
  `audio-daemon`.
* `loopback.rs` test harness: `UnixStream::pair()`, test client speaks
  `Hello`, streams a WAV of "what time is it" as 20 ms mic frames into a
  daemon built with `--mock` reasoning + `Sine` TTS, asserts
  `Asr(Transcript)` mirror and `Pcm(STREAM_TTS)` frames come back, asserts
  seq continuity, `UtteranceStart`-before-first-frame ordering with matching
  `utterance_id`s, and drain semantics. **No ssh, no mic, CI-safe.**
* Challenge/response tests against the daemon's real `VoiceState` path
  (§3.6): a mock reply ending in "?" arms the gate and the loopback client
  receives `UtteranceStart` with text + `Some(challenge)`; accept and
  decline round-trips through `ConfirmResponse` (accept → the executor's
  `verify_and_consume_confirmation` succeeds; decline → gate is `Idle`
  immediately); `ConfirmResolved` emitted on every settlement path. Negative
  tests, each asserting the gate is unchanged and no `Confirmed` results:
  **wrong nonce** (random u128), **replayed nonce** (second response after
  consumption), **nonce from a previous epoch** (drop and reconnect the
  loopback pair mid-challenge, answer with the pre-drop nonce, assert the
  re-issued challenge carries a different one), **nonce for a different
  pane/command** (re-arm, answer with the superseded nonce).
* Demo: `cargo test remote_audio::loopback` + run `handlr audio-daemon` and
  poke it with the test client binary.

**Phase 4 — bridge + link** (handlr; ~450 lines)
* `bridge.rs` (pump + autostart + pidfile dance), `link.rs`
  (`RemoteAudioLink`, `build_audio_ssh_command`, reconnect supervisor),
  subcommand dispatch for `audio-bridge`.
* Tests: bridge pumped via `socat`-equivalent (spawn `handlr audio-bridge`
  as a child with piped stdio against a phase-3 daemon — the "mock ssh" is
  just *not having ssh*, since the bridge only sees stdio); autostart race
  test (two bridges, one daemon survives); stale-pidfile test.
* Demo: `ssh localhost handlr audio-bridge --autostart` end-to-end on one
  machine.

**Phase 5 — A-side integration** (handlr; ~350 lines)
* `local.rs`, `--remote-audio` flag, `TransportConfig` fields +
  `config.default.toml` doc block, mutual-exclusion with `--remote`,
  earcons, the `ConfirmUi` trait + v1 terminal prompt (§7) wired to
  `ConfirmResponse`/`ConfirmResolved`, `CpalAudioSink::len()`
  (libconversate, ~10 lines).
* Demo: laptop ↔ dev box over real ssh; speak, hear the remote brain answer.

**Phase 6 — hardening + measurement** (~250 lines)
* Reconnect supervisor polish, gap concealment, `StateSync` poller,
  Ping/Pong keepalive, `Gap` accounting, 60 s gate-reset-on-outage.
* **Latency methodology:** (a) `Ping`/`Pong` RTT logged continuously;
  (b) mouth-to-transcript: A logs `timestamp_us` of the frame closing an
  utterance, B's `Transcript` mirror arrives with the event — A computes
  wire+VAD+decode delta against its own clock (both ends of the measurement
  are A-clock: frame send → mirror receive); (c) barge-to-silence: time from
  `FlushPlayback` receipt to sink `reset()` (target < 50 ms); (d) a
  `--loopback-audio` dev flag on the daemon that echoes STREAM_MIC straight
  back as STREAM_TTS for pure-transport A/B (target: RTT + < 25 ms at 20 ms
  frames). Acceptance: added mouth-to-transcript latency over local ≤ RTT +
  40 ms at p95 on a LAN.

Total new code ≈ 2,200 lines across both repos, ~40 % of it tests.

---

## 11. Open Questions / Deferred Decisions

1. **Opus/FLAC compression.** 32–50 KB/s doesn't need it on LAN/VPN; over a
   phone tether it might. *Recommendation: defer; the negotiation hook
   already exists (`SampleFormat` is append-only — add `OpusFloat` later and
   gate on `Hello`).*
2. **Local VAD pre-gating (degraded mode 2).** Streaming silence 24/7 is the
   main bandwidth cost. A silero pass on A used only as a transmit gate
   (with 300 ms pre-roll) would cut idle traffic ~95 %, at the cost of a
   model on A and the confirmation-mode sync wrinkle. *Recommendation: build
   after v1 ships, behind `audio_tx_gate = true`; keep B's VAD authoritative
   regardless.*
3. **Multi-session / observer connections** (a second read-only client
   watching mirrors). *Recommendation: keep refusing; add a separate
   `observe` hello capability if a real need appears — appended variant.*
4. **Sharing `build_ssh_command` with libherdr** instead of mirroring ~20
   lines. *Recommendation: mirror now; extract a `pub fn ssh_command_base`
   in libherdr the third time it's needed.*
5. **A-side hardware mute button** (an A→B state *request*). The wire
   already carries exactly one state-mutation message — the nonce-gated
   `ConfirmResponse` (§3.6) — so the audit rule is not "zero mutation
   messages" but §9's: every mutating variant is capability-gated or
   strictly fail-safe. A mute request is fail-safe (it can only suppress
   input) and would pass that bar trivially. *Recommendation: still v1.1 —
   for scope, not safety.*
6. **Which side renders earcons long-term** — local sine blips (v1) vs.
   B-synthesized status speech. *Recommendation: keep link-status sounds
   local forever; a dead link can't announce itself.*
7. **`SttBackend::Remote` for post-VAD utterance forwarding** (degraded mode
   between raw-PCM and transcripts). *Recommendation: don't build it; the
   two existing endpoints (raw PCM here, `--remote` for transcripts) bracket
   the useful space.*
8. **UI-confirmation-only mode.** A config knob (`require_ui_confirmation =
   true`) making B ignore transcribed affirmatives and accept only
   `ConfirmResponse` — strictly stronger than voice, since a hot mic, a TV,
   or a passer-by saying "yes" (§9's affirmative list) could no longer
   confirm anything. *Recommendation: cheap and attractive once the
   phase-5 `ConfirmUi` exists; build it then, default off.*
