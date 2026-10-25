---
title: Rename project to flankr across CLI, protocols, crates and apps
date: 2026-10-25
status: In implementation
covers:
  - "Renaming the root crate and CLI binary from `handlr` to `flankr`"
  - "Renaming workspace crates: `crates/handlr-*` and `crates/libhandlr_proto` to `crates/flankr-*` and `crates/libflankr_proto`"
  - "Updating wire protocols and constants: `flankr.harness`, `flankr.companion`, `server: \"flankr\"`, `flankr.link/1`, `flankr1…`, `flankr://`, `flankrkey1…`, `flankr.confirm/1`"
  - "Renaming the mobile companion app package to `dev.flankr.companion` and macOS Keychain / secure store identifier to `libmorph.flankr-companion`"
  - "Updating default filesystem paths: `~/.config/flankr/`, `~/.local/state/flankr/`, `~/Library/Application Support/dev.flankr.companion/`"
  - "Updating user-facing documentation (`README.md`, `AGENTS.md`, `PROMPT.md`), spoken agent names, and helper scripts"
  - "Explicitly preserving historical design documents, earlier entries in `CHANGES.md`/`VERSIONS.md`, and evidence logs unchanged"
sources:
  - "Owner (2026-10-25): \"rename project to flankr including all public strings, cli name, etc. update documentation. don't update logs/history or historical filenames. rename mobile app package to dev.flankr.companion and similar for any keychains on Mac. write a plan first, collecting all of the places (not a file list unlees that's the brst aporoach). plan for a limited transiti9n (app isn't public) Ill review the plan\""
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md
  - docs/design/20261022_04_COMPANION_APP_PLAN.md
  - docs/design/20261022_06_QRCODEURL_PLAN.md
---

# Rename project to flankr across CLI, protocols, crates and apps

## 1. Goal & Boundaries

This plan renames the project from `handlr` to `flankr`:
- The CLI binary becomes `flankr`, with subcommands `flankr serve`, `flankr ctl`, `flankr pair`, and `flankr session`.
- All workspace crates rename from `handlr-*` / `libhandlr_proto` to `flankr-*` / `libflankr_proto`.
- Wire protocols, Noise framing, URL schemes, and pairing strings cut over to `flankr.*` and `flankr1…`.
- The companion mobile application ID / package name becomes `dev.flankr.companion`.
- Desktop macOS secure storage / Keychain service namespace becomes `flankr-companion` (`libmorph.flankr-companion`), with data stored in `~/Library/Application Support/dev.flankr.companion/`.
- Default daemon configurations and socket directories move to `~/.config/flankr/` and `~/.local/state/flankr/`.
- User-facing documentation (`README.md`, `AGENTS.md`, `PROMPT.md`) is updated to reflect the new project name.

### Strict Boundaries
- **Historical files are NOT renamed:** Earlier plans in `docs/design/` (such as `20261023_09_HANDLR_DATA_PLAN.md`) retain their original file names.
- **Historical logs and records are NOT edited:** Past entries in `CHANGES.md`, `VERSIONS.md`, and evidence files under `out/**/LOG.md` are append-only historical records and remain untouched.
- **Limited transition:** Because the mobile application is not yet public and is currently used only in internal development and testing against the owner's devices, no complex multi-version compatibility shims or backward-compatibility translation gates are required. The cutover is clean and direct across the stack.

---

## 2. Collected Places by Domain

Rather than an unstructured list of files, the renaming touches eight distinct domain areas:

### Area 1: Workspace Manifest & Crate Structure
- **Root `Cargo.toml`:**
  - `package.name`: `"handlr"` → `"flankr"`.
  - Produces binary target `flankr` (replaces `handlr`).
  - Workspace members and path dependencies:
    - `crates/handlr-audio` → `crates/flankr-audio` (`name = "flankr-audio"`)
    - `crates/handlr-companion` → `crates/flankr-companion` (`name = "flankr-companion"`)
    - `crates/handlr-markdown` → `crates/flankr-markdown` (`name = "flankr-markdown"`)
    - `crates/handlr-speech` → `crates/flankr-speech` (`name = "flankr-speech"`)
    - `crates/libhandlr_proto` → `crates/libflankr_proto` (`name = "libflankr_proto"`)
- **`apps/companion/Cargo.toml`:**
  - Update path dependencies from `handlr-*` and `libhandlr_proto` to `flankr-*` and `libflankr_proto`.
  - Package description updated to "flankr's companion app...".

### Area 2: CLI Binary & Daemon Commands
- **Binary invocation:**
  - `flankr serve` (runs the background orchestration daemon)
  - `flankr ctl` (sends JSON-RPC requests to the daemon socket)
  - `flankr pair` (displays the Noise pairing QR code and URL)
  - `flankr session` (`tail`, `peek`, `content`)
- **CLI diagnostics, help, and error messages:**
  - `src/ctl.rs`: `"connect {}: {e} (is `flankr serve` running?)"`
  - CLI usage strings, examples, and argument parser diagnostics.
- **Environment variables:**
  - `HANDLR_AGY_EXAMPLE` → `FLANKR_AGY_EXAMPLE`.

### Area 3: Wire Protocol & Framing Constants (`libflankr_proto`)
- **Protocol Identifiers:**
  - `protocols::HARNESS`: `"handlr.harness"` → `"flankr.harness"`
  - `protocols::COMPANION`: `"handlr.companion"` → `"flankr.companion"`
- **Server Handshake:**
  - `server: "handlr"` → `server: "flankr"`
- **Noise Link Framing:**
  - `PROLOGUE_PREFIX`: `b"handlr.link/1"` → `b"flankr.link/1"`
  - `STATIC_MARKER`: `b"handlr.link/static-key"` → `b"flankr.link/static-key"`
- **Pairing Formats:**
  - `PAIRING_PREFIX`: `"handlr1"` → `"flankr1"`
  - `PAIRING_URL_SCHEME`: `"handlr"` → `"flankr"` (`flankr://<host>:<port>/pair#flankr1…`)
  - `DEVICE_CODE_PREFIX`: `"handlrkey1"` → `"flankrkey1"`
- **Confirmation Nonce Context (`src/core/confirm.rs`):**
  - `NONCE_CONTEXT`: `b"handlr.confirm/1"` → `b"flankr.confirm/1"`
- **Noise Key Store Kind (`apps/companion/src/keys.rs`):**
  - `KIND`: `"handlr.noise.x25519.v1"` → `"flankr.noise.x25519.v1"`

### Area 4: Configuration & Filesystem Paths
- **Daemon Configuration:**
  - Default config path: `~/.config/flankr/config.toml` (was `~/.config/handlr/config.toml`).
  - Fallback migration: If `~/.config/flankr/config.toml` is missing but `~/.config/handlr/config.toml` exists, copy it over cleanly during transition.
- **Daemon Socket & State:**
  - Default Unix domain socket: `~/.config/flankr/flankr.sock`.
  - Server private key: `~/.config/flankr/link_key`.
  - Authorized paired clients: `~/.config/flankr/auth.toml`.
  - Approval journals & session state: `~/.local/state/flankr/sessions/`.
- **Desktop Companion Fallback:**
  - Config & keys: `~/.config/flankr-companion/` (was `~/.config/handlr-companion/`).
  - Vocabulary cache: `~/.config/flankr-companion/vocab.txt`.
- **Log Files & Gitignore:**
  - Default log patterns: `flankr*.log` (updated in `.gitignore`).

### Area 5: Mobile Companion App (`apps/companion`) & Android Integration
- **Application ID / Package Name:**
  - `dev.tmzt.handlr` → `dev.flankr.companion` (configured in `apps/companion/build.rs` and `apps/companion/src/lib.rs:APP_ID`).
- **Android Manifest & Metadata:**
  - App label: `"flankr"`
  - URL scheme intent filters: `flankr://`
  - Foreground service declaration reason: `"Keeps the encrypted link to the user's own flankr daemon open..."`
- **Companion Helper Scripts (`apps/companion/scripts/`):**
  - `android.sh`: `APP_ID=dev.flankr.companion`
  - `push-pair.sh`: `APP_ID=dev.flankr.companion`
  - `push-vocab.sh`: `APP_ID=dev.flankr.companion`
  - `push-stt.sh`: `APP_ID=dev.flankr.companion`
  - `push-model.sh`: `APP_ID=dev.flankr.companion`

### Area 6: Desktop Companion & macOS Keychain
- **macOS Application Support:**
  - Data and cache directory: `~/Library/Application Support/dev.flankr.companion/`.
- **Secure Store / Keychain Namespace:**
  - `MorphApp::SECURE_STORE_ID`: `"flankr-companion"` (resulting in macOS Keychain service `libmorph.flankr-companion`).

### Area 7: Spoken, Displayed & Status Strings
- **UI Screen Titles:**
  - Overview header and main app bar: `"flankr"`.
- **Spoken & Shown Agent Names:**
  - `"the flankr agent"` (was `"the handlr agent"`)
  - `"the flankr shell"` (was `"the handlr shell"`)
  - `"Claude in flankr"`, `"Agy in flankr"`
- **System Dialogs & Alerts:**
  - Settings screen: `"flankr is offline"`
  - Battery optimization explanations: `"Android may hold back flankr's network when idle"`

### Area 8: Documentation & Repository Tooling
- **`README.md`:**
  - Title: `# flankr`
  - Project summary and usage guides.
  - Architectural decisions table (summarising this rename plan).
- **`AGENTS.md`:**
  - System prompt architecture and operational rules for `flankr`.
- **`PROMPT.md`:**
  - Gemini runtime system prompt grounded in the `flankr` environment.
- **`repowiki.toml`:**
  - Route navigation and prefix references.

---

## 3. Transition Strategy

Because the application is not yet public:
1. **Direct Cutover:** Both daemon and companion switch protocol strings (`flankr.companion`), handshake names (`server: "flankr"`), and link prologues (`flankr.link/1`) in step.
2. **Device Clean-up:**
   - On the S23 Ultra, running `adb uninstall dev.tmzt.handlr` followed by `./apps/companion/scripts/android.sh run` installs `dev.flankr.companion`.
   - On macOS, new pairings populate the `libmorph.flankr-companion` keychain and `~/Library/Application Support/dev.flankr.companion`.
3. **Daemon Config Migration:**
   - On startup, if `~/.config/flankr/config.toml` does not exist but `~/.config/handlr/config.toml` is found, the daemon copies the configuration and link key into `~/.config/flankr/`, logging a one-line notification.

---

## 4. Implementation Steps

1. **Wire protocol and core library crates:**
   - Rename `crates/libhandlr_proto` directory to `crates/libflankr_proto` and update `Cargo.toml` (`name = "libflankr_proto"`).
   - Update protocol names, constants, pairing prefixes (`flankr1`), URL schemes (`flankr://`), and device codes (`flankrkey1`).
   - Rename `crates/handlr-speech` to `crates/flankr-speech` and `crates/handlr-markdown` to `crates/flankr-markdown`.
   - **Done when:** `cargo test -p libflankr_proto -p flankr-speech -p flankr-markdown` passes cleanly.

2. **Companion library and audio engine:**
   - Rename `crates/handlr-companion` to `crates/flankr-companion` and `crates/handlr-audio` to `crates/flankr-audio`.
   - Update imports and internal references to `libflankr_proto`, `flankr-speech`, and `flankr-audio`.
   - Update default socket and key store paths to `~/.config/flankr/` and `~/.config/flankr-companion/`.
   - **Done when:** `cargo test -p flankr-companion` passes all unit and link tests.

3. **Daemon binary and CLI:**
   - Update root `Cargo.toml`: `name = "flankr"`, workspace members, and path dependencies.
   - Update CLI binary name, subcommands (`flankr serve`, `flankr ctl`, `flankr pair`, `flankr session`), help strings, and error messages.
   - Update default daemon socket path (`~/.config/flankr/flankr.sock`), auth (`auth.toml`), and state directory (`~/.local/state/flankr/sessions`).
   - Update naming rules: `"the flankr agent"`, `"the flankr shell"`.
   - **Done when:** `cargo test -p flankr` passes all daemon, link, and subscription tests, and `./target/debug/flankr --help` displays `flankr`.

4. **Mobile companion app (`apps/companion`) & macOS Keychain:**
   - In `apps/companion/Cargo.toml`: update dependency crate references.
   - In `apps/companion/build.rs`: set Gradle package to `dev.flankr.companion`, label to `"flankr"`, and URL scheme to `"flankr"`.
   - In `apps/companion/src/lib.rs`: update `APP_ID = "dev.flankr.companion"`, `SECURE_STORE_ID = "flankr-companion"`.
   - In `apps/companion/src/keys.rs`: update `KIND = "flankr.noise.x25519.v1"`.
   - In `apps/companion/scripts/*.sh`: update `APP_ID=dev.flankr.companion`.
   - Update UI headers and text strings from `"handlr"` to `"flankr"`.
   - **Done when:** `cargo test` in `apps/companion` passes, and `./apps/companion/scripts/android.sh build` generates an APK with package `dev.flankr.companion`.

5. **Documentation, prompts & repository tooling:**
   - Update `README.md` introduction, CLI examples, and architectural decisions table.
   - Update `AGENTS.md` and `PROMPT.md`.
   - Update `repowiki.toml` and `.gitignore`.
   - **Done when:** `cargo fmt --check` and `cargo clippy --all-targets` succeed with zero warnings across all crates.

6. **End-to-end verification & live test:**
   - Start the new daemon with `flankr serve`.
   - Pair or connect via `flankr ctl` and `flankr session peek`.
   - Install `dev.flankr.companion` on the S23 Ultra and verify live connection.
   - **Done when:** Live verification passes and is recorded in the evidence log.

## Amendment — 2026-10-25 12:18 EDT: dynamic agent names vs static strings

This amendment clarifies Section 2 (Area 7: Spoken, Displayed & Status Strings) and Section 4 (Step 3).

### Replaced Text
In Section 2, Area 7:
> - **Spoken & Shown Agent Names:**
>   - `"the flankr agent"` (was `"the handlr agent"`)
>   - `"the flankr shell"` (was `"the handlr shell"`)
>   - `"Claude in flankr"`, `"Agy in flankr"`

In Section 4, Step 3:
> - Update naming rules: `"the flankr agent"`, `"the flankr shell"`.

### New Text & Clarification
1. **Dynamic Naming Logic:**
   Spoken and shown agent phrases such as `"the handlr agent"`, `"the handlr shell"`, and `"Claude in handlr"` are dynamically computed at runtime by `src/core/naming.rs` from workspace labels and directory basenames (`cwd`) provided by `herdr`. They are not static strings in the naming logic and require no code changes in the naming resolution algorithm.
2. **Static Strings Updated:**
   The static strings updated in Area 7 are:
   - Daemon prompt in `src/transport.rs`: `"handlr itself asks the user to approve..."` → `"flankr itself asks the user to approve..."`.
   - Companion UI header in `apps/companion/src/overview.rs`: `heading("handlr", ...)` → `heading("flankr", ...)`.
   - Status strings in `apps/companion`: `"flankr is offline"`.
3. **Test Fixtures:**
   Mock data in unit tests (`naming.rs`, `notify.rs`, `agent_card.rs`) using `cwd: ".../handlr"` or `label: "handlr"` are updated to `"flankr"` to maintain test consistency.

## Amendment — 2026-10-25 12:20 EDT: implementation started

Implementation has started. This document is now immutable.


