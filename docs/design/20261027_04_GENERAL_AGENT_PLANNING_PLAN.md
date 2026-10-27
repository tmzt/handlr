---
title: Planning tools for the general agent, and its prompt from ADDITIONAL_PROMPTS.md
date: 2026-10-27
status: In implementation
covers:
  - "`run_command` renamed `send_agent_command`, carrying both allow-listed
    commands and free-text requests to a coding agent"
  - "A new use, `[llm.uses.planning]`, off in the code: it gives the general
    agent's own turns, on its own model, read-only access to the current
    project's files (`list_files`, `read_file`) under configured patterns,
    and optionally the project's agent instructions (`agents_prompt`)"
  - "The general agent's final system prompt assembled from `PROMPT.md` and
    a `planning` prompt in `ADDITIONAL_PROMPTS.md`"
  - "The turn's current project: the session open on the phone, or, on the
    overview, the agent Gemini last focused"
  - "`flankr prompt`, which prints the assembled prompt"
  - "Not covered: writing files; the other uses' prompts (`card_summaries`,
    `spoken_summaries`), which keep theirs; the app showing file reads"
sources:
  - "Owner (2026-10-27), in order: \"let's expand the tool availability to
    Gemini/general agent. plan this. if run_command is used to submit
    messages to the coding agent, it should be renamed submit_coding_request
    (or a better name), limited access to list_files and read_files should
    be given via a configuration, starting with \"*.md\" and \"docs/**\".
    also, if agents_prompt is enabled the AGENTS.md (or CLAUDE.md or
    fallback filenames) should be included in the prompt. this is
    llm.uses.planning and disabled by default in the code\"; \"it should
    use ADDITIONAL_PROMPTS and build a final prompt out of it\";
    \"send_agent_command is probably better, prompt command cli command is
    fine. only the current session, but the overview agent should be able
    to query a project by changing focus. yes, planning just enables the
    functionality for the primary general agent (it's turns and model)\""
  - "flankr at `c916792`:
    - `src/reasoning.rs:212`: `declare_gemini_tools()`, the general agent's
      four tools (`focus`, `run_command`, `watch_pane`, `unwatch_pane`);
    - `src/transport.rs`: `HerdrToolExecutor::execute_run_command`, which
      checks free text with `policy::check` and `validate_arbitrary_command`
      (ASCII, no `/` or `-`, 256 characters at most); and
      `HerdrContextContributor`, which adds the herdr layout and sessions
      before each turn;
    - `src/core/policy.rs`: `check_message`, the check for messages typed
      in the app (`allow_messages`, `validate_message`, always confirmed);
    - `src/llm/service.rs:69`: `GeminiFactory`, which builds the general
      agent's client once per use, with `PROMPT.md` and the tools fixed;
    - `src/prompts.rs`: `ADDITIONAL_PROMPTS.md` on libtypedmd, with
      `--prompts <path>`;
    - `libherdr::protocol` pane `cwd`;
    - `libflankr_proto::companion::TurnStartParams { turn_id, text }`, which
      does not say which session the phone has open"
related:
  - docs/design/20261027_02_INFERENCE_CONFIG_PLAN.md (`[llm]` and its uses; its 00:59 EDT amendment, item 3, defers aligning uses with named prompts, which this begins)
  - docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md (`ADDITIONAL_PROMPTS.md` and its format)
  - docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md (the general agent's turns)
  - docs/design/20261023_02_BULK_INPUT_PLAN.md (`check_message`, decision 6 as amended)
---

# Planning tools for the general agent, and its prompt from ADDITIONAL_PROMPTS.md

## 1. Goal

Gemini can send a coding agent an allow-listed command, or free text that
passes the voice check, which forbids `/`, `-` and anything over 256
characters, so most real coding requests can't be sent. It can't see the
project it is talking about. After this plan, with `planning` turned on, it
can read the current project's Markdown and docs, see its agent
instructions, and send a full request to the coding agent, approved by the
user like a typed message. Its prompt for this is text in
`ADDITIONAL_PROMPTS.md`, not code.

## 2. Decisions

1. **`run_command` is renamed `send_agent_command`.** One tool, both kinds of
   text:
   - `send_agent_command(agent, text, handle)`. `agent` is what `pane_id`
     accepts today (a name or a pane id).
   - **Allow-listed text** (`continue`, `/compact`, `/usage`) is checked by
     `policy::check` as today, with the agent's `require_confirmation`.
   - **Any other text** is checked by `policy::check_message`, as a message
     typed in the app: the agent's `allow_messages`, then `validate_message`.
     It is always confirmed, whatever `require_confirmation` says. The old
     `allow_arbitrary_commands` path (`validate_arbitrary_command`) is no
     longer used by Gemini; it stays for `flankr ctl` and MCP.
   - The executor still accepts `run_command` and `send_command` as names,
     for transcripts in flight. The declaration lists only
     `send_agent_command`.
   - What reaches herdr does not change: herdr's `run_command` call, which
     types the text and Enter.
   - `PROMPT.md`, written by Gemini, only has the tool's name and parameter
     names changed (lines 15, 19, 35, 37 and 40 at `c916792`).
2. **`[llm.uses.planning]`, off in the code.** Unlike the other uses, it is
   off unless set, even with `[llm] enabled = true`:

   ```toml
   [llm.uses.planning]
   enabled = true          # default false
   agents_prompt = true    # default false
   read = ["*.md", "docs/**"]   # the default when unset
   ```

   - `planning` is not a separate client or model. It extends the general
     agent's turns on the general agent's backend and model. A `backend` or
     `model` key in this table is logged once as ignored.
   - It needs `general_agent` available; while `general_agent` is off,
     `planning` is off with that reason.
   - It is a new `Use::Planning`, so it appears in the `[llm]` log lines and
     reloads every 5 s like the others. A change to `planning` drops the
     general agent's client, so the next turn is built with the new tools
     and prompt.
3. **The current project.** File tools and `agents_prompt` act on one
   project: the working directory (`cwd`, from herdr's snapshot) of the
   turn's current agent.
   - **In the agent view,** the current agent is the session the phone has
     open. `TurnStartParams` gets `session: Option<String>` (a pane id),
     appended, with `#[serde(default)]`.
   - **On the overview,** there is none at first. Gemini's `focus` tool,
     called on an agent, makes that agent current for the rest of the
     conversation, as well as focusing it in herdr as today. A turn from the
     agent view sets it back to that session.
   - Without a current agent, `list_files` and `read_file` refuse with "No
     project is open: open a session, or focus an agent first", and no
     instructions are added.
4. **`list_files(glob?)` and `read_file(path)`.** Read-only, on the current
   project, paths relative to its root.
   - A path is allowed when it matches one of `read`'s patterns
     (`globset`, matched against the root-relative path with `/`
     separators, so `*.md` is the top folder only and `**/*.md` is every
     folder).
   - Always refused, whatever the patterns: `.private/**`, `.env`,
     `.env.*`, `.git/**`, anything whose canonical path is outside the root
     (`..`, absolute paths, symlinks out), and anything that isn't a regular
     UTF-8 file.
   - `list_files` returns at most 500 paths, sorted, and says when there
     were more. It skips `.git`, `target` and `node_modules` while walking.
   - `read_file` returns at most 64 KiB, and says when the file was cut.
   - Each call is logged at info with the project and path, never the
     contents.
5. **`agents_prompt`:** the first of `AGENTS.md`, `CLAUDE.md` and `GEMINI.md`
   found in the current project's root, cut at the prompt's
   `max_instructions_kb` (32). `@` imports in it are not followed. These
   files are read even when `read` doesn't match them.
6. **The final prompt.** `ADDITIONAL_PROMPTS.md` gets a `planning` prompt,
   named after the use:

   ```markdown
   # [Prompt] planning

   - [#] max_instructions_kb: 32
   - system: |
       (how to use list_files, read_file and send_agent_command for a
       coding request: what may be read, {{ [abc] read }}; reading before
       proposing; writing a request the agent can act on; a request waits
       for the user's approval)
   - instructions: |
       The current project is {{ [abc] project }}, where the {{ [abc] agent }}
       agent works. Its instructions, from {{ [abc] file }}:

       {{ [abc] instructions }}
   ```

   The general agent's prompt is assembled from:
   1. `PROMPT.md`, as now;
   2. with `planning` on, the `planning` prompt's `system`, its `read` slot
      filled from config. Items 1 and 2 are the client's system prompt,
      built with the client.
   3. with `agents_prompt` also on and a current agent, the `instructions`
      block, filled for that project. This changes from turn to turn, so it
      is added by `HerdrContextContributor` before each turn, after the
      herdr layout.

   A missing or broken `planning` prompt is a warning; `planning` is then
   off with that reason, and the general agent runs as without it.
   `--prompts <path>` replaces the shipped file, as now.
7. **The tools Gemini is given:** `focus`, `send_agent_command`,
   `watch_pane`, `unwatch_pane`, plus `list_files` and `read_file` only
   while `planning` is on.
8. **`flankr prompt [--use general_agent] [--agent <name>] [--prompts
   <path>]`** prints the assembled system prompt, the tools' names, and, with
   `--agent`, the `instructions` block for that agent's project. It reads
   the config and herdr; it calls no LLM.
9. **Versions.** `TurnStartParams.session` is an appended field: a patch
   bump of `libflankr_proto` (0.3.10 to 0.3.11), by the owner's ruling. The
   app sends it from the agent view.

## 3. Steps

1. **The rename** (the daemon): `send_agent_command` in
   `declare_gemini_tools`, the executor's two checks (decision 1), and
   `PROMPT.md`'s names.
   **Done when:** tests show an allow-listed command sent with the agent's
   `require_confirmation`, free text with `/` and `-` accepted and always
   awaiting approval, free text refused for an agent with `allow_messages =
   false`, and the old names still dispatched.
2. **`[llm.uses.planning]` and the `planning` prompt** (the daemon):
   `Use::Planning`, off by default, `agents_prompt` and `read`; the prompt in
   `ADDITIONAL_PROMPTS.md`; the system prompt assembled from both; tools
   chosen by it; the client rebuilt when it changes.
   **Done when:** tests show `planning` off without a table, on with
   `enabled = true`, off when `general_agent` is off, a reload changing the
   tools of the next client, and the assembled prompt with and without the
   `planning` block.
3. **`libflankr_proto` 0.3.11:** `TurnStartParams.session`, with
   `VERSIONS.md` and the tag `libflankr_proto-v0.3.11`.
   **Done when:** an old `turn.start` without `session` still parses.
4. **The current project and the file tools** (the daemon): the current
   agent from `session` and `focus`; `list_files`, `read_file` and their
   limits; the `instructions` block in the context.
   **Done when:** tests on a temporary project show `*.md` and `docs/**`
   read, `src/main.rs`, `.env`, `.private/key` and `../x.md` refused, a
   symlink out refused, the limits, `AGENTS.md` preferred to `CLAUDE.md`,
   `focus` changing the project on the overview, and a refusal with no
   current agent.
5. **`flankr prompt`** (the daemon).
   **Done when:** it prints the assembled prompt for a config with
   `planning` on and an agent named.
6. **The app sends `session`** from the agent view (the companion).
   **Done when:** a test shows `turn.start` from the agent view carrying the
   open session's pane id, and none from the overview; then, on the S23,
   Gemini reads this repository's docs from the agent view and after
   focusing an agent from the overview.

Step 1 is one daemon commit; 2, 4 and 5 are one each; 3 is
`libflankr_proto`'s; 6 is the app's. 2 needs 1; 4 needs 2 and 3; 6 needs 3.

## 4. Open questions for the owner

1. **Pattern depth:** `*.md` as the top folder only, with `**/*.md` for
   every folder (as written), or `*.md` at any depth, as in `.gitignore`?
2. **Focus on the overview:** `focus` makes the agent current *and* moves
   herdr's focus on the desktop, as written. Or should picking the project
   leave the desktop alone (a separate `open_project(agent)` tool)?

## Amendment — 2026-10-27 11:20 EDT: the project focus split from herdr's focus; the pattern syntax

The owner (2026-10-27): "the focus was meant to be split, not change
herdr's focused tab, but that would be an option in the settings
tab_follows_focus"; then, to the proposal: "yes, use the **/*.md for *.md in
all folders, but keep the syntax limited, only use a crate for it if minimal
and trusted".

1. **`focus_project(agent)`, a new tool,** offered only while `planning` is
   on. It makes that agent's project current for the rest of the
   conversation, for `list_files`, `read_file` and the `instructions` block.
   It does not touch herdr. A turn from the agent view still sets the
   current project to the open session. This replaces decision 3's
   overview bullet ("Gemini's `focus` tool, called on an agent, makes that
   agent current …"), and adds `focus_project` to decision 7's list.
2. **`focus` is unchanged:** it moves herdr's focus when the user asks to be
   shown something, and no longer picks the project.
3. **`tab_follows_focus`,** a toggle in the app's Settings, under the general
   agent, off by default. While it is on, `focus_project` also focuses that
   agent's tab in herdr. The phone sends it in `client.options`, a field
   appended to `ClientOptions` beside `general_agent` and `spoken_output`,
   in the same `libflankr_proto` 0.3.11 patch as `TurnStartParams.session`
   (decision 9, step 3). Step 6 also adds the toggle, its copy in
   `SETTINGS_COPY.md`. Step 4's **Done when:** reads "`focus_project`
   changing the project, and herdr's focus only with `tab_follows_focus`" in
   place of "`focus` changing the project on the overview". This answers
   open question 2.
4. **Patterns, a limited syntax, matched by flankr's own code** in place of
   decision 4's `globset`:
   - a pattern is `/`-separated segments, matched against the
     root-relative path's segments;
   - `**` as a whole segment matches zero or more segments;
   - `*` within a segment matches any run of characters except `/`;
   - every other character is literal, and a pattern containing `?`, `[`,
     `]`, `{`, `}`, `!` or `\`, a leading `/`, or `**` inside a segment
     (`a**`) is refused when the config is read: logged once as a warning
     and left out;
   - so `*.md` is the top folder only, and `**/*.md` is every folder, as
     decision 4 says. The default stays `["*.md", "docs/**"]`. This answers
     open question 1.

   `list_files` walks with `walkdir`, already in `Cargo.lock`, without
   following symlinks. No new crate is added for this plan.

The plan stays `Draft` until the owner approves it.

## Amendment — 2026-10-27 11:22 EDT: no built-in read patterns; the template suggests docs/**/*.md

The owner (2026-10-27): "the defaults in the toml should be docs/**/*.md
with no paths built-in (so the list/read feature is disabled by default)".

1. **`read` has no built-in patterns.** Unset, it is empty, and an empty
   `read` turns the file tools off: `list_files` and `read_file` are not
   declared to Gemini, and calls to them are refused. This replaces decision
   2's `read = ["*.md", "docs/**"]   # the default when unset` and the
   11:20 amendment's "The default stays `["*.md", "docs/**"]`".
2. **`config.default.toml` suggests `read = ["docs/**/*.md"]`,** commented
   out with the rest of `[llm.uses.planning]`, beside the existing `[llm]`
   examples. Step 2 adds it.
3. **What else depends on `read`:**
   - `agents_prompt` does not: it reads only the three instruction files
     (decision 5), with or without patterns.
   - `focus_project` is declared while `planning` is on and either `read`
     is non-empty or `agents_prompt` is on.
   - The `planning` prompt's `system` text describes the file tools only
     while they are declared: the prompt gets a separate `files` block,
     added after `system` only then, as `spoken_summary`'s `points` is.
4. **Tests:** step 2's **Done when:** adds "no file tools declared with
   `read` unset or empty, and both declared with `read = ["docs/**/*.md"]`";
   step 4's tests set `read` explicitly, and use `docs/**/*.md` in place of
   `*.md` and `docs/**` (with `docs/design/x.md` read, and `README.md`,
   `docs/x.txt` and `src/main.rs` refused).

The plan stays `Draft` until the owner approves it.

## Amendment — 2026-10-27 11:22 EDT: the template suggests the top folder's Markdown too

The owner (2026-10-27), right after the 11:22 amendment: "['*.md',
'docs/**/*.md']".

1. **`config.default.toml` suggests `read = ["*.md", "docs/**/*.md"]`,**
   still commented out. This replaces the 11:22 amendment's item 2
   (`read = ["docs/**/*.md"]`). There are still no built-in patterns.
2. Step 2's **Done when:** declares both file tools with
   `read = ["*.md", "docs/**/*.md"]`; step 4's tests use those patterns,
   with `README.md` and `docs/design/x.md` read, and `docs/x.txt`,
   `src/README.md` and `src/main.rs` refused. This replaces the 11:22
   amendment's item 4 where they differ.

The plan stays `Draft` until the owner approves it.

## Amendment — 2026-10-27 11:25 EDT: focus_project for every general agent turn, not only planning

The owner (2026-10-27): "no, focus_project isn't only offered with
planning, it's just split from the herdr focus now for all general agent
uses".

1. **`focus_project(agent)` is always declared to the general agent,**
   whether or not `planning` is on. It sets the turn's current agent
   (decision 3) for the rest of the conversation, and moves herdr's focus
   only with `tab_follows_focus`. This replaces the 11:20 amendment's
   item 1 "offered only while `planning` is on", and the 11:22 amendment's
   item 3 condition on declaring it ("while `planning` is on and either
   `read` is non-empty or `agents_prompt` is on").
2. **What the current agent does without `planning`:**
   - the context added before each turn names it ("Current agent: …"), so
     Gemini knows which session the user means;
   - `send_agent_command`'s `agent` may be left out to mean the current
     agent; with neither, it is refused with "Which agent? Name one, or
     focus a project first".

   With `planning` on, it is also the project for the file tools and the
   `instructions` block, as before.
3. **Decision 7's tool list** becomes: `focus`, `focus_project`,
   `send_agent_command`, `watch_pane`, `unwatch_pane`, plus
   `list_files` and `read_file` only while `planning` is on and
   `read` is non-empty.
4. **Steps:** `focus_project`, the current agent and `tab_follows_focus`
   move from step 4 to a new step 3b, after step 3 (it needs
   `TurnStartParams.session` and `ClientOptions.tab_follows_focus`) and
   before step 4. **Done when:** tests show `focus_project` declared with
   `planning` off, the current agent named in the context, from the open
   session or `focus_project`, `send_agent_command` without `agent`
   going to it, and herdr's focus moving only with `tab_follows_focus`.
   Step 4 then needs 3b.

The plan stays `Draft` until the owner approves it.

## Amendment — 2026-10-27 11:28 EDT: approved; implementation has started; the document is now immutable

The owner (2026-10-27): "approve and build". The plan is approved with its
amendments of 11:20, 11:22 (both), and 11:25 EDT, and the settings plan's
(20261024_02) amendment of 11:27 EDT for the *Tab follows focus* row. The
11:20 amendment's `ClientOptions` is `ClientOptionsParams`, the type's
real name.

Step 3 (`libflankr_proto` 0.3.11) is the first implementing commit, since
steps 3b, 4 and 6 need it; steps 1, 2, 3b, 4, 5 and 6 follow. From here the
document changes only on the owner's explicit instruction.
