---
title: Spoken summaries of complex replies, from structured prompts
date: 2026-10-23
status: Draft
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
