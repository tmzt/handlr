---
title: Choice questions, with Claude Code's AskUserQuestion answered from the phone
date: 2026-10-29
status: In implementation
covers:
  - "A flankr format for an agent's multiple-choice questions in
    libflankr_proto, agent-agnostic and additive: `PendingQuestion::choices`
    (`ChoiceSet`, `ChoiceQuestion`, `ChoiceOption`) and a `choice.answer`
    method whose answers are pairs of picked options and Other text"
  - "Detection in Claude Code's reader: an `AskUserQuestion` `tool_use` with
    no `tool_result` is the pending question, with its choices; it goes when
    the result appears, wherever it was answered"
  - "Answering by keystrokes into Claude Code's question screen, the key
    sequences derived from the session history and the shipped client's
    code (not from a test session), with screen checks between steps and
    the answer confirmed by reading the new `tool_result`"
  - "Permission: `allow_messages`, a tap counted as confirmed, Other text
    through `validate_message`; plain messages refused while a choice
    question is open"
  - "The phone's card: header chip, question, option rows with their
    descriptions, check marks for multi-select, Other…, one question at a
    time with a review and Submit, previews; and the question in other
    sessions' cards and the drawer"
  - "Not covered: answering by voice (Later, the owner); other agents'
    question tools (their readers map them to the same format later);
    Claude Code's `text` and `number` question kinds and `title` (shown,
    answered on the Mac)"
sources:
  - "Owner (2026-10-29, verbatim): \"plan to support Claude's
    AskUserQuestions like the recent one, it will need a flankr format
    which works the same (text, response pairs)\""
  - "Owner (2026-10-29, verbatim), to the lead's proposal: \"id prefer you
    just read the history. voice later\". The proposal, as settled: the
    format of decision 1; detection in the Claude Code reader; answers as
    keystrokes into Claude's question screen, the keys derived by reading
    history and the client rather than by running a test session; the
    permission of decision 6; the card of decisions 7-9 (single questions
    skip the review, previews collapsed in monospace and expanded on a tap,
    both the lead's recommendations); voice answers later"
  - "Claude Code session transcripts under `~/.claude/projects/`, read
    2026-10-29 for `AskUserQuestion` `tool_use` inputs, their `tool_result`s
    and `toolUseResult` only (finding 1): 22 files, 34 calls, Claude Code
    2.1.227 to 2.1.310, among them the lead's two-question call of
    2026-10-29 00:11 EDT that prompted this plan"
  - "The shipped Claude Code client, `~/.local/bin/claude` →
    `~/.local/share/claude/versions/2.1.312` (a Bun binary whose JavaScript
    chunks are plain text in it), read-only (findings 2-5): the tool's
    definition and `mapToolResultToToolResultBlockParam`, the question
    screen (`fPe`, `iZ`, `K7`, the review `dZ`), its reducer, the select
    and multi-select components and the default keybindings table"
  - "herdr at the default socket, read-only: `herdr pane list` and
    `herdr pane read --source visible` of the two `blocked` Claude panes
    (neither was showing a question; finding 6); `herdr api schema --json`
    (protocol 19) for `pane.send_text`, `pane.send_input` and
    `pane.send_keys`"
  - "flankr at `1e10f1c` (branch `main-terminal`), read 2026-10-29:
    - `crates/libflankr_proto/src/common.rs:210-220` (`Turn`, exhaustive,
      `Asking { question }`), `:470-521` (`PendingQuestion`, `Answer`),
      `:961-989` (`CardBody`), `:557-590` (`SendKind`, `AgentsSendParams`
      with `confirmed`);
    - `src/sessions/claude.rs:190-230`: a `tool_use` becomes an `Item` of
      `EntryKind::ToolUse` whose text is `<name> <input JSON>` cut to
      `TOOL_TEXT_MAX` (4000), `call_id` its id; a `tool_result` an `Item`
      with `call_id` its `tool_use_id`; `toolUseResult` is not read;
    - `src/sessions/mod.rs:60-80` (`Item`), `src/sessions/pending.rs`
      (`pending_question`: the last assistant `Text` entry's final
      paragraph, when it asks and the pane waits or the file is quiet),
      `src/sessions/turn.rs:27-33` (`Turn::Asking` from it),
      `src/sessions/sections.rs:364-392, 474-496` (the live Question card
      on the pending question's reply; resolved ones faded);
    - `src/server/conn.rs:1330-1380` (`tapped`, `send_message`:
      `policy::check_message`, then `run_command`, herdr's
      `pane.send_input { text, keys: [\"Enter\"] }`);
      `src/commands.rs:184-212` (`validate_message`, one line, at most
      `MAX_MESSAGE_LEN` 4000); `src/server/terminal.rs:672-680`
      (`HerdrRequest::Custom` for a herdr method libherdr lacks);
    - `crates/flankr-companion/src/client.rs:31-35, 548-551` (`Input` and
      `Event`, `#[non_exhaustive]`), `:2726-2743` (the live Question card
      given its pending question's `answers`), `:3814-3860`
      (`suggested`); `crates/flankr-companion/src/view.rs:27-58`
      (`AgentCard`, a plain struct);
    - `apps/companion/src/agent.rs:297-305, 648-700, 4253-4258` (the
      question card's answers and buttons);
      `apps/companion/src/drawer/session_row.rs:80` (the drawer's line and
      colour from `Turn::Asking`); `apps/companion/src/session_cards.rs`
      (other sessions' cards)"
  - "libcontrols 0.10.6 (tag `v0.10.6`, released): `PopupItemKind::Toggle`,
    `PopupItem::toggle`, `PopupMenuEvent::Toggled`, `PopupRow` and
    `PopupCell` (`dim`)"
related:
  - docs/design/20261027_06_SUGGESTION_RULES_PLAN.md (In implementation;
    `PendingQuestion::answers`, which a choice question does not use;
    decision 4)
  - docs/design/20261027_03_UNIFIED_PROMPT_CARD_PLAN.md (In implementation;
    the typed card that Other… opens; decision 8)
  - docs/design/20261023_06_SESSION_READERS_PLAN.md (Implemented; the
    agnostic `SessionReader` and `Item` this plan extends; decision 2)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (In implementation;
    living section L3, the agent view: one amendment pointing here, step
    5)
  - docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md (In implementation;
    the card's typed input row and the libcontrols pin; decision 8)
  - docs/design/20261028_03_SESSION_DRAWER_PLAN.md (In implementation; the
    drawer's waiting dot, unchanged code; decision 9)
  - docs/design/20261028_05_BINARY_FILES_PLAN.md (In implementation; takes
    libflankr_proto 0.5.4 and a flankr-companion patch before this plan)
---

# Choice questions, with Claude Code's AskUserQuestion answered from the phone

## 1. Goal

When Claude Code asks with its `AskUserQuestion` tool, the Mac shows a
screen of numbered options, an Other row and, for several questions, tabs
and a review. Today flankr does not see the question at all: it is a
`tool_use`, not a reply ending in `?`, so the phone shows no Question card,
and a Yes or a typed reply sent to the pane would land in the question
screen as keystrokes (an Enter there picks the focused option).

After this plan:

1. The daemon sees the open question and gives it to the phone in a flankr
   format of questions and options, the same for any agent.
2. The phone shows a card that works as Claude's screen does: pick one
   option, or several, or type Other; one question at a time; a review and
   Submit when there are several.
3. The daemon answers by sending Claude's own keys to the pane, checking
   the screen as it goes, and then reads the `tool_result` Claude writes to
   confirm the answers it got are the ones tapped.

## 2. Findings that shape the plan

The key sequences of decision 5 rest on these. Each says where it comes
from; nothing here was found by running a test session (the owner's
instruction).

1. **The session history** (34 `AskUserQuestion` calls in 22 transcripts,
   Claude Code 2.1.227 to 2.1.310):
   - The input is always `{"questions": [...]}`, 1 to 4 questions, each
     `{question, header, multiSelect, options}`, 2 to 4 options of
     `{label, description}` and sometimes `preview` (5 calls).
   - 23 calls were answered. `toolUseResult` is
     `{questions, answers, annotations}`; `answers` maps each question's
     full text to one string. Of 39 answers, 31 are an option's `label`
     exactly and 8 are free text (Other) as typed. The one multi-select
     call was answered with a single label. A preview answer adds
     `annotations[<question>].preview`, the chosen option's preview text.
     Two older calls have `toolUseResult: null` (one with no `version`);
     their `tool_result` content carries the same answers.
   - The `tool_result` content is
     `Your questions have been answered: "<q1>"="<a1>", "<q2>"="<a2>". You
     can now continue with these answers in mind.` when every answer is a
     label, and `The user answered: "<q>"="<a>", …. Read the answers
     carefully — …` when any is free text; a preview answer adds
     ` selected preview:\n<preview>` after its pair.
   - 11 calls ended with `is_error: true` and content starting `The user
     doesn't want to proceed with this tool use. The tool use was rejected
     …`: 4 plainly (Esc, or Cancel on the review), 7 followed by `To tell
     you how to proceed, the user said:` and `The user wants to clarify
     these questions.`, listing each question with `Answer: <label>` or
     `(No answer provided)` (the "Chat about this" row).
   - Several questions are answered in one result, in the order asked.
2. **The tool in 2.1.312** (`AskUserQuestion` definition and its result
   mapping):
   - Bounds: 1-4 questions, 2-4 options, `header` at most 12 characters
     (`YJs=12`), question texts unique, labels unique within a question;
     "There should be no 'Other' option, that will be provided
     automatically".
   - Multi-select answers are the picked labels joined with `", "`, a
     label containing `", "` or `"` written as a JSON string (`LLt`); the
     order is the order they were toggled, Other's text last.
   - An extended form exists behind a flag (`YIe()`): a `title`, and
     questions of `kind` `"text"` or `"number"` with `min`, `max`, `step`,
     `unit`; `description` becomes optional on options.
   - An optional idle timeout, the setting `askUserQuestionTimeout`
     (`"60s"`, `"5m"`, `"10m"`, `"never"`; default never), resolves the
     screen with `No response after Ns …`; the owner's settings do not set
     it.
   - Results also carry `response`, `followUp` and `annotations.notes`,
     and `admitCardAnswer` accepts answers from an outside client; none of
     these is reachable from herdr, so keystrokes are the way in.
3. **The question screen** (`fPe`, `iZ`, the reducer `lIo`):
   - Each question's rows are its options, then **Other** (an input row,
     placeholder `Type something.`, `Type something` for multi-select),
     then, below a rule, `<N+2>. Chat about this` (not for multi-select).
     The hint line is `Enter to select · ↑/↓ to navigate · Esc to cancel`,
     or `Tab/Arrow keys to navigate` with several questions.
   - Answering a single-select question sets its answer and moves to the
     next (`set-answer` with `shouldAdvance`). With **one** question that
     is not multi-select (`hideSubmitTab`), the answer is submitted at
     once: no review.
   - Otherwise a tab row shows `← ☐ <header> … ✔ Submit →`, and after the
     last question comes the review: `Review your answers`, each question
     with `→ <answer>`, `Ready to submit your answers?`, then a select of
     `1. Submit answers` and `2. Cancel` (`mr`, focus on Submit). Cancel,
     and Esc anywhere, deny the call (the rejected results of finding 1).
   - Tab, Shift+Tab, ← and → switch questions (the `Tabs` context), except
     while typing in an input row.
4. **The select components** (2.1.312's select chunk):
   - Single-select: a digit `1`-`9` picks that row at once; on the Other
     row it **focuses** the input when the input is empty (and submits its
     text when it is not). Enter on an option picks it; Enter in the Other
     input submits its text when not empty. ↑/↓, `j`/`k`, Ctrl+N/Ctrl+P
     move.
   - Multi-select: a digit toggles that row **without moving focus**;
     Enter or Space toggles the focused row; ↓ from the last row (Other)
     focuses the `Next` button (`Submit` on the last question); Enter on
     that button goes on. Typing in the Other row selects it when its text
     is not empty; Enter in the Other row toggles it (so it is not used).
   - Questions where an option has a `preview` and that are single-select
     use a side-by-side view (`K7`): a digit only moves focus, Enter picks,
     `n` adds notes, and there is no Other row.
   - Screen-reader mode replaces every screen with `y`/`n` prompts.
5. **herdr** (protocol 19): `pane.send_text { pane_id, text }` sends
   literal text with no Enter; `pane.send_input { pane_id, text?, keys? }`
   and `pane.send_keys { pane_id, keys }` as libherdr uses them;
   `pane.read { source: "visible" }` reads the frame. libherdr has no
   request for `pane.send_text`, so it goes as `HerdrRequest::Custom`, as
   `pane.scroll` does.
6. **The rendered screen.** No pane was showing a question during the
   read-only check: the two `blocked` Claude panes showed other prompts.
   The screen's strings above come from the client's code; the Mac and S23
   check (step 7) confirms them on a real screen.

## 3. Decisions

### The flankr format

**1. Types in `libflankr_proto::common`, all additive.**

```rust
pub struct PendingQuestion {
    // … today's fields, then appended:
    /// The question is a choice question (plan 20261029_01): its options,
    /// answered with `choice.answer`. When present, `answers` is empty and
    /// `text` is the first question's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<ChoiceSet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChoiceSet {
    /// The agent's id for the call (Claude Code: the `tool_use` id,
    /// `toolu_…`), at most 128 characters.
    pub id: String,
    /// 1 to `CHOICE_QUESTIONS_MAX`, in the order asked.
    pub questions: Vec<ChoiceQuestion>,
    /// Who asked and how: `<reader kind>/<tool>`, as `claude_code/AskUserQuestion`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChoiceQuestion {
    /// A short chip, at most `CHOICE_HEADER_MAX` characters; may be empty.
    pub header: String,
    /// The question, at most `PENDING_QUESTION_MAX` characters.
    pub text: String,
    /// Several options may be picked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi: bool,
    /// A free-text answer may be given (Claude Code: every question but a
    /// preview one).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub other: bool,
    /// 2 to `CHOICE_OPTIONS_MAX`.
    pub options: Vec<ChoiceOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChoiceOption {
    /// At most `CHOICE_LABEL_MAX` characters.
    pub label: String,
    /// The option's description, at most `CHOICE_DETAIL_MAX`; may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Monospace text shown when the option is looked at, at most
    /// `CHOICE_PREVIEW_MAX` characters and `CHOICE_PREVIEW_LINES` lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

pub const CHOICE_QUESTIONS_MAX: usize = 4;
pub const CHOICE_OPTIONS_MAX: usize = 8;
pub const CHOICE_HEADER_MAX: usize = 32;
pub const CHOICE_LABEL_MAX: usize = 120;
pub const CHOICE_DETAIL_MAX: usize = 400;
pub const CHOICE_PREVIEW_MAX: usize = 4000;
pub const CHOICE_PREVIEW_LINES: usize = 60;
```

Each struct has a `new` constructor (they are `#[non_exhaustive]`, so other
crates cannot build them with a literal). All text is redacted by the
daemon. A call a reader cannot fit in these bounds (more questions or
options, a label too long) gets no `choices`: it stays a plain pending
question, answered on the Mac.

**2. `choice.answer`, the method.** `methods::CHOICE_ANSWER =
"choice.answer"`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceAnswerParams {
    /// The pane, resolved as `agents.send`'s.
    pub pane_id: String,
    /// `ChoiceSet::id`.
    pub id: String,
    /// One per question, in order.
    pub answers: Vec<ChoiceAnswer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ChoiceAnswer {
    /// Indexes into the question's options. One for a single-choice
    /// question unless `other` is given; any number for `multi`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<usize>,
    /// The free-text answer, only where the question has `other`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChoiceAnswerResult {
    /// The agent's result has every answer as sent.
    Answered,
    /// The agent's result differs: `got` is what it recorded, one line per
    /// question ("<header>: <answer>").
    Mismatch { got: Vec<String> },
    /// The keys were sent but no result was written within
    /// `CHOICE_RESULT_WAIT_S`.
    NoResult,
}

pub const CHOICE_RESULT_WAIT_S: u64 = 5;
```

`pane_id` is added to the lead's `{ id, answers }`: every other method
names its pane, and the daemon must not search every session for an id.
The pairs of `picked` and `other` are the owner's "text, response pairs":
each question with what was chosen or typed. A single-choice answer is
exactly one of one `picked` index or `other`; a multi answer is any
non-empty mix. Errors: `INVALID_PARAMS` for a shape that does not fit the
question; `NOT_PERMITTED` (decision 6); `HERDR_UNAVAILABLE`; and three new
codes in `envelope::codes`: `CHOICE_STALE` (-32040) when `id` is not the
pane's open choice question (answered on the Mac, or gone),
`CHOICE_UNSUPPORTED` (-32041) for a `source` without a driver, and
`SCREEN_MISMATCH` (-32042) when a screen check fails (decision 5), with the
step in the message. `codes::ALL` is unchanged, so its type stays
`[i64; 15]`, as when the plugin codes were added. The method is for
companions; a `flankr.harness` client gets `METHOD_NOT_FOUND`.

**3. `Turn` is not changed.** `Turn::Asking { question }` is a variant of
an exhaustive enum, and adding a field to it would be breaking; the turn is
`Asking` with `question` the first question's text, and the choices travel
on the `PendingQuestion` that `agents.list`, `session.peek` and the
subscription already carry. The drawer, the overview and other sessions'
cards need nothing more (decision 9).

### Detection

**4. Claude Code's reader** (`src/sessions/claude.rs`), behind the agnostic
`Item` (the session readers plan):
- `Item` gains `choices: Option<ChoiceSet>` and `choice_result:
  Option<ChoiceOutcome>` (crate-internal), `None` from every other reader
  and every other line.
- A `tool_use` named `AskUserQuestion` is parsed from the **whole** input,
  before the 4000-character cut: each question's `header`, `question`,
  `multiSelect`, `options[].{label, description, preview}`; `other` is
  true unless the question is single-select and an option has a
  `preview` (finding 4). An input with `title`, or a question whose `kind`
  is present and not `"choice"`, gets no `choices` (finding 2's extended
  form is not driven).
- A `tool_result` line whose `toolUseResult` has `questions` and
  `answers` gives `ChoiceOutcome { answers: Vec<Option<String>>, rejected:
  false }`, one per question by its text; with `toolUseResult` null or a
  string, the content's `"<q>"="<a>"` pairs are parsed; `is_error: true`
  gives `rejected: true` (Esc, Cancel or Chat about this).
- **The pending question** (`pending::pending_question`): the newest
  `ToolUse` item with `choices` whose `call_id` has no `ToolResult` after
  it, when only context follows it and the pane waits or the file is quiet
  (today's rule), is the pending question, ahead of any text question:
  `PendingQuestion { text: questions[0].text, ts: the tool_use's ts,
  choices: Some(set), answers: [], about_plan: false, … }`.
  `SUGGESTIONS.md` rules are not applied to it.
- When the result appears, from the phone or the Mac, the next read has no
  pending question, and the card goes, as a text question's does.
- **Sections** (`sections.rs`): the pending choice question gives a live
  Question card at the `tool_use`'s line (slot `SLOT_QUESTION`), title
  `<name> is asking`, text the first question's text; an answered one a
  resolved, faded card whose text is one line per question,
  `<header>: <answer>`, or `Closed on the Mac` when rejected.

### Answering

**5. The Claude Code driver** (`src/sessions/choices.rs`, new). For
`claude_code/AskUserQuestion`, with N the question's option count and k a
1-based option number, the daemon sends, through libherdr, one step per
call with 150 ms between calls:

| Case | Keys, per question | Confidence | Evidence |
|---|---|---|---|
| A. Single-select, option k | `k` | High | finding 4, digit picks; finding 3, the answer advances, or submits at once for one question |
| B. Single-select, Other text T | `N+1`; `pane.send_text` T; `enter` | Medium-high | finding 4: the digit focuses the empty input; Enter submits non-empty text |
| C. Multi-select, options k1…km | each `ki`; then `down` N+1 times; `enter` | Medium | finding 4: digits toggle in place; ↓ ×N reaches Other, one more the button; Enter goes on |
| D. Multi-select with Other T | each `ki`; `down` N times; `pane.send_text` T; `down`; `enter` | Medium | finding 4: typing selects Other; ↓ from Other reaches the button; Enter in Other is avoided |
| E. Preview question, option k | `k`; `enter` | Medium | finding 4 (`K7`): the digit focuses, Enter picks |
| F. After the last question, when there are two or more questions, or one multi-select question | `1` on the review | Medium-high | finding 3: the review's `1. Submit answers`; finding 4: a digit picks |

A single single-select question (cases A, B, E with one question) is
submitted by its answer: no review step. The daemon never sends `escape`
(it rejects the whole call) and never sends Tab or the arrows ← → (they
switch questions).

**Screen checks.** Before each question's keys and before the review's `1`,
the daemon reads the pane's visible frame (`pane.read`, `visible`, `text`),
joins its lines, strips the box-drawing gutter and folds whitespace, and
polls every 100 ms for at most 1 s until it finds:
- before question i: the first 40 characters of question i's text, folded
  the same way, and the hint `to select`; for cases B and D also the Other
  row's placeholder `Type something` (an Other row already holding text
  would submit it on the digit, so the daemon refuses); for cases C and D
  the focus pointer `❯` on the row starting `1.`;
- before the review's key: `Review your answers` and `Ready to submit
  your answers?`.
A check that fails stops the answer where it is, with `SCREEN_MISMATCH`
naming the step ("question 2 did not appear"); the screen is left for the
Mac, never cancelled.

**The result.** After the last key the daemon re-reads the session file
until the `tool_result` for `id` appears, for at most
`CHOICE_RESULT_WAIT_S` (5 s): each question's recorded answer must be, for
a pick, its label; for Other, the text as sent (after `validate_message`'s
cleaning); for multi, the set of picked labels plus the Other text, parsed
from the `", "`-joined string with JSON-quoted labels (finding 2; order is
not compared). All equal is `Answered`; otherwise `Mismatch` with what was
recorded; a rejected result is `Mismatch` with `got: ["Closed on the
Mac"]`; no result is `NoResult`. The phone shows the last two as a line on
the card and a notification (decision 7), since nothing can be taken back.

**Version.** The driver is written against 2.1.312 and the history from
2.1.227; the line's `version` field is logged with each answer. A version
outside `2.1.227..=2.1.312` is still driven (the screen checks guard it)
and logged as untested.

### Permission

**6. Who may answer.**
- The pane's agent has an `[[agents]]` entry with `allow_messages`, and
  the principal may reach it (`policy::check_message`'s rules): an answer
  is the user telling the agent something.
- A tap on the phone is the confirmation (`tapped`'s rule: a companion
  connection; buttons in the app are confirmed). No `confirm.requested`
  is raised.
- An Other text goes through `validate_message` (one line, no leading `/`,
  `!` or `#`, at most 4000 characters) and `check_paths`, through
  `policy::check_message`; a refusal is `NOT_PERMITTED` with its reason.
  No provenance prefix is added (the app's own messages have none).
- **Messages are refused while a choice question is open.** An
  `agents.send` of either kind to a pane whose pending question has
  `choices` is refused with `NOT_PERMITTED`: "Claude is showing a
  question; answer it with its options". This covers the general agent's
  `send_agent_command` too, which would otherwise type into the question
  screen.

### The phone

**7. The card** (the live Question card of a choice question; the app,
with flankr-companion carrying the data):
- A **header chip** (the question's `header`, in the session's colour,
  small caps size 12 pt) above the question text (15 pt). With two or more
  questions, `1 of N` at the chip's right.
- Each option a row: its label (15 pt), its `detail` beneath in the dim
  colour (13 pt), wrapped; rows 44 pt at least, 8 pt apart.
- **Single-select:** a tap on a row chooses it. With one question it sends
  at once; with several it moves to the next question.
- **Multi-select:** each row is a libcontrols 0.10.6 toggle item
  (`PopupItem::toggle`, its check mark, `PopupMenuEvent::Toggled`), laid
  out in place with `PopupRow` cells for the label and the dim detail; a
  **Done** button under the rows, enabled once one row is checked or Other
  has text, moves on (or sends, after the review).
- **Other…** (when `other`): a row at the end that opens the typed card
  (decision 8); its text, once given, shows in the row in quotes and
  counts as the answer (single) or as one more checked row (multi).
- **Previews:** an option with a `preview` shows it under its row folded to
  3 lines in the monospace face with `…`; a tap on the preview expands it
  to its whole text (at most 60 lines, scrolling with the view), and a tap
  again folds it. A tap on the row still chooses.
- **Several questions:** one at a time, `1 of N`; a back arrow at the
  chip's left returns to the previous question with its answer kept; after
  the last, a short **review**: each header and its answer on one line,
  and **Submit** (approve style) with **Back**. A single question skips the
  review (the lead's recommendation, decided).
- **Sending:** Submit (or the single question's tap) sends
  `choice.answer`; the card shows a spinner and its options greyed until
  the result. `Answered`: the card goes, as an answered text question's
  does. `Mismatch`: the card shows "Claude recorded: …" with `got`, and a
  notification says so. `NoResult` or `SCREEN_MISMATCH`: "Not answered:
  finish on the Mac", the card stays live with the choices kept.
- The card's record button and Yes/No are not shown for a choice question
  (decision 6 refuses them).

**8. The typed card for Other.** Other… opens the unified prompt card's
typed card for this pane with the label "Other: <header>" and empty text
(or the text given before). Its send button does not send a message: it
returns the text to the choice card, which checks it locally with the same
rules as `validate_message` (one line, not starting with `/`, `!`, `#`) and
shows it in the Other row. Paths are checked by the daemon on Submit. The
bottom bar plan's row (the keyboard toggle; no + here) is the typed card's
as usual.

**9. Elsewhere on the phone.** With `Turn::Asking` carrying the first
question's text, the session drawer's line and its question colour (the
waiting dot), the overview's agent card and other sessions' cards
(`<agent> is asking` and the text) show the choice question with no change
to their code. A tap on another session's card opens that session, where
the choice card is.

### Later

**10. Voice answers** (the owner: "voice later"). Answering a choice
question by speaking ("the second one", "both", "other: …") through the
general agent or the dictation path is a later plan; until then the
general agent sees the question as text and its sends to the pane are
refused (decision 6). Also later: other agents' question tools, mapped by
their readers to `ChoiceSet` with their own drivers; Claude Code's `text`
and `number` kinds; and Chat about this (open question 1).

## 4. Steps

Each step is one crate (Rule 3.1) and passes its formatter check, clippy
with no new warnings and its tests (`-p libflankr_proto`, `-p flankr`,
`-p flankr-companion`, `-p companion`).

1. **libflankr_proto: the choice types and `choice.answer`.** Decision 1
   and 2's types, constants, `methods::CHOICE_ANSWER`, the error codes
   `CHOICE_STALE`, `CHOICE_UNSUPPORTED` and `SCREEN_MISMATCH`;
   `PendingQuestion::choices` appended; constructors;
   the `VERSIONS.md` entry and tag (Versioning). This commit sets this plan
   to `In implementation` with its amendment.
   **Done when:** round-trip tests pass for a `PendingQuestion` with and
   without `choices` (an old question reads as before, `choices` left off
   the wire when `None`), a `ChoiceSet` of two questions with a preview, a
   `ChoiceAnswerParams` with `picked` and `other`, and each
   `ChoiceAnswerResult`.
2. **flankr: reading Claude's questions.** Decision 4: `Item::choices`,
   `Item::choice_result`, the reader's parsing, `pending_question`'s
   choice rule, `Turn::Asking` from it, the live and resolved cards in
   `sections.rs`, redaction and the bounds.
   **Done when:** tests over JSONL fixtures written from finding 1's
   shapes (made-up questions; no transcript text) show: an open call is the
   pending question with its `choices` and the turn `Asking`; a call with a
   result is not, and its resolved card lists `<header>: <answer>`; a
   rejected one says `Closed on the Mac`; an input with `title` or a `text`
   kind has no `choices`; a preview's `other` is false; a 6000-character
   input with previews is parsed whole; `toolUseResult: null` falls back to
   the content's pairs.
3. **flankr: refusing messages while a choice question is open.** Decision
   6's last rule in `agents.send` (both kinds) and the general agent's
   send path.
   **Done when:** tests show a message and a command to a pane with an open
   choice question refused with the reason, and accepted once its result
   is read.
4. **flankr: `choice.answer` and the Claude Code driver.** Decision 5:
   `src/sessions/choices.rs` (the key plan for each case, the screen
   checks, the result comparison), `pane.send_text` as
   `HerdrRequest::Custom`, the method in the companion dispatcher with
   decision 6's checks and decision 2's errors.
   **Done when:** unit tests give the exact key steps for cases A-F
   (including the 3-question mix of A, B and C, and a single multi-select
   question with its review); against herdr's mock transport with scripted
   frames, an answer whose frames match is sent and returns `Answered` when
   a fixture result is appended, `Mismatch` when the fixture differs,
   `NoResult` after 5 s, `SCREEN_MISMATCH` when question 2's text never
   appears, and an Other row with text is refused before any key; a
   harness client gets `METHOD_NOT_FOUND`; `allow_messages` off is
   `NOT_PERMITTED`.
5. **flankr-companion: choices in the view and `Input::Choose`.**
   `AgentCard::choices: Option<ChoiceSet>` appended, given to the live
   Question card from its pending question as `answers` are today;
   `Input::Choose { id, answers: Vec<ChoiceAnswer> }` sending
   `choice.answer` for the open view's pane; `Event::ChoiceAnswered { id,
   outcome: String, got: Vec<String> }`; a failed request as
   `Event::ChoiceAnswered` with `outcome` `failed` and the error message;
   the request's timeout at least 15 s. The `VERSIONS.md` entry and tag.
   **Done when:** tests show the card gets the choices only while its text
   matches the pending question; `Input::Choose` sends the params with the
   pane; each result and an error give the event; an older daemon's
   question (no `choices`) behaves as today.
6. **companion: the choice card.** Pin libcontrols 0.10.6 (or the newer
   version the bottom bar plan has pinned by then) and flankr-companion;
   decisions 7-9: rows, chip, `1 of N`, toggles and Done, Other… through
   the typed card, previews folded and expanded, the review and Submit, the
   sending state and results; record and Yes/No hidden. This commit
   appends an L3 amendment to the harness protocol plan pointing here.
   **Done when:** layout tests at 411 pt show a two-question card (single
   then multi) with its chip, rows, dim details and `1 of 2`; taps choose,
   toggle and move on; Done is disabled with nothing checked; Other… opens
   the typed card and its text returns to the row; a `/` first is refused
   on the card; a preview folds to 3 lines and expands; the review lists
   both answers and Submit gives `Input::Choose` with the right indexes; a
   single single-select question sends on its tap with no review.
7. **The Mac and S23 check.** In a herdr pane on the Mac, Claude Code asks
   `AskUserQuestion` calls covering cases A-F (one single-select, one with
   Other, one multi-select, one multi with Other, one with previews, one of
   three questions), each answered from the S23 (adb
   `timothys-s23-ultra`, after asking the owner); the rendered screen read
   with `pane.read` at each step and saved; each `tool_result` read back.
   Findings, with the frames, in `out/choices/<YYYYMMDD_NN_name>/` and the
   area's log `out/choices/LOG.md`; a key sequence that differs is a
   `CHANGES.md` addendum and a fix in `choices.rs`.
   **Done when:** every case returns `Answered` and its `tool_result`
   matches; the evidence entry names each case's frames; the owner has
   answered one question from the phone.

Order: 1, then 2, 3 and 4 in that order; 5 after 1; 6 after 5; 7 last.

## 5. Versioning (Rule 5)

| Crate | From | To | Why |
|---|---|---|---|
| `libflankr_proto` | 0.5.4 (the binary files plan's build) | the next free patch at its commit | additive: a field appended to `PendingQuestion` (defaulted, left off the wire when `None`), new `#[non_exhaustive]` types, a method name, constants and three error codes (`codes::ALL` unchanged) |
| `flankr-companion` | its version then | the next free patch at its commit | additive: a field appended to `AgentCard`, a variant each on the `#[non_exhaustive]` `Input` and `Event` |
| `libcontrols` | — | no change | 0.10.6's toggles are released |
| `flankr` (daemon), `companion` (app) | — | no bump | applications: manifest and lock only |

`libherdr` is not changed: `pane.send_text` goes as a `Custom` request.

## 6. Risks

- **Keys handled before the screen updates.** Claude Code's handlers are
  memoised per render; keys in one write could reach a stale question.
  Hence one step per herdr call, 150 ms apart, and a screen check before
  each question.
- **The user at the Mac at the same time.** A key typed on the Mac between
  steps moves focus; the screen checks catch most of it, and the result
  check catches the rest (`Mismatch`, reported, not undone).
- **A Claude Code update.** The screen, its keys or the result's format
  may change; the result check reports it, the version is logged, and step
  7 is repeated for a new major screen.
- **The idle timeout.** With `askUserQuestionTimeout` set, Claude answers
  for the user after the timeout; the phone then sees a result it did not
  send and the card goes. Not driven around.
- **Multi-select focus.** Cases C and D count ↓ presses from the first row;
  if the first row is not focused (the user moved it on the Mac) the check
  refuses rather than guess.

## 7. Open questions for the owner

1. **Chat about this.** Claude's screen has a row that rejects the call and
   lets the user talk instead (7 of the 34 calls in the history ended so).
   Offer it on the card as **Discuss…**, which sends `N+2` and then opens
   the message card? Recommended: yes, as a follow-up once cases A-F are
   confirmed.
2. **Cancel.** The review's Cancel and Esc reject the call. Offer a
   **Dismiss** on the card that sends `escape`? Recommended: no; the card's
   back arrow and leaving it alone are enough, and a rejection is easy to
   send by mistake.
3. **Preview notes.** A preview question takes notes (`n`) instead of
   Other. Offer notes from the phone? Recommended: later.
4. **Messages refused while a question is open** (decision 6). Today a
   typed reply would go into the question screen; refusing it is safer.
   Recommended: yes, as written.
5. **The untested-version rule.** Drive any version with the screen
   checks (as written), or refuse versions newer than the last one step 7
   confirmed? Recommended: drive and log, as written.

## Amendment — 2026-10-29 00:40 EDT: approved with the open questions' recommendations

The owner approved the plan on 2026-10-29 ("approve the choice questions
plan with those answers"), the answers being the open questions'
recommendations: (1) "Chat about this" is offered on the card as
**Discuss…**, as a follow-up after this plan's steps; (2) no Dismiss button
that sends Esc; (3) notes on preview questions from the phone later; (4)
plain messages (`agents.send`, Gemini's sends included) are refused while a
choice question is open; (5) Claude Code versions not seen yet are driven
with the screen checks and logged, not refused. The status stays `Draft`
until the first implementing commit (Rule 1.5).

## Amendment — 2026-10-29 00:45 EDT: implementation started; the document is now immutable

Implementation starts with this amendment, in the commit of step 1
(libflankr_proto, at the next free patch version when it is committed) on
`main-terminal`: `status` is `In implementation`, and the body above,
with its amendments, is frozen (AGENTS.md Rule 1.5). Departures are
recorded in `CHANGES.md`.

## Amendment — 2026-10-30 16:42 EDT: a choice question cancelled by dragging its card away

On the owner's instruction (Rule 1.5), 2026-10-30: dragging a pinned card
away removes it by kind (plan 20261022_03, the L3 entry of 2026-10-30
15:20 EDT: "a choice question: cancelled, as the agent's own dismiss
does"), and, asked "Add `choice.cancel`, so dragging a choice question
presses Esc?": "yes". This replaces the 00:40 EDT amendment's answer (2)
("no Dismiss button that sends Esc") for the drag only (there is still no
Dismiss button), and the rule after the keys table that the daemon never
sends `escape`:

1. **`choice.cancel`** (libflankr_proto, additive): params
   `ChoiceCancelParams { id: String, pane_id: String }` (the question's
   `ChoiceSet` id, as `choice.answer` takes it); result
   `ChoiceCancelResult { cancelled: bool }`. Offered wherever
   `choice.answer` is.
2. **The daemon** sends one `escape` to the pane through the same Claude
   key driver, after the same screen check as before question 1 (its text
   and `to select`, polled every 100 ms for at most 1 s). If the check
   fails it sends nothing and answers `cancelled: false`. One Esc rejects
   the whole call, as finding 1 says; Tab and ← → are still never sent.
3. **The client:** `Input::CancelChoice { id }` (additive) sends it and,
   on `cancelled: true`, ends the choice as the agent's own dismiss does.
4. **The app:** dragging a choice question's pinned card away sends it,
   then adds the dim flow line *Dismissed the question*; on
   `cancelled: false` the card springs back and a short note says it
   could not be dismissed.
5. **Versions.** libflankr_proto and flankr-companion each take the next
   patch; the daemon's version and the app's each take the next patch
   with their `UPDATES.md` entries (FLANKR RULES 1 and 2).
