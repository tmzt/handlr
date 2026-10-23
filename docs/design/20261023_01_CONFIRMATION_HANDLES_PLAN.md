---
title: Confirmation handles, in-turn and out of sequence
date: 2026-10-23
status: Draft
covers:
  - "The two paths a confirmation handle takes: within the turn that made
    it, and out of sequence (a later turn, `confirm.status`, another
    principal, a standing confirmation)"
  - "An entropy measure for out-of-sequence matching: bits per matched
    word, a threshold, a margin, and what happens below them"
  - "What the companion shows and speaks, and when"
  - "Why loose matching never weakens approval"
  - "A review of `PROMPT.md` against the daemon as it is now, with minimal
    fixes"
  - "Not covered: spoken handles (plan 08 made them unspoken), changing
    the token or nonce, or rewriting the prompt"
sources:
  - "Owner (2026-10-23 01:26 EDT, plan 08's last amendment): Gemini
    validates the handle in the current turn; a handle that comes back later
    is matched by entropy; a comprehensive plan is wanted, with a prompt
    review"
  - "handlr at `89a51bd`: `src/core/confirm.rs` (`settle_handle`,
    `best_handle_match`, `loose_match`, `stands_apart`, `Broker::status`,
    `nonce`), `src/transport.rs` (`execute_run_command`, `VoiceTurn`),
    `src/server/companion.rs` (`confirm_text`, `confirm_requested`),
    `src/reasoning.rs` (`declare_gemini_tools`, `RunCommandParams`),
    `PROMPT.md` (61 lines)"
  - "The live run of 2026-10-23 05:11–05:17 UTC (`/tmp/claude-501/serve-real.log`):
    confirmation 2, Gemini's `send-continue-again-handlr` replaced by
    `sin-did-again`; confirmation 3, Gemini then sent
    `sin-continued-to-handler` itself"
  - "CHANGES.md, 2026-10-22 17:20 EDT, \"Confirmations: handle, token and nonce\""
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (the confirmation broker, decision 8)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (12:06 EDT: voice confirmations are made by the client; `confirm.requested` fields)
  - docs/design/20261022_05_SPOKEN_NAMES_PLAN.md (spoken names, spelled tab codes)
  - docs/design/20261022_08_PROMPT_CONFIRMATION_PLAN.md (the handle argument, loose matching, and the 01:26 EDT split this plan defines)
---

# Confirmation handles, in-turn and out of sequence

## 1. Goal

A confirmation has an **id**, a **handle** (a descriptive name written by
the requester, for voice turns by Gemini) and a **token** (three BIP-39
words, shown only to approvers). The approver proves it saw the token with
a **nonce**: `hex(BLAKE2s-256-MAC(key = token, "handlr.confirm/1" ||
confirm_id as 8 bytes big-endian || handle))`. The handle is never spoken.

Plan 08 made Gemini supply the handle and the daemon settle it. Its last
amendment split the problem in two and left the second half open:

- **In the turn**, Gemini's handle is right by definition: Gemini has the
  utterance and resolves speech-to-text errors ("sin" for "send"). The
  daemon normalises it and keeps it unique, and no longer rebuilds it from
  the transcript. The live run shows why: the daemon's rebuild produced
  `sin-did-again`, and on the next request Gemini copied that style and
  sent `sin-continued-to-handler` itself.
- **Out of sequence**, a handle must be matched by how much information
  its matched words carry, not by a word count. This plan defines that
  measure, its threshold and margin, where it is used, what the companion
  shows, why it never weakens approval, and what in `PROMPT.md` must
  change.

## 2. The two paths

### 2.1 In the turn

The path is `turn.start` → Gemini → `run_command {pane_id, command,
handle}` → `HerdrToolExecutor::execute_run_command` → `settle_handle` →
`Broker::request(…, Lifetime::VoiceTurn)` → the tool result
`awaiting_user_approval {handle}` → Gemini's one-sentence reply →
`confirm.requested` to the companion → the companion speaks
`confirm_text` and matches "approve" itself → `confirm.reply {confirm_id,
approve, nonce}`.

Nothing on this path matches a handle against another. The companion
answers by **id**, and the nonce is over the id and the stored handle. So
in the turn the daemon does exactly three things to the handle:

1. **Normalise** (`normalise_handle`: lowercase, runs of anything but
   `a-z0-9` folded to one `-`, trimmed, cut to 64).
2. **Fill in** only a missing or empty handle, from the command and the
   agent's spoken name (never from the utterance).
3. **Keep it apart** from the requester's pending handles: at least 3
   words, `-2`/`-3` for an exactly equal pending handle, and a
   distinguishing word (the tab code spelled out, else a BIP-39 word) until
   it *stands apart* under section 3's measure.

`HandleRequest.utterance` and the two-thirds check in `settle_handle`
(`from_user`) go. `VoiceTurn.utterance` stays, for logging only.

### 2.2 Out of sequence

A handle arrives out of sequence when the thing that carries it is not the
`run_command` call that made it:

| Where | Method or path | Who | Pending set matched against |
|---|---|---|---|
| Status by handle | `confirm.status {handle}` → `Broker::status(StatusKey::Handle)` | the requester (the broker answers no one else) | the requester's open confirmations that have a handle |
| A later turn | a future Gemini tool that names a confirmation (none exists today; `run_command` never takes a handle back) | the requester's session | the same |
| Another principal | `confirm.reply {handle, approve, nonce}` (step 4, new: `handle` in place of `confirm_id`) | an approver, such as `handlr ctl` reading the handle off the phone | the open confirmations that approver was sent `confirm.requested` for |
| Standing confirmation | `agents.send` by an MCP requester, then `confirm.status {handle}` up to an hour later (`DEFAULT_TTL`) | the MCP requester | its own open confirmations |

All four use one function, `confirm::entropy_match`, in place of
`best_handle_match`'s `loose_match`. An exactly equal handle (after
normalising) still wins first, so `X` and `X-2` are told apart.

## 3. The entropy measure

### 3.1 Words

Both the returned string and each candidate handle are split by
`match_words` as today: lowercase, split on `-`, space and punctuation,
digits written as words. Two words match by `words_match` (edit distance
at most 1, or 2 from 6 letters; or one word equals two adjacent words
joined). Each candidate word is matched at most once.

### 3.2 Bits per matched word

A matched word `w` of candidate `H` is worth

    bits(w) = -log2( max( prior(w), shared(w) / n ) ) - fuzzy(w)

where

- `n` is the number of candidates and `shared(w)` is how many of the
  **other** candidates contain a word matching `w`. A word every pending
  handle shares is worth at most `-log2((n-1)/n)`, under 1 bit; a word only
  this handle has keeps its prior's value;
- `fuzzy(w)` is 1 when the match is not exact (edit distance or join),
  floor 0: a near-miss says less than the word itself;
- `prior(w)` comes from a `WordPriors` table the server builds from the
  config and the herdr snapshot and sets on the broker
  (`Broker::set_word_priors`), refreshed when either changes. Without one,
  only the stop words and digits apply.

| Class | prior | bits | Examples |
|---|---|---|---|
| Stop words | 1 | 0 | `a an the to in on and of for it that this please send run tell ask agent pane now again` |
| Command words | 1/4 | 2 | every word of every `allowed_commands` entry of every agent: `continue compact usage clear` |
| Agent words | 1/4 | 2 | each agent's spoken name and `[[agents]]` name: `claude omp agy handlr` |
| Project and workspace names | 1/4 | 2 | `highbay bubble` |
| Digit words | 1/8 | 3 | `two three` (the `-2` suffix) |
| Spelled letters | 1/26 | 4.7 each | `aitch bee` for tab code `HB` |
| Any other word | 1/64 | 6 | `tests deploy branch` |
| The recorded distinguishing word | 1/2048 | 11 | a BIP-39 word the daemon appended |

The broker records which word it appended as the distinguishing word
(`Entry.distinguisher: Option<String>`), so a BIP-39 word that is also an
ordinary English word is still worth 11 bits in the handle it was added
to. A stop word never counts, even when it is the only match.

### 3.3 The rule

For a returned string `R` and candidates `H_1..H_n`, with `E_i` the sum of
bits over `R`'s matches in `H_i`:

1. **Exact.** If `normalise_handle(R)` equals a candidate, that one.
2. **Coverage.** If every non-stop word of **exactly one** candidate is
   matched, that one. A full match in other words or another order is not
   a chance match, whatever its bits.
3. **Entropy.** Otherwise the candidate with the highest `E_i` is chosen
   when `E_i >= 6` bits and, if `n >= 2`, `E_i - E_runner >= 3` bits.
4. **Nothing.** Otherwise the result is `Ambiguous(candidates with E >=
   3)` or `None`. `confirm.status` then returns a new error, `-32009
   ambiguous handle` (the next free code after `-32008 BadNonce` in
   `ConfirmError::code`), with `data.pending` listing the requester's pending handles,
   newest first, as `confirm.status` already does on a miss. A Gemini tool
   would say the same in its result so the model can ask the user which
   one.

The numbers: 6 bits is one distinctive word, or a command word and an
agent word with a project; 3 bits of margin is one digit word or more
than one shared word. Below these, a match on words such as "claude" and
"highbay" alone, which every request to that agent shares, is not taken.

### 3.4 Worked examples

Pending (A) `send-continue-claude-highbay`, (B) `send-usage-claude-highbay`,
n = 2. `claude` and `highbay` are shared by the other: `max(1/4, 1/2)` →
1 bit each. `continue` and `usage` are not shared → 2 bits.

| Returned | A | B | Result |
|---|---|---|---|
| `send usage claude highbay` | — | coverage | B |
| `usage claude highbay` | 2 | 4, coverage | B (coverage; bits alone would fail the 6-bit floor) |
| `send claude highbay` | 2 | 2 | none: tie, below floor; list A and B |
| `the usage one` | 0 | 2 | none: 2 bits does not rule out chance; list A and B |
| `continue claude` | 3 | 1 | none: 3 < 6; list A and B |

Pending (A) and (A2) `send-continue-claude-highbay-2`, after the same request
twice. Returned `continue claude highbay two`: A covers fully (3 bits),
A2 covers fully (1+1+1 from shared words, `two` 3 bits = 6); two
coverages fall to entropy: 6 ≥ 6 and margin 3 → A2. Returned `continue
claude highbay`: only A is fully covered → A.

Pending (A) and (C) `continue-claude-highbay-ripple`, where `ripple` was the
daemon's distinguishing word. Returned `the ripple one`: C scores 11 → C.
Returned `continue claude highbay`: only A is fully covered → A. With a
tab code instead, `continue-claude-highbay-aitch-bee`, `aitch bee` alone
scores 9.4 → C.

One pending handle, n = 1: no margin applies, and coverage or 6 bits is
enough. `send usage claude highbay` returned for `send-usage-claude-highbay`
is coverage; `the usage` alone (2 bits) is not, and the one pending handle
is listed, which is the right answer to a vague reference an hour later.

### 3.5 Standing apart

`stands_apart(handle, pending)` becomes: `entropy_match(handle, pending +
[handle])` chooses `handle` itself. A handle equal to a pending one gets
`-2` first, as today; one that fully covers a pending one (`continue-
claude-highbay` against `send-continue-claude-highbay`, since `send` is a
stop word) gets a distinguishing word, as today, because two coverages
with equal bits have no margin.

## 4. The companion: what it shows and speaks

Handles are not spoken. The companion already receives `handle` in
`confirm.requested` and `text` without the "For …" ending for a voice turn
(`confirm_text` with `None` when `lifetime == VoiceTurn`). This plan adds
no protocol field; it fixes what the phone does with what it has:

1. **On `run_command`'s result,** Gemini says one sentence ("Sending
   continue to Claude in Highbay once you approve."). Nothing shown yet.
2. **On `confirm.requested`,** the companion speaks `text` and shows a
   card: the command in quotes, the agent's shown name and project, and
   the handle as words (`send continue claude highbay`) in smaller type,
   with the id. The token is stored, not shown: the phone is the approver
   and computes the nonce itself. A card for a `Standing` confirmation
   (from an MCP requester) is spoken with the "For …" ending, as plan 02
   decided, since there is no turn to tie it to.
3. **On "approve" or anything else,** the card shows "Approving…" then
   the `confirm.resolved` outcome: "Sent", "Denied", "Expired" (60 s for
   a voice turn), "Cancelled". A wrong-nonce error shows the remaining
   tries.
4. **Out of sequence,** `handlr ctl confirm.status --handle "…"` prints the
   matched handle, how it matched (`exact`, `coverage`, `entropy <bits>`)
   and the outcome; on `-32009` it prints the pending handles. The phone
   has no out-of-sequence path until a Gemini tool takes a handle.

## 5. Security

**Matching chooses, the nonce approves.** `entropy_match` only decides
which pending entry a string refers to. The nonce is computed over the
**stored** handle and id, keyed by that entry's token (`nonce`,
`nonce_matches`, constant time). A loose match therefore cannot approve
anything: an approver still needs the token of exactly that entry, which
only approvers were sent in `confirm.requested`. A nonce for entry A is
useless for entry B even with the same handle text, because B's id and
token differ.

**A confused model** (wrong, garbled or copied handle) can, in the turn,
only name its own new confirmation badly; the user still hears the real
command and agent in `confirm_text`. Out of sequence it can only query the
status of its own entries (`Broker::status` filters by `requested_by`), or
be told the match is ambiguous. A requester cannot approve its own request
(`a_requester_cannot_approve_its_own_command`).

**An attacker** who can reach the socket but is not an approver sees no
token, so no nonce verifies. One who is an approver can approve only what
they were asked to approve, by id or by an unambiguous handle; a
misdirected approval needs the other entry's token too. Five wrong nonces
deny the entry (`MAX_BAD_NONCES = 5`, `five_wrong_nonces_deny`), and a
voice turn expires in 60 s. Status queries by handle leak nothing across
principals and are not rate limited; they read only the caller's entries.

**What is not defended:** a user who approves without listening. The
measure cannot help there; the spoken question names the command and
agent, and that stays.

## 6. `PROMPT.md` review

Section by section against the daemon at `89a51bd`. The owner wants the
prompt kept largely as Gemini wrote it, so each fix is the smallest that
removes a real conflict.

**Must fix**

1. **Line 35, the handle's source.** "taken from the user's own words"
   produced confirmation 3's `sin-continued-to-handler`: after the daemon
   rebuilt one handle from the transcript, Gemini copied the transcript's
   misheard words into its next. Under the 01:26 decision Gemini is the
   one who validates. Replace the clause with:

   > … with a `handle`: a short name for this action in lowercase words
   > joined by `-`, naming the command and the agent (and its project) as
   > the user meant them, e.g. `send-continue-claude-highbay`. Spell the
   > command and agent correctly even when the transcript misheard them.
   > handlr matches the approval to it. Do not ask "Should I send…?"
   > yourself.

2. **Line 19, the tool's arguments.** `run_command(pane_id, command)`
   omits `handle`, which `declare_gemini_tools` marks required
   (`test_run_command_declares_a_required_handle`). Make it
   `run_command(pane_id, command, handle)`.

3. **Line 11, inspection tools.** "You do not need to call inspection
   tools (`list_panes`, `inspect_pane`, etc.)" names tools that are not
   declared. A model told it need not call them may still try. Change to
   "There are no inspection tools: all current multiplexer state and
   terminal logs are already in your turn context."

**Optional**

4. **Line 37.** "Never call `run_command` again for the same request" is
   right for the turn, but after a denial the user may ask again. Add "in
   this turn"; a later request is a new one.
5. **Line 31.** "confirm that rephrasing with the user" reads as a question
   before the call. Say "state the rephrasing in your reply; handlr's
   approval question shows the exact text". Plan 08 left this; it is still
   harmless when Gemini states it and calls in the same turn.
6. **Line 40.** "the confirmation exchange required for `run_command`" is
   the old gate's language. "the approval handlr asks for after
   `run_command`" is accurate; the meaning is unchanged.
7. **Line 60.** "in workspace two" is what plan 05 replaces with project
   names ("in Highbay"). The example could use one. Leave unless replies
   show numbered workspaces.
8. **Line 1.** "voice reasoning engine", "compiler jobs": plan 08's open
   question. Left.

**Should the prompt say more about handles?** No. The out-of-sequence
paths do not involve Gemini today, and item 1 as fixed above already asks
for the distinctive words (command, agent, project) that give a handle its
bits. One sentence may be added to the `run_command` declaration instead,
where it costs no spoken-prompt attention: "Use the command's and agent's
real names; the returned `handle` is the name handlr kept." If a later
plan gives Gemini a tool that takes a handle back, that tool's description
carries the rules, not the prompt.

## 7. Steps

1. **In-turn trust.** In `settle_handle`, drop `from_user` and the
   utterance rebuild: a given handle is kept after normalising and
   padding; only a missing or empty one is built from the command and
   agent. Remove `HandleRequest.utterance`. Replace the test
   `a_handle_not_from_the_utterance_is_replaced` with
   `a_garbled_utterance_keeps_geminis_handle` ("sin did again" with
   `send-continue-again-handlr` → `send-continue-again-handlr`). Update
   `test_voice_turn_confirmations_carry_a_settled_handle` in
   `src/transport.rs` likewise.

   **Done when:** `cargo test -p handlr` passes; `settle_handle` reads no
   utterance; the garbled test passes.

2. **The measure.** `confirm::WordPriors` (stop words and digits built in;
   `with_commands`, `with_agents`, `with_projects`, `with_letter_names`),
   `Broker::set_word_priors`, `Entry.distinguisher`, and
   `entropy_match(returned, &[HandleInfo]) -> HandleMatch { Exact(i),
   Coverage(i), Entropy(i, bits), Ambiguous(Vec<usize>), None }`.
   `best_handle_match`, `loose_match` and `scores_two_thirds` go;
   `stands_apart` uses the new function. `settle_handle` returns the
   distinguishing word it added so the broker can record it. Unit tests
   for every row of section 3.4, for `stands_apart` on `continue-claude-
   highbay` against `send-continue-claude-highbay`, and for the fuzzy
   discount (`sin` against `send` is worth 0, not 1 less than 0).

   **Done when:** those tests pass, and
   `the_nonce_is_over_the_stored_handle_after_a_loose_match` still passes
   with a coverage match and an entropy match.

3. **Status and priors.** `Broker::status` uses `entropy_match`;
   `Ambiguous` becomes `-32009` with `data.pending`. The server builds
   `WordPriors` from `config.agents` (allowed commands, names, spoken
   names), the snapshot's project and workspace names, and `[naming]`'s
   letter names, and sets them at start and on every snapshot refresh.
   `handlr ctl confirm.status --handle` prints the match kind and bits.

   **Done when:** `status_by_handle_is_private_to_the_requester` and a new
   `status_by_handle_lists_pending_when_ambiguous` pass; `handlr ctl
   confirm.status --handle "the ripple one"` against `serve --mock` with
   two pending handles returns the distinguished one.

4. **Reply by handle** (after the owner's answer to open question 2).
   `confirm.reply {handle, approve, nonce}` with `handle` in place of
   `confirm_id`, matched among the open confirmations that approver was
   sent; the nonce is over the matched entry's id and stored handle. Wrong
   nonces count toward the matched entry's five.

   **Done when:** a `handlr ctl confirm.reply --handle … --approve` with the
   right token sends once; with another entry's token it is refused and
   counts one bad nonce.

5. **Prompt and declaration.** Section 6's must-fix items 1 to 3, the
   `run_command` declaration sentence, and the optional items the owner
   picks. The `src/reasoning.rs` prompt test gains "There are no inspection
   tools" and `run_command(pane_id, command, handle)`.

   **Done when:** `git diff PROMPT.md` touches only the lines named; fmt,
   clippy and test pass.

6. **Companion card** (companion crate, its own commit): the card of
   section 4, items 2 and 3.

   **Done when:** a voice-turn confirmation shows the command, agent,
   project and handle words, speaks no handle, and shows the outcome.

7. **Live check on the S23** (`adb timothys-s23-ultra:5555`), against the
   owner's live herdr with the handlr agent pane, as on 2026-10-23:
   - "send continue to claude in bubble" misheard or not: the
     `confirm.requested` handle is Gemini's (`send-continue-claude-bubble`
     or close), never built from the transcript;
   - the same request again while the first is pending: `-2`;
   - "approve" sends once; the phone shows "Sent";
   - from the Mac, `handlr ctl confirm.status --handle "continue claude
     bubble"` while both are pending: `-32009` with both listed;
     `--handle "continue claude bubble two"`: the second.

   **Done when:** all four hold in one session and the log is appended to
   `out/` per Rule 3.5.

## 8. Open questions

1. **The thresholds.** 6 bits and a 3-bit margin are set so that a command
   word plus a shared agent and project do not match on their own. Does
   the owner want vaguer references ("the usage one", 2 bits) to be taken
   when only two handles are pending, or always to be asked back?
2. **Reply by handle** (step 4): is approving from `handlr ctl` by handle
   wanted now, or is approving by id, read off the phone's card, enough?
3. **`-2` versus a distinguishing word.** A digit suffix is worth 3 bits and
   is easy to mishear. Should an exact duplicate get the tab code or a
   BIP-39 word instead of `-2`, as the loose duplicate does?
4. **Priors from the snapshot.** Project names change as workspaces open
   and close. Is refreshing `WordPriors` on each snapshot acceptable, or
   should the priors be fixed from the config only?
5. **The optional prompt items** (section 6, 4 to 7): which, if any, go in
   with step 5?
