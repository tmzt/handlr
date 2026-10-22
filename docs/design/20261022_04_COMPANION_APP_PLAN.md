---
title: The companion app, on handlr-companion and libvoicexn
date: 2026-10-22
status: Draft
covers:
  - `crates/handlr-companion`: the companion client's logic, with no audio
    and no UI. It covers pairing, the link connection, turns, the Esc-like
    interrupt, approving and denying by voice, notice queueing and key
    storage behind a trait
  - `apps/companion-tty`: a terminal front end on libvoicexn, for the
    desktop
  - `apps/companion`: a libmorph front end on libvoicexn, for a desktop
    window and for Android
  - Not covered: secure key storage on Android (a libmobile plan); scanning
    the QR code with the camera; changing the STT vocabulary without
    reloading the model
sources:
  - Owner's discussion (2026-10-22): the companion app is built in this
    repository on libvoicexn, as logic only in `crates/handlr-companion`,
    with front ends in `apps/companion` and `apps/companion-tty` (or a
    TUI); the desktop voice loop is replaced by the same code as the
    mobile app; voicexn implements the voice side of confirmations, with
    "approve" or "approved" as the answer.
  - libvoicexn 0.2.0 at `595197e` (no git remote yet): `TtsWorker` with
    `TtsRequest::{Speak, Stop}` and `TtsEvent::{Started, FirstAudio,
    Finished, Stopped, …}`; `SttWorker` with `SttEvent::{Partial,
    Utterance, Gate, …}` and `SttRequest::{Arm, Close}` under
    `vad-earshot`; `SttConfig::vocab`, a phrase file read when the model
    loads, under `stt-moshi-xn-boost`; the front ends `examples/voicexn`
    (libmorph) and `examples/voicexn-tty` (cpal, Enter as the tap).
  - The companion protocol and link of plan 20261022_02 and its
    amendments, and the confirmation phrases and labels of plan 20261022_01's
    12:06 EDT amendment.
related:
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (this depends on it)
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md
---

# The companion app, on handlr-companion and libvoicexn

## 1. Goal

One body of client code talks to handlr, both on the desktop and on the
phone. You pair it once with a pasted string. Then you tap or press Enter
and say what you want, and you hear the answer. When handlr asks you to
confirm a command, you say "approve". A tap while it is thinking or
speaking interrupts it. The logic lives in a crate with no audio and no UI,
so it is tested in handlr's ordinary build. The front ends only join it to
libvoicexn's microphone, STT and TTS, and to a terminal or a screen.

## 2. Decisions

1. **Layout.**
   - `crates/handlr-companion` is a member of the root workspace. It
     depends on `libhandlr_proto` (with `link`), `serde`, `serde_json`,
     `tokio` (`rt`, `net`, `time`, `sync`, `io-util`) and `log`. It has no
     libvoicexn, no libmorph and no audio crate, so `cargo test` at the
     root covers it.
   - `apps/` is a workspace of its own (`apps/Cargo.toml`, members
     `companion` and `companion-tty`), and the root manifest excludes it
     with `exclude = ["apps"]`. Cargo loads every member's manifest, even
     ones outside `default-members`, so a member that depends on a missing
     sibling checkout breaks every build. That is what the missing
     `../libconversate` did to handlr. A separate workspace keeps the ML
     stack and the Android tooling out of handlr's own build.
   - The apps depend on `handlr-companion` by path
     (`../../crates/handlr-companion`). They pin libvoicexn by git
     revision once it has a remote. Until then they use `path =
     "../../../libvoicexn"`, which can only break the `apps` workspace.

2. **The client is a worker**, in libvoicexn's style: a `net` thread runs
   a Tokio current-thread runtime, and the app talks to it over channels.

   ```rust
   pub struct Companion { /* the net thread and its two channels */ }

   impl Companion {
       pub fn spawn(config: CompanionConfig, keys: Box<dyn KeyStore>) -> Self;
       pub fn send(&self, input: Input);
       pub fn poll(&self) -> impl Iterator<Item = Event> + '_;
   }

   pub enum Input {
       Pair(String),          // a handlr1… pairing string
       Utterance(String),     // what STT heard, or a typed line
       Tap,                   // the tap or Enter
       Approve,
       Deny,                  // buttons, where the front end has them
       Speaking(bool),        // TTS started or finished
       Listening(bool),       // the STT gate opened or went idle
   }

   pub enum Event {
       Connected { server: String },
       Disconnected { retry_in_ms: u32 },
       Paired { name: String },
       PairFailed(String),
       Say(String),           // speak this
       StopSpeaking,          // stop playback now
       Arm,                   // arm the STT gate
       Confirm { text: String, label: Option<String>, phrase: String },
       Resolved { outcome: String },
       TurnEnded { outcome: String },
       Vocabulary(Vec<String>),
       Error(String),
   }
   ```

   `CompanionConfig` holds the client name, the voice, and how long to
   back off between reconnects (1, 2, 4 … 30 s, per plan 02).

3. **States**, kept by a pure state machine (`client::State`) that the
   `net` thread drives:
   - `Unpaired`: only `Pair` does anything.
   - `Idle`: connected. `Tap` gives `Arm`, and `Utterance` sends
     `turn.start`.
   - `Waiting { turn_id }`: a turn is running. `Tap` gives `StopSpeaking`
     and sends `turn.interrupt`. `Utterance` is ignored, because the gate
     is closed while waiting and an utterance can only arrive late.
   - `Confirming { confirm_id }`: a `confirm.requested` arrived, and its
     `text` has been given as `Say`. The next `Utterance` is matched as
     plan 02's amendment says. "approve" or "approved" sends `confirm.reply
     {approve: true}`. Anything else sends `approve: false` and then
     `turn.start` with the utterance. `Approve` and `Deny` do the same
     without matching. `Tap` arms, so the answer can be heard.

   `turn.say` always gives `Say`.

4. **Notices** are held while `Speaking(true)` or `Listening(true)` is in
   effect, or in `Confirming`. They are then given as `Say`, oldest first,
   at most 8, dropping the oldest beyond that. This is the phone's half of
   plan 02 decision 12. The daemon's half (holding notices during a
   confirmation) is already on the server.

5. **Keys.**

   ```rust
   pub trait KeyStore: Send {
       fn secret(&mut self) -> Result<Option<[u8; 32]>, String>;   // this client's static key
       fn set_secret(&mut self, key: [u8; 32]) -> Result<(), String>;
       fn server(&mut self) -> Result<Option<ServerRecord>, String>;
       fn set_server(&mut self, server: ServerRecord) -> Result<(), String>;
   }

   pub struct ServerRecord {
       pub name: String,
       pub public_key: [u8; 32],
       pub addresses: Vec<String>,
       pub session_id: Option<String>,
   }
   ```

   `FileKeyStore` keeps the key, base64, in `<dir>/key` with mode 0600, and
   the server record in `<dir>/server.toml`. It refuses to read a key file
   that anyone but its owner can read. The tty and the desktop window use
   it with `~/.config/handlr-companion/`. Android uses a `KeyStore` from
   the libmobile plan.

6. **Vocabulary.** On `Vocabulary`, the front end writes the words to
   `vocab.txt` next to its keys. libvoicexn reads that file when the STT
   model loads (`SttConfig::vocab`, under `stt-moshi-xn-boost`), so new
   names take effect at the next start. Changing the booster while the
   model is running needs a libvoicexn change, and is left for later.

7. **`apps/companion-tty`**, modelled on `voicexn-tty`:
   - cpal opens the default devices. `Mic::push` and `Consumer::fill` are
     its callbacks.
   - Enter is `Tap`.
   - It prints `> ` and what was heard, `< ` and what was said, and `? `
     with the confirmation text, its label and its phrase.
   - Usage: `companion-tty [--pair <handlr1…>] [--quant …] [--voice …]`.
     It is built with `vad-earshot` and `stt-moshi-xn-boost` by default.
   - It is a line-based terminal program, not a full-screen TUI. A TUI can
     replace it later without touching the crate.

8. **`apps/companion`**, modelled on `examples/voicexn`: one libmorph app
   for a desktop window and for Android. Its screen, built with
   libcontrols, shows the connection, the last thing heard and said, and,
   while confirming, the command and label with Approve and Deny buttons.
   The tap anywhere else is `Tap`. You pair it by pasting a `handlr1…`
   string.

## 3. Steps

1. **The logic.** `crates/handlr-companion` with the state machine, the
   notice queue and `FileKeyStore`, added to the root workspace in its
   first commit. Unit tests drive `State` with scripted inputs and server
   messages:
   - a turn and its `Say`;
   - `Tap` while waiting gives `StopSpeaking` and `turn.interrupt`;
   - "approve", "Approved." and "APPROVE" approve, and "yes" and "approve
     it" deny and start a turn;
   - notices wait for speaking, listening and confirming, and the queue is
     capped at 8;
   - a key file at 0644 is refused.

   **Done when:** `cargo test -p handlr-companion` passes, and `cargo tree
   -p handlr-companion -e normal` shows no xn, ptts, libmorph or cpal.

2. **The link client.** The `net` thread: pairing from a string,
   reconnecting with `IK` and the stored `session_id`, and backoff. This
   follows plan 02's steps 3 and 4. `handlr-companion` has a text example
   (`cargo run -p handlr-companion --example text`), in which typed lines
   are utterances and an empty line is `Tap`.

   **Done when:** against a running `handlr serve`, the example pairs from
   the printed string and reconnects after the daemon restarts. "send
   continue to claude", then "approve", sends `continue` once. An empty
   line during a turn ends it as `interrupted`.

3. **The terminal app.** The `apps` workspace and `companion-tty`
   (decision 7).

   **Done when:** on the M4:
   - It pairs from a pasted string.
   - Saying "send continue to claude" is answered aloud with `Approve
     sending "continue" to claude?`.
   - Saying "approve" sends it once.
   - Enter while it is speaking stops the speech within 100 ms and
     interrupts the turn.
   - A watched agent's notice is spoken only after the reply has finished.
   - `cargo build` at the repository root does not build any of
     `apps/`.

4. **The libmorph app.** `apps/companion` (decision 8), first as a desktop
   window, then on Android. This step waits for the libmobile plan for
   secure key storage.

   **Done when:** the desktop window passes step 3's checks with the tap
   and the buttons. On the S23 Ultra, the app pairs with a pasted string
   over Tailscale, keeps its key in secure storage, and completes a
   spoken turn, an approval and an interrupt.

## 4. Open questions

1. Plan 02's step 3 and 4 example client in `libhandlr_proto` overlaps with
   this crate's text example. Should plan 02 be amended so that step 4's
   text mode lives here instead?
2. Should "approve" also be accepted inside a longer utterance ("ok,
   approve")? This plan requires the whole utterance, which is the safer
   check on what was heard.
3. Is a full-screen TUI wanted later, for example showing the agents and
   their status beside the conversation?
4. When will libvoicexn get a remote, so the apps can pin it by revision?

## Amendment — 2026-10-22 16:23 EDT: pairing the libmorph app, key storage, runtime vocabulary, libvoicexn pin

Agreed with the owner. This amendment changes decisions 1, 6 and 8 and
step 4, and settles open questions 1 and 4. Where it disagrees with
earlier text, this amendment wins.

- **libvoicexn pin (decision 1).** libvoicexn is published at
  `https://github.com/tmzt/libvoicexn`. The apps pin the `v0.2.0` commit,
  `32b8032082997a34190b458f79716d998a0d850a`, or a later tagged release.
  The temporary path dependency is not used.
- **Pairing the libmorph app (decision 8).** libmorph has no keyboard,
  text-input or clipboard support yet, so a pasted string cannot reach
  the app.
  - The desktop window takes `--pair <handlr1…>`.
  - On Android, the app reads a pairing string from `pair.txt` in its
    data directory, pushed with `adb`, as voicexn does with its models
    and `vocab.txt`. The app deletes the file once pairing succeeds or
    fails.
  - Pasting and QR scanning wait for text input and camera support in
    libmorph.
- **Key storage on Android (step 4).** Android keys need their own plan
  in libmobile or libmorph, for secure storage. Step 4's Android part
  waits for it. There is no file-based fallback on Android in the
  meantime. The terminal app and the desktop window use `FileKeyStore`.
- **Runtime vocabulary (decision 6).** Changing the STT booster while the
  model runs is to be planned in libvoicexn; it has been handed to the
  libvoicexn session. Until then, decision 6 stands: `Vocabulary` writes
  `vocab.txt`, which is read at the next start. Once libvoicexn has it,
  the front ends also apply `Vocabulary` to the running STT worker
  through that API, recorded as an addendum when it is done.
- **Open questions.** Question 1 is settled by plan 02's 16:23 EDT
  amendment: the text example stays in this crate, and plan 02 is tested
  with it. Question 4 is settled by the pin above.
