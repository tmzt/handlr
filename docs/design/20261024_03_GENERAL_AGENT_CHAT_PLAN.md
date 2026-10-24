---
title: The general agent as a realtime chat, from its own centre button
date: 2026-10-24
status: Draft
covers:
  - "Splitting the controls: a centre button for the general agent
    (Gemini), apart from the record and submit flow that messages coding
    agents"
  - "A realtime chat with the general agent: listening continuously,
    words shown as they are heard, replies streamed and spoken as they
    come, barge-in"
  - "The daemon brokering it: the phone's speech-to-text and
    text-to-speech stay on the phone; the daemon holds the Gemini key,
    the context and the tools"
  - "The general agent knowing which coding agent is focused, and what
    it is doing"
  - "Not covered: Gemini's Live API (audio to the model directly), which
    the owner set aside for the daemon's control; the coding agents'
    record and submit flow, unchanged"
sources:
  - "Owner (2026-10-24): \"I want to split the agent controls, so a new
    center button triggers the general agent which also knows if any
    coding agent is focused, this would have a realtime chat, not the
    record/submit flow. for now, turning off general agent stops all
    user-directed Gemini turns (summaries are the daemon's concern)\";
    asked whether Gemini's Live API on the phone or the daemon brokering:
    \"daemon, it wouldn't be the same live experience but we have more
    control\""
  - "handlr at `bffe1d7`: `handlr.companion` v1 (`turn.start {turn_id,
    text}`, `turn.say`, `turn.interrupt`, `TurnOutcome`) in
    `crates/libhandlr_proto/src/companion.rs`; the reasoning engine
    (`src/reasoning.rs`, `libinfer`, `libinfer-gemini`); the phone's
    speech in `crates/handlr-audio` (libvoicexn 0.2.0: Kyutai streaming
    STT, the earshot VAD, Phonon TTS); the Gemini strip and overlay
    (plan 20261023_04, A5)"
  - "The direction in memory: voice moves to the companion apps on
    libvoicexn"
related:
  - docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (the general agent toggle)
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (the Gemini strip and overlay, A5)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (L3, the agent view's controls)
---

# The general agent as a realtime chat, from its own centre button

## 1. Goal

Today one record button serves two different things: dictating a message
to a coding agent, and asking Gemini to act. Both use record, then
submit, and Gemini answers only once the turn is over. After this plan,
the controls are split. Record and submit stay for coding agents. A new
centre button opens a realtime chat with the general agent: it listens
continuously, shows your words as you speak, streams and speaks its
reply as it comes, and stops talking when you talk over it. The general
agent always knows which coding agent is focused and what it is doing.

## 2. Decisions

1. **The centre button.** The controls row gets a round centre button,
   larger than its neighbours, in the Gemini accent, on the overview and
   in the agent view. Tapping it opens the chat; tapping it again, or
   saying "that's all", closes it. Record (left) and submit/message
   (right) keep their roles for coding agents. With the general agent
   off (the settings plan), the centre button is hidden.
2. **The daemon brokers it.** Audio stays on the phone:
   - the phone's streaming speech-to-text (libvoicexn's Kyutai STT)
     sends partial and final transcripts;
   - the daemon runs Gemini with streaming responses, with the key, the
     system prompt (`PROMPT.md`), the context and the tools;
   - the daemon streams the reply's text back, sentence by sentence;
   - the phone speaks each sentence as it arrives (Phonon TTS).

   This is not Gemini Live's audio experience, but the daemon keeps
   control: what context goes in, which tools run, and what is logged.
3. **The chat protocol,** `handlr.companion` v1 additions (a patch
   release by the owner's rulings; new methods and notifications only):
   - `chat.open {}` → `{chat_id}`; `chat.close {chat_id}`.
   - `chat.heard {chat_id, seq, text, final}` (phone → daemon,
     notification): the words so far of one utterance; `final` ends it.
     The daemon starts a reply on a final utterance.
   - `chat.reply {chat_id, reply_id, seq, text, done}` (daemon → phone,
     notification): a reply's next sentence; `done` ends it.
   - `chat.barge {chat_id, reply_id}` (phone → daemon): the user spoke over
     the reply. The daemon stops the model's stream and drops what was
     not yet sent. The phone stops speaking at once, without waiting.
   - `chat.action {chat_id, text}` (daemon → phone): a line saying what a
     tool did ("Sent 'continue' to the handlr agent"), shown in the
     transcript.
4. **Barge-in on the phone.** While a reply is spoken, the VAD keeps
   listening. Speech above the threshold for 300 ms stops the speech and
   sends `chat.barge`, and the words go on as the next utterance. The
   phone's echo cancellation, if the platform gives it, is used for the
   microphone; otherwise, the barge threshold is raised while speaking.
5. **Context: the focused coding agent.** Each reply's prompt carries
   herdr's focus. That is the focused pane's agent, its name, its turn
   state (working, stopped, asking), its pending question and its newest
   sections (up to 2,000 characters, redacted as for the phone), and a
   one-line list of the other agents and their states. When no coding
   agent is focused, it says so. The existing tools (send to a pane,
   answer a question, focus, list) work as now, and every confirmation
   rule (the confirmation gate, `allowed_commands`) applies unchanged.
6. **The transcript.** The Gemini overlay (plan 20261023_04, A5) shows
   the chat live:
   - your partial words in dim text, turning to normal when final;
   - the reply as it streams;
   - action lines in the accent.

   Closing the chat leaves the transcript in the overlay, as now.
7. **The general agent toggle** (the settings plan, as amended at the
   same time as this plan): while it is off, `chat.open` is refused with
   `GENERAL_AGENT_OFF`, and the daemon starts no user-directed Gemini
   turn. Spoken summaries are the daemon's concern and are not affected.
8. **Old turns.** `turn.start` stays for the record and submit flow until
   this is built, then is used only for messages that go straight to
   coding agents. Whether it remains for Gemini at all is open question 3.

## 3. Steps

1. **The protocol:** the chat methods and notifications, and
   `GENERAL_AGENT_OFF`, with the libhandlr_proto patch release and its
   VERSIONS.md entry.
   **Done when:** each round-trips, and existing messages read as
   before.
2. **The daemon:** chats on `libinfer-gemini` with streaming,
   sentence-split replies, barge stopping the stream, the focused agent's
   context, the tools and their gates, and `chat.action` lines.
   **Done when:** daemon tests (Gemini mocked) show:
   - a final utterance gets a streamed reply in sentences;
   - a barge stops it, and nothing more is sent;
   - the prompt carries the focused agent's state and pending question,
     and says so when nothing is focused;
   - a tool call yields an action line and passes the confirmation gate
     as before;
   - a closed chat sends nothing;
   - `chat.open` is refused with the general agent off.
3. **The client:** chat state, partial and final transcripts from
   `handlr-audio`'s streaming STT, replies queued to speech, barge from
   the VAD, and `Event::Chat` for the app.
   **Done when:** client tests show:
   - opening and closing;
   - partial words sent with rising `seq`, and a final;
   - replies spoken in order;
   - a barge that stops speech and sends `chat.barge`;
   - a refused open shown as such.
4. **The app:** the centre button, the split controls, and the live
   transcript in the overlay.
   **Done when:**
   - app tests place the three buttons, open and close the chat, draw
     the transcript's partial, final, reply and action lines, and hide
     the centre button with the general agent off;
   - on the S23, the owner talks to the general agent with a coding
     agent focused, hears the reply begin before it is complete, and
     interrupts it mid-sentence.

Each step is one crate or subproject, one commit (Rule 3.1). 2 needs 1; 3
needs 1; 4 needs 3. The settings plan's general agent toggle (its step 4
and 5) can land before this; the centre button then reads the same
setting.

## 4. Open questions for the owner

1. **Opening the chat:** a tap that leaves it open until closed (as
   written), or press-and-hold to talk?
2. **Wake word:** later, should "hey handlr" open the chat hands-free, or
   is the button enough?
3. **The old Gemini turns:** once the chat exists, should the record
   button stop going to Gemini entirely (messages to coding agents only),
   or keep a fallback when no coding agent is in view?
4. **Model:** the configured `[gemini] model`, or a faster one for chat?

## Amendment — 2026-10-24 11:40 EDT: tap for one exchange, long press for a continuous chat; the centre button on every screen, an X while voice input can be cancelled

The owner (2026-10-24), answering open question 1: "both, long press
toggle"; then "that should be its own plan, just the parts that affect
settings and overview in theirs"; "right, but it's on all screens, only
replaced with X when it means cancel voice input". This replaces decision
1's "tapping it opens the chat; tapping it again, or saying 'that's all',
closes it", and its placement "on the overview and in the agent view".

1. **Tap: one exchange.** The chat opens and listens until the utterance
   ends. The reply streams and is spoken, barge-in works, and the chat
   closes after the reply.
2. **Long press (500 ms or more): a continuous chat.** It stays open
   across exchanges, and listening resumes after each reply. The centre
   button shows a steady ring. A short haptic tick marks the long press.
   Another long press, a tap, or "that's all" closes it.
3. **On every screen.** The centre button is in the middle of every
   screen's controls row: the overview, the agent view, and any later
   screen with one.
4. **An X while voice input can be cancelled.** While voice input is
   being taken, the centre button is drawn as an X, and tapping it
   cancels that input:
   - while dictating a message to a coding agent, it discards the
     dictation (in place of the dictation's own cancel button);
   - while the chat hears an utterance, it discards the utterance. A
     single exchange then closes; a continuous chat goes back to
     listening.

   At any other time it is the general agent's button.
5. **Where each screen's part lives.** The overview's controls row and
   its Gemini strip and overlay are plan 20261023_04's (frozen; recorded
   as a departure in CHANGES.md). The agent view's controls row is plan
   20261022_03's living section L3 (amended at the same time). The toggle
   is the settings plan's. Decisions 1 and 6 here stand for the chat's
   behaviour, and those documents hold each screen's layout.

Step 4's tests also cover the tap's single exchange, the long press's
continuous chat and its closing, the X during dictation and during an
utterance, and the button on both screens.
