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
