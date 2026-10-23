---
title: Dictated messages to the focused agent
date: 2026-10-23
status: Draft
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
