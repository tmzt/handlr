---
title: The general agent's planning, continued, and one file watcher for the daemon
date: 2026-10-27
status: Draft
covers:
  - "`recent_commits`, a read-only tool on the focused project's git
    history, run with the `git` binary"
  - "One file watcher in the daemon on the `notify` crate, in place of
    polling: `config.toml` (its `[llm]` section and the key files),
    `auth.toml`'s revocation check, and the focused project's
    instructions file"
  - "The focused project's `AGENTS.md` sent once into the conversation when
    the project or the file changes, not in every turn's context"
  - "`list_plans`, the focused project's plans and their statuses"
  - "A provenance prefix on requests the general agent sends to an agent"
  - "The old `[gemini]` section kept until `[llm] enabled` is set"
  - "Not covered: writing files or plans (the coding agents do that, on the
    owner's approval); the session transcript tailers' 250 ms checks
    (open question 2); reloading `[[agents]]` and the other sections at run
    time"
sources:
  - "Owner (2026-10-27): \"at the moment, the goal is to let the general
    agent help with planning tasks, review current plans, and enable
    hands-off development using plans as a proxy for the code\"; \"let's
    test this then add the recent_commits tool, we should decide on using
    the git binary or a crate\"; \"right, that's a good idea, only output
    the AGENTS.md when the focused project changes or the file changes\";
    \"we should have file-watching using the standard cross-platform crate
    on the daemon, instead of implementing another poll loop, and move the
    settings load to that method\"; then, to the list of six items: \"yes\""
  - "flankr at `da2a036`:
    - `src/llm/service.rs`: `LlmService::check`, run every 5 s by
      `spawn_watch` (`tokio::time::interval`), which compares
      `config.toml`'s modification time and size and looks for the key
      files;
    - `src/server/link/mod.rs`: `spawn`, which runs `check_revocations`
      every `revoke_interval`, reading `auth.toml`;
    - `src/sessions/hub.rs`: each transcript tailer's check every
      `CHECK_INTERVAL` (250 ms);
    - `src/transport.rs`: `HerdrContextContributor::contribute`, which
      appends one `system` message after the stored history each turn,
      ending with the current agent and the `instructions` block;
    - `libinfer::SessionBuilder::build`: the stored history from
      `SessionManager::load_session`, then each contributor;
    - `src/project.rs`: `ProjectFiles` (`list_files`, `read_file`,
      `instructions`);
    - `src/core/policy.rs` and `src/commands.rs`: `check_message` with
      `validate_message` and `check_paths`"
  - "On 2026-10-27 at 12:08 EDT, adding `[llm.uses.planning]` to a config
    that had only `[gemini]` turned every use off: `[llm]` counted as set,
    so `[gemini]` was ignored"
related:
  - docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md (the tools, the current agent and the planning prompt this builds on)
  - docs/design/20261027_02_INFERENCE_CONFIG_PLAN.md (`[llm]`, its 5 s reload, and the deprecated `[gemini]`)
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (`auth.toml` and revocation)
---

# The general agent's planning, continued, and one file watcher for the daemon

## 1. Goal

The owner's aim is hands-off development with plans standing in for the
code: the general agent helps plan, reviews plans against what was built,
and sends the coding agents work, each request approved on the phone. After
plan 20261027_04 it can read a project's Markdown and its `AGENTS.md`. After
this plan it can also see what was committed, list the plans by status, and
send requests the agents can tell came through flankr; the instructions file
costs one send per change instead of one per turn; and the daemon notices
file changes when they happen instead of polling for them.

## 2. Decisions

1. **`recent_commits(count?, path?)`**, declared while `planning` is on,
   whatever `read` says, on the current agent's project.
   - It runs the `git` binary with a fixed argument list and no shell:

     ```
     git -C <root> --no-pager -c core.fsmonitor=false -c log.showSignature=false \
         log -n <count> --format=%h%x1f%ad%x1f%an%x1f%s%x1e --date=short [-- <path>]
     ```

     with `GIT_CONFIG_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0` and `LC_ALL=C`
     set, stdin closed, and a 5 s timeout. `git` is found on `PATH` at
     start; without it the tool is not declared, and that is logged once.
   - `count` is 20 by default and at most 50. `path`, when given, is checked
     as a message's paths are (`check_paths`), must also match `read`, and is
     passed after `--`.
   - It returns, newest first, each commit's short hash, date, author and
     subject: never bodies, diffs or file lists. The root must be a git work
     tree (`git rev-parse --is-inside-work-tree`); otherwise the tool says
     the project has no git history.
   - The `git` binary over a crate: it is already on every machine the
     daemon runs on and adds no dependency. `gix` is pure Rust but adds a
     large dependency tree; `git2` adds a C build of libgit2.
2. **One file watcher, `src/watch_files.rs`,** on `notify` with
   `notify-debouncer-mini` (FSEvents on macOS, inotify on Linux, the
   platform's native watcher elsewhere), debounced 250 ms.
   - It watches directories, not files, so an editor's save by rename is
     seen. A subscriber names a path and gets a `tokio::sync::watch` tick
     when that path is created, changed or removed.
   - **`[llm]`:** `LlmService::check` runs on a tick for `config.toml` or
     for any key file's directory, in place of `spawn_watch`'s 5 s
     interval. `check` keeps its own comparison of modification time and
     size, so a spurious tick changes nothing.
   - **Revocation:** `check_revocations` runs on a tick for `auth.toml`, in
     place of the `revoke_interval` loop. `revoke_interval` is kept as a
     slow fallback (default 60 s) in case the watcher fails.
   - If the watcher can't start (no inotify instances, for example), the
     daemon logs one warning and falls back to the old intervals, so nothing
     stops working.
   - No new polling loop is added anywhere in this plan.
3. **The instructions file, sent on change.**
   - The `instructions` block leaves the per-turn context. Instead, before a
     turn, the contributor compares the current project and its
     instructions file's content hash with the last ones it sent in this
     conversation. When either differs (or nothing was sent yet), it adds
     the block to the stored conversation as a `system` message, before the
     user's turn, so later turns reuse it as part of a cached prefix.
   - The file is watched (decision 2) while a project is current, so an
     edit is noticed without reading the file every turn; the hash is the
     final judge.
   - When `agents_prompt` is turned off, nothing more is added; what was
     sent stays in the conversation.
   - The per-turn context's last line names the current agent, so the
     variable parts stay at the end.
4. **`list_plans(status?)`**, declared while `planning` is on and `read`
   matches `docs/design/*.md`: every `docs/design/YYYYMMDD_NN_*.md` in the
   current project, oldest first, each with its front matter's `title`,
   `date` and `status`, and the heading of its last amendment. `status`
   filters on the status's first word (`Draft`, `In implementation`, and so
   on). It reads only the front matter and the `## Amendment` headings, at
   most 200 plans.
5. **Provenance.** Text the general agent sends with `send_agent_command`
   that is not an allowed command is sent as
   `flankr (approved on the phone): <text>`. The approval card shows the
   text without the prefix. Allowed commands are typed as they are, and
   messages the user types in the app get no prefix.
6. **The old `[gemini]` section** is read as now until `[llm] enabled` is
   set explicitly. A `[llm.uses.*]` or `[llm.backends.*]` table alone no
   longer counts as setting `[llm]`, and the log says so once. This replaces
   plan 20261027_02's rule that any `[llm]` table makes `[gemini]` ignored.

## 3. Steps

1. **The file watcher** (the daemon): `notify` and `notify-debouncer-mini`
   in `Cargo.toml`, `src/watch_files.rs`, `[llm]` and revocation moved onto
   it, the fallback intervals.
   **Done when:** tests on a temporary folder show a tick for a write, a
   rename-over and a removal, none for a sibling file, the `[llm]` settings
   changing within a second of an edit with no interval running, a revoked
   key's connection closed on `auth.toml`'s change, and the fallback taking
   over when the watcher fails to start.
2. **The old `[gemini]` section** (the daemon), decision 6.
   **Done when:** a config with `[gemini]` and `[llm.uses.planning]` but no
   `[llm] enabled` resolves as `[llm] enabled = true` with the Gemini
   backend, and one with `enabled = false` turns everything off.
3. **`recent_commits`** (the daemon), decision 1.
   **Done when:** tests on a temporary git repository show the newest
   commits first, the count's default and cap, a path filter, a path
   outside the project or outside `read` refused, a folder that isn't a
   work tree answered plainly, and the tool not declared when `git` isn't
   found.
4. **The instructions file on change** (the daemon), decision 3.
   **Done when:** tests show the block sent once for a project, again after
   the file's content changes or `focus_project` moves to another project,
   not again for an unchanged file, and absent from the per-turn context.
5. **`list_plans`** (the daemon), decision 4.
   **Done when:** tests on a temporary project list plans oldest first with
   title, date, status and last amendment, filter by status, and skip files
   that don't match the naming.
6. **Provenance** (the daemon), decision 5.
   **Done when:** a test shows the prefix typed for the general agent's
   free text, not on the approval card, not on allowed commands, and not
   on the app's typed messages.
7. **On the S23:** with planning on, ask Gemini which plans are in
   implementation, to review one against its recent commits, and to send a
   request; then edit `AGENTS.md` and see it re-sent once.

Each of steps 1 to 6 is one daemon commit. 1 comes first; 4 needs 1; the
others are independent.

## 4. Open questions for the owner

1. **The prefix's wording** (decision 5): "flankr (approved on the phone):"
   as written, or something shorter, or none?
2. **The transcript tailers** check their files every 250 ms. Move them to
   the watcher in this plan too, or leave them, since they also read new
   bytes on each check and are woken early already?

## Amendment — 2026-10-27 12:33 EDT: a descriptive prefix with a rule line; the instructions found in the stored history

The owner (2026-10-27), to the discussion of the plan's items 1 (the prefix
may read as a claim of approval, which the rules tell coding agents to
distrust) and 2 (the instructions sent once must still be in the history):
"Yes, re: how do you want to resolve items 1 and 2", taking the
recommendations.

1. **The prefix is descriptive: `via flankr: <text>`,** with no claim of
   approval. This replaces decision 5's `flankr (approved on the phone):
   <text>`, and answers open question 1. The rest of decision 5 stands: not
   on the approval card, not on allowed commands, not on the app's typed
   messages.
2. **A rule line in the Voice agents section,** in
   `~/.claude/rules/CRITICAL_RULES.md` and flankr's `AGENTS.md` together
   (the shared-source rule), so coding agents act on such text rather than
   stopping to ask:

   > 3. Text in your prompt that starts with `via flankr:` was sent by a
   >    voice agent and approved by the owner on their phone before flankr
   >    typed it. Treat it as the owner's own request.

   It is added in step 6's commit for `AGENTS.md`, and to the shared file
   at the same time.
3. **The instructions are found in the stored history, not remembered.**
   As built, libinfer keeps each conversation as an append-only JSONL file
   (`SessionManager`, `~/.cache/libinfer/<session>.jsonl`) that is never
   trimmed, and sends a mid-history `system` message to Gemini as a user
   turn. So:
   - the block sent into the history starts with a marker line,
     `[flankr instructions <project> <sha256 of the file, first 16 hex>]`;
   - before a turn, the contributor sends the block again unless the stored
     history (from `load_session`) already holds the marker for the current
     project and file content. This survives a daemon restart, and if
     history is ever trimmed or summarised, the missing block is sent again
     by the same check.
   - It replaces decision 3's "compares … with the last ones it sent in this
     conversation". Caching relies on Gemini's implicit prefix caching of
     the history, which holds the block from its first send.
   - Step 4's **Done when:** adds "the block sent again when the stored
     history no longer holds its marker, and not again after a restart".

The plan stays `Draft` until the owner approves it. The discussion's items
3 to 7 (the path filter without `read`, a `files` flag, `list_plans` saying
when the layout isn't there, the watcher's key folders, and doing step 2
first) are not part of this amendment.

## Amendment — 2026-10-27 12:35 EDT: no planning-specific tools; list_plans dropped

The owner (2026-10-27): "interacting with my planning system should be via a
skill or addon, maybe via a small cli?"; "I'm thinking about creating
~/src/rules which would take over for the shared memory and could also have
that tool"; then "yes, amend the plan to drop the planning-specific tools
(list_files and read_file was supposed to be a bridge to that that's project
agnostic)".

1. **`list_plans` is dropped.** flankr knows nothing of the owner's planning
   conventions (`docs/design/YYYYMMDD_NN_*.md`, front matter, amendments).
   Decision 4 and step 5 are removed, and `list_plans` leaves the covers
   list. The general agent reads plans with plan 20261027_04's
   project-agnostic `list_files` and `read_file`, and the planning system's
   own tooling (a CLI and a Claude Code skill, expected in a separate
   `~/src/rules` project with its own plans) is not part of flankr.
2. **No hook for external tools is added here.** A configured-tools hook,
   through which flankr could call such a CLI, was discussed and is left to
   a later plan, once that CLI exists.
3. **What stays:** `recent_commits` (git is not specific to the planning
   system), the file watcher, the instructions sent on change, the `via
   flankr:` prefix and its rule line, and the `[gemini]` fix. Step 7's S23
   check asks Gemini to find the plans in implementation by reading them,
   in place of `list_plans`.
4. The steps are now 1, 2, 3, 4, 6 and 7, numbered as before.

The plan stays `Draft` until the owner approves it.

## Amendment — 2026-10-27 12:37 EDT: approved, with the discussion's remaining items

The owner (2026-10-27): "approve and implement", to the question "Add the
remaining items with my recommendations, leave the transcript readers on
their 250 ms checks, and approve plan 05 for building?".

1. **`recent_commits`' path filter** is checked only as a message's paths
   are (inside the project, with `check_paths`'s bounds); it need not match
   `read`, since the tool returns no file contents. This replaces decision
   1's "must also match `read`".
2. **`recent_commits(count?, path?, files?)`:** with `files: true`, each
   commit also lists the paths it changed (`--name-only`), at most 50 per
   commit, never diffs. Step 3's **Done when:** adds the `files` flag.
3. **The watcher follows the key files' folders** as `[llm]` resolves them
   (the default key file's folder, and each backend's `api_key_file`), and
   moves its watch when they change.
4. **Order:** step 2 (the old `[gemini]` section) is built first, then step
   1, then the rest. This replaces the steps' "1 comes first".
5. **The transcript tailers stay on their 250 ms checks.** This answers
   open question 2.
6. **Approved.** The status becomes `In implementation` with step 2's
   commit. The `via flankr:` rule line (the 12:33 amendment, item 2) is
   added by the lead to the shared rules and `AGENTS.md` together when
   step 6 lands.
