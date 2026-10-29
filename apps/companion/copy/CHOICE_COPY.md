The choice card's copy (docs/design/20261029_01_CHOICE_QUESTIONS_PLAN.md,
decisions 7 and 8), compiled into the app at build time with every other
file in `copy/`, in the same layout: one object tagged Copy per piece, its
head the key the code names, in capitals, as `copy::<KEY>`. A key is
defined once across all the files. The `choice_*` keys are here.

The card (`src/agent/choice_card.rs`) draws these as plain text, not
Markdown, wrapped or cut with "…" to the room they have.

With two or more questions, the counter at the chip's right.

# [Copy] choice_count

- [abc] n: the question shown, from 1
- [abc] count: how many questions the agent asked
- text: {{ [abc] n }} of {{ [abc] count }}

The last row of a question that takes a free-text answer, before any is
given; a tap opens the typed card.

# [Copy] choice_other

- text: Other…

The typed card's header while it takes the Other text, in place of
"Message to <agent>". Its send button gives the text back to the choice
card; nothing is sent to the agent until Submit.

# [Copy] choice_other_header

- [abc] header: the question's chip, or its text when it has none
- text: Other: {{ [abc] header }}

The button under a multi-select question's rows: enabled once a row is
checked or Other has text.

# [Copy] choice_done

- text: Done

The review after the last of several questions: its chip, one line per
question, and its two buttons.

# [Copy] choice_review

- text: Review

# [Copy] choice_review_line

- [abc] header: the question's chip, or its text when it has none
- [abc] answer: the picked labels and the Other text, joined with ", "
- text: {{ [abc] header }}: {{ [abc] answer }}

# [Copy] choice_no_answer

- text: (no answer)

# [Copy] choice_submit

- text: Submit

# [Copy] choice_back

- text: Back

While `choice.answer` is on its way: the options are greyed.

# [Copy] choice_sending

- text: Sending…

The lines a result or a refusal leaves on the card. A mismatch: what the
agent recorded, from the daemon's `got`, its lines joined with "; ".

# [Copy] choice_recorded

- [abc] agent: the agent's shown name
- [abc] answers: what it recorded, one "<header>: <answer>" per question
- text: {{ [abc] agent }} recorded: {{ [abc] answers }}

No result in time (`no_result`) or a screen check that failed
(`SCREEN_MISMATCH`): the card stays live with the choices kept.

# [Copy] choice_not_answered

- text: Not answered: finish on the Mac

`CHOICE_STALE`: the question was answered or closed on the Mac.

# [Copy] choice_stale

- text: Already answered or closed on the Mac

`CHOICE_UNSUPPORTED`: no driver for this agent's question tool.

# [Copy] choice_unsupported

- text: This agent's questions can't be answered from the phone yet: answer on the Mac

`NOT_PERMITTED`, with the daemon's reason.

# [Copy] choice_not_permitted

- [abc] reason: the daemon's message
- text: Not allowed: {{ [abc] reason }}

Any other failure: the daemon's message, the link closing, or no answer
in time.

# [Copy] choice_failed

- [abc] reason: why, one line
- text: Not sent: {{ [abc] reason }}

The card's own check of an Other text, with the daemon's rules for a
message (one line, not a command, at most 4000 characters).

# [Copy] choice_other_empty

- text: Type an answer first

# [Copy] choice_other_one_line

- text: An answer is one line

# [Copy] choice_other_command

- [abc] mark: the first character, `/`, `!` or `#`
- text: An answer can't start with {{ [abc] mark }}

# [Copy] choice_other_too_long

- text: An answer is at most 4000 characters

The alert when the agent recorded something other than what was tapped,
while flankr is not on screen.

# [Copy] choice_mismatch_alert

- [abc] answers: what it recorded, as on the card
- text: Recorded a different answer: {{ [abc] answers }}
