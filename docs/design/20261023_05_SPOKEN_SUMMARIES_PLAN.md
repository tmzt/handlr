---
title: Spoken summaries of complex replies, from structured prompts
date: 2026-10-23
status: Implemented
covers:
  - "Read aloud (the agent view's play button) sends a reply that is
    complex, by a basic coding measure or because it has backquotes, to
    Gemini for a short spoken summary under the reading rules, instead of
    reading it word for word"
  - "ADDITIONAL_PROMPTS.md: structured prompts other than the main
    PROMPT.md, read with libtypedmd; the first is the spoken summary"
  - "A daemon method, `speech.summarize`, that runs a prompt with no tools
    and no session history"
  - "Not covered: changing PROMPT.md; summaries for Gemini's own turns
    (they are already spoken text); other prompts beyond the first"
sources:
  - "Owner (2026-10-23): \"the PROMPT should abbreviate a path like the one
    in the last spoken message (I only need the English parts of it)\";
    \"(such as \\\"the overview screen plan\\\")\"; \"right, what we should be
    doing is passing those messages to Gemini if they are complex (such as
    a basic coding metric) or contain backquotes. This would get the basic
    request to summarize using the rules we just discussed. this would be a
    new plan, using libtypedmd to read a new ADDITIONAL_PROMPTS.md
    containing the structured prompts\""
  - "The reading rules of the 15:39 EDT CHANGES.md entry, in
    `handlr_companion::speakable`: links as their text; a path as \"the\"
    and the English words of its last part; inline code as its words; a
    number or version as it is; a commit hash not at all; no code blocks,
    Markdown marks, headings or bullets"
  - "libtypedmd v0.2.0 (`a61c091e6420a1dfc40e89b8600cd26e628e5f7a`,
    github.com/tmzt/libtypedmd): Markdown as a typed object graph; a
    `# [Tag] head` object with `- name: value` members, `- name: |` literal
    blocks (0.2.0), and `{{ [abc] expr }}` slots whose resolution is the
    consumer's; `parse(&str) -> Stream`; zero dependencies"
  - "handlr at `2931563`:
    - `src/reasoning.rs` (`PROMPT_MD_RAW = include_str!(\"../PROMPT.md\")`,
      `parse_system_prompt`, `load_system_prompt_from_file`);
    - `src/server/companion.rs` (the Gemini engine built from the key in
      `.private/gemini_api_key`, `turn_start`);
    - `crates/libinfer/libinfer` (`ReasoningEngine::process_turns(&[Message],
      &dyn ToolExecutor)`), `crates/libinfer/libinfer-gemini`
      (`GeminiReasoningEngine::with_model(key, model, system_prompt, tools)`);
    - `src/sessions/mod.rs` (entries redacted, then cut to
      `ENTRY_SUMMARY_MAX`, 500 characters, for clients);
    - `crates/handlr-companion/src/client.rs` (`Input::ReadAloud`,
      `peeked` queueing replies as notices, `speakable`)"
related:
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3, the agent view and its read aloud; redaction, decision 7)
  - docs/design/20261022_07_LIBINFER_PROVIDERS_PLAN.md (libinfer and libinfer-gemini, which run the prompt)
  - docs/design/20261022_08_PROMPT_CONFIRMATION_PLAN.md (PROMPT.md, which this plan leaves alone)
  - docs/design/20261023_03_SESSION_SUBSCRIPTIONS_PLAN.md (sections and serials, which can later name the reply instead of its time)
---

# Spoken summaries of complex replies, from structured prompts

## 1. Goal

Read aloud speaks an agent's replies. A plain reply reads well word for
word. A reply about code does not: paths, identifiers, hashes and lists of
changes read badly even after `speakable` cleans them, and a long one
takes 40 seconds to say. Such a reply should be summarised for the ear by
Gemini, under the same reading rules, in a sentence or three.

The prompt for that is not part of PROMPT.md, which Gemini wrote and which
stays as it is. It lives in a new `ADDITIONAL_PROMPTS.md`, structured with
libtypedmd so the daemon reads each prompt's settings and text as data.

## 2. Decisions

1. **The phone decides what is complex; the daemon summarises.** The
   client already has the reply when it queues it for reading. It sends
   only complex replies to the daemon, which holds the Gemini key.
2. **Complex is a basic coding measure** over the reply as the client has
   it, computed by `handlr_companion::complexity(&str) -> Complexity`:
   * it has a backquote: complex;
   * else its words are counted (split on whitespace), and a word is
     *code-like* when it has `/`, `::`, `_`, `=`, `{`, `}`, `<`, `>`, `#`,
     `$`, a `.` between two letters (`foo.rs`, `a.b`), an inner capital
     after a lower-case letter (`AgentInfo`), or letters and digits mixed
     (`v0`, `355f5f0`);
   * complex when code-like words are at least 10% of the words, or at
     least 4 of them;
   * complex when it has 3 or more list items (lines starting `- `, `* `
     or `1. `) or a table row (a line starting `|`);
   * complex when it is longer than 600 characters.

   Otherwise it is read through `speakable` as now. The thresholds are
   constants in the client, with tests on recorded replies.
3. **The full reply is summarised, not the 500-character summary.** The
   client names the reply by its pane and its time (`ts`); the daemon
   reads that entry's whole text from the session file through the same
   reader and redaction as `session.peek`, so Gemini never sees what a
   client would not, and is never given a reply cut off mid-sentence. The
   client's text is a fallback when the entry is not found. When session
   subscriptions land, a section's serial replaces the time.
4. **One method: `speech.summarize`.** Params `{pane_id, ts, text}`;
   result `{spoken}`. It runs the `spoken_summary` prompt with no tools
   and no session history: one system text, one user message, one
   answer. It is served to companions only, with the same permission as
   `session.peek` for the pane.
5. **Bounded.** At most one summary in flight per connection; a second
   waits. A 10 s timeout. The daemon keeps the last 64 summaries by
   `(pane_id, ts)`, so the same reply is not summarised twice. An answer
   longer than the prompt's `max_words` is cut at a sentence. On any
   failure the client reads the reply through `speakable` instead, so
   read aloud never goes silent.
6. **Order is kept.** Replies are read in order: a reply waiting for its
   summary holds the ones after it, up to the timeout.
7. **ADDITIONAL_PROMPTS.md is libtypedmd.** One object per prompt, tagged
   `[Prompt]`, its head the prompt's name. Members are its settings; the
   system text and the user message are literal blocks; `{{ [abc] name }}`
   slots in them are filled by the daemon. Unknown members are kept and
   ignored; a prompt missing `system` or `user` is an error at load, logged,
   and that prompt is unavailable. The file is compiled in with
   `include_str!`, as PROMPT.md is, and `--prompts <path>` reads another
   file in its place.
8. **The first prompt is the summary, with the reading rules in it.**

   ```markdown
   # [Prompt] spoken_summary

   - [#] max_words: 60
   - [abc] temperature: 0.2
   - system: |
       You turn a coding agent's reply into what a person hears read
       aloud on their phone. Say what the agent did, found or asks, in
       one to three short sentences, at most {{ [#] max_words }} words.
       Plain spoken English only: no Markdown, no lists, no code.
       Say a path as "the" and the English words of its last part:
       docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md is "the overview
       screen plan". Say a link as its text. Say an identifier as its
       words: AgentInfo.asking is "agent info asking". Leave out commit
       hashes, line numbers and error codes unless the reply is about
       them. Keep a number or a version as it is. If the reply asks the
       user something, end with that question.
   - user: |
       The {{ [abc] agent }} agent in {{ [abc] project }} replied:

       {{ [abc] reply }}
   ```

9. **libtypedmd is pinned by revision**, as every dependency here:
   `libtypedmd = { git = "https://github.com/tmzt/libtypedmd", rev =
   "a61c091e6420a1dfc40e89b8600cd26e628e5f7a" }` (v0.2.0), in the daemon
   crate only.

## 3. The daemon

```rust
// src/prompts.rs
pub const ADDITIONAL_PROMPTS_RAW: &str = include_str!("../ADDITIONAL_PROMPTS.md");

pub struct Prompt {
    pub name: String,
    pub settings: BTreeMap<String, String>,   // every member but system and user
    pub system: String,                       // with its slots unfilled
    pub user: String,
}
pub struct Prompts { by_name: BTreeMap<String, Prompt> }

impl Prompts {
    /// Every `[Prompt]` object in `text`; errors are per prompt.
    pub fn parse(text: &str) -> (Prompts, Vec<String>);
    pub fn get(&self, name: &str) -> Option<&Prompt>;
}
impl Prompt {
    /// The text with each `{{ [sigil] name }}` replaced from `values`, then
    /// from the prompt's settings; an unknown name is an error.
    pub fn fill(&self, text: &str, values: &BTreeMap<&str, String>) -> Result<String, String>;
}
```

`speech.summarize` lives in `src/server/speech.rs`: it resolves the pane
as `session.peek` does, reads the entry by `ts` (new
`Sessions::reply_text(&AgentRef, ts) -> Option<String>`, redacted, at most
`TAIL_MAX` of text), fills the prompt with `agent`, `project` and
`reply`, and runs it on a `GeminiReasoningEngine` built once with the
filled system text and no tools, through `process_turns` with one user
message and an executor that refuses every tool.

## 4. The protocol

In `libhandlr_proto::common`:

```rust
pub const SPEECH_SUMMARIZE: &str = "speech.summarize";
pub struct SpeechSummarizeParams { pub pane_id: String, pub ts: String, pub text: String }
pub struct SpeechSummarizeResult { pub spoken: String }
```

Errors: `UNKNOWN_AGENT`, `NOT_PERMITTED` as `session.peek`;
`UNAVAILABLE` (new, -32015) when Gemini has no key, the prompt did not
load, or the call failed or timed out. The method and types are additive;
they go in libhandlr_proto 0.2.0 with whichever of plans 20261023_03 and
20261023_04 releases first, or as 0.1.1 on their own if this lands
before both.

## 5. Steps

1. **ADDITIONAL_PROMPTS.md and `src/prompts.rs`** (the daemon crate). The
   file with `spoken_summary`; `Prompts::parse` and `Prompt::fill` on
   libtypedmd.
   **Done when:** tests parse the shipped file, fill `spoken_summary`, and
   report a prompt missing `user`, an unknown slot name, and an unknown
   member kept.
2. **libhandlr_proto: `speech.summarize`.**
   **Done when:** its params and result round-trip.
3. **The daemon: `speech.summarize`.** `src/server/speech.rs`,
   `Sessions::reply_text`, the cache, the timeout, one in flight.
   **Done when:** tests with the herdr mock, a session fixture and a mock
   engine show the full reply (past 500 characters) reaching the engine
   redacted, the cache answering a second call, the timeout giving
   `UNAVAILABLE`, and a shell pane giving `UNKNOWN_AGENT`.
4. **The client: complexity and summaries.** `complexity`, the request
   for complex replies, reading in order, and the fallback to `speakable`.
   **Done when:** tests on recorded replies (a plain one read as it is;
   one with backquotes and one with a list of paths summarised; a refused
   or timed-out summary read through `speakable`) pass, and a reply
   waiting for its summary holds the next one.
5. **On the S23**, with read aloud on while an agent works: plain replies
   read as they are, complex ones as a short summary, under the reading
   rules. Logged under `out/speech/YYYYMMDD_NN_spoken_summaries/` with the
   replies, their measures and what was said.
   **Done when:** that log shows both kinds.

Each step is one crate, one commit (Rule 3.1). 2 needs nothing; 3 needs 1
and 2; 4 needs 2; 5 needs 3 and 4.

## 6. Open questions for the owner

1. **The thresholds** of decision 2 (10% or 4 code-like words, 3 list
   items, 600 characters) are a starting point. Should anything always go
   to Gemini, or never?
2. **Summaries and cost.** Every complex reply while read aloud is on is
   one Gemini call. Is that fine, or should there be a cap per hour?
3. **More prompts.** Which other prompts belong in ADDITIONAL_PROMPTS.md
   later (for example, the confirmation question's wording, or a summary
   of what several agents did for the overview)?

## Amendment — 2026-10-23 15:45 EDT: the daemon decides what is complex

The owner (2026-10-23): "the phone wouldn't decide, the daemon would".
This replaces decision 1, the client side of decision 2, the params and
result in decision 4 and section 4, and step 4's `complexity` in the
client.

* The client sends every reply it is about to read aloud to
  `speech.summarize` `{pane_id, ts, text}`. The daemon reads the full
  reply (decision 3) and measures it with decision 2's rules, as
  `complexity(&str) -> Complexity` in the daemon (`src/server/speech.rs`
  or a module beside it), on the full text rather than the client's 500
  characters.
* The result is `{spoken: Option<String>, complex: bool}`. Complex:
  `spoken` is Gemini's summary. Not complex: `spoken` is `None`, no Gemini
  call is made, and the client reads the reply through `speakable` as now.
  The 64-entry cache keeps both answers.
* The client keeps reading in order, the 10 s timeout and the fallback to
  `speakable` on any error. It has no complexity measure of its own.
* Step 3's tests also cover the measure on recorded replies: a plain one
  not complex, one with backquotes and one with a list of paths complex.
  Step 4's tests cover a `None` answer read through `speakable`.

## Amendment — 2026-10-23 15:45 EDT: implementation has started; the document is now immutable

The owner approved building this plan on 2026-10-23. Implementation starts
with step 1, `ADDITIONAL_PROMPTS.md` and `src/prompts.rs`. The open
questions of section 6 stay open; the plan's values (the thresholds of
decision 2, no cap on calls, one prompt) are used as written. From here
the document changes only by amendment on the owner's instruction.

## Amendment — 2026-10-23 16:28 EDT: reading rules first, relative times, and Gemini only in speaking mode

On the owner's instruction (2026-10-23): "we jumped right to sending long
messages to be summarized, we should look at a heuristic version for the
paths, and adjust times (at hh:mm:ss) to relative"; "it should only invoke
an LLM if the phone's flag says it's in speaking mode (spoken output
enabled)"; and earlier, "the read-aloud flag will have to be sent back as a
flag/options". Agreed in discussion the same day. This replaces, where
they disagree: decision 2's length rule and the order of measuring; the
15:45 EDT amendment's "Not complex: `spoken` is `None` … the client reads
the reply through `speakable`"; and the client's role as the reader of
plain replies. The steps below are added to section 5.

**A1. The reading rules first, then the measure.** The daemon applies the
reading rules to every reply (links as their text; a path as "the" and the
English words of its last part; inline code as its words; a commit hash
dropped; no code blocks, Markdown marks, headings or bullets), then
measures what is left. Complex is now only:
- code-like words still at least 10% of the words, or at least 4 of them;
- 3 or more list items, or a table row, in the reply as written.

Length alone is no longer complex, and a backquote alone is not either:
backquoted code is read as its words. The 600-character rule and the
backquote rule of decision 2 are dropped.

**A2. Clock times are read as relative.** A clock time (`16:13:48`,
`16:13`, "at 16:13:48", `20:02:49 UTC`, `20:02:49Z`, `4:13 PM`,
`4:13 PM EDT`) is read relative to when it is spoken:

| Difference | Read as |
|---|---|
| under 10 s | "just now" |
| under a minute | "N seconds ago" |
| under an hour | "N minutes ago" ("a minute ago" for one) |
| under 12 hours | "about N hours ago" ("about an hour ago" for one) |
| 12 hours or more, or not a time today | the clock time, as "four thirteen PM" |

A future time reads "in N minutes" and so on. "at" before a relative time
is dropped ("at 16:13:48" is "three minutes ago"). A time with no zone is
the daemon's local time today; `UTC`, `Z` and the US zone abbreviations
(`EDT`, `EST`, `CDT`, `CST`, `MDT`, `MST`, `PDT`, `PST`) are
converted. Dates and durations ("3 s", "10 seconds") are left alone.

**A3. The daemon always says what to read.** `speech.summarize`'s result
`{spoken, complex}` keeps its type, but `spoken` is always set: Gemini's
summary when the reply is complex and spoken output is on (A4), otherwise
the reply under the reading rules. The client reads `spoken`, and uses
its own copy of the rules only when the daemon cannot be reached or does
not answer within the timeout.

**A4. Gemini only in speaking mode.** A new companion method
`client.options` `{spoken_output: bool}`, answered `{}`, sets a flag on
the connection. It starts off. The client sends it when read aloud is
turned on or off, and again after every `hello`. While it is off,
`speech.summarize` makes no Gemini call, whatever the reply; it answers
with the reading rules and `complex` as measured. Unknown options are
ignored, so more can be added. This also answers "the read-aloud flag will
have to be sent back as a flag/options".

**A5. One copy of the rules.** The reading rules and the relative times
move to a new crate, `crates/handlr-speech` (no dependencies beyond
`std`; times take "now" as a parameter so tests are fixed):

```rust
pub fn read_aloud(text: &str, now: LocalTime) -> String;   // the reading rules and A2
pub fn complexity(text: &str) -> Complexity;              // A1, on the reply as written
pub struct LocalTime { pub hour: u8, pub minute: u8, pub second: u8, pub utc_offset_minutes: i16 }
```

The daemon and `handlr-companion` both use it; `speakable` and the
daemon's own `complexity` are replaced by it.

**A6. Versions.** libhandlr_proto gains `CLIENT_OPTIONS`,
`ClientOptionsParams { spoken_output: bool }` (unknown fields ignored):
additive, so 0.1.2, with its VERSIONS.md entry and the tag
`libhandlr_proto-v0.1.2`.

**Steps added:**
6. **`crates/handlr-speech`**, with the rules and A2, added to the
   workspace in its first commit. **Done when:** tests on recorded replies
   show a long plain reply and a reply full of paths read through the
   rules and not complex, a list of changes complex, and each time form
   of A2 read relative to a fixed now, with the 12-hour and future cases.
7. **libhandlr_proto 0.1.2: `client.options`.** **Done when:** it
   round-trips, and an unknown option is ignored.
8. **The daemon:** `client.options`, the flag, `speech.summarize` on
   `handlr-speech` with `spoken` always set. **Done when:** tests show a
   complex reply with the flag off making no engine call and answering
   with the rules, the flag on making one call, and a new connection
   starting off.
9. **The client:** `client.options` on read aloud and after `hello`;
   reading `spoken`; `handlr-speech` as the fallback. **Done when:**
   tests show the option sent on toggling and after a reconnect, and a
   reply read from `spoken`.

Step 5 (the S23 check) now follows step 9.

## Amendment — 2026-10-23 16:55 EDT: implemented

Every step is done and its **Done when:** holds. Step 5 was checked on the
S23 by the owner ("that worked") and logged in
`out/speech/LOG.md` (run `out/speech/20261023_01_spoken_summaries/`),
which shows a plain reply read through the rules and a complex one
summarised by Gemini only with spoken output on. The commits:

| Step | Commits |
|---|---|
| 1. ADDITIONAL_PROMPTS.md and `src/prompts.rs` | `296f4d3` |
| 2. libhandlr_proto 0.1.1 | `505f408` (tag `libhandlr_proto-v0.1.1`) |
| 3. The daemon's `speech.summarize` | `e23fc10`; CHANGES.md `a1d3d2a` |
| 4. The client | `1c94817` |
| 6. `crates/handlr-speech` | `42e9ffc` |
| 7. libhandlr_proto 0.1.2 | `4e3d562` (tag `libhandlr_proto-v0.1.2`) |
| 8. The daemon on handlr-speech, `client.options` | `12a4fee`; CHANGES.md `68aaa2b` |
| 9. The client on `client.options` | `3a59fde` |
| 5. On the S23 | this amendment |

Still open from section 6 and the amendments: the prompt's temperature is
not applied (libinfer-gemini has no setting yet); the thresholds and a cap
on Gemini calls are the owner's to revisit.
