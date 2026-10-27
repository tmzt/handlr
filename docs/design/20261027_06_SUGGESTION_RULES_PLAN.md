---
title: A question's suggested answers, from word rules in SUGGESTIONS.md
date: 2026-10-27
status: Draft
covers:
  - "`SUGGESTIONS.md`, a libtypedmd file of rules: each picks a set of
    answers for a pending question by the words it expects, and each answer
    is a label, a style and the text it sends, with a few replacements
    (`detected_plan_id`, `agent`, `project`)"
  - "The daemon applying the rules and sending the answers with the
    question, so the app draws what it is given"
  - "The plan-question heuristic of plan 20261022_03's L3 amendments
    (2026-10-27 13:21, 13:24 and 14:04 EDT) moved into the file as its
    first rule"
  - "Not covered: the agent view's row of standing suggestions (cont,
    cmpct), open question 3; anything a model decides"
sources:
  - "Owner (2026-10-27): \"we could write a plan to move these suggestions
    to typedmd with a rules-based matcher\"; then \"start the plan to move
    the rules to typedmd, should just be based on expected words and render
    a few replacements (like detected_plan_id)\""
  - "flankr at `65c28b3`:
    - `src/sessions/links.rs`: `plan_question(question, context)`, the
      word heuristic (\"approve\" with \"plan\" or \"draft\") and the plan id
      after \"plan\" or \"draft\", from the question or the rest of the reply;
    - `src/sessions/pending.rs`: `question_in`, which fills
      `PendingQuestion { text, ts, about_plan, plan, links }`;
    - `crates/flankr-companion/src/client.rs`: `Suggestion` (yes, no,
      continue, compact, approve, discuss, implement), its `send()` texts
      and `plan_answer`;
    - `apps/companion/src/agent.rs`: `actions(card)`, which draws yes and
      no, or approve, discuss and implement when `about_plan`;
    - `ADDITIONAL_PROMPTS.md` (`src/prompts.rs`) and
      `apps/companion/SETTINGS_COPY.md`: the libtypedmd files already read,
      with `{{ [abc] name }}` slots"
related:
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3: the plan-question amendments this moves into the file)
  - docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md (ADDITIONAL_PROMPTS.md, the libtypedmd format for daemon text)
  - docs/design/20261023_02_BULK_INPUT_PLAN.md (the 2026-10-27 13:25 EDT amendment: a tapped answer is sent confirmed)
  - docs/design/20261027_05_PLANNING_FOLLOWUPS_PLAN.md (the file watcher that reloads the file)
---

# A question's suggested answers, from word rules in SUGGESTIONS.md

## 1. Goal

Which answers a pending question offers, and what each one sends, is spread
over three places in code: the daemon's `plan_question` decides a question
is about a plan, the client's `Suggestion` holds the texts, and the app's
`actions` picks the buttons. Changing a word means a rebuild of all three.
After this plan, one file of rules decides: a rule names the words a
question must have, and the answers it then offers, with their labels and
texts. The daemon applies the rules and sends the answers with the
question, and the app draws them. The rules are expected words, never a
model, and a few replacements such as the detected plan id fill in the
texts.

## 2. Decisions

1. **`SUGGESTIONS.md`** at the repository root, libtypedmd, compiled into
   the daemon as `ADDITIONAL_PROMPTS.md` is (`include_str!`), with
   `--suggestions <path>` in place of the shipped file. Two kinds of
   object:

   ```markdown
   # [Rule] plan
   - when: approve, plan | draft
   - offers: approve, discuss, implement

   # [Rule] question
   - offers: yes, no

   # [Answer] approve
   - label: approve
   - style: green
   - sends: Approve {{ [abc] detected_plan_id }}
   - answers: true

   # [Answer] discuss
   - label: discuss
   - style: link
   - starts: Discuss {{ [abc] detected_plan_id }}:
   ```

2. **A rule's `when`** is the words the question must have:
   - groups separated by `,` must all be present; words separated by `|`
     within a group are alternatives;
   - a word is matched whole, ignoring case and the punctuation around it
     ("approve" matches "Approve," but not "approved");
   - an optional `unless` lists words that must not be present, with the
     same syntax;
   - a rule without `when` matches every question.

   The rules are tried in file order, and the first that matches gives the
   question its answers. The shipped file ends with `question` (yes and no),
   so every question gets answers, as now.
3. **An answer** has:
   - `label`: what the button or text shows;
   - `style`: one of `approve`, `deny`, `green`, `link`, `yellow`, mapped
     by the app to its theme's roles as now (yes and no as buttons in
     `approve` and `deny`, the others as plain coloured text);
   - exactly one of `sends` (the text sent when tapped, as a message) and
     `starts` (the text the message card opens with, for the user to finish,
     as **discuss** does now);
   - `command: true` to send `sends` as one of the agent's allowed
     commands rather than a message (for a later `cont` or `cmpct`);
   - `answers: true` when a tap answers the question, so its card goes, as
     yes, no, approve and implement do now.

   A sent answer is sent with `confirmed: true` (plan 20261023_02, the 13:25
   EDT amendment), at most 200 characters after replacement.
4. **The replacements**, filled by the daemon as `{{ [abc] name }}` slots:
   - `detected_plan_id`: "plan 06" when the question, or the rest of its
     reply, gives a plan id after "plan" or "draft" (the L3 amendments of
     13:21, 13:24 and 14:04 EDT, kept as they are in code); otherwise "the
     draft" when the question says "draft" and not "plan", and "the plan"
     otherwise;
   - `agent`: the agent's spoken name ("Claude");
   - `project`: the project's name ("flankr").

   An unknown slot is an error in the file (decision 6).
5. **On the wire.** `PendingQuestion` gains `answers: Vec<Answer>`,
   appended (`libflankr_proto` 0.3.13, a patch release by the owner's
   ruling), with `Answer { label, style, sends: Option<String>, starts:
   Option<String>, command: bool, answers: bool }`, texts already filled. The
   daemon also keeps filling `about_plan` and `plan` (as "the rule `plan`
   matched" and `detected_plan_id`) for older apps, and their stale doc
   comments are corrected.
6. **Loading.** The daemon reads the file at start and, when it is given by
   `--suggestions`, again on a change (plan 20261027_05's watcher). A rule or
   answer with an error (an unknown key, an answer named in `offers` that
   isn't defined, an unknown slot, both or neither of `sends` and
   `starts`) is logged once and left out; the rest load. If no rule loads,
   the daemon offers yes and no, as the app's fallback does.
7. **The app** draws `answers` when the question has them, in order, with
   their styles, and taps them by their texts. Without them (an older
   daemon), it keeps today's `actions`. The client's `Suggestion::Approve`,
   `Discuss` and `Implement` and `plan_answer` stay only for that fallback.

## 3. Steps

1. **`SUGGESTIONS.md` and `src/suggestions.rs`** (the daemon): the file with
   the two shipped rules and five answers (yes, no, approve, discuss,
   implement), the parser on libtypedmd, `when` and `unless` matching, the
   slots, and the errors.
   **Done when:** tests show the plan rule matching "Approve plan 03?" and
   "Approve the draft?" and not "Restart the daemon on the plan 05 build?",
   the default rule taking the rest, an `unless` word stopping a match,
   `detected_plan_id` filled from the question, from the reply, and as "the
   draft" and "the plan", and each error left out and logged.
2. **`libflankr_proto` 0.3.13:** `PendingQuestion.answers` and `Answer`,
   with `VERSIONS.md` and the tag.
   **Done when:** a question without `answers` still parses, and one with
   them round-trips.
3. **The daemon fills `answers`** in `question_in`, and `--suggestions` with
   its reload.
   **Done when:** a test peeks a session whose question is a plan question
   and gets approve, discuss and implement with "Approve plan 04" filled in,
   and an edit to a given file changes the next peek's answers.
4. **The client and the app draw and tap `answers`** (one commit each).
   **Done when:** tests show the answers drawn in order with their styles,
   a `sends` tap sending the filled text confirmed, a `starts` tap opening
   the message card with its text, and today's buttons when `answers` is
   empty.
5. **On the S23:** a yes/no question, a plan question with an id, and an
   "Approve the draft?" after a reply naming its plan.

Steps 1, 2 and 3 are one daemon or protocol commit each, 4 is two. 3 needs
1 and 2; 4 needs 2.

## 4. Open questions for the owner

1. **Where the rules run:** in the daemon, as written, so a change needs no
   app install and every app gets the same answers? Or in the app, from a
   build-time file like `SETTINGS_COPY.md`?
2. **Reloading the shipped file:** only a file given by `--suggestions` is
   watched, as written. Should the daemon also look for a user file (for
   example `~/.config/flankr/SUGGESTIONS.md`) that adds to or replaces the
   shipped rules, as `themes.toml` does for the app?
3. **The standing row** (cont and cmpct under the view): move it into the
   file as well, as a rule for idle agents, or leave it in the app?

## Amendment — 2026-10-27 14:13 EDT: the open questions answered: in the daemon; a user file merged over the shipped one; the standing row stays

The owner (2026-10-27): "to answer your questions: yes, in the daemon. 2. it
should merge the two if possible, with overrides via the ast, if that's not
possible just the shipped file for now. they might stay for now, with a
common_suggestions or idle_suggestions added later".

1. **The rules run in the daemon,** as decisions 1 and 5 have it. This
   answers open question 1.
2. **A user file, merged over the shipped one.** The daemon also reads
   `~/.config/flankr/SUGGESTIONS.md` when it exists, and merges it with the
   shipped file (or the `--suggestions` file, which still replaces the
   shipped one) on the parsed libtypedmd objects, not the text:
   - an object with the same tag and head as a shipped one (`# [Rule] plan`,
     `# [Answer] approve`) replaces it where it stands, so the order of the
     shipped rules is kept;
   - a new `[Rule]` goes before the shipped rules, in the user file's
     order, so it is tried first; a new `[Answer]` is added;
   - the merged set is then checked as decision 6 says, so a user answer an
     unknown rule names, or a broken user object, is logged and left out,
     and the shipped object it would have replaced stays.

   The user file is watched (plan 20261027_05's watcher) and the merge done
   again on a change. If merging by object turns out not to be possible
   with libtypedmd as it is, only the shipped file is read for now, and that
   is recorded in CHANGES.md. This replaces decision 6's "when it is given
   by `--suggestions`, again on a change" with: the `--suggestions` file
   and the user file are both watched. It answers open question 2.
   Step 3's **Done when:** adds a user file replacing `[Answer] approve`'s
   `sends`, adding a rule tried before `plan`, and a broken user object
   leaving the shipped one in place.
3. **The standing row (cont, cmpct) stays in the app** for now. A later
   rule set, such as `common_suggestions` or `idle_suggestions`, may move
   it into the file. This answers open question 3.

The plan stays `Draft` until the owner approves it.
