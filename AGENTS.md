# `flankr` — Voice Architecture & System Prompt (AGENTS.md)

`flankr` is the voice-native command line interface and LLM tool-calling bridge for the `herdr` terminal multiplexer. It sits between a real-time voice session (`libconversate`) and the `herdr` local Unix socket or remote SSH tunnel, converting spoken intent into precise workspace orchestration.

This document defines the core architecture and the specific system prompt used to ground Gemini in the terminal environment.

## CRITICAL RULE 0: THE CHANGES.MD PROTOCOL

Before writing, modifying or deleting code for a new planned work item:

1. Only planned new work, feature plans and architectural changes are
   appended to the bottom of `CHANGES.md`. Routine implementation work that
   follows an existing plan does not need its own addendum.
2. Each addendum is timestamped (`## YYYY-MM-DD HH:MM TZ — <subject>`, from the
   system clock) and names the plan it belongs to. Entries are appended, never
   edited.
3. Departures from a frozen plan (Rule 1.7) are recorded as addenda here.
4. Updates are small, incremental steps. No large, unannounced redesigns of
   the codebase.

## CRITICAL RULE 1: SPECS AND PLANS

1. Specs and plans live in `docs/design/`, named `YYYYMMDD_NN_NAME.md`. `NN` is
   the sequence within the day, and plans end in `_PLAN`. Each starts with YAML
   front matter: `title`, `date`, `status`, what it covers, its sources, and
   related or dependent documents.
2. Plans are written in plain engineering language that a person could build
   from alone: concrete interfaces, data layouts, numbers and paths, and
   numbered steps, each ending with **Done when:** stating what works at that
   point.
3. `status` is one of:
   - `Draft`: being worked out, changed only by amendments;
   - `Hold`: no work may start or continue until an amendment returns it to
     `Draft`;
   - `In implementation`: frozen.
   - `Implemented`: every step is done and its **Done when:** holds. It stays
     frozen, and living sections stay open. It is set by an amendment that lists
     the commits implementing each step, in the commit that completes the last
     step or in a later one.
   - `Superseded (by <plan>)`: replaced before implementation by the named
     plan, which carries its content forward. Nothing in it is implemented.
     It is set with an amendment naming the replacement and what moved where.
   - `Retired (by <plan>)`: implemented, in whole or in part, and then
     removed by the named plan; its code no longer exists. It is set with an
     amendment naming that plan, the commit that removed the code, and the
     last commit that still has it, so the code can be recovered from
     history (for example `git checkout <commit> -- <paths>`). It stays
     frozen.

   The front-matter `status` is the only part of a document edited in place,
   and every status change gets its own amendment.
4. An amendment is appended to the end of the document as
   `## Amendment — YYYY-MM-DD HH:MM TZ: <subject>`. The body is never rewritten.
   An amendment states which earlier text it replaces; where they disagree, the
   later amendment wins. Timestamps come from the system clock, never typed by
   hand, and a wrong one is corrected by a later note, not by editing.
5. The first commit that implements a plan sets `status: In implementation`
   and appends an amendment saying implementation has started and the document
   is now immutable. After that it changes only on the owner's explicit
   instruction.
6. A plan may name **living sections** in its front matter. These stay open
   after implementation starts: new entries are appended as amendments, and an
   entry is fixed once its item is implemented. Implementation records, such as
   which commits implemented an entry, are appended as amendments, not filled in
   place.
7. When implementation departs from a frozen plan:
   - small departures needed to make the plan work are recorded in an addendum
     to this file, naming the plan, the step and the reason;
   - a departure that changes behaviour or an interface others depend on is
     discussed with the owner first (Rule 2), then recorded;
   - a frozen plan is never edited to match the code. A larger change of
     direction becomes a new dated plan that names the one it replaces.

## CRITICAL RULE 2: DISCUSS BEFORE CHANGING A PLAN

1. A change to a spec or plan is proposed to the owner first: what changes,
   where and why. Nothing is written to the document until the owner agrees.
2. Refinements made during the discussion go into the proposal, not the file.
   Once agreed, the whole change is written as **one** amendment, not a series
   of amendments that replace one another.

## CRITICAL RULE 3: COMMITS AND EVIDENCE

1. At most one crate or subproject per commit. Repository-level files go in
   their own commits: this file, the workspace manifest, evidence logs, tooling
   config and scripts. The one exception is adding a crate to the workspace in
   that crate's first commit.
2. A plan's status change and its immutable marker go in the commit that starts
   implementing it.
3. Commits name their paths explicitly (`git add <paths>`,
   `git commit -- <paths>`), so unrelated work in progress, including another
   agent's, is never swept in. Commits already on the shared branch are never
   amended or rewritten.
4. Before each code commit, the crate passes `cargo fmt -p <crate> -- --check`,
   `cargo clippy -p <crate> --all-targets` with no new warnings, and
   `cargo test -p <crate>`.
5. Renders and other generated evidence go under the gitignored `out/`. Each
   area keeps its own log file. Entries are appended under dated headings, with
   `./`-relative links and descriptions that stand on their own without the
   images. Earlier entries are never edited. Each run of evidence gets its
   own directory in its area, named like the plans: `YYYYMMDD_NN_name/`, with
   `NN` the sequence within the day (for example
   `out/fonts/20261023_01_cap_clipping/`).
6. Other documents under `docs/` that record findings, such as
   `docs/FONT_<NAME>.md`, are append-only. Each addition is a section headed
   `## YYYY-MM-DD HH:MM TZ — <subject>`, timestamped from the system clock.
   Earlier sections are never edited; a correction is a later dated section
   that says what it corrects.

## CRITICAL RULE 4: DECISION STATUS IN THE README

1. `README.md` is laid out in this order, so every section has a fixed
   place:
   1. a short introduction: what the project is, in a paragraph or two;
   2. the rest of the README: usage, features, building, licences and so on;
   3. `## Architectural decisions`, last: one GitHub Markdown table listing
      every spec and plan in `docs/design/`, oldest first. Nothing follows
      the table.

   The critical rules live in `AGENTS.md` only; the README does not repeat
   or summarise them.
2. Columns: **Document** (the title, as a relative link such as
   `[Camera view and capture](docs/design/20261014_01_CAMERA_VIEW_AND_CAPTURE_PLAN.md)`;
   no absolute or GitHub URLs), **Date** (from its front matter), **Status**
   (exactly its front-matter `status`, including `Superseded (by <link>)`
   and `Retired (by <link>)`),
   and **Decision** (one line: what it decides, and why it is at that status).
3. A row is added in the commit that adds the document, and updated in the
   same commit as every status-change amendment. This is an exception to
   Rule 3.1. The table is a summary: the documents and their amendments stay
   the record.

## CRITICAL RULE 5: VERSIONS AND TAGS

1. The commit that changes the crate's `version` in `Cargo.toml` is tagged
   `v<version>` (for example `git tag v0.3.0 <commit>`) as soon as it is
   committed. A tag that has been pushed is never moved or deleted.
2. `VERSIONS.md` records each version change, and only version changes:
   one entry per version, appended oldest first, headed
   `## <version> — YYYY-MM-DD (<short commit>, tag v<version>)`. The entry
   lists the public API changes since the previous version: what was added,
   changed or removed, with breaking changes marked **Breaking**. Internal
   changes are not listed, and commits that do not change the version get no
   entry. Earlier entries are never edited.
3. The `VERSIONS.md` entry is in the same commit as the version change. This
   is an exception to Rule 3.1.
4. While the version is `0.x`, the bump follows the public API change. A
   release whose public API changes are not strictly additive bumps the minor
   version (`0.x.y` to `0.(x+1).0`): anything removed, renamed or changed in
   signature or type, behaviour a caller can observe changing, or a variant or
   field added to an exhaustive public enum or struct. A release whose changes
   are strictly additive, or change no public API, bumps the patch version
   (`0.x.y` to `0.x.(y+1)`). This applies from 0.3.0 on; earlier versions are
   not re-labelled.
5. The public API rule is recursive through dependencies. When a crate with
   a public API moves a dependency to a version that is not semver-compatible
   with the one it used (for a `0.x` dependency, a new minor version; from
   1.0, a new major version), its own release is not strictly additive and
   bumps its minor version under Rule 5.4 (its major version from 1.0), even
   when its own code does not change. Every crate with a public API that
   depends on it does the same when it takes that release, and so on up the
   chain. The `VERSIONS.md` entry names the dependency with its old and new
   versions and marks the change **Breaking**. A repository with no public
   API (an application) only updates its manifest and lock file.

## Voice agents

Voice agents (such as flankr's general agent) read this file to talk about
the project, and their replies are spoken.

1. Say a plan or document by the words of its name, without its date,
   sequence number or path:
   `docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md` is "the general
   agent planning plan".
2. Leave out commit hashes, line numbers and timestamps unless asked for
   them.

## FLANKR RULES

These rules are flankr's own, beside the critical rules above.

### FLANKR RULE 1: THE APP'S VERSION AND `apps/UPDATES.md`

1. `apps/UPDATES.md` is the companion app's changelog, shown in the app.
   It lists only what someone using the app can see or do differently:
   screens, controls, gestures, text, sounds, speech and behaviour.
2. The app's version is `version` in `apps/companion/Cargo.toml`. It is
   separate from the crates' versions under Critical Rule 5, which follow
   public APIs: a crate release that changes nothing the user sees does
   not change the app's version, and an app-only change does.
3. Every commit that makes a user-visible change bumps the app's version
   and appends its entry to `apps/UPDATES.md`, in that same commit. This
   holds wherever the change is made: in `apps/companion`, in
   flankr-companion, in the daemon, or in a library such as libcontrols or
   libmsdf. When the change is outside `apps/`, the bump and the entry
   go in the commit that moves the app onto it. This is an exception to
   Critical Rule 3.1.
4. The bump, while the version is `0.x`: a new feature, or behaviour a
   user would notice changing, bumps the minor version (`0.x.y` to
   `0.(x+1).0`); only fixes and small polish bump the patch version
   (`0.x.y` to `0.x.(y+1)`). Changes landed together may share one bump.
5. An entry is headed `## <version> — YYYY-MM-DD` and lists its items
   under **Added**, **Changed**, **Fixed** and **Removed**, one short line
   each, in plain user language: no commit hashes, crate or protocol
   names, or plan names or numbers. A library is mentioned only by what
   it changed for the user (for example, new symbols in a menu).
6. Entries are appended, newest last, and never edited; a correction is a
   later entry that says what it corrects. The "History before
   versioning" section records the changes before this rule, with no
   versions, and is not added to.
7. The APK carries the app's version: `versionName` is the version, and
   `versionCode` is `major * 1000000 + minor * 1000 + patch`. Every build
   installed on a device carries the version of the newest entry it
   contains.
8. The commit that bumps the app's version is tagged
   `flankr-app-v<version>`, as Critical Rule 5.1 tags crate versions.

### FLANKR RULE 2: THE DAEMON'S VERSION AND `UPDATES.md`

1. `UPDATES.md` at the repository root is the changelog of the flankr
   daemon and CLI, for the person who runs `flankr`. It lists only what
   they can see or do differently: commands, subcommands and flags,
   config keys and their defaults, what the daemon does with herdr and
   the agents, what it logs or refuses that they act on, and the files it
   reads or writes in their folders.
2. The daemon's version is `version` in the root `Cargo.toml` (the
   `flankr` crate). The root crate has no public API, so Critical Rule 5
   gives it no bumps of its own; this rule is what moves it.
3. FLANKR RULE 1's items 3 to 6 apply to it as they do to the app, with
   the daemon's version and this file: every commit that makes a change
   visible to the person running `flankr` bumps the daemon's version and
   appends its entry here, in that same commit; minor for a new feature
   or behaviour they would notice changing, patch for fixes and polish;
   entries headed `## <version> — YYYY-MM-DD`, in plain language, a flag
   or config key named as it is typed; append-only, with the "History
   before versioning" section not added to.
4. A daemon change the app's user also notices gets an entry in both
   logs, each in its own reader's terms, and bumps both versions.
5. The commit that bumps the daemon's version is tagged `v<version>`, as
   Critical Rule 5.1 says for a crate, following the existing `v0.1.0`.

---

## 1. Core Architecture

`flankr` is designed as an asynchronous Rust binary (using `smol` or `async-std`) to bridge conversational AI with terminal execution. 

### A. The Conversational Loop (`~/src/libconversate` & `~/src/libgenai`)
`flankr` constructs its speech-to-speech pipeline using `libconversate`'s `s2s` builder pattern.
* **Audio Pipeline:** `libconversate` handles the Voice Activity Detection (VAD), slicing audio buffers, and passing them through STT/TTS.
* **LLM Processor Injection:** The core reasoning loop is injected into the pipeline via `libconversate`'s `with_processor` (or equivalent trait/callback) method.
* **Gemini Engine (`libgenai`):** The injected processor uses `~/src/libgenai`, a custom asynchronous wrapper around `gemini-async`. `libgenai` handles the system prompt injection, binds the tool calling schema, and executes the Gemini requests, catching tool calls to route to the transport layer before returning text to the TTS engine.

### B. The Transport Layer (Socket & SSH)
* **Local Mode:** Connects directly to the `herdr` UDS (e.g., `/tmp/herdr.sock`).
* **Remote Mode:** Uses a pure-Rust SSH client (via `--remote user@host`) to multiplex the connection and execute commands without added latency.

---

## 2. Tool Schema

* `list_panes()`: Returns the structural JSON layout of workspaces, tabs, and `agent_status`.
* `inspect_pane(pane_id, lines)`: Reads the raw tail buffer of a specific terminal.
* `run_command(pane_id, command)`: Executes a command in the target pane.
* `send_keys(pane_id, keys)`: Sends literal keystrokes (e.g., `ctrl+c`, `enter`).
* `manage_layout(action, target_id)`: Handles spatial commands (`split`, `zoom`, `close`).
* `focus(target_id)`: Shifts the user's active visual focus to a specific pane or workspace (project).

---

## 3. Command Permissions & the Confirmation Gate

Commands are gated in two independent layers before they reach a pane.

### A. Per-Agent Permissions (`[[agents]]` in `config.toml`)
Each pane's agent is resolved from the `herdr` snapshot (`extract_agent_name_for_pane`) and matched against a `[[agents]]` entry. A pane whose agent has no entry can never be sent a command.

* `allowed_commands` — an exact allow list (`"continue"`, `"/compact"`, `"/usage"`); matched case-insensitively.
* `allow_arbitrary_commands` — permits free-form spoken instructions that are *not* on the allow list. Because these are typed verbatim into the agent's prompt, `commands::validate_arbitrary_command` restricts them to ASCII letters, digits, spaces and the punctuation `. , ' ? :` — rejecting `/` (slash commands, paths), `-` (CLI flags), shell metacharacters, control characters (which would inject a second line of input), and anything over 256 characters.
* `require_confirmation` — enables the mechanical gate below.
* A top-level `allow_arbitrary_commands` key, or `--allow-arbitrary-commands` / `--no-arbitrary-commands`, forces the setting for every agent at once.

### B. The Mechanical Confirmation Gate (`VoiceState::confirmation_gate`)
The gate is a state machine (`Idle -> Armed -> Confirmed`) that the model cannot drive on its own:

1. The model answers with a confirmation question. `commands::parse_confirmation_request` extracts the pane and command it named and **arms** the gate with them, flushing queued audio so the user's reply is captured cleanly.
2. The user's next utterance either confirms the gate (an affirmative such as "yes"/"go ahead") or resets it.
3. `HerdrToolExecutor` calls `verify_and_consume_confirmation`, which requires a `Confirmed` gate *and* a pane/command match against what was announced, then consumes it — so a confirmation authorizes exactly one execution of exactly the command the user heard.

---

## 4. Gemini System Prompt

The runtime system prompt used by `flankr` is maintained in `PROMPT.md`. It is loaded at session startup and injected into Gemini to ground it in the terminal multiplexer environment.