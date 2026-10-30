---
title: Dictated messages to the focused agent
date: 2026-10-23
status: In implementation
covers:
  - "A bulk input mode in the companion: utterances collect into one
    message, which is submitted to an agent as a single send"
  - "The target is herdr's current focus, chosen with the existing `focus`
    tool. The status data carries that focus, with its project, agent and
    tab label"
  - "A message send class in the daemon, separate from allowed and
    arbitrary commands, which always needs approval"
  - "Not covered: a target picker or dropdown (the owner set it aside); an
    LLM rewrite of the message"
sources:
  - "Owner (2026-10-23): \"a bulk input mode which collects a paragraph and
    then allows it to be submitted\"; target \"with the focus tool\"; \"we
    should have the base name, agent, and tab label displayed … showing the
    current focus if any\"; \"ignore the drop down but include the focus
    data in the status response\""
  - "handlr at `49a9832`:
    - `libhandlr_proto::common` (`AgentInfo` with `focused`, `tab_label`,
      `AgentsListResult`, `agents.focus`, `agents.send`);
    - `src/commands.rs` (`validate_arbitrary_command`: ASCII letters,
      digits, spaces and `. , ' ? :`, at most 256 characters);
    - `src/core/confirm.rs` (the broker);
    - `crates/handlr-companion` (`State`, `Input`, `Event`);
    - `apps/companion` (the card, from plan 20261023_01)"
related:
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (the companion protocol, voice confirmations by the client)
  - docs/design/20261022_04_COMPANION_APP_PLAN.md (the companion's states and UI)
  - docs/design/20261022_05_SPOKEN_NAMES_PLAN.md (shown and spoken names)
  - docs/design/20261023_01_CONFIRMATION_HANDLES_PLAN.md (the card, handles)
---

# Dictated messages to the focused agent

## 1. Goal

Some replies to an agent are a paragraph, not a command: answering an
agent's questions, for example, or describing a change. Today every
utterance is its own Gemini turn, and free-form sends are limited to short
spoken commands.

This plan adds a mode in which the companion collects utterances into one
message, shows it as it grows, and submits it to the focused agent. You
approve it once, and your own words arrive.

## 2. Decisions

1. **The buffer is on the client.**
   - `handlr-companion`'s `State` gains a dictation mode with a buffer.
   - While it is on, each final utterance is added to the buffer, joined
     with a space, instead of starting a turn.
   - Nothing reaches Gemini or the daemon until submit.
   - The buffer is capped at 4000 characters. Past the cap, utterances are
     refused with a short spoken "The message is full."

2. **Words that control the mode.** These are matched on the whole
   utterance, normalised as the companion matches "approve" (plan 04):

   | Utterance | Effect |
   |---|---|
   | "take a message", "dictate" (outside the mode) | start; the target is fixed now (decision 3) |
   | "submit", "send it" | submit (decision 5) |
   | "cancel", "clear" | discard the buffer and leave the mode |
   | "scratch that" | remove the last utterance |
   | "read it back" | speak the buffer, then stay in the mode |
   | "approve", "deny" | still answer a pending confirmation, as now; never added to the buffer |

   - **Anything else** is text.
   - **Buttons:** the app also has Start, Submit, Clear and Scratch.
   - **A tap** while it is speaking stops the speech, as now.
   - **Leaving the mode:** if 10 minutes pass with no utterance, the mode
     ends and the buffer is kept as a draft, which "take a message" offers
     to resume.

3. **The target is herdr's focus, fixed at start.**
   - When the mode starts, the companion takes the focused agent from the
     status data (decision 4) and fixes it for this message.
   - You choose the target beforehand with the existing `focus` tool ("focus
     Claude in highbay").
   - If nothing is focused, or the focused pane has no agent, the mode
     still collects, but Submit is refused until a target is set. "focus
     <agent>" during the mode sets it.
   - If focus moves while dictating, the target does not follow. The card
     shows "focus moved to <name>". Saying "use the focus" retargets
     explicitly.

4. **Focus in the status data.**
   - `AgentsListResult` gains a top-level `focus: Option<FocusInfo>`, the
     pane herdr reports as focused.
   - The same value is pushed when it changes, as a companion notification,
     `focus.changed { focus }`, sent alongside `vocabulary.changed` after
     the daemon's snapshot polls.

   ```rust
   pub struct FocusInfo {
       pub pane_id: String,           // a handle, never shown or spoken
       pub project: Option<String>,   // the base name: basename of the cwd, or the workspace label (plan 05)
       pub agent: Option<String>,     // e.g. "Claude"; None for a pane without an agent
       pub tab_label: Option<String>, // as the user typed it, e.g. "-ad"
       pub tab_number: Option<u32>,
       pub name: String,              // the shown name (plan 05)
       pub spoken: String,            // the spoken name (plan 05)
   }
   ```

   - The fields come from the same snapshot and naming code as `AgentInfo`.
     Both are additive.
   - The card shows "Message for **Claude** · libmorph · tab `-ad`". When
     nothing is focused, it shows "No agent focused — say 'focus …'".

5. **Submitting is one confirmed send, in the message class.**
   - Submit sends `agents.send { pane_id, text, kind: "message", handle }`
     over the companion's connection.
   - The handle is built by the companion from the target's names and the
     message's first words, for example `message-claude-libmorph-please-run`,
     under plan 20261023_01's rules.
   - The daemon opens a confirmation as for any send. The approval question
     is `Approve sending your message to Claude in libmorph?`. The card
     shows the full text.
   - "approve" sends it once; "deny" keeps the buffer, so the message can be
     edited and submitted again. After it is sent, the buffer is cleared.

6. **The message send class.** This is a third class beside allowed commands
   and arbitrary commands (`src/commands.rs`):
   - **Allowed:** printable Unicode text, including punctuation, `/` and
     `-`, at most 4000 characters, with runs of whitespace folded to one
     space.
   - **Refused:** control characters, including line breaks and escape,
     and any text that starts with `/`. A leading slash would run a slash
     command in Claude Code and similar agents, which is the job of allowed
     commands.
   - **Always confirmed,** whatever the agent's `require_confirmation`
     says.
   - **Sent as text** through herdr's `pane.send_text`, then one Enter
     keystroke, as allowed commands are today.
   - **Per-agent switch:** `[[agents]] allow_messages`, default `true` for
     every built-in agent (claude, omp, agy). A pane whose agent has no
     entry is refused, as for any send.

7. **No Gemini.** The text goes as dictated. A later amendment may add an
   optional "fix the transcription" pass. It is out of scope here.

## 3. Steps

1. **The protocol** (`libhandlr_proto`):
   - `FocusInfo`, and `AgentsListResult::focus`;
   - the `focus.changed` notification;
   - `kind` on `agents.send`'s params: `"command"` by default, or
     `"message"`;
   - round-trip tests.

   **Done when:** fmt, clippy and test pass for `-p libhandlr_proto`.
2. **The daemon** (handlr):
   - `focus` filled in from the snapshot, and `focus.changed` sent on
     change;
   - the message class (decision 6), with `validate_message` and its tests:
     unicode text, `/` inside the text, a leading `/` refused, a control
     character refused, 4000 and 4001 characters, whitespace folding;
   - always-confirmed;
   - `allow_messages` in `[[agents]]`.

   **Done when:**
   - fmt, clippy and test pass for `-p handlr`;
   - a `tests/companion.rs` test sends a message to a mock pane, gets one
     confirmation whose question names the agent, approves it, and sees the
     exact text sent once, then Enter;
   - a denied one sends nothing.
3. **The companion logic** (`handlr-companion`):
   - the mode, the buffer, the control words, the draft and the time-out;
   - the fixed target;
   - a `Dictation` event for the UI (buffer, target, state);
   - unit tests driving `State` with scripted utterances, including
     "approve" during dictation.

   **Done when:** fmt, clippy and test pass for `-p handlr-companion`.
4. **The app** (`apps/companion`):
   - the card for dictation: the target line from `FocusInfo`, the text as
     it grows, "focus moved" when it does;
   - Start, Submit, Clear and Scratch buttons.

   **Done when:**
   - the apps gates pass;
   - on the S23: "focus Claude in <tab>", "take a message", three
     sentences, "scratch that", "submit", and "approve" deliver the text
     once to that pane, as dictated, minus the scratched sentence.

## 4. Open questions

1. Are the control words right ("take a message", "submit", "scratch
   that"), or do you prefer others? "send it" may be said inside a message;
   should "submit" be the only spoken submit?
2. Should `allow_messages` default to `true` for the built-in agents, as
   proposed, or `false`, like `allow_arbitrary_commands`?
3. Is 4000 characters the right cap?

## Amendment — 2026-10-23 01:57 EDT: the VAD decides when the message is done

On the owner's instruction: "use the vad to decide when done, at least 8
seconds (configurable) or button pressed".

This replaces three things:
- the "submit" and "send it" rows of decision 2's table;
- decision 2's 10-minute time-out and draft;
- open question 1.

Everything else stands.

**When the message is done.** The message is finished, and submitted as in
decision 5, when either of these happens:
1. **Silence:** the voice-activity gate stays closed for the silence time
   after the last utterance, with no speech in between. The companion
   already gets the gate as `Input::Listening(bool)` from `handlr-audio`'s
   earshot gate. `State` starts the timer when the gate closes, resets it
   when the gate opens, and needs no new audio input. A timer tick
   (`Input::Tick`, or the net thread's clock) fires the submit.
2. **The Submit button** is pressed.

**There is no spoken submit word,** so "send it" inside a message is just
text.

**The silence time:**
- **Default 8 seconds.** It's configurable as
  `CompanionConfig::dictation_silence` (a `Duration`), and set in the app
  with `--dictation-silence <seconds>` on the desktop and a setting on
  Android.
- **Minimum 8 seconds.** Values below 8 s are raised to 8 s, so a breath
  between sentences never submits.
- **Counted from the gate closing,** so time spent speaking never counts.
  Speech that comes back before it runs out continues the same message.
- **The card counts down.** It shows "Submitting in 3 s…" for the last 3
  seconds. Any speech, Scratch or Clear cancels the countdown.

**What still works during the mode:**
- "cancel", "clear", "scratch that" and "read it back", and the buttons;
- "approve" and "deny" still answer a pending confirmation.

Saying "read it back" pauses the timer until the reading ends.

**Submit is still not a send.** Whether by silence or by button, it opens
the single approval of decision 5. "deny" keeps the text, and the mode
resumes, so the message can be fixed and submitted again.

**Tests, added to step 3** (driven with a scripted gate and a fake clock):
- 7.9 s of silence does not submit, and 8.0 s does;
- speech at 7 s resets the timer;
- a configured 5 s is raised to 8 s, and 12 s is honoured;
- the Submit button submits at once;
- "read it back" pauses the timer;
- "deny" resumes the mode with the text kept.

## Amendment — 2026-10-23 01:58 EDT: the strict symbol filter; open questions settled; approved

On the owner's word: "yes, still with strict symbol filter".

**This replaces decision 6's allowed and refused text.** Messages use the
same strict character filter as arbitrary commands
(`validate_arbitrary_command`).
- **Allowed:** ASCII letters, digits, spaces, and the punctuation
  `. , ' ? :`.
- **Not allowed:** `/`, `-`, shell metacharacters, quotes other than
  `'`, and any control character, line breaks included.

What differs from arbitrary commands:
- **The length:** at most 4000 characters, not 256.
- **Always confirmed.**
- **Gated by `allow_messages`,** not `allow_arbitrary_commands`.

Runs of whitespace are folded to one space, and trailing `.` and `,` are
cleaned as for arbitrary commands. A message with a character outside the
filter is refused with the offending character named, and the companion
keeps the buffer so it can be fixed with "scratch that". The rule about a
leading `/` is moot, since `/` is refused anywhere. Step 2's
`validate_message` tests change to match.

**Open questions settled:**
- 2: `allow_messages` defaults to `true` for the built-in agents.
- 3: the cap is 4000 characters.
- 1 was settled by the VAD amendment of 01:57 EDT.

The owner approved the plan. Its status is `In implementation`, and this
document is immutable (AGENTS.md, Rule 1.5), set in this commit ahead of
the first implementing commit, which follows.

## Amendment — 2026-10-23 02:28 EDT: the Submit button is the approval

On the owner's instruction: "if I use UI to send it doesn't need a separate
confirmation".

This replaces decision 5's single approval, for button submits only.

**Submitted with the Submit button:** the press is the approval. The
companion approves its own message's confirmation itself, with
`confirm.reply {approve: true}`, without speaking or showing the question.
- The daemon is unchanged. It still opens the confirmation and logs it as
  approved by the phone, so the audit trail stays.
- A companion may already approve its own voice-turn sends, so nothing new
  is trusted.
- The confirmation is recognised by its id, from `agents.send`'s answer,
  or, when the question arrives first, by its text. The text is compared
  after the daemon's cleaning: runs of whitespace folded, and trailing `.`
  and `,` dropped.

**Submitted by the silence:** asked as before. Nothing has confirmed it
yet.

**Tests:**
- a button submit is approved once, with no question, whichever of the
  question and the answer arrives first;
- a silence submit is still asked.

## Amendment — 2026-10-26 01:14 EDT: text filter removes symbols instead of rejecting; threshold logic

On the owner's instruction: "also, the text filter should remove symbols not reject the in0ut unless its above a certain threshold, and single symbol followed by space (such as period or comma) shpuldnt trigger that threshold".

This amends the 01:58 EDT amendment on the strict symbol filter for `validate_arbitrary_command` and `validate_message`:

1. **Remove symbols instead of rejecting input:**
   - Unallowed symbols are stripped from the input text rather than immediately rejecting the entire input.
   - Allowed characters remain: ASCII letters, digits, spaces, and the punctuation `. , ' ? :`.

2. **Threshold rejection for excessive symbols:**
   - Input is rejected if the number of threshold-triggering symbols exceeds 3.
   - **Threshold exception:** An isolated single symbol followed by a space (or trailing at the end of the input), such as `. `, `, `, `! `, `? `, `: `, or `- `, is treated as standard speech transcription / sentence punctuation and does **not** count toward the threshold.
   - Embedded symbols (such as `/` in paths or `-` in CLI flags) or runs of symbols (e.g. `!@#$%`, `---`, `...`) do count toward the threshold. When the count exceeds 3, the input is rejected.

## Amendment — 2026-10-26 01:40 EDT: text filter applies to both general agent and session input

The owner (2026-10-26): "the filter applies to general wnd session input".

1. **Uniform application:**
   - The symbol removal, threshold rejection, and length checks apply uniformly to both session messages (`validate_message`) and general agent chat turns (`turn.start`).
   - Disallowed symbols are stripped from both inputs; excessive threshold-triggering symbols (limit of 0 for command execution / 3 for free text) reject the turn or message.

2. **Multi-line support:**
   - Newlines (`\n`, `\r\n`) and tabs are preserved in messages and general agent input so multi-line text input (from the 3-line `TextArea` and desktop entry) is retained rather than rejected as a control character or flattened.



## Amendment — 2026-10-27 13:25 EDT: a tap is the approval: agents.send's confirmed flag

The owner (2026-10-27): "we should also fix the bug where a short approval
is shown for a command chosen via UI, but then immediately dismissed. that
approval dialog should not be shown for UI buttons, only voice commands";
then "tapping a UI button should send confirmed: true (or ui: true and that
should be taken as confirmed without a confirmation step)".

The 2026-10-23 02:28 EDT amendment made a button submit its own approval,
which the app gave by approving the confirmation the daemon opened. The
daemon sends that confirmation's `confirm.requested` before its
`agents.send` answer, so the app could show it for a moment. This replaces
that mechanism:
1. **`AgentsSendParams.confirmed: bool`,** appended, off when left out
   (`libflankr_proto` 0.3.12, a patch release by the owner's ruling that
   appending to a struct is additive).
2. **The daemon** takes `confirmed: true` from a companion connection that
   may approve (`is_companion` and `can_approve`) as the approval: it opens
   no confirmation and sends at once, answering `Sent`, for a command and
   for a message alike. Every other check still runs (the agent's entry and
   reach, the allow list, `allow_messages`, the message check and paths).
   From any other connection the flag is ignored and a confirmation opens
   as before. The log says "confirmed by the tap".
3. **The app** sets it for every tapped suggestion (yes, no, cont, cmpct,
   approve, implement) and for a submitted, reviewed message (the 02:28
   EDT amendment's Submit). Voice commands, the general agent's requests and
   anything else keep their confirmation.
4. The app's own approval of such confirmations stays, for daemons older
   than this.

## Amendment — 2026-10-30 13:37 EDT: nothing is spoken over voice input

The owner (2026-10-30, `docs/OWNER_REPORTS.md`, the 13:34 EDT entry):
"spoken messages shouldn't interrupt an interactive voice input".

Plan 20261022_04's decision 4 holds spoken notices only while the speaker
plays, the STT gate listens (`Listening(true)`) or a confirmation waits,
and gives every `turn.say` as `Say` at once. During a dictated message the
gate closes after each sentence until the client arms it again, so a reply
read aloud or a notice could start in that gap, and a Gemini line or a
turn's "stopped" sentence could start at any time. This amendment adds a
hold for voice input. It applies to this plan's dictation mode and to
Gemini's voice prompt (plans 20261024_03 and 20261028_06), which is the
same mode in `flankr-companion`'s `State`.

1. **Voice input** is in progress while a prompt is open in voice mode
   (the record button's message card, or Gemini's card from the centre
   button) and still collecting: from its start until it is submitted,
   cancelled or cleared, or leaves voice mode. A submitted message waiting
   for its own confirmation is no longer voice input (that confirmation
   already holds notices, as before). A typed prompt (keyboard mode) is
   not voice input.
2. **What is held.** While voice input is in progress, nothing that is not
   the voice input's own speech is given as `Say`: replies read aloud
   (the play button), the daemon's notices, Gemini's `turn.say` lines (when
   *Speak agent (Gemini) responses* is on) and a tapped turn's "stopped"
   sentence all go to the client's notice queue instead, in the order they
   come.
3. **When it plays.** When voice input ends (submitted, by the button or by
   the silence; cancelled; cleared; or the card closed), the held speech
   is given as `Say`, oldest first, under the queue's usual rules: not
   while the speaker plays, the gate listens or a confirmation waits.
4. **What is dropped.** Nothing is dropped by age. The queue keeps its cap
   of 8 (plan 20261022_04, decision 4): past it the oldest held item goes.
   *Stop reading* drops the held items too (plan 20261028_06, the
   2026-10-29 00:43 EDT amendment). With *All sound* off nothing is
   queued; muted, the held speech is dropped by the app when it is given,
   as every `Say` is (plan 20261028_06, the 2026-10-29 00:52 EDT
   amendment).
5. **Starting voice input stops what is playing.** Starting a voice prompt
   (and *continue by voice* from a typed one) while the speaker plays sends
   `StopSpeaking`, as a tap does (decision 2): what was cut off is not said
   again. Gemini's prompt already did this for a running turn.
6. **What is still spoken** during voice input, because the voice input
   asks for it: "read it back"; "The message is empty." and "The message
   is full."; a refused submit and its reason; "I couldn't focus …"; and a
   message that failed to send. A confirmation question is also still
   spoken at once: while dictating, the next sentence answers it
   (decision 2's "approve" and "deny"), so it is part of the flow. A
   replay the user taps is spoken at once.
7. **Not changed.** Sounds that are not speech (the done ding, the record
   chimes) are not held.

## Amendment — 2026-10-30 13:45 EDT: correction to the 13:37 EDT amendment's item 7

The 13:37 EDT amendment's item 7 names "the record chimes" among the
sounds that are not speech; the app has no record chime. The only such
sound is the done ding (`Event::Ding`), which still plays during voice
input. Nothing else changes.
