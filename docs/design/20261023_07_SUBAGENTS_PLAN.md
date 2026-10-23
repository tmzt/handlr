---
title: Subagents, listed and shown in the agent view as the main agent is
date: 2026-10-23
status: Draft
covers:
  - "Finding an agent session's subagents (Claude Code's background and
    foreground agents) and their state, behind the reader-agnostic trait"
  - "Selecting a subagent in the agent view, as in Claude Code's terminal
    interface, and showing its transcript in the same standard view as
    the main agent, with its cards, questions, read aloud and diffs"
  - "Linking the parent's tool call that started a subagent to it, and
    counting running subagents on the overview's card"
  - "Not covered: starting, stopping or messaging a subagent; subagents of
    agents other than Claude Code (each reader adds them by amendment)"
sources:
  - "Owner (2026-10-23, by voice; \"the quad to be\" read as \"the Claude
    TUI\"): \"we should have a plan for handling sub agents similar to the
    way that they're shown in the [Claude TUI]. So having a way to select
    the sub agent and then that would show in the standard card view. The
    … same way that the main agent is shown\"; then \"Yes, create the
    subagents plan\""
  - "Claude Code's files, found on 2026-10-23 (names and keys only): a
    session's subagents are `~/.claude/projects/<project>/<session
    id>/subagents/agent-<agent id>.jsonl`, in the session's own JSONL
    format, each with `agent-<agent id>.meta.json` holding `agentType`,
    `description`, `toolUseId` (the parent's tool call), `spawnDepth`,
    `requestShape` (`background` or not), `finished`, and, for a worktree,
    `worktreePath` and `worktreeBranch`. The parent's `Agent` tool result
    (`toolUseResult`) holds `agentId`, `description`, `status`, `isAsync`
    and `outputFile`. This session had 22 subagents"
  - "Plans 20261023_06 (the agnostic `SessionReader`), 20261023_04 (the
    overview's cards and `Turn`), 20261022_03 L3 (the agent view)"
related:
  - docs/design/20261023_06_SESSION_READERS_PLAN.md (the trait the children hang off)
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (the card's subagent count)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3, the agent view)
  - docs/design/20261023_03_SESSION_SUBSCRIPTIONS_PLAN.md (subscribing to a subagent's session when that plan is built)
---

# Subagents, listed and shown in the agent view as the main agent is

## 1. Goal

A Claude Code pane often runs subagents: a background build in a
worktree, a search, a review. Claude Code's terminal interface lists them,
and selecting one shows its work. On the phone today they are invisible:
the parent's flow shows only a dim `Agent` tool call, and herdr reports
the pane `working` while one runs.

After this plan, the agent view lists the pane's subagents, and selecting
one shows its transcript in the same view the main agent uses: its
scrollback, its cards and questions, read aloud, and diffs. Back returns
to the parent.

## 2. Decisions

1. **A subagent is a child session.** It has its own transcript in the
   parent reader's format, so the same reader reads it and everything
   above the readers (turns, questions, cards, redaction, summaries)
   applies unchanged. The trait of plan 20261023_06 gains:

   ```rust
   /// The sessions started from `at`, newest first. None by default.
   fn children(&self, at: &Located) -> Vec<Child> { Vec::new() }

   pub struct Child {
       pub id: String,              // Claude Code's agent id
       pub located: Located,        // its transcript
       pub description: String,     // as the parent gave it
       pub kind: String,            // `general-purpose`, `Explore`, …
       pub background: bool,
       pub finished: bool,
       pub started_by: Option<String>,  // the parent's tool call id
       pub worktree: Option<String>,    // the branch, not the path
   }
   ```

   Claude Code's reader fills it from `<session>/subagents/*.meta.json`.
   A missing or unreadable meta file gives a child with its id as the
   description and `finished` from the transcript's quiet (5 s, as
   `Located::quiet`).
2. **The protocol names a session by pane and an optional child.**
   `session.peek`, `speech.summarize` (and `session.subscribe` when built)
   take `child: Option<String>`; without it they read the main session,
   as now. `agents.list`'s `AgentInfo` gains `children: Vec<ChildInfo>`
   (`id`, `description`, `kind`, `background`, `finished`, `turn`), at
   most 16, running first then newest. `ChildInfo.turn` is the overview's
   `Turn` for that child.
3. **The selector, as in the terminal interface.** Under the agent view's
   header, a row of chips names the subagents: the description, cut to
   fit, a dot for running (blue) or finished (dim), and the count when
   there are more than fit (tap to show them all as a list). Tapping a chip
   opens that subagent in the same view; the header then reads "‹
   <parent name> › <description>", and Back returns to the parent. The
   chips are hidden when there are none.
4. **The parent's flow links to it.** The `Agent` tool call that started a
   subagent (matched by `started_by`) is drawn as a one-line chip in the
   flow, "⎿ <description> (running|finished)"; tapping it opens the
   subagent.
5. **Answering a subagent.** A subagent's question card shows, as the
   main agent's does, but its Yes and No are drawn inactive: a subagent
   takes no input from the pane, so answering it means answering the
   parent. The record button sends to the parent pane, as now.
6. **The overview card** of a pane with running subagents adds "· N
   running" to its line, and a subagent that is asking makes the card
   "needs input" (plan 20261023_04, A3), naming it.
7. **Privacy and limits.** Children are read with the same redaction and
   limits as the main session. A worktree's path is never sent, only its
   branch name.
8. **Controls, one per file:** `apps/companion/src/agent/subagent_chips.rs`
   (the selector) and `agent/subagent_link.rs` (the chip in the flow).

## 3. Steps

1. **The trait and Claude Code's children.** `children`, `Child`, the
   meta files read; fixtures under `tests/fixtures/sessions/home/` with a
   session holding two subagents (one running, one finished, one meta
   file missing). **Done when:** tests list them with their fields and
   order, and a subagent's transcript reads as items.
2. **The protocol** (`child` on the session methods, `AgentInfo.children`,
   `ChildInfo`), in the next libhandlr_proto release after 0.3.0, with
   its VERSIONS.md entry. **Done when:** they round-trip, and a method
   without `child` reads as before.
3. **The daemon.** `child` honoured by `session.peek` and
   `speech.summarize`; `AgentInfo.children` filled. **Done when:** daemon
   tests read a child's entries and question by pane and child, and an
   unknown child is `UNKNOWN_AGENT`.
4. **The client.** The view's child (`Input::OpenChild(id)`, Back to the
   parent), peeks with `child`, the children in `Event::AgentView`.
   **Done when:** client tests open a child, read it, and return.
5. **The app.** The chips, the link in the flow, the header, the inactive
   answers, and the overview's count. **Done when:** app tests place the
   chips and link and open a child from each; and on the S23, a running
   background subagent of a live pane is listed, opens, and shows its
   transcript.

Each step is one crate or subproject, one commit (Rule 3.1). 2 needs
nothing; 3 needs 1 and 2; 4 needs 2; 5 needs 3 and 4.

## 4. Open questions for the owner

1. **Nested subagents** (a subagent's own subagents, `spawnDepth` 2 and
   more): shown as children of the child, or flattened under the pane?
2. **Finished subagents:** keep listing them for the session's life, or
   only the last few?
3. **The chips' place:** under the header as proposed, or a list behind a
   button to save the scrollback's height?

## Amendment — 2026-10-23 17:47 EDT: the subagents as cyan words above the controls

Agreed with the owner on 2026-10-23: "visually I like the way Claude shows
them, but let's try the chips idea. where should they go in the UI?";
"right now they say something like general purpose then time elapsed and
tokens. the 2 agents isn't very helpful. I'm thinking we could get them to
one or two word summaries with main purpose, stop words filtered, as plain
text in cyan"; "elapsed in minutes, m in () only if more than 5"; then
"Yes, write it into the subagents plan". This replaces decision 3's chips
under the header and answers open question 3.

1. **Where.** One row at the bottom of the agent view, just above the
   suggestions and the controls, where the terminal interface shows its
   background agents; shown only while the pane has subagents, one line
   high. It scrolls sideways when the names do not fit.
2. **What it says.** Each subagent by one or two words of its purpose, in
   plain cyan text, separated by dim dots, for example
   `overview screen (12m) · spoken summaries · licences`:
   - bright cyan while running, dim cyan once finished;
   - a running subagent's elapsed minutes in parentheses, `(12m)`, only
     past 5 minutes, counted from its first entry; finished ones show no
     time; no agent type and no token counts.
3. **The short name** (`ChildInfo.short`), made by the daemon so every
   client and reader gets the same words: the description's words with
   stop words and task verbs removed (articles, prepositions,
   conjunctions, "plan", "plans", "step", "steps", "build", "implement",
   "fix", "add", "write", "update", "make", "run", "check" and the like,
   and plan numbers such as `20261023_05`), then the first two left, in
   their order, lower-cased except for words written in capitals. Nothing
   left: the agent type. So "Build the overview screen plan" is
   "overview screen" and "libtypedmd licences and README" is "libtypedmd
   licences".
4. **Tapping a name** opens that subagent in the standard view (decision
   3's opening and header stand). Inside a subagent, the row's first item
   is "‹ back to <parent>" in dim text, then the other subagents.
5. **The protocol** adds `ChildInfo.short: String` and
   `ChildInfo.started: u64` (seconds since the epoch, the first entry's
   time), so the phone keeps the minutes current without asking again.
6. **Controls:** `agent/subagent_row.rs` replaces decision 8's
   `subagent_chips.rs`; `subagent_link.rs` stands.

Step 1's tests gain the short names of recorded descriptions; step 5's,
the row's order, colours, the minutes shown past 5 and not before, and a
tap on each name.

## Amendment — 2026-10-23 17:48 EDT: "main" is the first name in the row

The owner (2026-10-23): "no back to, main is the first chip". This
replaces item 4's "Inside a subagent, the row's first item is '‹ back to
<parent>'": the row always starts with **main**, the pane's own agent,
followed by the subagents. Tapping main shows the main agent; the name
being viewed (main or a subagent) is underlined. The header's "‹" still
leaves the agent view for the overview.
