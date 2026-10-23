---
title: PROMPT.md and daemon-side approval
date: 2026-10-22
status: In implementation
covers:
  - "An analysis of `PROMPT.md`, the runtime system prompt, against the
    daemon as it is now (plans 20261022_01, 02 and 05)"
  - "The smallest change that stops the prompt from contradicting
    daemon-side approval: its Mechanical Confirmation Gate block, and the
    `run_command` tool description"
  - "Not covered: rewriting the rest of the prompt. It was written by
    Gemini, and the owner wants it kept largely as it is"
sources:
  - "Owner (2026-10-22): \"the PROMPT.md was written by Gemini, we probably
    don't want to make big changes to it\"; \"write that as a plan with your
    analysis of the prompt\""
  - "handlr at `dd6f751`: `PROMPT.md` (67 lines, last changed in `888c64f`);
    `src/transport.rs` (`HerdrToolExecutor`, the `awaiting_user_approval`
    result); `src/reasoning.rs` (`declare_gemini_tools`)"
  - "CHANGES.md, 2026-10-22, the entry for plan 20261022_02 step 4, which
    records this conflict as open"
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (the confirmation broker)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (12:06 EDT: voice confirmations are made by the client)
  - docs/design/20261022_05_SPOKEN_NAMES_PLAN.md (pane ids are never spoken)
---

# PROMPT.md and daemon-side approval

## 1. Goal

`PROMPT.md` is compiled into handlr (`include_str!` in `src/reasoning.rs`)
and sent to Gemini as the system prompt on every turn. Gemini wrote it while
handlr was a desktop voice loop, when the model itself drove the
confirmation gate. Confirmations now belong to the daemon:

1. `run_command` asks the broker for approval and returns at once with
   `"status": "awaiting_user_approval"`.
2. After the model's reply, handlr asks the user `Approve sending
   "<command>" to <agent>?`.
3. The user's answer goes to the broker, not to the model.

The prompt still tells the model to do the asking itself. This plan finds
what in the prompt conflicts with the daemon, and changes only that.

## 2. Analysis

The prompt has a preamble and five sections. Each one against the code:

| Part | Lines | Holds today? |
|---|---|---|
| Preamble (role) | 1 | Yes. It names Claude, Omp and Agy, which are still the default `[[agents]]`. |
| 1. RAG-Based Environment Context | 5–11 | Yes. `HerdrContextContributor` still injects the snapshot and each pane's recent lines, and no inspection tools are declared. |
| 2. Action Tools | 15–21 | Yes. The four tools listed are exactly `declare_gemini_tools`. The argument is still called `pane_id`, though the naming plan lets it be a name. |
| 3. Permitted Commands per Agent | 27–34 | Yes. The allow list and `validate_arbitrary_command`'s character rules match `src/commands.rs`. |
| 3. Mechanical Confirmation Gate (Strict) | 35–43 | **No.** See below. |
| 3. Watching Panes | 45–49 | Yes. Watching never needs approval. "Without the confirmation exchange required for `run_command`" is still true, because `run_command` still needs approval, now given by handlr. |
| 4. Data Processing & Situational Awareness | 53–58 | Yes. |
| 5. Voice Output Guidelines | 62–67 | Yes. Line 66 already says to use natural references, not "Workspace w2, Pane p4". |

**The conflict, lines 35–43.** The block says:
- "**NEVER** execute `run_command` in the initial request turn";
- "**Always ask for vocal confirmation first**";
- "Quote the command exactly … The gate is armed from the wording of your
  question";
- "Only invoke `run_command` when your immediately preceding assistant turn
  asked for confirmation AND the user's latest response is an affirmative".

That was the old `VoiceState::confirmation_gate` (AGENTS.md section 3.B),
which read the model's question to arm itself. That gate is gone. With real
Gemini, following the block now gives:

1. **Two questions.** The model asks "Should I send /usage to Claude?" in
   words. When the user says yes, it calls `run_command`, and handlr asks
   "Approve sending "/usage" to Claude?" a second time.
2. **The wrong answer path.** The user's first "yes" goes to the model, not
   to the broker. Only the second answer, "approve", sends the command.
   This undoes plan 02's 12:06 EDT decision that confirmations are made by
   the client.
3. **A pane id spoken aloud.** The second example says "Claude in pane
   w2:p1", which plan 05 forbids.

The tool result itself already tries to correct this. `awaiting_user_approval`
carries a `note`: "Do not ask for confirmation yourself and do not call
run_command again for this; say in a few words that it is waiting for their
approval". But the note arrives only after the call, and the block tells the
model not to make the call in the first place.

**Also stale: the `run_command` declaration** in `declare_gemini_tools`
ends "Requires explicit vocal confirmation from the user if configured for
that agent." Gemini reads that as its own job, the same as the block.

**Small things left alone:**
- **Line 31's "confirm that rephrasing with the user"** for a rephrased
  arbitrary command. It doesn't cause a second question when the model
  states the rephrasing in its reply, and handlr's approval question shows
  the exact text anyway.
- **"pane 6" (line 47) and "pane 2" (line 55)** are examples of what the
  user might say, not what the model says. They stay.
- **The tools' `pane_id` and `target_id` arguments** stay named as they are.
  `HerdrToolExecutor` already resolves agent names.

## 3. Decisions

1. **Replace lines 35–43 only**, with this text in the prompt's own style:

   ```markdown
   * **Confirmation Gate (handled by handlr):**
     1. When the user asks to send a command to an agent, call `run_command` in that turn. Do not ask "Should I send…?" yourself.
     2. If the result is `awaiting_user_approval`, handlr asks the user to approve it right after your reply. Say in a few words that it is waiting for their approval (e.g. "Sending /usage to Claude in Highbay once you approve."). Never ask for confirmation yourself, and never say it was sent.
     3. The user's "approve" or "no" goes to handlr, not to you. Never call `run_command` again for the same request.
   ```

   This agrees with the `note` the tool result already carries. Everything
   else in `PROMPT.md` is byte-for-byte unchanged.
2. **The `run_command` declaration's** last sentence becomes "handlr asks
   the user to approve it when that agent requires approval." The rest of
   the description is unchanged.
3. **A test** in `src/reasoning.rs` checks that the embedded prompt contains
   `awaiting_user_approval` and does not contain "NEVER** execute", so the
   old block cannot come back unnoticed.

## 4. Steps

1. **The change.** Decisions 1 to 3 in one handlr commit: `PROMPT.md` is
   compiled into the binary, so it goes with `src/reasoning.rs`. Add a
   `CHANGES.md` addendum naming this plan.

   **Done when:**
   - fmt, clippy and test pass for `-p handlr`;
   - `git diff` of `PROMPT.md` touches only the gate block.

2. **A live turn.** With a Gemini key set by the owner (in
   `.private/gemini_api_key`), against `handlr serve --mock`, so no live
   herdr pane receives anything:
   - a companion says "send continue to claude";
   - the reply is one sentence saying it is waiting for approval, followed
     by exactly one `Approve sending "continue" to …?`;
   - "approve" sends it once;
   - "no" sends nothing, and the model is not asked again.

   **Done when:** that holds for three requests: an allowed command, an
   arbitrary one, and one that is declined.

## 5. Open questions

1. Should the prompt's other references to the old design (the preamble's
   "voice reasoning engine", and "compiler jobs") be left for good, or
   revisited if Gemini's replies show they confuse it? This plan leaves
   them.

## Amendment — 2026-10-22 23:40 EDT: the model names the confirmation (its handle)

On the owner's instruction. The confirmation the model causes must carry
the confirmation name, its **handle**. The protocol matches the handle,
and the companion joins it with the token it stores, so the approval stays
tied to what the model meant (plan 20261022_01's broker; CHANGES.md
2026-10-22, "Confirmations: handle, token and nonce"). Today
`HerdrToolExecutor` passes no handle to `ConfirmBroker::request`.

This adds to decisions 1 and 2 and to steps 1 and 2. Where they disagree,
this amendment wins.

**`run_command` gains a `handle` argument** (string, required in the
declaration):
- It is a short name for this one action: lowercase words joined by `-`,
  naming the command and the agent, for example `usage-claude-highbay` or
  `continue-handlr-agent`.
- The declaration describes it in those words.

**The daemon** passes the handle to the broker. The broker's own rules
apply (3–64 characters of `a-z`, `0-9` and `-`, unique among the
requester's pending confirmations), with two allowances, so a slightly
malformed name never blocks a request:
- **Normalised first:** lowercased, runs of anything else folded to one
  `-`, leading and trailing `-` trimmed, cut to 64.
- **When still invalid or missing:** the daemon makes the handle from the
  command and the agent's spoken name, normalised the same way.
- **When already pending:** `-2`, `-3` and so on are appended.

The result returned to the model includes `"handle"`, so the model's
session keeps it. The approval question is the existing `confirm_text`:
`Approve sending "/usage" to Claude in Highbay? For usage claude highbay.`
The companion answers with the nonce over its stored token, the id and
this handle, as the protocol already defines.

**Decision 1's gate block**, item 1, becomes:

```markdown
  1. When the user asks to send a command to an agent, call `run_command` in that turn, with a `handle`: a short name for this action in lowercase words joined by `-`, naming the command and the agent (e.g. `usage-claude-highbay`). handlr reads the handle to the user and matches their approval to it. Do not ask "Should I send…?" yourself.
```

Items 2 and 3 are unchanged.

**Step 1 adds tests:**
- a given handle reaches the broker and the companion's `confirm.requested`;
- `Usage Claude!` becomes `usage-claude`;
- a missing or invalid handle is derived from the command and the agent;
- a handle that is already pending gets `-2`;
- the nonce computed over the returned handle approves, and one over a
  different handle does not.

**Step 2's live turn** also checks that the spoken question ends with "For
<the model's handle, read as words>".

## Amendment — 2026-10-22 23:44 EDT: the handle comes from the user's words, is not spoken, and is matched loosely on its way back from Gemini

On the owner's instruction. This replaces three parts of the 23:40 EDT
amendment:
- the spoken "For <handle>" in the approval question;
- the rules for when a handle is invalid;
- step 2's check of the spoken ending.

Everything else in that amendment stands: the required `handle`
argument, the normalising, `-2` for a handle already pending, and the
handle returned to the model.

**Written from the user's words.**
- Gemini writes the handle from what the user said in the turn. For
  example, "send usage to claude in highbay" becomes
  `send-usage-claude-highbay`.
- The daemon checks the handle against the turn's utterance. At least
  two-thirds of the handle's words must match words of the utterance, by
  the word rule below.
- A handle that fails this check, or is missing, is replaced. The daemon
  builds one from the utterance's words, normalised, after dropping
  articles ("a", "an", "the", "to", "in", "on").
- This ties the confirmation to the request the user made.

**Not spoken.**
- The approval question is `Approve sending "<command>" to <agent>?`, with
  no "For …" ending. For voice confirmations, `confirm_text` omits the
  handle.
- The handle still goes to the companion in `confirm.requested`, which
  may show it on screen, and the nonce is computed over it.

**Matched loosely on the way back.**
- The tool result gives Gemini the stored handle. When Gemini later
  returns a handle (in a tool call, or in `confirm.status` by handle),
  the daemon matches it against the requester's **pending** handles.
- **Normalising:** lowercase, split on `-`, spaces and punctuation, and
  write digits as words.
- **Word rule:** two words match when either:
  - their edit distance is at most 1, or 2 for words of 6 letters or more;
    or
  - a word matches two adjacent words joined ("highbay" and "high bay").
- **Score:** the share of the stored handle's words that the returned
  string matches, in any order.
- **Accepted:** only a best match scoring at least 2/3 that also matches at
  least one more word than the second best. Otherwise nothing matches, and
  the tool says which handles are pending.

**The exact string is what is checked.** The nonce is always computed over
the stored handle, never the returned one. Loose matching only chooses
which pending confirmation is meant, and the approval itself still needs
the token.

**Entropy.**
- A handle has at least 3 words. A shorter one is replaced, as above.
- A new handle that would match a pending one under the rule above gets
  a distinguishing word appended: the agent's tab code spelled out, or
  else a word from the BIP-39 list.
- So any two pending handles stay far enough apart that a returned
  handle cannot match both.

**Step 1's tests** gain:
- a handle not drawn from the utterance is replaced;
- "send usage claude high bay" and "usage-claude-highbay" match
  `send-usage-claude-highbay`;
- with `send-usage-claude-highbay` and `send-continue-claude-highbay`
  pending, "send claude highbay" matches neither;
- a new handle too close to a pending one gets a distinguishing word;
- the nonce over the stored handle approves after a loose match.

**Step 2's live turn** checks:
- the question has no "For …" ending;
- the `handle` in `confirm.requested` uses the user's words.

## Amendment — 2026-10-23 01:08 EDT: item 1's wording; implementation started

On the owner's approval. Item 1 of the gate block, as the 23:40 EDT
amendment wrote it, said "handlr reads the handle to the user". The later
amendment made the handle unspoken and drawn from the user's words, so
item 1 is:

```markdown
  1. When the user asks to send a command to an agent, call `run_command` in that turn, with a `handle`: a short name for this action in lowercase words joined by `-`, taken from the user's own words (e.g. `send-usage-claude-highbay`). handlr matches the approval to it. Do not ask "Should I send…?" yourself.
```

Step 1 is implemented in the commit that adds this amendment. The status is
now `In implementation`, and this document is immutable (AGENTS.md, Rule
1.5).

**Decided during implementation:**
- **Exact first.** An exactly equal handle, after normalising, wins before
  loose matching, so a pending `X` and `X-2` can be told apart.
- **What "stands apart" means.** A new handle gets a distinguishing word
  unless it would be chosen by loose matching alone among the pending
  handles and itself. The earlier wording, "would match a pending one",
  can never be met by adding words.
- **Where loose matching applies.** Only to `confirm.status` by handle:
  none of Gemini's tools take a handle back yet. A `confirm.status` by
  handle that finds nothing now lists the requester's pending handles.

**Step 2,** the live turn, follows with the daemon on the owner's live
herdr.

## Amendment — 2026-10-23 01:26 EDT: Gemini validates the handle in the current turn; entropy matching is for handles that come back later

On the owner's instruction, after the live test of 01:13 EDT. Gemini sent
`send-continue-again-handlr` for a request that speech-to-text heard as
"sin did again". The daemon's check against the utterance failed, so it
replaced Gemini's handle with `sin-did-again`.

This replaces two parts of the 23:44 EDT amendment: the check of the
handle against the turn's utterance, and building the handle from the
utterance.

**Within the turn** (the `run_command` call, and the companion's display
and immediate approval):
- The handle Gemini supplies is accepted as given, after normalising.
  Gemini validates it against what the user meant: it has the utterance
  and resolves speech-to-text errors such as "sin" for "send". The daemon
  no longer checks it against the utterance's words.
- Only a missing or empty handle is built by the daemon, from the command
  and the agent's spoken name.
- **Unchanged:**
  - the 3-word minimum, padded as before;
  - `-2` for an exactly equal pending handle;
  - the distinguishing word when it would not stand apart among the pending
    handles;
  - the handle returned to Gemini.

**Out of sequence** (a handle that comes back after the turn: a later
turn, `confirm.status` by handle, another principal, or a standing
confirmation):
- The handle is matched by **entropy**, not by word overlap alone. A
  match is accepted only when the words it matches carry enough
  information to pick out one pending handle and to rule out a chance
  match.
- Until the measure is defined, the existing loose rule (at least 2/3 and a
  margin of one word) and the distinguishing word stay in place. The
  comprehensive plan the owner has asked for defines the measure, its
  threshold and its tests.
- The nonce is still always over the stored handle.

**Step 1's tests change:**
- the test that "a handle not drawn from the utterance is replaced" is
  removed;
- a new test checks that a handle Gemini supplies within the turn is kept
  when the utterance is garbled ("sin did again" with
  `send-continue-again-handlr`);
- the rest are unchanged.
