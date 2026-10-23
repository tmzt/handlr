---
title: The overview (home) screen, a list of the agents
date: 2026-10-23
status: Draft
covers:
  - "The companion app's home screen becomes an overview: one row per
    agent pane, with its name, status, herdr's focus and a badge when it
    needs the user; tapping a row opens the agent view on that pane"
  - "How the app's controls are organised: one control per file, in the
    module of the screen that uses it, or in a common `controls` module
    when more than one screen uses it; written as candidates for
    libcontrols but not moved there"
  - "Not covered: each agent's latest line on its row (with session
    subscriptions, plan 20261023_03); following herdr's focus (left off);
    agents on more than one herdr"
sources:
  - "Owner (2026-10-23): \"if we're going to support multiple agents, we
    should have the list of them on the new \\\"overview\\\" (home)
    screen\"; \"yes, that makes sense, an overview module with a file per
    control. we should also think of these controls as perspective items
    belonging in libcontrols, but not moving them there. if we are building
    a control for ourself that applies to multiple screens it should be in
    a common place in the companion app\"; \"no, one control per file with
    the restructuring, not one controls.rs\"; \"are we using component
    structs in their own file? we should move in that direction without
    rewriting everything at once if we aren't\""
  - "The problem it answers (2026-10-23 15:30 EDT): the agent view opened
    from the focus row while herdr's focus was on another pane, showed
    \"(no session to show)\", and stayed there when the focus came back,
    because a view opens on whatever herdr has focused and does not follow
    it"
  - "handlr at `8b7b77b`:
    - `apps/companion/src/ui.rs` (961 lines: the main screen as one
      libcontrols `Router` with the title, two status lines, the
      conversation, its button row and the pairing field; `Screen`,
      `View`, `Hit`, `Buttons`, `Card`, `focus_words`);
    - `apps/companion/src/lib.rs` (`App`, `open_agent` from the focus row,
      `agent_hit`);
    - `apps/companion/src/agent.rs` and `src/agent/{control_row,
      dictation_card, square_button}.rs` (the agent view; the bottom row
      that controls the audio);
    - `crates/handlr-companion/src/client.rs` (`State.agents` from
      `agents.list`, refreshed after `hello`, on a notice, and when
      dictation starts; `Event::Focus`; notices of kind `agent_status`);
    - `crates/libhandlr_proto/src/common.rs` (`AgentInfo`: `pane_id`,
      `name`, `spoken`, `agent`, `project`, `cwd`, `workspace_id`,
      `tab_id`, `tab_label`, `tab_number`, `status`, `focused`,
      `session`)"
related:
  - docs/design/20261022_04_COMPANION_APP_PLAN.md (the companion app; its main screen is replaced by this one)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3, the agent view, which the overview opens)
  - docs/design/20261023_02_BULK_INPUT_PLAN.md (dictated messages to the focused agent, which stay as they are)
  - docs/design/20261023_03_SESSION_SUBSCRIPTIONS_PLAN.md (open question 2, how other panes are reached, which this answers; the latest line per row comes with it)
---

# The overview (home) screen, a list of the agents

## 1. Goal

The phone shows every agent pane herdr has, and the user picks the one to
look at. Today the only way into the agent view is the "focus: …" line,
which opens on whatever herdr has focused at that moment and then stays
there. Checking on another project means moving herdr's focus first, and
coming back to the terminal's pane does not bring the view with it.

After this plan, the home screen is an overview: a list of the agents,
the ones that need the user first. Tapping one opens the agent view on
that pane; Back returns to the list.

## 2. Decisions

1. **The home screen is the overview.** It replaces the main screen's
   conversation area and its focus line. The title, the two status lines
   (the link and the sound), the confirmation card and the dictation card
   stay where they are, and the pairing field stays for an unpaired phone,
   which has no list to show.
2. **One row per agent pane.** Panes whose `agent` is `shell` are left
   out. A row shows:
   * the shown name (`AgentInfo::name`, such as "Claude in handlr, tab
     17-hr"), in Roboto at 18 pt;
   * the status word under it, coloured: `working` blue, `blocked` red,
     `done` green, `idle` and anything else dim;
   * a focus marker, a 4 pt accent bar at the row's left edge, on the pane
     herdr has focused;
   * a badge at the right when the agent needs the user: **Approval** (a
     handlr confirmation pending for that pane, from the client's
     `confirms`), **Asking** (its pending question), or **Blocked** (status
     `blocked`). The highest one shows, coloured as the agent view's cards
     (approval amber, question blue, broke red).
3. **Order: needs the user, then working, then the rest**, and by
   `(workspace_id, tab_number)` within each group, so rows keep their
   places while statuses change. herdr's focus does not move a row.
4. **Tapping a row opens the agent view on that pane** (`Input::OpenAgent`
   with its `pane_id`). The view stays on that pane until Back, as now;
   nothing follows herdr's focus. This answers open question 2 of plan
   20261023_03 and keeps its Follow toggle off.
5. **The bottom row is the agent view's** (plan 20261022_03, L3, as
   amended today): play at the left and record at the right, record
   becoming send with a cancel in the middle while a message is taken. On
   the overview, play reads replies from the agent view's pane only, so it
   is shown dim and does nothing until a view has been opened; record
   takes a message for herdr's focused agent, as the main screen's Take a
   message does now (plan 20261023_02).
6. **The list stays fresh without polling the daemon more.** The client
   already asks `agents.list` after `hello`, on a notice and when dictation
   starts. It also asks on `focus.changed`, and every 15 s while the
   overview shows and nothing else asked, at most once per 5 s. A list
   answer gives `Event::Agents(Vec<AgentInfo>)`.
7. **"Asking" needs the daemon.** `AgentInfo` gains `asking: bool`, true
   when the pane's session has a pending question (decision 8 of plan
   20261022_03, found by `src/sessions/pending.rs`). The daemon fills it
   only for panes whose session is found on its machine; it is false
   otherwise. It is `#[serde(default)]` on the wire. Adding a field to the
   public struct is not strictly additive under Rule 5.4, so it goes in
   libhandlr_proto 0.2.0, together with plan 20261023_03's step 2 if that
   is approved first, or as 0.2.0 on its own if not.
8. **One control per file, by where it is used.** Controls the app
   builds for itself are component structs, one per file:
   * a control used by one screen lives in that screen's module:
     `src/overview/` for the overview, `src/agent/` for the agent view;
   * a control used by more than one screen lives in a common module,
     `src/controls/`.

   They are written as candidates for libcontrols (no app types in their
   interfaces beyond what they draw, inputs as plain data), but they stay
   in the app; moving any of them to libcontrols is its own decision.
   Existing code moves into this layout as it is changed, not all at
   once.
9. **The common controls, from this plan:** the bottom row and its
   buttons are used by both screens, so `src/agent/control_row.rs` and
   `src/agent/square_button.rs` move to `src/controls/control_row.rs` and
   `src/controls/square_button.rs`. `dictation_card.rs` stays in
   `src/agent/` until the overview's dictation card is moved onto it (it
   is the main screen's `Card` in `ui.rs` today).

## 3. Layout

On the phone (1080 × 2340 px at scale 2.0, as the tests' `PHONE`):

| Part | Where |
|---|---|
| Title and status lines | the top, as now |
| The confirmation card | under them, while one waits, as now |
| The list | from under the status lines to above the dictation card or the bottom row; it scrolls when longer than that |
| A row | 72 pt high, 12 pt side margin, a 1 px divider (`BAND`) between rows |
| The dictation card | above the bottom row while a message is taken, as now |
| The bottom row | square, 52 pt, play left and record right |

With no agents (herdr not running, or only shells), the list shows one dim
line: "No agents in herdr." Before the first `agents.list` answer it shows
"Looking for agents…".

## 4. Modules and files

```
apps/companion/src/
  overview.rs              mod overview: the Overview screen (layout, hits, draw)
  overview/agent_row.rs    AgentRow: one row (name, status, focus bar, badge)
  overview/badge.rs        Badge: the needs-the-user label and its colour
  overview/order.rs        the ordering of decision 3, a pure function
  controls.rs              mod controls (common)
  controls/control_row.rs  ControlRow (moved from src/agent/)
  controls/square_button.rs SquareButton (moved from src/agent/)
```

```rust
// overview/agent_row.rs
pub struct AgentRow<'a> {
    pub agent: &'a AgentInfo,
    pub focused: bool,
    pub badge: Option<Badge>,
}
impl AgentRow<'_> {
    pub const HEIGHT: f32 = 72.0;
    pub fn lay_out(&self, x: f32, y: f32, w: f32, s: f32) -> Flow;
}

// overview/badge.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Badge { Approval, Asking, Blocked }
impl Badge {
    pub fn of(agent: &AgentInfo, confirms_for_pane: bool) -> Option<Badge>;
    pub fn label(self) -> &'static str;   // "Approval", "Asking", "Blocked"
    pub fn colour(self) -> [f32; 4];
}

// overview/order.rs
pub fn order(agents: &[AgentInfo], badges: &[Option<Badge>]) -> Vec<usize>;

// overview.rs
pub struct Overview { /* rows, scroll offset, layout, hits, dirty */ }
pub enum OverviewHit { Agent(String /* pane_id */), Bottom(AgentHit) }
impl Overview {
    pub fn set_agents(&mut self, agents: Vec<AgentInfo>, pending: &[String]);
    pub fn set_layout(&mut self, layout: Layout);
    pub fn touch(&mut self, …) -> Option<OverviewHit>;   // tap and drag, as AgentScreen
    pub fn draw(&mut self, list: &mut DrawList, ctx: &DrawContext, term: Option<&Terminal>) -> bool;
}
```

`handlr_companion` gains `Event::Agents(Vec<common::AgentInfo>)`, given
whenever the list changes, and `State::pending_panes() -> Vec<String>`,
the panes with a confirmation waiting.

## 5. Steps

1. **libhandlr_proto and the daemon: `asking`.** `AgentInfo.asking`, set
   by the daemon from the pending question of the pane's session; libhandlr_proto
   0.2.0 (decision 7), with its VERSIONS.md entry and tag.
   **Done when:** a daemon test with the herdr mock and a session fixture
   ending in a question lists that pane with `asking: true`, and one
   ending in a reply with `false`.
2. **The client: the list.** `Event::Agents`, `pending_panes`, the
   refresh rules of decision 6.
   **Done when:** client tests show the event on each list answer that
   changes, a refresh on `focus.changed`, at most one request per 5 s, and
   none while the agent view is open.
3. **The common controls.** Move `ControlRow` and `SquareButton` to
   `src/controls/`, unchanged.
   **Done when:** the app's tests pass unchanged.
4. **The overview's controls.** `Badge`, `order`, `AgentRow`, each with
   tests: the badge's priority, the order with ties and changing
   statuses, a row's parts in place at the phone's width, and shells left
   out.
   **Done when:** those tests pass.
5. **The overview screen.** `Overview`, replacing the main screen's
   conversation and focus line in `ui.rs`; taps open the agent view;
   scrolling; the empty and loading lines; the bottom row of decision 5.
   **Done when:** tests show a tap on a row giving that pane, the list
   scrolling past the screen, and the bottom row's hits; and on the S23,
   the list shows the herdr agents, the focused one marked, and tapping
   one, then Back, then another, opens each one's view.

Each step is one crate or subproject, one commit (Rule 3.1). Step 2 needs
step 1 for `asking` but can land first without the badge; 3 and 4 need
nothing; 5 needs 2, 3 and 4.

## 6. Open questions for the owner

1. **The conversation with Gemini.** The main screen shows the voice
   conversation today. With the list in its place, does the conversation
   move to its own screen, show as the last few lines above the bottom
   row, or go?
2. **The focus line.** This plan drops it, since the focus bar on a row
   replaces it. Keep a line naming herdr's focus above the list as well?
3. **The latest line per row.** With plan 20261023_03, each row could
   show its agent's newest line, with one subscription per row (up to
   8). Wanted on the overview, or only in the agent view?
