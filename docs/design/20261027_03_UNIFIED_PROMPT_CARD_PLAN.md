---
title: One prompt card for the general agent and dictated messages
date: 2026-10-27
status: Draft
covers:
  - "One `PromptCard` control, shared by the overview and the agent view,
    with two modes (the general agent, and a message to a session) and the
    same states: pinned, expanded as an overlay, and (for the general agent)
    hidden to two bars"
  - "One text area and one keyboard route for both modes: typing, the
    cursor, a dismissable IME, and a physical keyboard"
  - "History in both modes: the conversation with Gemini, and the messages
    sent to a session"
  - "Expanding and collapsing only on the user's choice: an up arrow to
    expand, a grab bar to collapse"
  - "One card open at a time, each remembering its state"
  - "Not covered: what the general agent does (plan 20261024_03 and its
    daemon side); the agent's own pinned card; other sessions' cards"
sources:
  - "Owner (2026-10-27): \"plan to unify the app support for cards for
    general agent and dictated message, support the expanded history in
    both modes as an overlay and normal mode as pinned card. when the cards
    are hidden, the general agent keeps the input and output pinned bars.
    text input should work in the text areas, including the dismissable
    ime. expand and collapse can use the arrow to expand and bar to
    collapse. review current implementation and discuss\"; then, to the
    questions: \"just the input message history for session messages. 2.
    keep the same behavior for now 3. it should switch which ones is open,
    unless opening in overlay mode would be better? 4. the 2s hold should
    open the base card, the user chooses when to expand/collapse\"; and to
    the answer on 3 (switch, each card remembering its state): \"yes, write
    it as a plan\""
  - "flankr at `5727bfa`, reviewed 2026-10-27:
    - three `TextArea`s: `apps/companion/src/agent/dictation_card.rs` (the
      agent view's message card), `overview.rs`'s `panel()` (the overview's
      own copy of it), and `overview/turn_overlay.rs` (the Gemini overlay's
      input);
    - `overview/turn_strip.rs` (`TurnStrip`, the one-line Gemini pinned card)
      and `TurnOverlay` with `overview/turn_log.rs`, owned by the overview
      and borrowed by the agent view through `lib.rs`;
    - `lib.rs` routes the keyboard through two hits, `AgentHit::TextArea` and
      `AgentHit::GeneralAgentTextArea`, with `start_text_input` and
      `stop_text_input` in about ten places;
    - the centre button's 2 s hold opens the overlay"
related:
  - docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md (its pinned card, overlay and 2 s hold, which this replaces in the app)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3: the keyboard glyph and prompt card amendments of 2026-10-24 16:15 EDT and 2026-10-25 17:53 to 23:55 EDT, which this replaces in part)
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (the overview's strip and overlay)
---

# One prompt card for the general agent and dictated messages

## 1. Goal

Speaking or typing to Gemini, and dictating or typing a message to a coding
agent, are two cards built three ways, with three text areas and two
keyboard routes. They expand differently, and only Gemini's has a history.
After this plan, one card serves both, on both screens, with the same states,
the same text input, the same history view, and the same expand and collapse
controls, which only the user operates.

## 2. Decisions

1. **One control, `PromptCard`,** in `apps/companion/src/prompt/card.rs`, used
   by the overview and the agent view. It has two modes:
   - *General agent*: header "✦ Gemini" in the Gemini accent.
   - *Message*: header "Message to <agent>" in that session's colour (the L3
     amendment of 2026-10-24 15:56 EDT).

   Each screen owns its own card and overlay. Nothing is borrowed across
   screens through `lib.rs`.
2. **States.**
   - **Pinned** (the default): above the controls, where the message card
     and Gemini's pinned card sit today. It shows the header, the latest
     exchange, and the input area while there is input.
   - **Expanded:** an overlay sheet, about two thirds of the screen high, as
     `TurnOverlay` is today. It holds the mode's history, scrolling, with the
     input area pinned at its bottom.
   - **Hidden** (*General agent* only): the card goes, and two bars stay
     pinned above the controls, each one line high:
     - an input bar, with what is being heard or typed, or "Listening…";
     - an output bar, with Gemini's latest line.

     A tap on either bar opens the card again, in the state it was left in.
   - *Message* has no hidden state, as now: the centre button's X cancels it,
     and sending closes it.
3. **History.**
   - *General agent*: the conversation with Gemini, as `TurnLog` holds it now:
     the user's turns and Gemini's replies, oldest at the top.
   - *Message*: only the messages sent to that session from this view while
     it is open, oldest at the top, each with its time. The agent's replies
     are not shown here; they are in the scrollback.
4. **Expanding and collapsing, only by the user.**
   - An up arrow (˄) at the pinned card's top right expands it.
   - A grab bar at the overlay's top collapses it to pinned. A swipe down on
     the bar, and Back, also collapse it.
   - Nothing expands or collapses on its own. A message being sent leaves the
     card as it was until it closes.
5. **One card at a time.**
   - Opening one card switches to it. Gemini's card goes to its bars while a
     message card is open. When the message is sent or cancelled, Gemini's
     card comes back in the state it was in.
   - Each card remembers whether it was last pinned or expanded, and opens
     that way next time: per screen for *General agent*, and per session for
     *Message*.
6. **Text input.**
   - One `TextArea` inside `PromptCard`, 3 lines at 12 pt minimum, as now,
     growing with its text.
   - One keyboard route in `lib.rs`: a single `focused_prompt` (screen and
     mode) in place of `AgentHit::TextArea` and `AgentHit::GeneralAgentTextArea`.
     `start_text_input` and `stop_text_input` are called in one place each.
   - A tap on the text area focuses it, places the cursor, and shows the soft
     keyboard. Dismissing the keyboard leaves the card and its text; a tap
     shows it again. A physical keyboard types directly and keeps the soft
     keyboard hidden (the L3 amendment of 2026-10-25 23:34 EDT, kept).
   - The keyboard's height (`keyboard_inset`) lifts the pinned card or the
     overlay's input above it.
7. **The centre button's 2 s hold** opens Gemini's card in its pinned state,
   no longer the overlay. A tap starts listening, as now (plan 20261024_03,
   the amendment of 2026-10-26 12:35 EDT).
8. **What this replaces in the app:**
   - plan 20261024_03's amendment of 2026-10-26 00:52 EDT, item 2 (the 2 s
     hold opening the overlay);
   - its amendment of 2026-10-26 01:14 EDT, items 1 and 2 (the pinned card
     and overlay, now `PromptCard`);
   - L3's prompt card amendments of 2026-10-25 17:53 and 23:41 EDT, for their
     layout, which `PromptCard` now owns. Their behaviour (placeholders, the
     shaded input, 3 lines at 12 pt) carries over.

   Plan 20261024_03's and L3's other amendments stand.

## 3. Steps

1. **`PromptCard` and its overlay,** in `apps/companion/src/prompt/`
   (`card.rs`, `overlay.rs`, `bars.rs`, `history.rs`), built from today's
   `DictationCard`, `panel()`, `TurnStrip`, `TurnOverlay` and `TurnLog`, with
   both modes, all three states, the arrow and the grab bar.
   **Done when:** unit tests lay out each mode in each state; the arrow
   expands and the grab bar collapses; the history shows Gemini's
   conversation in one mode and only the sent messages in the other; and the
   hidden state shows the two bars.
2. **Both screens on it,** with the old card code removed: the overview's
   `panel()`, `turn_strip.rs` and `turn_overlay.rs`; the agent view's
   dictation card layout; and `lib.rs`'s borrowing of the overview's overlay.
   **Done when:** the existing overview and agent view tests pass on
   `PromptCard`; one card is open at a time; Gemini's card returns to its
   state after a message; and each card reopens as it was left.
3. **One keyboard route:** `focused_prompt`, one `start_text_input`, one
   `stop_text_input`, the IME's dismissal and return, a physical keyboard,
   and the keyboard inset, in `lib.rs` and the client if needed.
   **Done when:** tests show typing into either mode's text area, the
   keyboard dismissed and re-shown by a tap, a physical keyboard typing
   without the soft keyboard, and the card lifted by the inset.
4. **The 2 s hold** opening the pinned card; then the S23 check.
   **Done when:** a test shows the hold opening the pinned card, not the
   overlay; and on the S23, both modes on both screens pin, expand by the
   arrow, collapse by the bar, take typed and dictated input with the
   keyboard dismissed and brought back, and switch between each other.

Each step is one commit to the app (Rule 3.1), plus the client if step 3
needs it. 2 needs 1; 3 needs 2; 4 needs 2.

## 4. Open questions for the owner

1. **The bars' look:** the input bar in the text colour and the output bar
   dim, each with a small "✦" at the left? Or both in the Gemini accent?
2. **Message history's lifetime:** only while the view is open, as written,
   or kept per session for the app's run (cleared when the session's file
   changes)?
