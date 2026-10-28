---
title: The session drawer, with remote cards and switching the active remote
date: 2026-10-28
status: Draft
covers:
  - "A left-hand drawer in the companion app, built on libcontrols 0.10.1's
    `DrawerLayout` and `Drawer`: up to 8 remote cards (4 across) with a **+**
    in the last slot when fewer than 8 are paired, then the active remote's
    sessions, one row each"
  - "A session row: a vertical bar in the session's colour at its left, a
    one-line summary, and an activity indicator (a Braille spinner while
    the session, or one of its subagents, is processing), judged from the
    time of the session's last event and not only herdr's status"
  - "A tap on a session opens its agent view directly, never passing
    through the overview; a tap on **+** opens the pairing window's add
    mode"
  - "Switching the active remote from its card (the switching that plan
    20261028_01 left for later): the client input, the net thread's
    switch, what the screens show meanwhile, an open agent view or prompt,
    failures, and the per-remote caches"
  - "The remote's nickname on its card (the settings plan's 17:56 and 17:57
    EDT amendments), its default and its disambiguation"
  - "The data the drawer needs from the daemon (additive `AgentInfo`
    fields) and from flankr-companion (new inputs, a breaking minor bump)"
  - "Moving the app to libcontrols v0.10.1, and its label change"
  - "Not covered: listing other remotes' sessions (a later option, 3.6);
    a general agent across remotes; any gesture inside libcontrols"
sources:
  - "Owner (2026-10-28, through the lead, verbatim): \"plan the new drawer
    after this work finishes, it will have a one-line version of the
    session summary, the color indicator as a vertical bar to the left, a
    set of up to 8 cards (4 across) for the remotes above the list. the
    active state of each session will be shown based on last updated
    event, not just herdr status. it will use the Braille-spinner for
    actively processing sessions (including subagents). clicking will open
    the session without returning to the overview screen. this will use
    the new libcontrol's controls Drawer and DrawerLayout. also, it will
    have a + as the last slot if there are fewer than 8 remotes which will
    trigger the pairing modal flow.\""
  - "Owner (2026-10-28, through the lead, verbatim): \"you can write the
    plan now I mean, just wait to implement it until the other work is
    finished.\""
  - "Owner (2026-10-28, through the lead, verbatim): \"this is the
    'switching remotes' feature, that's what it was waiting on\""
  - "Owner (2026-10-28, through the lead, verbatim): \"these remote cards
    are very small, so they will probably have a new 'nickname' which
    defaults to the base hostname and optional port, and should be
    settable in the remote details modal (the fingerprint should be an
    expandable)\"; then \"port if there's a conflict between two with the
    same hostname, not just if non-default\""
  - "libcontrols at 6f42272 (0.10.1, tags v0.10.0 and v0.10.1), read
    2026-10-28: docs/design/20261028_01_DRAWER_PLAN.md (and its amendments
    of 11:21, 11:22, 11:24, 11:26, 11:28, 12:32 and 12:37 EDT),
    docs/design/20261028_04_DRAWER_LAYOUT_PLAN.md,
    docs/design/20261028_02_LIBCONTROLS_SPEC.md (8.5, and the amendments of
    12:37 and 12:50 EDT: 6.14), CHANGES.md 2026-10-28 12:50 EDT, VERSIONS.md
    0.10.0 and 0.10.1, src/drawer/{mod,layout,layers,state}.rs, src/ease.rs"
  - "flankr at 0da05ca-bc64dab (main-terminal), read 2026-10-28: the files
    cited in section 2"
related:
  - docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md (In implementation;
    its decision 7, "shaped for switching later", and decisions 8, 17 and
    20; this plan is the switching it left for later)
  - docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md (In implementation;
    the pairing window the **+** opens)
  - docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (its living section's
    2026-10-28 entries: 17:46 the details modal, 17:48 standard rows, 17:56
    the nickname and the collapsible fingerprint, 17:57 the port on a
    conflict)
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (the cards, `Turn`, the
    list cadence of decision 6)
  - docs/design/20261023_07_SUBAGENTS_PLAN.md (Draft; Claude Code's
    subagent files, which 3.4 reads for activity only)
  - docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md and plan 20261022_03's
    L3 amendment of 2026-10-24 12:06 EDT (the Finished cards' summaries)
  - docs/design/20261023_09_HANDLR_DATA_PLAN.md (the per-remote session
    store, in each remote's `state.redb`)
---

# The session drawer, with remote cards and switching the active remote

## 1. Goal

Today the phone shows one remote's sessions on the overview, and reaching
another session from an agent view means going back to the overview first.
Plan 20261028_01 lets the phone pair with up to 8 remotes but always uses
one of them, chosen at pairing.

After this plan:

- a drawer slides in from the left over whatever screen is showing (the
  overview or an agent view), opened by a menu button on the screen's
  header and closed by a tap on the scrim, Back, or choosing something in
  it;
- at its top, the paired remotes as small cards, 4 across and up to 2 rows,
  each showing its nickname and whether it is in use and online; when fewer
  than 8 are paired, the slot after the last card is a **+** that opens the
  pairing window;
- a tap on a card that is not in use **switches the active remote**: the
  phone leaves the current daemon and connects to that one, and the list
  below fills with its sessions;
- under the cards, the active remote's sessions, one row each: a vertical
  bar in the session's colour, one line of summary, and at the right a
  Braille spinner while the session or one of its subagents is working,
  judged from when its session last changed;
- a tap on a session closes the drawer and opens that session's agent view
  at once, from the overview or from another agent view, without the
  overview being shown between them;
- while the drawer is closed it costs nothing on the GPU (libcontrols'
  layered drawer, section 3.9).

## 2. Findings (read 2026-10-28)

### 2.1 libcontrols 0.10.1's drawer

1. **`DrawerLayout`** (src/drawer/layout.rs:20) holds two `Router`s: the
   main view, in the layout's coordinates, and the drawer's content, in
   the sheet's own coordinates (layout plan 2, 3.1). It forwards the
   drawer's calls `open`, `close`, `toggle`, `set_open_now`,
   `set_progress` and `settle` (layout.rs:89-109), routes events with
   `deliver` (layout.rs:116), advances with `advance` (layout.rs:135), and
   draws with `render` (direct), `render_main` and `render_sheet`
   (layout.rs:152-167). `mark_main` (layout.rs:170) bumps the main view's
   revision for changes the layout cannot see.
2. **The drawer implements no gestures** (drawer plan, 11:22 EDT
   amendment): no edge swipe, drag, fling, scrim tap or Back. The app
   drives it by calls; `set_progress(t)` and `settle(open, velocity)` exist
   for an app that drags the sheet with its own gesture. `DrawerReason` has
   only `App` (libcontrols CHANGES.md 2026-10-28 12:50 EDT, item 9).
3. **Routing** (layout plan 3.2; `DrawerRoute`, mod.rs:274): a press
   belongs to where its `Down` landed; while the sheet is visible a `Down`
   inside it goes to the content (moved into sheet coordinates), one
   outside it is `DrawerRoute::Scrim` and reaches nothing; keys go to the
   content while the drawer is opening, open or dragging, else to the main
   view. `DrawerRoute::Main(Some(response))` carries the main router's
   answer (CHANGES.md item 10).
4. **One edge per drawer, Left by default; scrim only** (drawer plan, 11:21
   EDT amendment): the main view is darkened by the scrim, not moved or
   scaled. `DrawerEdge` derives `Default` as `Left` (mod.rs:33).
5. **Theme defaults, option overrides** (drawer plan, 11:28 EDT amendment;
   spec 8.5): `ControlTheme::drawer: DrawerTheme` holds every drawer's
   defaults (mod.rs:119-165: size 0.85 of the width up to 360, scrim
   black at 32%, `sheet_color: None` meaning the theme's `bg_color`,
   corner radius 16, shadow (16, 0.25), open 0.300 s on
   `EMPHASIZED_DECELERATE`, close 0.250 s on `EMPHASIZED_ACCELERATE`,
   `MainUpdates::Live`, `keep_warm` 0). `DrawerOptions` (mod.rs:170) has
   the same values as `Option`s, `None` taking the theme's;
   `with_theme` (mod.rs:214) resolves them. The default sheet colour is
   the theme's `bg_color` **at 85% alpha** (CHANGES.md item 13).
6. **No GPU cost while closed** (drawer plan section 4; `LayerPlan`,
   layers.rs:15). `DrawerLayers::prepare` answers `direct` while `t == 0`
   (CHANGES.md item 8): the app draws its main view straight to the
   surface as today, and no texture, pass or copy exists. While `t > 0`
   the app draws the main view into `main_view()` when `draw_main`, the
   sheet with `render_sheet` when `draw_sheet`, then one `composite`
   pass. The textures are freed on the frame the drawer reaches Closed
   (`keep_warm` 0).
7. **0.10.0's breaking change** (libcontrols VERSIONS.md 0.10.0): `Button`,
   `Radio` and `Options` draw a `"XX - Name"` label as written unless
   `with_abbreviate(true)`. libcontrols 0.10.1 pins the same libmsdf
   (`4de319c`) as the app (apps/companion/Cargo.toml:37 and :42), and
   `wgpu` 30.0.1 as libmorph and libmsdf do.

### 2.2 The app's screens and how a session opens

1. **The screens draw and route themselves; none is a libcontrols router
   at the top.** `App::render` (apps/companion/src/lib.rs:2980) picks the
   shown screen (`Shown`, lib.rs:141-147: Pairing, Overview, Agent,
   Settings; lib.rs:2996-3002) and calls its own `draw`; the overview
   first runs its own wgpu panel pass into `frame.view`, then the SDF list
   with a load (lib.rs:3044-3075); the pairing window runs its camera pass
   (lib.rs:3076 on). Touches go through `App::touch` (lib.rs:1916) and
   keys through `App::key` (lib.rs:2135), which pass them to the shown
   screen.
2. **`open_agent(pane_id)`** (lib.rs:929) builds an `AgentScreen` and
   sends `Input::OpenAgent(pane_id)`; `close_agent` (lib.rs:1186) closes
   its terminal and drops the view. Opening is reached from an overview
   card (lib.rs:2024) and from another session's card (lib.rs:2238).
   Nothing today opens one agent view from another.
3. **The session colour** is `theme().session(cross::colour_of(agent))`
   (overview/agent_card.rs:163-165): `colour_of` (crates/flankr-companion/
   src/cross.rs:79) is the FNV-1a hash of the session id, else the pane
   id, mod 16 (cross.rs:60-67), the same on every phone and theme. The
   overview card draws it as a 4 pt band at the card's top
   (agent_card.rs:156-159); herdr's focus is already a bar at the card's
   **left** edge (agent_card.rs:590-626).
4. **A card's text** comes from `Turn` (libflankr_proto common.rs:196):
   `Working { line }` (the newest line), `Stopped { line }` (the first line
   of the last reply), `Asking { question }`; `AgentCard::state` and `body`
   (agent_card.rs:224-253) pick the words. A shell (`agent == "shell"`,
   cross.rs:84-89) has its folder and no turn.
5. **No Braille spinner exists in the app.** The only Braille is in the
   terminal view, drawn from the symbol page of the terminal face
   (apps/companion/src/agent.rs:6223), which therefore has the glyphs.
6. **Buttons.** The app uses no `Radio`, `Options` or `SelectableLabel`.
   It makes `Button`s in four places, all with plain labels and none with
   the `"XX - Name"` form: the pairing screen (ui.rs:466), the agent
   view's buttons (agent.rs:5164), the confirm modal (settings/confirm.rs:66)
   and the info modal (settings/info.rs:120). So 0.10.0's change draws
   every label as it draws today.

### 2.3 How "active" is known today

1. **`AgentInfo`** (libflankr_proto common.rs:144-184) carries herdr's
   `status` (`working`, `idle`, `blocked`, `done`, `unknown`), the
   session's `turn` and `session` (`AgentSessionInfo`, common.rs:223: id,
   match, pending question). It carries **no time** and **no subagent
   activity**.
2. **The daemon's turn** (src/sessions/turn.rs:27) is Stopped when the last
   entry is the agent's reply and the file has been quiet for `TURN_QUIET`,
   5 s (src/sessions/pending.rs:39), or herdr says idle, blocked or done;
   Working otherwise. herdr reports a Claude Code pane `working` while one
   of its background agents runs after its turn ended (pending.rs:12-15).
   The quiet test uses `Located::modified()` (src/sessions/mod.rs:172-190),
   the session file's modification time; the daemon has it, the client
   does not.
3. **Subagents** are read nowhere for activity. Claude Code keeps them as
   `<session dir>/subagents/agent-<id>.jsonl` with
   `agent-<id>.meta.json` holding `finished` (subagents plan, sources);
   the Claude reader only notes their hand-backs (src/sessions/claude.rs:8,
   :373-390).
4. **The list's cadence.** The client asks `agents.list` every
   `LIST_INTERVAL`, 15 s, or `LIST_MIN_INTERVAL`, 5 s, after one is wanted
   (crates/flankr-companion/src/client.rs:405-410), and only while no agent
   view is open unless another session is watched
   (`list_if_due`, client.rs:4976-5000). `Event::Agents` is given when the
   list changes (client.rs:4090-4096).
5. **Summaries.** A Finished card's `spoken` summary is made by the
   daemon's summariser for sessions it tails (src/sessions/hub/summaries.rs:
   1-12, 84-94) and reaches the client only inside an open agent view
   (the bookends, client.rs:2502-2550). `agents.list` has no summary.

### 2.4 Remotes and switching

1. **`RemoteInfo`** (crates/flankr-companion/src/keys.rs:194-207): `id`,
   `name`, `host`, `fingerprint`, `paired_at`, `active`, `online`;
   `Event::Remotes(Vec<RemoteInfo>)` (client.rs, the `Remotes` variant)
   comes at start and on every change. `ServerRecord` (keys.rs:115-133)
   has `name` (the host's first DNS label, not unique), `addresses`,
   `key_name` and more. The name is made by `scan::server_name`
   (crates/flankr-companion/src/scan.rs:58-69): the first DNS label, or an
   IP address whole.
2. **Switching was planned for and partly built.** The list stores
   `active` (multiple-remotes plan, decision 7: "A later plan that switches
   remotes only writes `active` and reconnects"). The net thread already
   has the machinery: a `switch` flag that leaves the current connection
   and reconnects at once with the backoff reset (net.rs:220-221,
   365-431), `open_state` that closes one remote's `state.redb` and opens
   the active one's (net.rs:453-468), and `finish_remove` that, for the
   active remote, picks the next, sets `switch`, and clears the in-memory
   closed views with `Input::ClearCache` (net.rs:570-620). Decision 20 of
   that plan: the closed views are the active remote's and are cleared
   whenever the active remote changes.
3. **The enums are exhaustive.** `Input` (client.rs:33) and `Event`
   (client.rs:484) have no `#[non_exhaustive]`; flankr-companion is 0.3.0
   (VERSIONS.md, the `flankr-companion 0.3.0` entry), libflankr_proto 0.5.0.
4. **The pairing window's add mode** is opened by `open_add` (lib.rs:1329)
   from Settings → Remotes **+**; while `pairing.adding`, the pairing
   window is shown over everything (lib.rs:2997); Cancel, its "‹" and Back
   end it (`cancel_pairing`, lib.rs:1341; lib.rs:2137-2140), and the screen
   under it shows again. A new remote paired beside the active one does
   not become active (multiple-remotes plan, decision 8).
5. **The nickname** (settings plan, 17:56 and 17:57 EDT amendments): an
   additive `nickname: Option<String>` on the client's record and on
   `RemoteInfo`, set in the remote details modal; its default is the base
   hostname, with `:port` only when two or more listed remotes would
   otherwise show the same nickname.

## 3. Decisions

### 3.1 Layout of the drawer

1. **Edge and size.** A Left drawer (`DrawerEdge::Left`, the default), its
   size from the theme: 0.85 of the screen's width up to 360 pt. On the
   S23 Ultra (about 384 pt wide) the sheet is about 326 pt.
2. **Padding.** 12 pt at the sheet's left, right and top (the top below
   the status bar's safe-area inset, as the screens place their headers),
   and 12 pt above the bottom inset.
3. **Remote cards first.** A grid of 4 columns with 8 pt gaps: on a 326 pt
   sheet each card is (326 − 24 − 24) / 4 ≈ 69 pt wide and 52 pt high.
   Cards are in pairing order (as `Event::Remotes` gives them), filling
   the first row and then the second. **+** takes the slot after the last
   card when fewer than 8 are paired, so the grid is one row for 0 to 3
   remotes and two rows for 4 to 8. With 8 there is no **+** (Settings'
   dimmed **+** and its ⓘ say why).
4. **Then the session list,** 12 pt under the grid, filling the rest of
   the sheet in a libcontrols `ScrollPane` (vertical), so a long list
   scrolls inside the sheet while the cards stay.
5. **No title or header** in the sheet: the cards are its top.

### 3.2 A remote card

6. **What it shows**, centred in the card:
   - line 1, 13 pt, the theme's foreground: the remote's **shown nickname**
     (3.3), cut with "…" to the card's inner width (8 pt in from each side);
   - line 2, 11 pt, dim: *in use* for the active remote while online,
     *connecting…* for the active remote while not online, and nothing
     for the others (a session count is not shown: only the active
     remote's sessions are known, and line 1 needs the room);
   - the active card has a 2 pt outline in the accent colour; the others
     the card colour (`theme().card`) with no outline.
7. **What a tap does.** On a card that is not active: switch to it
   (3.7). On the active card: nothing (open question 4).
8. **The + slot**: the same size and colour as a card, with a **+** at
   22 pt in the accent colour and no text. A tap closes the drawer at once
   (`set_open_now(false)`) and calls `open_add` (lib.rs:1329), the same
   pairing window Settings' **+** opens (the pairing QR scan plan's flow,
   with the pairing feedback work landed before this plan, section 4).
   When the add mode ends (paired, cancelled, "‹" or Back), the screen
   under it shows again, as it does today when Settings is not open
   (2.4 item 4). A new remote shows as a new card the next time the
   drawer opens; it is not made active (multiple-remotes plan, decision 8).
9. **No remotes** (the app's first run shows the pairing window, so the
   drawer is reachable with none only after the last one is removed, when
   the app also shows the pairing window): the drawer cannot be opened
   while the pairing window shows (3.10), so the grid always has at least
   one card.

### 3.3 The nickname on a card

10. **The shown nickname** is the remote's `nickname` when set; otherwise
    its default, worked out over the whole list:
    - the **base hostname**: the first DNS label of its first address's
      host (`m4` from `m4.tail1a2b3c.ts.net:7347`), as `scan::server_name`
      makes it;
    - an **IP address is kept whole** (`192.168.1.20`, or an IPv6 address
      without brackets): it is not shortened when stored or shown in
      Settings. On a card it is cut from the **left** with "…" so its
      distinguishing end shows (`…1.20`), unlike a name, which is cut at
      the right;
    - `:port` is added **only when two or more listed remotes would
      otherwise show the same nickname**, whatever the port; then each of
      those shows `host:port` (the settings plan's 17:57 EDT amendment);
    - if they would still be equal (the same host and port: two daemons
      reached through different tailnets), each adds a space and the
      first group of its fingerprint (`m4 3f9a`). This last rule is this
      plan's, for the owner to confirm (open question 5).
    A set nickname is shown as set, even when it equals another's.
11. **Where it is made.** `Remotes::info` (keys.rs:212) already builds
    every row from the whole list, so it works out the shown nickname
    there, into a new `RemoteInfo::shown: String` beside the stored
    `RemoteInfo::nickname: Option<String>` (3.13). Settings' Remotes rows
    and the details modal use the same `shown`.
12. **Setting it** is the settings plan's details modal (its 17:56 EDT
    amendment, item 1): a field there sends `Input::SetNickname { id,
    nickname }` (3.13). The value is trimmed; empty is `None` (the
    default); at most 24 characters; control characters are refused.
    This plan adds the input; the modal and its field are the queued
    settings work (section 4).

### 3.4 Which sessions, and in what order

13. **The active remote's sessions only**, every pane `agents.list` gives,
    shells included, in `agents.list` order (the overview's order), so a
    row stays where it was between updates. Needs-input rows are not
    moved up; their indicator says so (3.5).
14. **All remotes, grouped**, is a later option, not in this plan: it
    would need a connection (or a probe) per remote, and the phone holds
    one connection by design (multiple-remotes plan, decision 17).

### 3.5 A session row

15. **Size.** 44 pt high (one line, a full touch target), the sheet's full
    inner width.
16. **The colour bar** at the row's left: 4 pt wide, the row's height less
    8 pt (4 pt top and bottom), corner radius 2 pt, in the session's
    colour (`theme().session(cross::colour_of(agent))`, 2.2 item 3), the
    same colour as the overview card's band and the other-session cards.
    It is not herdr's focus bar; the drawer does not show herdr's focus.
17. **The line**, starting 12 pt right of the bar and ending 8 pt left of
    the indicator column, 14 pt, one line, cut with "…":
    - first the **tab** in the foreground colour: the tab label
      (`17-fl`), else `tab <n>`, else the agent's shown name; then
      `" · "` and the **summary** in the dim colour;
    - the **summary**, by the first that applies:
      1. approval pending for the pane: *Approval needed* (amber);
      2. `Turn::Asking { question }`: the question's first line
         (periwinkle, as the overview's *Asking*);
      3. the latest Finished card's spoken summary's first line, when the
         daemon has one (`AgentInfo::summary`, 3.12);
      4. the turn's `line` (`Stopped`: the reply's first line; `Working`:
         the newest line);
      5. a shell: its folder's last component;
      6. otherwise herdr's status word.
18. **The indicator column** at the right, 20 pt wide, the glyph at 14 pt
    centred, from the row's **activity** (3.6):
    - **Processing**: a Braille spinner, the ten frames
      `⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏` (U+280B, 2819, 2839, 2838, 283C, 2834, 2826,
      2827, 2807, 280F), 80 ms each, in the session's colour, drawn from
      the terminal face's symbol page (2.2 item 5). Every spinning row
      shows the same frame (one clock), so the list does not shimmer;
    - **Needs input**: a filled 8 pt dot, amber for an approval,
      periwinkle for a question (`NeedsInput::colour`,
      agent_card.rs:50-55);
    - **Broken** (herdr `broke`, not a shell): a coral red dot
      (`theme().red`);
    - **Idle**: the time since the last event, dim, 11 pt: `now`, `1m` …
      `59m`, `1h` … `23h`, `1d` …; nothing when no time is known.
19. **The row open in the agent view under the drawer** has a background
    of the card colour, so the owner sees where they are.

### 3.6 "Active": what processing means

20. **The inputs**, per session, all on the phone's clock (3.12 says how
    they arrive):
    - `last_event`: when the session file (or any of its subagents'
      files) was last written, from the daemon's modification times;
    - `subagents_running`: subagents not marked finished whose files were
      written within `SUBAGENT_LIVE`;
    - `turn`, `status` and needs-input, as today.
21. **Thresholds** (constants in flankr-companion's new `activity`
    module):
    - `RECENT` = 10 s: a write this recent means something is happening;
    - `TOOL_WAIT` = 10 min: the longest a `Working` turn whose herdr status
      is `working` counts as processing with no write (a long build or
      test run writes nothing until its result);
    - `SUBAGENT_LIVE` = 60 s: a subagent counts as running while its file
      was written this recently and its meta does not say finished.
22. **The rule**, first match wins:
    1. a shell: **Idle** (its terminal is not a session);
    2. needs input (approval or question): **Needs input**;
    3. herdr `broke`: **Broken**;
    4. `subagents_running > 0`: **Processing** (this is the case herdr
       reports `working` with the main turn Stopped, 2.3 item 2);
    5. `now − last_event ≤ RECENT`: **Processing**;
    6. `turn` is `Working`, herdr's status is `working`, and
       `now − last_event ≤ TOOL_WAIT`: **Processing**;
    7. otherwise **Idle**.
    So herdr's `working` alone never spins a row (a stale pane, a
    background agent whose file is quiet for more than a minute), and a
    session whose file is being written spins whatever herdr says (an
    agent herdr has not noticed yet).
23. **Without times** (an older daemon, a remote herdr over SSH where
    session files are not on the daemon's machine, 2.3 item 1): rules 5
    and 6 fall back to `turn` `Working` and herdr `working`, as the
    overview judges today; the idle age is not shown.
24. **Re-judged on the phone** every spinner frame while the drawer is
    visible, so a row stops spinning `RECENT` after its last write even
    between lists.

### 3.7 Switching the active remote

25. **The input.** A tap on a card that is not active sends
    `Input::SwitchRemote(RemoteId)` (3.13). The app does its own part
    first, in the same event (item 28), so no frame shows the old remote's
    view with the new remote's list.
26. **The net thread** (net.rs), in `net_input`:
    - ignore it, logging why, when `id` is already active, is not in the
      list, or is being removed (`removing`);
    - set `remotes.active = Some(id)` and write the list
      (`write_remotes`) first, so a crash after this connects to the new
      remote at the next start;
    - `open_state()` (net.rs:453): the old remote's `state.redb` is
      closed and the new one's opened (multiple-remotes plan, decision 17);
    - `state.set_session_id(record.session_id)` for the new remote, as
      `finish_remove` resets it (net.rs:580);
    - the state forgets the old remote's per-connection data with a new
      `State::leave_remote()` (item 29), giving its events;
    - set `switch = true`: the current session ends (`End::Closed`), the
      run loop skips the backoff and connects to the active remote with
      its own key (`connect`, net.rs:824: `active_record()` and
      `static_key(&key_name)`), as removal already does;
    - `show_remotes()`: `Event::Remotes` with the new card active and not
      yet online (*connecting…*).
27. **No new event** is needed: `Event::Remotes` says which is active,
    `Event::Disconnected` and `Event::Connected` say how the connection
    goes, and `Event::Agents` fills the list.
28. **The app, on the tap, before sending the input:**
    - an **open agent view** is closed (`close_agent`, which closes its
      terminal): its pane belongs to the old daemon. The screen under the
      drawer becomes the overview;
    - a **dictation or typed message in progress** for a pane (the prompt
      card in its message mode) is dropped with its card: its target is
      on the old daemon. A **general-agent (Gemini) turn** in progress is
      ended as an offline drop ends it today (`State::disconnected`,
      client.rs:1900); its conversation lines stay on the overview's
      Gemini card, since the general agent is app-wide (multiple-remotes
      plan, the 13:03 EDT amendment, item 2);
    - a **pending confirmation** card is dropped: it was the old daemon's
      to answer, and its nonce means nothing to the new one;
    - the drawer **stays open**, so the owner watches the new remote's
      sessions arrive and taps one.
29. **`State::leave_remote()`** clears what belonged to the old daemon and
    gives the events that say so: the agents (`Event::Agents(vec![])`),
    the pending panes (`Event::PendingPanes(vec![])`), herdr's focus
    (`Event::Focus(None)`), the other sessions' cards
    (`Event::SessionCards(vec![])`), open confirmations, the open view,
    the terminals, `listed`/`listed_at`, and the in-memory closed views
    (as `Input::ClearCache` does, client.rs:2083; multiple-remotes plan,
    decision 20). App-wide settings and the Gemini conversation stay.
30. **Meanwhile, in the drawer:** the new card is outlined and reads
    *connecting…*; the session list shows one dim row, *Connecting to
    &lt;nickname&gt;…*. The overview under it shows its usual offline
    status line.
31. **If the new remote cannot be reached,** the phone **stays on it** and
    retries with the usual backoff; the list row reads *Can't reach
    &lt;nickname&gt; · retrying in &lt;n&gt; s* from `Event::Disconnected
    { retry_in_ms }`. There is no automatic revert: the owner chose that
    remote, it may be waking, and the old one is one tap away on its own
    card. If the new remote answers that it does not know this phone (the
    "pair again" path, net.rs:358-366 in the multiple-remotes plan's
    decision 23), the row says so and the remote's details in Settings
    offer Remove, as today.
32. **Switching back** is the same tap on the old card. Its `state.redb`
    (and, once the data plan is built, its session store in that file)
    is opened again, so persisted sessions come back from disk; the
    in-memory closed views do not.
33. **During a pairing** beside the connection (the probe), switching is
    allowed: the probe is its own task and does not use the main
    connection (multiple-remotes plan, decision 10). The drawer cannot be
    opened while the pairing window is showing anyway (3.10).

### 3.8 Opening a session from the drawer

34. **A tap on a row** closes the drawer with its animation (`close()`)
    and, in the same event:
    - if no agent view is open: `open_agent(pane_id)`;
    - if an agent view is open on another pane: `close_agent()`, then
      `open_agent(pane_id)`; both happen before the next frame, so the
      overview is never drawn between them;
    - if the agent view open is the row's own: nothing more.
    During the close animation the new agent view is the main layer under
    the sheet.
35. **The list is asked for at once** after the switch of view
    (`Input::OpenAgent` already re-subscribes; the drawer's list cadence
    follows 3.12 item 46).

### 3.9 The drawer in the app: `DrawerLayout`, drawing and cost

36. **One `DrawerLayout`** owned by `App`, made at start with the
    surface's rect, `DrawerEdge::Left` and `DrawerOptions::new()` (no
    overrides), its `DrawerState` kept for the app's life;
    `set_rect` on every resize. Its **content router** holds the drawer's
    controls (3.11). Its **main router** stays empty: the app's screens
    draw and route themselves (2.2 item 1), so the app gives the layout
    events only while the drawer is visible (item 38) and calls
    `mark_main()` on every frame the shown screen drew anything (`drew`,
    lib.rs:3026-3034), and on every layout or theme change. Wrapping the
    screens in one router node (the layout plan's suggestion for "a main
    view that draws and routes itself") would move the whole app's input
    and drawing into libcontrols' router for no gain here; it can come
    later if the screens become controls.
37. **Theme defaults, not options** (spec 8.5): the app sets the theme's
    drawer values once where it sets the other libcontrols colours
    (lib.rs:2988-2989): `draw.theme.drawer.sheet_color = Some(theme().bg)`
    with alpha 1.0 (the default 85% sheet would show the screen through
    it), and the scrim, corner radius, shadow and motion left at
    `DrawerTheme::default()`. Every drawer the app makes later takes the
    same values; this one overrides nothing.
38. **Input.**
    - Drawer **not visible** (`!drawer().is_visible()`): touches and keys
      go to the app's screens exactly as today; the layout is not asked.
    - Drawer **visible**: each touch is turned into a libcontrols
      `InputEvent` (as the settings screen does for its router,
      settings/screen.rs:1233) and given to `layout.deliver`. `Drawer(_)`:
      the content took it (its hits are read back from the controls,
      3.11). `Scrim`: on the `Up` of a press that began on the scrim,
      `close()`. `Main(_)`: cannot happen while visible except for a press
      that began before the drawer opened; the app ends it as a cancel.
    - **Back** while the drawer is visible: `close()`, and the key is
      taken. Escape on the desktop the same.
39. **Drawing, by `LayerPlan`** (2.1 item 6), each frame:
    - `direct` (the drawer closed): today's `render`, unchanged, straight
      to `frame.view`: **no texture, no pass, no copy**;
    - otherwise: when `draw_main`, the app draws the shown screen as today
      but into `layers.main_view()` instead of `frame.view` (the
      overview's panel pass takes its target view as an argument,
      lib.rs:3044-3057, so it draws into the texture the same way; the
      shown screen is invalidated first so it draws whole); when
      `draw_sheet`, `layout.render_sheet` into a second `DrawList` and
      `layers.render_sheet`; then `layers.composite` into `frame.view`.
    - The other-session cards (`session_cards.draw`, lib.rs:3037-3041) are
      part of the main layer.
40. **Cost while open.** Two textures: the main view at the surface's size
    (1440 × 3088 × 4 bytes ≈ 17.8 MB on the S23 Ultra at its full
    resolution, 10.1 MB at 1080 × 2340) and the sheet at the sheet's size
    (about 0.85 of the width). A frame where neither side changed is one
    composite pass. The spinner redraws the sheet texture every 80 ms
    (12.5 times a second) while any row spins; the main view redraws
    when it changes (`MainUpdates::Live`), as it would without the drawer
    (an agent view's terminal keeps updating behind the scrim). The
    textures are freed on the frame the drawer closes (`keep_warm` 0).
41. **Frames.** While the drawer is moving, or open with a row spinning,
    `App::update` asks libmorph for the next frame as it does for the
    overview's animations; `layout.advance(dt)` moves the drawer.

### 3.10 How the app opens and closes it

42. **A menu button** (≡, three 2 pt bars, 44 pt touch target) at the
    **left of the header** on the overview and in the agent view, where
    the agent view's "‹" back moves one slot right. A tap is `open()`.
    This is the proposed way in; open question 1 asks whether the owner
    wants it in both places and whether the agent view's "‹" stays.
43. **A left-edge swipe** is **not** in the first version: on Android with
    gesture navigation the left edge is the system Back gesture, and
    libmorph exposes no system-gesture exclusion (libmorph src/input.rs
    has none). If the owner wants one (open question 2), the app would
    recognise it (a press within 20 pt of the left edge, past an 8 pt
    slop to the right), drive the sheet with `set_progress(dx / extent)`
    and end it with `settle(open, velocity)`, as the drawer plan's
    11:22 EDT amendment provides; a drag to the left on the open sheet
    the same way to close.
44. **Closed by**: a tap on the scrim; Back; a tap on a session row (3.8);
    the **+** (at once, 3.2 item 8); the drawer is also closed at once
    (`set_open_now(false)`) when the pairing window or Settings opens,
    and it cannot be opened while either shows (no menu button there).
45. **Lifecycle.** The `DrawerState` is the app's for its life; it stays
    as it was across suspend and resume (the textures are dropped on
    suspend by `DrawerLayers` being rebuilt with the surface, and
    re-created on the next visible frame). On a theme change the content
    is rebuilt and `mark_main()` called. On a resize, `set_rect` and the
    content is laid out again at `drawer().sheet_open().size()`.

### 3.11 The drawer's controls

The app's own controls, one per file, in a new `apps/companion/src/drawer/`
module, as the settings screen's rows are (settings/remote_row.rs:157-200,
`DrawControl` and `HostedControl`):

- `drawer/remote_card.rs`: `RemoteCard` (a remote's `RemoteInfo`, or the
  **+**), drawing 3.2 and giving `DrawerHit::Remote(RemoteId)` or
  `DrawerHit::AddRemote` on a tap;
- `drawer/session_row.rs`: `SessionRow` (an `AgentInfo`, its activity,
  whether approval is pending, whether it is the one open), drawing 3.5
  and giving `DrawerHit::Session(pane_id)`;
- `drawer/spinner.rs`: the frames, and the frame for a time;
- `drawer/content.rs`: `DrawerContent`, which builds the content router
  (the grid, then the `ScrollPane` of rows) from `Event::Remotes`,
  `Event::Agents` and `Event::PendingPanes`, keeps the list's
  `ScrollState` across rebuilds, and bumps `content_mark` when anything it
  draws changes, the spinner's frame included;
- `drawer.rs`: the module, `DrawerHit`, and the glue in `App` (3.9, 3.10).

### 3.12 Data from the daemon (libflankr_proto, additive)

46. **`AgentInfo` gains three optional fields**, appended (additive by the
    owner's ruling on appended struct fields):

    ```rust
    /// Milliseconds since the session file (or any of its subagents'
    /// files) was last written, on the daemon's clock, as this list was
    /// made. Absent without a session on the daemon's machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_ms: Option<u64>,
    /// The subagents of the session that are running: not marked
    /// finished, and written within 60 s. Absent when the agent's reader
    /// does not read subagents (only Claude Code's does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents_running: Option<u32>,
    /// The first line of the latest Finished card's spoken summary, when
    /// the daemon has one; redacted, at most `TURN_TEXT_MAX` characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    ```

    An **age, not a time**, so the phone's clock and the daemon's need
    not agree: the client sets `last_event = received_at − idle_ms` on its
    own clock when the list arrives.
47. **The daemon** (`agent_session`, src/server/conn.rs:696-711) fills
    them: `idle_ms` from `Located::modified()` (src/sessions/mod.rs:181)
    and the newest of the subagents' files; `subagents_running` from a new
    `SessionReader` method with a default of `None`, implemented for
    Claude Code by reading `<session dir>/subagents/*.meta.json`
    (`finished`) and the files' modification times; `summary` from the
    hub's summary store for the session's latest Finished card, only when
    it is already known (no summary is asked for because of a list).
48. **The list's cadence while the drawer shows.** The client lists the
    agents every `DRAWER_LIST_INTERVAL` = 3 s while the app says the
    drawer is visible (`Input::DrawerShown(bool)`, 3.13), **also while an
    agent view is open** (today `list_if_due` lists nothing then unless a
    session is watched, client.rs:4986-4989), and once at once on
    `DrawerShown(true)`. When it is hidden, the cadence is today's.

### 3.13 flankr-companion changes (a breaking minor: 0.4.0)

49. **New inputs** on the exhaustive `Input` enum:
    - `SwitchRemote(RemoteId)` (3.7);
    - `SetNickname { id: RemoteId, nickname: Option<String> }` (3.3 item
      12): validated as there, written to the list, then `Event::Remotes`;
    - `DrawerShown(bool)` (3.12 item 48).
    A variant added to an exhaustive public enum is not strictly additive
    (Rule 5.4): **flankr-companion 0.3.0 → 0.4.0**, its `VERSIONS.md`
    entry marking each **Breaking**. If the owner prefers, `Input` and
    `Event` could become `#[non_exhaustive]` in the same release, so later
    variants are patch bumps (open question 6).
50. **`ServerRecord::nickname: Option<String>`**, appended, stored as an
    optional `nickname = "…"` key in each `[[remote]]` of the `remotes`
    blob (absent = `None`; an older build ignores it).
51. **`RemoteInfo`** gains `nickname: Option<String>` (as stored) and
    `shown: String` (3.3 item 11), appended.
52. **`activity` module:** `Activity { Processing, NeedsInput(NeedsInput),
    Broken, Idle { since: Option<Duration> } }` and `activity(agent,
    approval, last_event, now) -> Activity` with the constants of 3.6;
    the app calls it, so the rule is tested once, in the crate.
53. **`State::leave_remote()`** (3.7 item 29) and the switch in `net.rs`
    (item 26), and the list's received time kept per list so
    `last_event` can be derived.

### 3.14 libcontrols v0.10.1 in the app

54. `apps/companion/Cargo.toml:42` moves from `52e6c07` (0.9.1) to
    `6f42272` (0.10.1, tag `v0.10.1`), with its comment updated. 0.10.0's
    label change needs no code change in the app: it has no `Radio`,
    `Options` or `SelectableLabel`, and its four `Button` sites pass plain
    labels without `"XX - Name"` (2.2 item 6), so none gains
    `with_abbreviate(true)`. The move is checked by the app's tests and by
    looking at the four buttons (pairing, agent view, confirm, info) on
    the S23. No libmsdf or libmorph change: libcontrols 0.10.1 pins the
    same libmsdf and the same wgpu.

### 3.15 Copy

55. The drawer's words go in **`apps/companion/copy/DRAWER_COPY.md`**, in
    the format of `SETTINGS_COPY.md` (`# [Copy] <key>`, `- text:`), beside
    the settings copy once the copy move lands (section 4):
    - `drawer_menu` (the menu button's accessibility label): *Sessions and
      remotes*;
    - `drawer_add_remote` (the **+**'s accessibility label): *Pair another
      remote*;
    - `remote_card_in_use`: *in use*;
    - `remote_card_connecting`: *connecting…*;
    - `drawer_connecting`: *Connecting to {{ [abc] nickname }}…*;
    - `drawer_unreachable`: *Can't reach {{ [abc] nickname }} · retrying in
      {{ [123] seconds }} s*;
    - `drawer_not_paired_here`: *{{ [abc] nickname }} doesn't know this
      phone: pair again or remove it in Settings*;
    - `drawer_no_sessions`: *No sessions on {{ [abc] nickname }}*;
    - `session_approval`: *Approval needed*;
    - `session_idle_now`: *now*; the ages `1m`, `1h`, `1d` are formatted,
      not copy.

## 4. Dependencies and order

1. **Not before** the work in progress lands on `main-terminal`: the
   pairing feedback work in `apps/companion` (another agent's worktree),
   then the queued settings follow-ups: the remote details modal (settings
   plan, 17:46 EDT entry) with its nickname field and collapsible
   fingerprint (17:56 and 17:57 EDT entries), the standard Remotes rows
   (17:48 EDT entry), and the move of the app's copy to
   `apps/companion/copy/`. This plan's steps 1 to 4 touch only crates
   that work does not (libflankr_proto, the daemon, flankr-companion) but
   wait for it too, as the owner asked; steps 5 to 8 need it.
2. libcontrols 0.10.1 is published (tags `v0.10.0`, `v0.10.1`); nothing
   in libcontrols changes.
3. The data plan (20261023_09) is not needed: without it, switching back
   to a remote starts its views from the daemon, as a cold start does.

## 5. Testing

- **flankr-companion (host):** `activity` for each rule of 3.6 at each
  threshold's edges, and without times; the shown nickname: the base
  hostname, an IPv4 and an IPv6 address, two remotes with the same host
  showing `host:port`, the same host and port showing the fingerprint
  group, a set nickname winning; `SetNickname` (trimmed, empty to `None`,
  25 characters refused, a control character refused) written and given
  as `Event::Remotes`; the `remotes` blob with and without `nickname`;
  `SwitchRemote`: ignored for the active, an unknown and a removing
  remote; otherwise the list written with the new `active` before the
  connection ends, the old `state.redb` closed and the new one opened,
  `leave_remote`'s events in order, the next connection to the new
  remote's addresses with its key (with the net thread's test transport,
  as the removal tests do); an unreachable new remote stays active with
  the backoff; `DrawerShown(true)` lists at once and every 3 s with an
  agent view open, and `false` returns to today's cadence.
- **libflankr_proto (host):** the three fields round-trip, are left out
  of the wire when absent, and an older list without them parses.
- **Daemon (host):** `idle_ms` from a session file's modification time;
  a Claude Code session with a running and a finished subagent
  (fixture files with set modification times) gives
  `subagents_running = 1` and `idle_ms` from the newest file; `summary`
  only when the store has the card's summary.
- **App (host):** the drawer's layout at 384 pt wide with 0, 3, 4, 7 and 8
  remotes (the **+** in the right slot, none at 8); a row's bar colour
  equals the overview band's for the same session; each summary rule;
  each indicator; the spinner's frame at given times; a tap on a row
  with no view, with another view and with its own view (the overview is
  never `Shown` between, checked from `App::render`'s `Shown` sequence);
  a tap on a card sends `SwitchRemote` after closing an open agent view;
  **+** closes the drawer and opens the add mode; Back and a scrim tap
  close; `LayerPlan::direct` on every frame while closed (no texture:
  `has_textures()` false), and textures while open (with libcontrols'
  GPU test pattern, skipped without an adapter).
- **On the S23 Ultra** (asking the owner before installing): open the
  drawer from the overview and from an agent view; switch between two
  remotes and back; open a session from the drawer over another; watch a
  working session and one running a subagent spin and stop; pair from
  **+**; check the four buttons' labels after the libcontrols move;
  check frame pacing while the drawer animates over the agent view's
  terminal. Evidence under `out/drawer/<YYYYMMDD_NN_name>/` with its log.

## 6. Steps

1. **libflankr_proto 0.5.1:** the three `AgentInfo` fields (3.12 item 46),
   additive, with `VERSIONS.md` and tag.
   **Done when:** its tests pass and the round-trip and older-list tests
   pass.
2. **The daemon fills them** (3.12 item 47): `idle_ms`, the Claude
   reader's `subagents_running`, `summary` from the store.
   **Done when:** the daemon tests of section 5 pass and `agents.list`
   from a live daemon shows the fields for a Claude Code pane.
3. **flankr-companion 0.4.0, nickname and activity:** `ServerRecord::
   nickname`, `RemoteInfo::{nickname, shown}`, `Input::SetNickname`, the
   `activity` module, `Input::DrawerShown` and the list cadence; the
   libflankr_proto 0.5.1 dependency.
   **Done when:** those tests of section 5 pass.
4. **flankr-companion 0.4.0, switching:** `Input::SwitchRemote`,
   `State::leave_remote`, the net thread's switch; `VERSIONS.md` entry
   (0.4.0, **Breaking**: the three `Input` variants) and tag
   `flankr-companion-v0.4.0`.
   **Done when:** the switching tests pass.
5. **The app on libcontrols 0.10.1** (3.14).
   **Done when:** the app builds and its tests pass, with no
   `with_abbreviate` added.
6. **The drawer's controls and copy** (3.11, 3.15): `drawer/` and
   `copy/DRAWER_COPY.md`, built from events, without the layout yet.
   **Done when:** the layout, summary, indicator and spinner tests pass.
7. **The drawer in the app** (3.9, 3.10, 3.8): the `DrawerLayout`, the
   menu button, input, Back, the layered drawing, opening sessions, the
   **+**, the theme's sheet colour.
   **Done when:** the app tests of section 5 pass, including no texture
   while closed.
8. **Switching in the app** (3.7 item 28, 30-31): the tap, closing the
   view, dropping a prompt and a confirmation, the connecting and
   unreachable rows.
   **Done when:** a host test switches with an agent view and a typed
   message open and ends on the overview with the new remote's list, and
   the S23 checks of section 5 pass with the owner.

## 7. Risks

1. **The daemon's work per list.** At 3 s with the drawer open, each list
   peeks every agent's session and now reads the subagents' metadata.
   With 20 sessions that is 20 small reads and a directory listing each
   3 s, only while the drawer shows. If it shows in profiles, the
   subagent read can be cached by the directory's modification time.
2. **Memory while open.** About 18 MB for the main texture on the S23
   Ultra at full resolution, plus the sheet's; freed on close. A low
   memory signal while open (`on_trim_memory`) should close the drawer
   at once.
3. **The left edge.** A future edge swipe fights Android's Back gesture
   (3.10 item 43); the button avoids it.
4. **Small cards.** 69 pt is about 7 characters at 13 pt; long nicknames
   are cut. The nickname exists for this (3.3).
5. **Switching drops work in progress** (a typed message, a pending
   confirmation): said in 3.7 item 28; a confirmation dropped on the phone
   still times out on the old daemon as an unanswered one does today.
6. **Activity guesses.** `TOOL_WAIT` (10 min) may spin a row for a stuck
   tool call; `RECENT` (10 s) may stop the spinner during a slow model
   reply that writes nothing. Both are constants to tune on the phone.

## 8. Open questions for the owner

1. **The menu button:** at the left of the header on the overview and in
   the agent view (the agent view's "‹" moving one slot right), or only on
   the overview, or somewhere else?
2. **An edge swipe** to open (and a drag to close), recognised by the app
   (3.10 item 43), given Android's Back gesture at the left edge: wanted
   now, later, or not?
3. **A row's text:** the tab label then the summary (as proposed), or the
   summary alone, or the agent's shown name first?
4. **A tap on the active remote's card:** nothing (as proposed), or open
   its details modal in Settings?
5. **Equal `host:port`:** two remotes with the same host and port add the
   fingerprint's first group (3.3 item 10's last rule): agreed?
6. **`#[non_exhaustive]` on `Input` and `Event`** in the same breaking
   0.4.0, so later variants are patch bumps?
7. **Activity thresholds:** `RECENT` 10 s, `TOOL_WAIT` 10 min,
   `SUBAGENT_LIVE` 60 s; and the idle age at the right of an idle row
   (`4m`): wanted?
8. **The overview's cards** could use the same activity rule and spinner
   later; in this plan only the drawer changes. Wanted on the overview
   too?

## Amendment — 2026-10-28 18:48 EDT: the open questions answered; approved; EditableLabel; flankr-companion 0.5.0

The owner (2026-10-28), offered the lead's answer to each open question
with "Approve the drawer plan with those answers and start it?": "let's
approve and start the drawer, we can switch to EditableLabel as part of
that plan (when it's ready)". Where this disagrees with the body, it wins.

1. **The ≡ button** is on both the overview and the agent view; the agent
   view's "‹" moves one slot right (open question 1).
2. **No left-edge swipe** for now; the button only (open question 2).
3. **Row text:** the tab label, " · ", the summary (open question 3).
4. **A tap on the active remote's card** opens its details modal (the
   settings plan's 2026-10-28 entry), rather than nothing (open question 4).
5. **Same host and port:** the fingerprint's first group is appended to
   tell the two apart (open question 5).
6. **`Input` and `Event` become `#[non_exhaustive]`** in this release, so
   later variants are patch bumps (open question 6).
7. **The thresholds** stay (10 s, 10 min, 60 s) and idle rows show their
   age ("4m") (open question 7).
8. **The overview's cards** take the same activity rule and spinner later,
   in their own change (open question 8).
9. **Versions.** flankr-companion 0.4.0 shipped with the settings follow-up
   (nicknames, `RemoteInfo`'s full fingerprint), so steps 3-4's release is
   **0.5.0**; the nickname parts of step 3 already exist and are not
   repeated.
10. **EditableLabel.** The nickname field in the remote details modal moves
    to libcontrols' new `EditableLabel` (a label with a pencil that turns
    into a check and an optional X), being added to libcontrols as 0.10.2;
    the app takes it in step 5 when 0.10.2 is published, else in a later
    step of this plan.

The plan is approved; the status becomes `In implementation` with its
first code commit.

## Amendment — 2026-10-28 18:51 EDT: a connection manager: every remote stays connected, the inactive ones in the background

The owner (2026-10-28): "connection shouldn't be dropped, we should have a
connection manager and background events can continue, but they should
drop to non-active (background only)". This replaces section 3.7's
switching (closing the old link, forgetting the old daemon's state, and
reconnecting) and the body's one-connection model. Where this disagrees
with the body or the earlier amendments, it wins.

1. **A connection manager in flankr-companion** holds one link per listed
   remote (at most 8), each with its own key, reconnecting on its own
   backoff. Exactly one is **active**; the rest are **background**.
2. **Background means the daemon's existing background mode:** the
   client sets `ClientOptionsParams.background = true` on a background
   remote's link (the same option the app's own backgrounding uses,
   `libflankr_proto::common`'s `ClientOptionsParams.background`), so that
   daemon sends only what a backgrounded app gets (the notices,
   questions, approvals and finished summaries it already sends while the
   link sleeps), and no session streams, terminals or live agent lists
   beyond what that mode sends. The app keeps showing and notifying those
   background events, labelled with the remote's nickname.
3. **Switching** (`Input::SwitchRemote(id)`) writes `active` first, then
   sets the old active remote's link to background and the new one's to
   foreground (`background = false`), and swaps which remote's sessions,
   cards and agent views the app shows. No link is closed; nothing is
   reconnected; each remote's in-memory state and its `state.redb` stay
   open while it is listed. An agent view, a terminal or a prompt bound to
   the old remote is closed on the phone (its terminal is closed on that
   daemon), as the body says.
4. **The drawer** refreshes the active remote's agent list every 3 s while
   shown; background remotes' cards show their last state and a dot when a
   background event (a question, an approval) is waiting.
5. **Removal** closes that remote's link and its `state.redb` as before;
   pairing a new remote adds a background link (or the active one when it
   is the first).
6. **Limits and costs:** up to 8 links held open; a background link sends
   little, and the app's own backgrounding sets every link to background.
