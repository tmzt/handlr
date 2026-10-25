---
title: Extract libinfer into standalone repository ~/src/libinfer
date: 2026-10-25
status: Draft
covers:
  - "Extracting `crates/libinfer` from `flankr` into a new standalone repository `~/src/libinfer`"
  - "Creating root workspace and configuration for `~/src/libinfer` with members `libinfer` and `libinfer-gemini`"
  - "Setting up `AGENTS.md`, `CHANGES.md`, `VERSIONS.md`, `README.md`, `repowiki.toml`, and registration in repowiki"
  - "Updating `flankr` to depend on `~/src/libinfer` via path dependencies (`../libinfer/libinfer`, `../libinfer/libinfer-gemini`)"
  - "Removing `crates/libinfer/` from the flankr workspace"
sources:
  - "Owner (2026-10-25): \"libinfer is a candidate for extraction\" -> \"yes\""
  - "docs/design/20261022_07_LIBINFER_PROVIDERS_PLAN.md (decisions 1 & 2: nested layout designed for later breakout)"
related:
  - docs/design/20261022_07_LIBINFER_PROVIDERS_PLAN.md
  - docs/design/20261025_01_RENAME_PROJECT_TO_FLANKR_PLAN.md
---

# Extract libinfer into standalone repository ~/src/libinfer

## 1. Goal & Motivation

Plan [20261022_07](20261022_07_LIBINFER_PROVIDERS_PLAN.md) structured `crates/libinfer` as two self-contained sub-crates under one directory:
- `crates/libinfer/libinfer/`: The provider-neutral reasoning core (traits `ReasoningEngine`, `ToolExecutor`, `TurnContributor`, data structures `Message`, `Role`, `RoutingReasoningEngine`, `MockReasoningEngine`, `MockToolExecutor`, `SessionManager`).
- `crates/libinfer/libinfer-gemini/`: The Gemini provider (`GeminiReasoningEngine` wrapping `gemini-rust` 1.7).

Neither crate depends on `herdr`, `flankr`, protobuf wire formats, audio, or multiplexer protocols.

Extracting `libinfer` into its own repository (`/Users/tmeade/src/libinfer`) makes reasoning engines reusable across all `~/src/` projects, simplifies `flankr`'s workspace, and establishes a dedicated repository where provider-neutral tool declarations (`ToolSpec`) and future providers (`libinfer-openai`, `libinfer-anthropic`, local `llama.cpp`) can evolve independently.

---

## 2. Target Layout in `~/src/libinfer`

```text
/Users/tmeade/src/libinfer/
├── .git/
├── .gitignore
├── AGENTS.md
├── Cargo.lock
├── Cargo.toml
├── CHANGES.md
├── docs/
│   └── design/
├── libinfer/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── libinfer-gemini/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── LICENSE-APACHE
├── LICENSE-MIT
├── README.md
├── repowiki.toml
└── VERSIONS.md
```

### Workspace Root Manifest (`Cargo.toml`)
```toml
[workspace]
members = [
    "libinfer",
    "libinfer-gemini",
]
default-members = [
    "libinfer",
    "libinfer-gemini",
]
resolver = "2"
```

### Sub-crates
- `libinfer/Cargo.toml`: `name = "libinfer"`, `version = "0.1.0"`, `edition = "2024"`.
- `libinfer-gemini/Cargo.toml`: `name = "libinfer-gemini"`, `version = "0.1.0"`, `edition = "2024"`, dependency `libinfer = { path = "../libinfer" }`.

### Project Infrastructure
- `AGENTS.md`: Standard tmeade / tmzt critical rules adjusted for Rust crates (`cargo fmt -p <crate> -- --check`, `cargo clippy -p <crate> --all-targets`, `cargo test -p <crate>`).
- `CHANGES.md`: Initial entry recording repository creation and adoption of the planning system.
- `VERSIONS.md`: Initial release entry `## 0.1.0 — 2026-10-25 (<commit>, tag v0.1.0)` documenting public traits and types.
- `repowiki.toml`:
  ```toml
  default_route = "/README"
  static_dirs = ["out"]

  [crate_resolution]
  prefixes = ["docs", "libinfer", "libinfer-gemini"]
  ```
- Register repository in `~/src/repowiki/sites.toml`.

---

## 3. Flankr Transition

In `~/src/handlr` (flankr):
1. Update root `Cargo.toml`:
   - Dependencies:
     ```toml
     libinfer = { path = "../libinfer/libinfer" }
     libinfer-gemini = { path = "../libinfer/libinfer-gemini" }
     ```
   - Remove `"crates/libinfer/libinfer"` and `"crates/libinfer/libinfer-gemini"` from `workspace.members` and `workspace.default-members`.
2. Remove directory `crates/libinfer/`.
3. Verify that `cargo fmt -p flankr -- --check`, `cargo clippy -p flankr --all-targets`, and `cargo test -p flankr` pass cleanly.

---

## 4. Implementation Steps

### Step 1: Create and initialize `~/src/libinfer` repository
- Create directory `/Users/tmeade/src/libinfer`.
- Initialize git repository.
- Migrate `crates/libinfer/libinfer` and `crates/libinfer/libinfer-gemini` source directories into `libinfer/` and `libinfer-gemini/`.
- Add workspace `Cargo.toml`, `.gitignore`, `LICENSE-MIT`, and `LICENSE-APACHE`.
- Add `AGENTS.md`, `CHANGES.md`, `repowiki.toml`, and `README.md` with Architectural decisions table.
- **Done when:** `cargo fmt --check`, `cargo clippy --all-targets` (0 warnings), and `cargo test` pass across both crates in `~/src/libinfer`.

### Step 2: Versioning, tags, and repowiki registration
- Add `VERSIONS.md` with initial entry `0.1.0`.
- Create git tag `v0.1.0` on the version commit.
- Append `/Users/tmeade/src/libinfer` to `~/src/repowiki/sites.toml`.
- **Done when:** `git tag -l` lists `v0.1.0` and `make check` / repowiki config succeeds.

### Step 3: Cut over flankr workspace
- In `~/src/handlr/Cargo.toml`: switch `libinfer` and `libinfer-gemini` dependencies to `{ path = "../libinfer/libinfer" }` and `{ path = "../libinfer/libinfer-gemini" }`.
- Remove `crates/libinfer/` from `workspace.members` and delete `crates/libinfer/`.
- Update docstrings in `src/reasoning.rs` or `src/server/companion.rs` if needed.
- **Done when:** `cargo test -p flankr` passes all 276 unit tests, 40 companion tests, 11 daemon tests, 9 link tests, 1 live naming test, and 22 subscription tests.
