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

## Amendment — 2026-10-23 17:48 EDT: a dot marks the name being viewed

The owner (2026-10-23): "use a dot to the left instead of underline". This
replaces the previous amendment's underline: the name being viewed (main
or a subagent) has a small dot to its left, in its own cyan; the others
have none.

## Amendment — 2026-10-23 17:49 EDT: approved

The owner approved the plan on 2026-10-23 ("approve the subagents plan").
It stays `Draft` until its first implementing commit, which sets `In
implementation` (Rule 1.5). It is built after two things it needs: the
overview's libhandlr_proto 0.3.0 (plan 20261023_04, being built) and the
agnostic trait (plan 20261023_06, step 2), whose `children` it extends.
Open questions 1 (nested subagents) and 2 (how long finished ones are
listed) stay open; until the owner answers, nested subagents are listed
flat under the pane and finished ones for the session's life, as the
smallest choices, and either can change by amendment.

## Amendment — 2026-10-28 20:46 EDT: the subagents in a title-bar dropdown, not a row

Agreed with the owner on 2026-10-28: "plan to add agent/subagent dropdown
menu to titlebar (before plugins menu)"; then, to the lead's proposal,
"yes, but this isn't a plugin just giving the ui location". This replaces
the 17:47 EDT amendment's items 1 and 4 and its control
`agent/subagent_row.rs` (the row above the controls), and the two 17:48
EDT amendments (main first in the row, the dot for the name viewed). Item
2's colours and minutes, item 3's short names and item 5's protocol
fields stand, applied to the menu. Decision 4's link in the flow stands.

1. **The app's own control, not a plugin.** The selector is part of the
   agent view, built into the app. "Before the plugins menu" names only
   where it sits: in the agent view's title bar, just left of the ⋮ of
   plan 20261028_04 (decision 3.6). It uses no plugin interface.
2. **The button.** Shown only while the pane has subagents (none: the
   title bar is as without it). It reads ◉ (U+25C9) while any subagent
   is running, followed by the number running, and ○ (U+25CB) once all
   have finished; drawn from the terminal face, in cyan as item 2.
3. **The menu** is a libcontrols `PopupMenu` (0.10.3, libcontrols plan
   20261028_06) anchored to the button, opening below it:
   - **main** first, the pane's own agent; then each subagent by its short
     name, running ones first, then finished, newest first within each;
   - ● (U+25CF) bright cyan before a running one, ○ dim cyan before a
     finished one; a running one's elapsed minutes, `(12m)`, only past 5
     minutes;
   - the session being viewed is the menu's checked item.
4. **Choosing** an item opens it in the same standard view (decision 3's
   opening and header stand); choosing **main** returns to the main agent.
   A tap outside, Back or Escape closes the menu without changing the view.
5. **No row.** The bottom row above the suggestions and controls is not
   built; the transcript keeps that line. The menu lists no other panes'
   agents (the session drawer, plan 20261028_03, does that).
6. **Open questions 1 and 2** keep the 17:49 EDT amendment's smallest
   choices: nested subagents flat under the pane, finished ones listed for
   the session's life.
7. **Control:** `apps/companion/src/agent/subagent_menu.rs` replaces
   `agent/subagent_row.rs`. Step 5's tests: the button hidden with no
   subagents, its glyph and count, the menu's order, marks, colours and
   minutes (shown past 5 and not before), the checked item, and choosing
   a subagent and main.

## Amendment — 2026-10-28 20:51 EDT: no colour; ⊙ is the icon, the dots a radio selector

The owner (2026-10-28), to the lead's two ways of colouring the menu: "no
color, it just echoes the agent selector in Claude, the empty dot is the
menu icon then the dot is used as a radio selector in the menu"; then
"yes" to this wording. This replaces the 20:46 EDT amendment's items 2
and 3 and the colour of the 17:47 EDT amendment's item 2 (no cyan,
bright or dim); the 20:46 EDT amendment's items 1 and 4–7 stand, with
item 7's tests following this one.

1. **The icon.** The title-bar button is the empty dot ⊙ (U+2299), in
   the title bar's own text colour, from the terminal face; shown only
   while the pane has subagents. It carries no count and no running or
   finished state. Until the terminal symbol page has U+2299 (libmsdf's
   glyph additions), it is drawn as a plain ring of the same size.
2. **A radio selector.** Every item of the menu has a radio mark: ⦿
   (filled) on the session being viewed, ⊙ (empty) on the others. There
   is no check mark.
3. **No colour.** Labels are in the menu's own text colour. The order
   is **main**, then a **Running** heading and the running subagents,
   newest first, each with its elapsed minutes past 5, `(12m)`; then a
   **Finished** heading and the finished ones, newest first. A heading
   with no subagents under it is left out. Headings are the
   `PopupMenu`'s own (`PopupItemKind::Heading`).
4. **libcontrols.** The menu's labels are in Roboto, which has neither
   dot, so the marks are drawn by the `PopupMenu` itself: a radio style
   for its mark column, a ring with a filled dot for the selected item,
   drawn as its check is now (an additive libcontrols release, 0.10.4,
   with its own plan in that repository).
5. **Step 5's tests** check the icon hidden with no subagents and shown
   as a ring or ⊙, the menu's order and headings, the minutes shown past
   5 and not before, one filled radio on the session viewed, and
   choosing a subagent and main.
