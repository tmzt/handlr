---
title: Moving the checkout to ~/src/flankr, and this Claude Code session with it
date: 2026-10-27
status: Draft
covers:
  - "Moving the local checkout from `~/src/handlr` to `~/src/flankr`, the
    last place the old name is still the working name"
  - "The remotes: `flankrapp/flankr` as `origin`, and what becomes of
    `tmzt/handlr`"
  - "Moving this Claude Code session (its transcript, memory and herdr
    pane) so it continues from the new folder"
  - "Everything outside the repository that names the old path: repowiki,
    the daemon's launch, herdr's pane, and this session's memory"
  - "Not covered: the code, protocol and package rename, which plan
    20261025_01 already did; history and earlier records, which keep the
    old name"
sources:
  - "Owner (2026-10-27): \"review recent work, plan rename of repo to
    flankr and moving session to that folder\""
  - "Plan 20261025_01 (Implemented): crates, CLI, protocols, app package
    `dev.flankr.companion` and config paths (`~/.config/flankr/`) are
    already `flankr`"
  - "Found on 2026-10-27:
    - remotes `origin` = `git@github.com:tmzt/handlr.git` and `flankrapp` =
      `git@github.com:flankrapp/flankr.git`, both level with `main`;
    - repowiki `~/src/repowiki/sites.toml` line 47 has `source =
      \"/Users/tmeade/src/handlr\"` (site id `src--handlr`);
    - this session's data is `~/.claude/projects/-Users-tmeade-src-handlr/`
      (147 MB: the transcript `992c3f44-….jsonl`, its subagent folder, and
      `memory/`); one memory file names `src--handlr`;
    - in the repository, only two plans name the absolute path, and they
      are history;
    - the daemon last ran from `~/src/handlr/target/debug/handlr` and
      stopped on 2026-10-25 06:52 UTC (exit 143)"
related:
  - docs/design/20261025_01_RENAME_PROJECT_TO_FLANKR_PLAN.md (the rename this completes)
---

# Moving the checkout to ~/src/flankr, and this Claude Code session with it

## 1. Goal

The project is `flankr` in its code, protocols, app and config, but its
checkout is still `~/src/handlr`, its `origin` is still `tmzt/handlr`, and
this Claude Code session, repowiki and the herdr pane all point there.
After this plan, the checkout is `~/src/flankr`, `origin` is
`flankrapp/flankr`, and this session carries on from the new folder with
its history and memory.

## 2. Decisions

1. **One move, not a copy.** `mv ~/src/handlr ~/src/flankr`, with no work in
   progress and no worktrees (`.claude/worktrees/` is empty). The build
   targets (`target/`, `apps/target/`) move with it. They hold no absolute
   paths that matter, but the first build after the move may rebuild.
2. **A transition symlink.** `~/src/handlr` becomes a symlink to
   `~/src/flankr` for a week, so old shells, scripts and the herdr pane keep
   working. It is removed by a later note in CHANGES.md.
3. **Remotes.**
   - `flankrapp/flankr` becomes `origin`, and `main` tracks it.
   - The old `origin` is kept as `old-origin` (`tmzt/handlr`) until the owner
     decides open question 1.
   - Pushes go to the new `origin` only, and nothing is ever force-pushed.
4. **This session's data.**
   - Claude Code keys a project by its folder, so `~/.claude/projects/
     -Users-tmeade-src-handlr/` is copied to `-Users-tmeade-src-flankr/`
     (transcript, subagent folder, `memory/`).
   - The old folder is kept until the session has resumed from the new one,
     then removed (with the owner's yes, Rule: ask before deleting).
   - Memory files that name the old path or site id are updated in the copy,
     and the memory index (`MEMORY.md`) is checked for them.
5. **Resuming the session.** The session can't move its own working
   folder. So:
   - this session finishes its turn and is closed;
   - the owner starts `claude --resume 992c3f44-18e7-4cf8-814f-e437b90209b6`
     from `~/src/flankr`, in the same herdr pane after `cd ~/src/flankr` or
     in a new pane;
   - the resumed session confirms its working folder and memory.
6. **Outside the repository:**
   - **repowiki:** `sites.toml`'s `source` becomes `/Users/tmeade/src/flankr`,
     and the site id becomes `src--flankr`. Old links to `#/src--handlr/…`
     break; repowiki belongs to another session, which is asked to make the
     change, and to keep an alias if it supports one.
   - **The daemon:** it is started from `~/src/flankr` as `target/debug/flankr
     serve`, rebuilt after the move. It is currently down (it stopped on
     2026-10-25), so the phone stays disconnected until then.
   - **herdr:** the pane's working folder follows the restarted session.
     herdr finds the session by its folder, so the new folder's transcript
     is the one shown on the phone.
   - **Peer sessions** that name `~/src/handlr` in messages (libmorph) are told
     the new path once.
7. **Records.** Earlier plans, CHANGES.md entries, VERSIONS.md and evidence
   keep the old path, as plan 20261025_01 decided. New records use the new
   path.

## 3. Steps

1. **Remotes** (in place, before the move).
   **Done when:** `git remote -v` shows `origin` = `flankrapp/flankr` and
   `old-origin` = `tmzt/handlr`, `main` tracks `origin/main`, and `git status
   -sb` shows it level.
2. **repowiki** (by its owning session): the new source path and site id.
   **Done when:** `make serve` in repowiki serves `#/src--flankr/README`.
3. **The move:** the session's data copied, its memory updated, the checkout
   moved and the symlink made, in one command run by the owner or by this
   session as its last action.
   **Done when:** `~/src/flankr` holds the checkout, `~/src/handlr` is a
   symlink to it, and `~/.claude/projects/-Users-tmeade-src-flankr/` holds
   the transcript and memory.
4. **Resume:** the owner resumes the session from `~/src/flankr`.
   **Done when:** the resumed session reports its working folder as
   `~/src/flankr`, finds its memory, builds the daemon, and (with the
   owner's yes) restarts it; and the phone reconnects.
5. **Clean-up** after a week: remove the symlink and the old project data,
   with the owner's yes, recorded in CHANGES.md.

Steps 1, 2 and 3 change no code. Each repository-level change (README
row, this plan) is its own commit (Rule 3.1).

## 4. Open questions for the owner

1. **`tmzt/handlr` on GitHub:** archive it, rename it to point people at
   `flankrapp/flankr`, or delete it later?
2. **The symlink:** keep `~/src/handlr` → `~/src/flankr` for a week as
   written, or no symlink at all?
3. **Who runs the move:** this session as its last action (it then can't
   continue until resumed), or the owner from a shell?
