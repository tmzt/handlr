---
title: The bottom bar updated, with a volume menu, tap or hold on the centre and right, and the keyboard toggle and + in the text input
date: 2026-10-28
status: In implementation
covers:
  - "The bottom bar of the agent view (four evenly spaced buttons: volume,
    terminal switch, centre, right) and of the overview (three: volume,
    centre, right); the keyboard glyph leaves the bar"
  - "The volume button: a speaker glyph drawn by the app, with sound waves
    while something is spoken; a tap opens a libcontrols `PopupMenu` of two
    check items that toggle and keep it open (**Speak coding responses**,
    **Speak agent (Gemini) responses**) and **Skip** while speaking"
  - "The two speak options as device settings and Settings rows, in place
    of today's in-memory read-aloud toggle and the ⋮ menu's Read aloud"
  - "The centre (✦ Gemini) and right (record) buttons: a tap starts voice,
    a 2 s press opens the text input card for that target, with the app's
    2 s hold feedback"
  - "A new row in the text input card: a **+** at its left (tap: the media
    chooser; 2 s press: a menu of **Paste text** and **Attach image…**)
    and the soft keyboard's show/hide toggle at its right"
  - "The terminal's keys row: **+** at its far left, ESC TAB CTRL ALT and
    the d-pad as now, the keyboard toggle at its far right; the row shown
    while the terminal is armed, not only while the keyboard is up"
  - "The terminal **+**: an image uploaded through the binary files plan's
    `images` namespace and its path pasted into the pane; **Paste text**
    from the phone's clipboard as a herdr paste; the terminal's input
    rules, not `allow_messages`"
  - "Desktop builds"
  - "Not covered: the binary files plan's transfers, grid and conversion
    (that plan); the media chooser itself (libmobile's and libmorph's media
    request plans); the clipboard call itself (named in 6.4, planned in
    libmobile and libmorph); what the general agent does with images (not
    at all: the binary files plan, its 21:29 EDT amendment, item 9)"
sources:
  - "Owner (2026-10-28, verbatim): \"plan the update the bottom bar, play
    button should be volume glyph if available and expand to checkmark menu
    (boolean options) Speak coding responses Speak agent (Gemini)
    responses. center and right open text input on 2s press and voice on
    tap. the keyboard icon is switched to show/hide ime and centered between
    center and right buttons. text input gets a + icon which triggers media
    request. should keyboard icon move to this input card also instead
    since we're adding a row?\""
  - "Owner (2026-10-28), \"yes, write it as a draft\", to the lead's
    proposal, which settled: the volume glyph drawn by the app (no speaker
    glyph in the atlases) with waves while speaking; a tap opening a
    `PopupMenu` with two check items that toggle and keep it open, and Skip
    while speaking; a 2 s press on volume doing nothing special; the two
    options as device settings and Settings rows, replacing the single
    read-aloud setting; centre and right tap for voice, 2 s press for the
    text input card of that target; the keyboard toggle moving into the
    input card's new row with a + at its left, and in terminal mode into
    the keys row; four evenly spaced buttons on the agent view and three
    on the overview"
  - "Owner (2026-10-28, verbatim), asking about the + in terminal mode:
    \"does that mean + works as paste in terminal?\"; then, to the lead's
    proposal of a terminal +: \"on the left of the terminal keys bar, kbd
    on right\""
  - "Owner (2026-10-28, verbatim), on the + menu: \"can make that a long
    press menu on + in both cases\""
  - "flankr at `b3b5960` (branch `main-terminal`), read 2026-10-28:
    - `apps/companion/src/controls/control_row.rs`: `ControlRow::lay_out(x0,
      x1, bottom, s, dictating, general_agent_listening)`: play at `x0`,
      the centre at `(x0 + x1 - sq) / 2`, the keyboard glyph at
      `x1 - 2 sq - 12 pt` with no background, record at `x1 - sq`; all
      52 pt squares (`BUTTON_H`);
    - `apps/companion/src/agent.rs:3384-3439` (`arrange`): the terminal /
      list glyph placed at `pad + sq + 12 pt`, only when the keyboard glyph
      is in the row and the session is not terminal-only; a terminal-only
      session keeps the keyboard glyph only while armed;
      `AgentHit::{ReadAloud, TerminalToggle, GeneralAgent, Keyboard,
      Message}` at `agent.rs:277-342`;
    - `apps/companion/src/controls/square_button.rs`: `keyboard_glyph`,
      `glyph_box` (62% of the button's width, 0.62 of that high),
      `terminal_glyph` and `list_glyph` as `push_line` strokes and fills;
      `ReadAloud` drawn as `PLAY_GLYPH`, dim when off, `SKIP_GLYPH` while
      speaking;
    - `apps/companion/src/lib.rs:2302-2316`: play toggles `self.read_aloud`
      (a field, `false` at every start, never saved) and sends
      `Input::ReadAloud`; a tap while speaking sends `Input::Skip`;
      `lib.rs:474-490, 2686-2747, 3818-3825`: `CenterHold`, which starts
      Gemini's voice **on touch-down** and opens Gemini's pinned card at
      2 s (`Prompts::hold_open`);
    - `apps/companion/src/menu.rs`: the ⋮ menu's own item **Read aloud**
      (`app/read_aloud`, the agent view only, checked while on), one shared
      `Menu` around a libcontrols `PopupMenuState`;
    - `apps/companion/src/overview.rs:434-470`: the overview's row from the
      same `ControlRow`; a confirmation's Approve and Deny are in a panel
      above the row, not in it;
    - `apps/companion/src/prompt/card.rs:375-548`: `PromptCard::lay_out`,
      the typed input's shaded box (at least 3 lines of 12 pt) and, while
      replying to a card, a `ControlRow` inside the card;
    - `apps/companion/src/agent/keys_row.rs`: `KEYS_H` 44 pt, seven equal
      columns (ESC, TAB, CTRL, ALT, the d-pad's three), `GAP` 4 pt;
      `AgentScreen::keys_row_shown` is `keys_row && term_shown`, the row
      shown only while the soft keyboard is raised into the terminal;
    - `apps/companion/src/drawer.rs:47` (`HOLD`, 2 s, `SLOP` 10 pt) and
      `drawer/remote_card.rs:135-145`: the hold's progress, a 3 pt bar
      inset 6 pt along the card's bottom in the accent colour, filling over
      the 2 s;
    - `apps/companion/src/settings.rs`: `Settings` has *All sound*, *Ding*,
      *Microphone*, *General agent* and the rest, and no read-aloud key;
      `allows(AgentHit::ReadAloud)` is `sound`;
    - `crates/flankr-companion/src/client.rs:290-321` (`Settings`, a plain
      struct with public fields), `:2249-2258` (`Input::ReadAloud`),
      `:4930-4943` (`turn.say` always pushes `Event::Say` for the running
      turn; `sound` gates speech downstream)"
  - "libcontrols at `ef860f0` (0.10.3, the rev flankr pins) and `7f82af4`
    (0.10.4): `src/popup_menu.rs`: `PopupMenuState::choose` always closes
    the menu and queues `PopupMenuEvent::Chosen`; `open` while open
    replaces the items; `PopupItemKind` and `PopupMenuEvent` are
    `#[non_exhaustive]`; placement right-aligns to the anchor, clamped
    inside the viewport, below when it fits, else above"
  - "libdevice `src/clipboard.rs`: `read_text() -> Option<String>`, macOS
    only, `None` elsewhere; libmorph (rev `c9b432d`) exposes no clipboard
    call; libmobile 0.3.1 has none"
related:
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (In implementation;
    living section L3, the agent view: this plan supersedes the parts named
    in decision 13)
  - docs/design/20261027_03_UNIFIED_PROMPT_CARD_PLAN.md (In implementation;
    decisions 6 and 7 superseded in part, decision 13)
  - docs/design/20261027_07_TERMINAL_MODE_PLAN.md (In implementation;
    decision 7's placement of the terminal glyph, and the keys row through
    L3's 2026-10-28 10:42 EDT amendment)
  - docs/design/20261028_05_BINARY_FILES_PLAN.md (Draft; the attachment
    grid of its 21:29 EDT amendment, item 1; needs one amendment, decision
    10)
  - docs/design/20261028_04_PLUGINS_PLAN.md (In implementation; the ⋮
    menu's Read aloud, decision 3.6 item 2; the `PopupMenu` and title
    icons)
  - docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (In implementation;
    its living section of sections and rows, the 13:03 EDT amendment)
  - docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md (the centre button's
    tap; unchanged)
  - docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md (herdr's
    `Paste(String)` client message on the attachment)
  - ~/src/libmobile/docs/design/20261028_02_MEDIA_REQUEST_PLAN.md (Draft;
    the chooser)
  - ~/src/libmorph/docs/design/20261028_02_MEDIA_REQUEST_PLAN.md (Draft;
    `MorphContext::media_request`, `Unsupported` on the desktop)
  - ~/src/libcontrols/docs/design/20261028_06_POPUP_MENU_PLAN.md (the menu
    this plan asks an additive change of, decision 3)
---

# The bottom bar updated, with a volume menu, tap or hold on the centre and right, and the keyboard toggle and + in the text input

## 1. Goal

The bottom bar today has a play button that is one toggle and a skip, a
keyboard glyph squeezed left of record, and a centre button that starts
listening the moment it is touched. After this plan:

- the bar is four evenly spaced buttons on the agent view (volume,
  terminal switch, centre, right) and three on the overview (volume,
  centre, right);
- the volume button opens a small menu that says what is spoken, coding
  replies and Gemini's separately, and skips what is being said;
- the centre and right buttons start voice on a tap and open typed input on
  a 2 s press;
- typing gets its tools where typing happens: the text input card has a row
  with a **+** (media, paste) and the keyboard toggle, and the terminal's
  keys row has the same two at its ends.

## 2. Findings that shape the plan

1. **There is no stored read-aloud setting.** Read aloud is `lib.rs`'s
   `read_aloud` field, off at every start and never saved, toggled by play
   and by the ⋮ menu's **Read aloud**. Gemini's replies (`turn.say`) are
   always spoken while *All sound* is on. So the migration (decision 5) has
   nothing to read: the new keys' defaults reproduce today's start state.
2. **`PopupMenu` cannot stay open after a choice.** `choose` closes the
   menu and queues `Chosen`. The app could re-open it at once with new
   items (`open` replaces items while open), but the menu would lose its
   highlight and flash its modal entry. Decision 3 names the additive
   libcontrols change.
3. **The centre button starts voice on touch-down** (`CenterHold`, so the
   2 s hold opens Gemini's card while already listening). A hold that
   opens typed input cannot also have started voice, so voice moves to the
   release (decision 6).
4. **The keys row exists only while the soft keyboard is up.** With the
   keyboard toggle in that row, hiding the keyboard would remove the button
   that brings it back. The row's visibility changes (decision 9).
5. **Buttons do not press in** (L3, the 2026-10-23 16:19 EDT amendment,
   withdrawn): the hold feedback is the drawer's progress bar alone, with
   no press colour.
6. **No clipboard on Android.** libdevice reads the clipboard on macOS
   only and libmorph does not expose it; libmobile has nothing. Paste text
   needs one new call (decision 11.4).
7. **Approve and Deny are not on the bar.** On the agent view they are on
   the Approval card; on the overview they are in the confirmation panel
   above the bar. Neither moves.

## 3. Decisions

### The bar

1. **Slots, evenly spaced.** `ControlRow` lays out `n` slots of 52 pt
   squares (`BUTTON_H`, unchanged) between `x0` and `x1`, slot `i` at
   `x = x0 + i * (x1 - x0 - sq) / (n - 1)`, all at `y = bottom - sq`:
   - **agent view, `n = 4`:** 0 volume, 1 terminal switch, 2 centre,
     3 right;
   - **overview, `n = 3`:** 0 volume, 1 centre, 2 right;
   - **a card being replied to** (the row inside `PromptCard`), `n = 3`:
     volume, ✕, send, as today.

   On a 411 pt wide phone (the S23 Ultra) with the 12 pt pad: slots at
   x = 12, 123.7, 235.3 and 347 pt on the agent view (about 60 pt between
   buttons), and 12, 179.5 and 347 pt on the overview. A slot whose button
   is not shown stays empty; nothing shifts:
   - the terminal switch is not drawn for a terminal-only session (L3,
     2026-10-28 00:47 EDT) or while a message is dictated (as today);
   - the centre is empty while the daemon offers no general agent (plan
     20261027_02, decision 9), except for the ✕ while dictating;
   - the right is empty for a terminal-only session (no messages).

   While dictating, slot 2 (the centre's) holds the ✕ and slot 3 the send,
   as now. While Gemini listens, slot 2 holds the listening button.
2. **The keyboard glyph leaves the bar** (`AgentHit::Keyboard` is removed).
   Its two jobs move: typing a message is the right button's 2 s press
   (decision 6); raising or hiding the soft keyboard is the toggle in the
   card (decision 8) and in the keys row (decision 9).

### The volume button

3. **A tap opens the volume menu,** the app's shared `Menu` (one libcontrols
   `PopupMenu`), anchored to the button's rect. The menu's own placement
   puts it above the bar, its left edge at the viewport margin. Items, top
   to bottom:
   - **Speak coding responses** (`app/speak_coding`), checked while on:
     the agent's replies are read aloud, as play does today (assistant text
     only, the reading rules of L3's 2026-10-23 16:02 EDT amendment);
   - **Speak agent (Gemini) responses** (`app/speak_gemini`), checked while
     on: Gemini's `turn.say` lines are spoken. Disabled (drawn dim) while
     the general agent is off or the daemon offers none;
   - while something is being spoken, a separator and **Skip**
     (`app/skip`), last, nearest the finger: it sends `Input::Skip` and
     closes the menu.

   The two speak items **toggle in place and keep the menu open.** This
   needs an additive libcontrols release (0.10.5, the next after 0.10.4's
   Scrollbar, planned and built in libcontrols):
   - `PopupItemKind::Toggle` (the enum is `#[non_exhaustive]`) and
     `PopupItem::toggle(id, label, checked: bool)`, an action item with
     `checked: Some(checked)`;
   - choosing a Toggle item (tap or Enter) flips its `checked` in the
     state's items, keeps the highlight, leaves the menu open and queues
     `PopupMenuEvent::Toggled { id: Symbol, checked: bool }` (the enum is
     `#[non_exhaustive]`); `Chosen` is not queued for it;
   - drawing, layout and the other kinds are unchanged; it composes with
     the Draft radio-mark style (libcontrols plan 20261028_07).

   While the menu is open, the app replaces its items (`open` on an open
   menu) when speaking starts or stops, so Skip comes and goes. An outside
   tap, Back or Escape closes it.
4. **The glyph,** drawn as strokes in `square_button.rs`
   (`volume_glyph(list, rect, s, color, waves: Waves)`) in the button's
   `glyph_box`, `b` (about 32 × 20 pt on a 52 pt button), stroke 1.8 pt,
   `cy` the box's middle:
   - the speaker: a body, the rectangle from `b.x + 0.06 b.w` to
     `b.x + 0.24 b.w` across `cy ± 0.22 b.h`, and a cone, lines from the
     body's right edge at `cy ± 0.22 b.h` to `b.x + 0.46 b.w` at
     `cy ± 0.5 b.h` and the cone's mouth between them;
   - **waves** (`Waves::Two`), two arcs centred at `(b.x + 0.46 b.w, cy)`,
     radii `0.22 b.w` and `0.40 b.w`, from -50° to +50°, each drawn as 6
     line segments;
   - **off** (`Waves::Off`): no arcs, and a ✕ of two lines from
     `(b.x + 0.62 b.w, cy - 0.28 b.h)` to `(b.x + 0.90 b.w, cy + 0.28 b.h)`
     and its mirror;
   - **speaking** (`Waves::Speaking(phase)`): the arcs appear in turn, the
     inner then both then none, stepping every 300 ms (the listening
     button's dingbat step), redrawn while speaking.

   Colour: `theme().fg` while either option is on; `theme().dim` with the
   ✕ while both are off. The button's background stays `theme().back`.
   *All sound* off draws it at `DISABLED_ALPHA` and it takes no taps
   (`settings::allows`, as play today). No atlas glyph is used: libmsdf's
   Material subset has no `volume_up` and 🔊 is not in the emoji set.
5. **A 2 s press on volume does nothing special:** the menu opens on the
   release, however long the press, with no hold feedback.

### The speak options as settings

6. **Device settings and Settings rows.**
   - `apps/companion/src/settings.rs`'s `Settings` gains
     `speak_coding: bool` and `speak_gemini: bool`, written to
     `settings.toml` as `speak_coding` (serde default `false`) and
     `speak_gemini` (serde default `true`). These defaults are today's
     behaviour at start (finding 1), so a file without them (every file
     today) loads as today; there is no other key to migrate. From then on
     the choice is kept across restarts, as every other setting is.
   - `lib.rs`'s `read_aloud` field goes; the app reads
     `settings::current().speak_coding`, sends `Input::ReadAloud` with it
     once the client starts and on every change (from the menu or from
     Settings), and sends `Input::Settings` for `speak_gemini`.
   - `flankr-companion`'s `Settings` gains `speak_gemini: bool` (default
     `true`), appended: off, a `turn.say` still gives
     `Event::Conversation` (the card shows the line) but no `Event::Say`.
     `speak_coding` stays `Input::ReadAloud`, so `client.options`'
     `spoken_output` (summaries for speech) is unchanged.
   - Turning an option off does not cut off what is being said; Skip does.
   - **Settings → Sound** gains two toggle rows after *Ding when an agent
     is done*: **Speak coding responses** and **Speak agent (Gemini)
     responses**, inactive while *All sound* is off, the second also while
     the general agent is off or not offered. The copy, shared by the rows
     and the menu items, is in `apps/companion/copy/SETTINGS_COPY.md`
     (`SPEAK_CODING`, `SPEAK_GEMINI`, `MENU_SKIP`). This is entered in the
     settings plan's living section by its own amendment (step 3).
   - **The ⋮ menu's Read aloud is removed** (plugins plan, decision 3.6
     item 2): the volume menu holds it now. The agent view's ⋮ then holds
     only plugin items, moved title icons and Settings (open question 1).

### The centre and right buttons

7. **A tap starts voice; a 2 s press opens the text input card.**
   - **Centre (✦ Gemini):** a tap starts listening (`Input::
     StartGeneralAgentPrompt(Voice)`), on the **release**, no longer on
     touch-down. A 2 s press opens Gemini's prompt card pinned with a typed
     prompt (`StartGeneralAgentPrompt(Keyboard)`), its text area focused
     and the soft keyboard up, without listening.
   - **Right (record):** a tap takes a dictated message (`StartDictation`),
     as now. A 2 s press opens the message card with a typed message
     (`StartTextPrompt`, then focus `PromptKind::Message`), as the
     keyboard glyph did: to this agent on the agent view, and on the
     overview to the agent herdr has in focus (L3, 2026-10-24 16:15 EDT,
     item 3).
   - **Not while active:** while Gemini listens, the centre is a tap target
     only (its tap does what it does now); while a message is dictated,
     the right is send and the centre ✕, taps only. No hold is timed for
     them.
   - **Timing and feedback, as the drawer's remote card:** `HOLD` = 2 s
     and `SLOP` = 10 pt. From touch-down, a 3 pt bar inset 6 pt from the
     button's sides and bottom fills left to right over the 2 s, in
     `theme().fg` (visible on the record button's blue and on the centre's
     `theme().back`), with no press colour (finding 5). At 2 s the action
     fires at once (before the release) and the release is then no tap. A
     finger that moves more than 10 pt, or a cancelled touch, ends the
     press with neither action. A release before 2 s is a tap.
   - **One timer for every hold in the bar and the input rows:** a new
     `apps/companion/src/controls/hold.rs`:

     ```rust
     pub const HOLD: Duration = Duration::from_secs(2);
     pub const SLOP: f32 = 10.0; // points

     pub struct Hold { pub id: u64, pub hit: AgentHit, start: Duration,
                       at: [f32; 2], fired: bool, moved: bool }
     impl Hold {
         pub fn start(id: u64, hit: AgentHit, at: [f32; 2], now: Duration) -> Self;
         /// The press's progress in 0..1 while it is timed; None once fired or moved.
         pub fn progress(&self, now: Duration) -> Option<f32>;
         /// True once, when 2 s have passed unmoved.
         pub fn fire(&mut self, now: Duration) -> bool;
         /// A move: past SLOP (scaled by `s`) the press is no tap and never fires.
         pub fn moved_to(&mut self, at: [f32; 2], s: f32);
         /// The release: a tap if it neither fired nor moved.
         pub fn is_tap(&self) -> bool;
     }
     ```

     `lib.rs`'s `CenterHold` is replaced by one `hold: Option<Hold>`,
     ticked in `update` as `CenterHold` is; the screens draw the bar from
     `set_hold(Option<(AgentHit, f32)>)`.

### The text input card's row

8. **A new row at the bottom of `PromptCard`,** shown while the card has
   typed input (`input.keyboard()`), pinned or in the overlay (under the
   overlay's input box, which is the same card's), 44 pt high and 4 pt
   under the typed box, the card's inner width:
   - **+** at its left, a 44 × 44 pt target, drawn as two 2 pt strokes
     18 pt long in `theme().accent`, no background (`AgentHit::
     PromptAttach(PromptKind)`):
     - a **tap** opens the media chooser (`MorphContext::media_request`
       with the binary files plan's request: the four image types,
       `CameraEntry::Photo`, `Picker::Chooser`, at most 4 items, 20 MiB,
       `Delivery::Bytes`); the images go into that plan's attachment grid
       (its 21:29 EDT amendment, item 1) and their `[image N]` tokens at
       the cursor;
     - a **2 s press** (decision 7's hold and feedback, the bar in
       `theme().accent` along the target's bottom) opens the shared menu
       anchored to the +, with **Paste text** (`app/paste_text`) and
       **Attach image…** (`app/attach_image`, the same chooser). Paste text
       inserts the clipboard's text at the cursor through the keyboard
       route's commit path, exactly as text typed on the soft keyboard; it
       is disabled while the clipboard has no text or the clipboard call is
       missing (decision 11.4);
     - shown on the **message** card only, while the daemon's
       `blob.namespaces` has `images`; not on Gemini's card, which takes no
       images (open question 3);
   - the **keyboard toggle** at its right, a 44 × 44 pt target
     (`AgentHit::PromptIme(PromptKind)`), on both cards:
     - while the soft keyboard is up: the keyboard glyph with a down
       chevron under it (hide); a tap hides the keyboard as its own
       dismissal does (`TextInputEvent::Dismissed`'s path: the card and
       its text stay);
     - while it is down: the keyboard glyph alone (show); a tap focuses
       the text area and raises the keyboard, as a tap on the text area
       does;
     - geometry: `keyboard_glyph` in the 44 pt target (about 27 × 17 pt),
       moved up 3 pt, and the chevron 8 pt wide and 4 pt high, 3 pt under
       it, stroke 1.8 pt, `theme().fg`;
     - while a physical keyboard is active (the L3 amendment of
       2026-10-25 23:34 EDT) the soft keyboard is suppressed: the toggle is
       drawn dim and takes no taps.

   The card grows by 48 pt (the row and its gap) while it shows. The
   replied-card row of controls stays below it.

### The terminal's keys row

9. **Nine columns, shown while armed.**
   - The row is **+**, ESC, TAB, CTRL, ALT, the d-pad's three (↑ over the
     middle), and the **keyboard toggle**: nine equal columns of
     `(w - 10 GAP) / 9` (about 41 pt on a 411 pt phone), `KEYS_H` 44 pt
     and `GAP` 4 pt as now. `TermKey` gains `Attach` and `Ime` (no token;
     not sent).
   - It shows **while the terminal is shown and armed**, whether the soft
     keyboard is up or not: with the keyboard up it sits just above it, as
     now; with it down it sits at the screen's bottom inset, under the
     bottom bar, which it lifts as now. Before the terminal is armed there
     is no row; a tap on the terminal arms it and raises the keyboard (L3,
     2026-10-27 21:58 EDT), and the row comes with it.
   - The **toggle** at the far right is the card's (decision 8): hide or
     show the soft keyboard into the terminal (`FocusedPrompt::Terminal`),
     the same glyphs. For a terminal-only session it replaces the bar's
     keyboard glyph while armed.
   - The **+** at the far left: a tap opens the media chooser; a 2 s press
     opens the menu of **Paste text** and **Attach image…**, as in the
     card; what they do is decision 10. It is drawn dim and takes no taps
     while neither works (no `images` namespace and no clipboard call);
     the column stays, so the keys do not shift.

### The terminal + and the binary files plan

10. **The terminal +** puts an image's path, or the clipboard's text, into
    the pane as a paste:
    1. **Images.** The chooser's images (at most 4) go up through the
       binary files plan's `images` namespace to the session's protected
       folder (`<temp>/flankr-<uid>/images/<pane>/`), as an attachment
       does. The agent view shows its notice line while they go
       ("Uploading image 1 of 2…"; a refusal's line from that plan's 4.6
       codes). When all are DONE, the client sends one `terminal.paste
       { terminal_id, images: [reference, …] }`.
    2. **The daemon pastes** the images' paths, joined by single spaces,
       as one herdr `Paste { text }` on the terminal's attachment, so herdr
       brackets it when the pane has bracketed paste on: Claude Code in the
       terminal attaches the image, and a shell gets the path at the
       cursor. With no matching attachment (an older herdr) the paths go as
       keystrokes (`pane.send_keys`, literal text, 64-byte batches). The
       references are consumed as a send consumes them.
    3. **The terminal's rules, not `allow_messages`:** `terminal.paste`
       needs what `terminal.input` needs: `[terminal] input`, the agent's
       `terminal_input`, a principal that may approve, the terminal opened
       on this connection, and the live arm (plan 20261027_07, decision 6,
       as L3's 10:42 EDT amendment changed it). It goes into the terminal,
       not through a sent message, so `allow_messages` and
       `allow_arbitrary_commands` do not apply. It is logged as a
       `terminal_input` notice with kind `paste`, the agent and the byte
       count, never the text or paths.
    4. **Paste text.** The client sends `terminal.paste { terminal_id,
       text }` with the clipboard's text: at most 8192 bytes of UTF-8 (the
       app refuses longer with a notice, "The clipboard has more than 8 KB
       of text."), `\r\n` and lone `\r` made `\n`, and every other control
       character but `\t` and `\n` refused. On the attachment it is a herdr
       `Paste { text }`. Without the attachment it goes as keystrokes, and
       a text with a `\n` is refused (`NOT_PERMITTED`, "A multi-line paste
       needs the attached terminal."), since an unbracketed newline would
       run the line.

    **Interface,** for the binary files plan's amendment to carry (it owns
    `images`, its references and `libflankr_proto`'s additions there):

    ```rust
    // libflankr_proto, methods
    pub const TERMINAL_PASTE: &str = "terminal.paste";

    #[derive(Serialize, Deserialize)]
    pub struct TerminalPasteParams {
        pub terminal_id: u32,
        /// Exactly one of `text` and `images`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub text: Option<String>,
        /// 1-4 references from DONE of `images` transfers for this pane.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub images: Vec<String>,
    }
    // result: {}; errors: NOT_PERMITTED, UNKNOWN_AGENT, HERDR_UNAVAILABLE,
    // INVALID_PARAMS (both or neither; an unknown or used reference).
    // An older daemon answers METHOD_NOT_FOUND: the app shows "This flankr
    // can't paste into the terminal." and keeps the images.
    ```

    `flankr-companion` gains `Input::TerminalPasteText(String)` and
    `Input::TerminalPasteImages(Vec<u8>)` (attached image numbers), on the
    `#[non_exhaustive]` `Input`.

    **The binary files plan needs one amendment** (it is a Draft, so the
    owner agrees its wording first, Rule 2): (a) 9.1's paperclip becomes
    this plan's + (decision 8), tap and hold as here; (b) a terminal paste
    path beside `agents.send`'s `parts`: `terminal.paste` as above, with
    the daemon's handler and the client's inputs in its steps 1, 6 and 7;
    (c) its version numbers, since `libflankr_proto` 0.5.3 has since been
    taken by the plugins plan's title icons. Step 8 of this plan waits for
    that amendment's steps.

### Desktop builds and the clipboard

11. **Desktop and the rest.**
    1. The bar, the volume menu, the holds (a mouse press is a touch) and
       the settings work as on the phone.
    2. **No +** on the desktop, in the card or the keys row: libmorph's
       media request answers `Unsupported` there. The card's row then
       holds only the toggle, and the keys row's + column is empty and
       inert.
    3. **No keyboard toggle** on the desktop: a physical keyboard is always
       active there, so the toggle would be dim forever; it is not drawn.
       The card's row is not shown when it would be empty.
    4. **The clipboard call.** Paste text needs one additive call:
       `MorphContext::clipboard_text(&self) -> Option<String>` in libmorph,
       the plain text of the clipboard's first item, read on the UI thread
       on the user's tap. Android: a new libmobile `clipboard` feature,
       `android::clipboard::read_text(app: &AndroidApp) -> Option<String>`
       (`ClipboardManager.getPrimaryClip()`, item 0, `coerceToText`; the
       app has focus, so Android 10's restriction allows it, and Android 12
       shows its own "pasted" toast); macOS: libdevice's
       `clipboard::read_text()`; elsewhere `None`. Neither plan is written:
       one is due in libmobile and one in libmorph, each additive. Until
       libmorph has it, **Paste text** is drawn disabled in both menus.

### What happens to the other plans

12. **Approve and Deny are unchanged** (finding 7). The confirmation panel
    on the overview stays above the bar; a card's own Approve and Deny stay
    on the card.
13. **What this plan supersedes.** The frozen plans are not edited
    (Rule 1.7); this list is the record, and where they disagree with this
    plan, this plan wins:
    - **L3** (plan 20261022_03's living section):
      - the 2026-10-23 16:02 EDT amendment's **bottom row** bullets ("play
        at the left") for the glyph and the **Read aloud** bullets "play,
        dull when off and bright when on" and "while something is being
        said, play shows `▶│` and a tap skips": replaced by decisions 3-6;
        what is read and the reading rules stand;
      - the 2026-10-24 11:40 EDT amendment's "the centre is the general
        agent's button": it is slot 2 of 4 on the agent view (decision 1);
        its roles stand;
      - the 2026-10-24 16:15 EDT amendment (the keyboard glyph left of
        record), all of it: decisions 2, 7 and 8;
      - the 2026-10-28 00:47 EDT amendment's terminal-only keyboard glyph
        while armed (as `arrange` has it): decision 9;
      - the 2026-10-28 10:42 EDT amendment, item 3's "shown while the
        keyboard is raised into the terminal" and its seven columns:
        decision 9. Items 1, 2 and 4 stand.
    - **Plan 20261027_03:** decision 7 ("the centre button's 2 s hold opens
      Gemini's card in its pinned state … A tap starts listening"):
      replaced by decision 7 here (the hold opens it with a typed prompt,
      the keyboard up; listening starts on the release). Decision 6 gains
      decision 8's row; the rest of decision 6 stands.
    - **Plan 20261027_07:** decision 7's placement of the terminal glyph
      "between play and the centre button … mirroring the keyboard glyph":
      it is slot 1 of 4 (decision 1); the glyphs and the swap stand.
    - **Plan 20261028_04:** decision 3.6, item 2's app item **Read aloud**:
      removed (decision 6). Settings and the rest stand.
    - **Plan 20261024_02:** its living section gets the two Sound rows by
      amendment (step 3).

## 4. Steps

Each step is one crate (Rule 3.1). Steps 3-7 and 9 are the app,
`apps/companion`; each passes `cargo fmt -p companion -- --check`,
`cargo clippy -p companion --all-targets` and `cargo test -p companion`.

1. **libcontrols 0.10.5: toggle items that keep the menu open** (in
   `~/src/libcontrols`, under its own plan there, written from decision 3):
   `PopupItemKind::Toggle`, `PopupItem::toggle`, `PopupMenuEvent::Toggled`,
   tests that a toggle flips, stays open with its highlight, queues one
   `Toggled` and no `Chosen`.
   **Done when:** libcontrols 0.10.5 is tagged with those tests passing and
   its existing menu tests unchanged.
2. **flankr-companion: `Settings::speak_gemini`.** Appended field (default
   `true`); `turn.say` gives `Event::Say` only while it and `sound` are on;
   `VERSIONS.md` entry and tag (Versioning).
   **Done when:** `cargo test -p flankr-companion` passes with a test that
   a `turn.say` with `speak_gemini` off gives `Event::Conversation` and no
   `Event::Say`, and the existing `a_turn_and_its_say` passes unchanged.
3. **companion: the volume button, its menu and the settings.** Pin
   libcontrols 0.10.5 and flankr-companion; `volume_glyph` (decision 4);
   `AgentHit::ReadAloud` renamed `Volume`, its tap opening the menu
   (decision 3), `Choice::{SpeakCoding, SpeakGemini, Skip}`, items replaced
   on speaking changes; `Settings::{speak_coding, speak_gemini}` and their
   TOML keys and Sound rows (decision 6); `read_aloud` field and the ⋮ Read
   aloud removed. This commit sets this plan to `In implementation` with
   its amendment, and appends the settings plan's living-section amendment
   for the two rows and an L3 amendment pointing here.
   **Done when:** tests show the glyph's three states; the menu opens above
   the button, toggles each item and stays open, shows Skip only while
   speaking; a `settings.toml` without the keys loads as `false`/`true`;
   the Settings rows and the menu change the same values; *All sound* off
   dims the button.
4. **companion: the bar in slots, and tap or hold on the centre and
   right.** `ControlRow::lay_out(x0, x1, bottom, s, &RowState)` with
   `RowState { slots, dictating, general_agent_listening, terminal:
   Option<&'static str>, record: bool }` (decision 1); `AgentHit::Keyboard`
   removed; `controls/hold.rs` and `CenterHold` replaced (decision 7); the
   progress bar drawn.
   **Done when:** layout tests give the slot positions of decision 1 at
   411 pt for both screens and every state (dictating, listening, terminal
   only, no general agent); a tap on the centre starts listening on release
   and not on touch-down; a 2 s press on the centre opens Gemini's card
   typed and focused without listening; a 2 s press on the right opens the
   message card typed and focused; a move of 11 pt cancels both; the
   existing control-row and agent view tests pass, updated for the slots.
5. **companion: the card's row with the keyboard toggle.** Decision 8's
   row, `AgentHit::PromptIme`, the glyphs, the dim state with a physical
   keyboard, the desktop rule (11.3); the + drawn only once step 7 lands.
   **Done when:** tests show the row on a typed card in both modes, pinned
   and in the overlay, and not on a dictated one; the toggle hides the
   keyboard leaving the text and shows it again with focus; the card's
   height grows by 48 pt.
6. **companion: the keys row in nine columns, shown while armed.**
   `TermKey::{Attach, Ime}`, the layout of decision 9, `keys_row_shown`
   while shown and armed; the terminal-only bar's keyboard glyph gone; the
   + drawn dim until step 8.
   **Done when:** layout tests give nine columns at 411 pt; the row shows
   armed with the keyboard down, under the bar, and above the keyboard when
   up; the toggle hides and raises the keyboard into the terminal; ESC,
   TAB, CTRL, ALT and the arrows behave as before.
7. **companion: the card's +.** Tap and 2 s press (decision 8), the menu's
   two items, `MorphContext::media_request` into the binary files plan's
   grid (once its step 9 has the grid and libmorph's media request is
   released), Paste text through `clipboard_text` when libmorph has it.
   **Done when:** on the S23, a tap on + opens Android's chooser and the
   chosen image lands in the grid with its token; a 2 s press opens the
   menu above the +; Paste text inserts the clipboard at the cursor (or is
   disabled before the clipboard call exists); on the desktop no + shows.
8. **companion: the terminal +.** After the binary files plan's amendment
   (decision 10) has `terminal.paste` in libflankr_proto, the daemon and
   flankr-companion: the keys row's + live, images uploaded with the
   notice line and pasted, Paste text sent.
   **Done when:** against a test daemon with herdr's mock attachment, a
   chosen image's path arrives as one `Paste` and, without the attachment,
   as keystrokes; a multi-line text without the attachment is refused with
   its line; on the S23, an image + into Claude Code's terminal attaches it
   and into a shell types its path.
9. **The S23 check.** Every screen's bar, the volume menu, both holds, the
   card row and the keys row on the phone; findings in the app's evidence
   log under `out/companion/`.
   **Done when:** the owner has seen the bar on both screens and the menu,
   and the step's evidence entry is written.

Order: 1 before 3; 2 before 3; 4 after 3 (both touch `square_button.rs`);
5 and 6 after 4; 7 after 5 and the binary files plan's step 9; 8 after 6
and the binary files plan's amendment steps.

## 5. Versioning (Rule 5)

| Crate | From | To | Why |
|---|---|---|---|
| `libcontrols` | 0.10.4 (flankr pins 0.10.3's rev) | **0.10.5** | additive: a variant on each of two `#[non_exhaustive]` enums and a constructor; its own repository and plan |
| `flankr-companion` | 0.5.1 | the next free patch (0.5.2 if the binary files plan has not taken it) | additive: a field appended to `Settings`, which has `Default` (the owner's ruling that appending a struct field is additive); the `Input` variants of decision 10 go in the binary files plan's release |
| `libflankr_proto`, `flankr` (daemon) | — | in the binary files plan's amendment | `terminal.paste` |
| `libmorph`, `libmobile` | — | their own plans | `clipboard_text`, the `clipboard` feature (11.4) |
| `companion` (app) | — | no bump | an application: manifest and lock only |

## 6. Risks

- **A late voice start.** Listening starts on the release, a tap's length
  (about 100 ms) later than now. The first word is not lost, since nothing
  is heard before the tap ends.
- **Holds and scrolls.** A 2 s press on the right button near the bottom
  of the scrollback could start a scroll; the 10 pt slop ends the press
  first, as in the drawer.
- **Pasting into a shell.** An image path pasted into a shell with no
  bracketed paste lands at the cursor and runs nothing (no newline). Text
  is checked as decision 10.4 says.
- **libcontrols 0.10.5 not ready.** The app could re-open the menu on each
  `Chosen` as a stop-gap; this plan does not do so (finding 2).

## 7. Open questions for the owner

1. **The ⋮ menu's Read aloud.** Removed as written (decision 6), since the
   volume menu holds both options? Recommended: yes.
2. **Speak coding responses kept across restarts.** Today read aloud is off
   at every start; as a setting it stays as left. Recommended: kept, as
   every setting is.
3. **A + on Gemini's card.** Gemini takes no images (binary files plan),
   so the card has only the keyboard toggle; the soft keyboard's own
   clipboard pastes there. Should Gemini's card have a + whose tap and hold
   both offer only Paste text? Recommended: no, not until Gemini takes
   images.
4. **A haptic at 2 s.** The holds give only the bar. A short vibration when
   one fires (a libmorph call that does not exist yet) would say so without
   looking. Recommended: later, with the libmorph call.
5. **Paste text's limit.** 8 KB, and multi-line text refused without
   herdr's attachment. Recommended: yes.
6. **The volume menu on the overview.** The same menu; *Speak coding
   responses* there reads the view being followed, as play does now.
   Recommended: yes.

## Amendment — 2026-10-28 23:39 EDT: a + on Gemini's card, for Paste text now

The owner (2026-10-28): "we can include the paste option on Gemini card,
with the plan later to support images as well". This answers open
question 3 the other way from its recommendation, and replaces the text
card row's "the + on the message card only":

1. **Gemini's typed card gets the + too**, in the same place in its new
   row. Until the general agent takes images, a tap on it opens its menu
   directly (there is nothing to attach), and the menu holds only
   **Paste text**, inserted at the cursor; a 2 s hold does the same.
2. **Images later.** When a later plan lets the general agent see an
   attached image (the binary files plan's 21:29 EDT amendment, item 9),
   Gemini's + takes the message card's behaviour: a tap opens the media
   chooser and the hold's menu adds **Attach image…**. Nothing of that is
   built by this plan.
3. Paste text stays disabled until libmorph's `clipboard_text` is
   released, as on the message card.

## Amendment — 2026-10-28 23:40 EDT: approved

The owner approved the plan on 2026-10-28 ("approve the bottom bar plan
and build"). Open questions 1, 2, 4, 5 and 6 take their recommendations
(the ⋮ menu's Read aloud removed; Speak coding responses kept across
restarts; a haptic later; Paste text at 8 KB, multi-line refused without
herdr's attachment; the same volume menu on the overview); question 3 is
answered by the 23:39 EDT amendment above (a + on Gemini's card for Paste
text). Steps that need other releases are built as those land: the
volume menu's toggles need libcontrols' toggle items (0.10.6, being
planned); Paste text needs libmorph's `clipboard_text` (being built);
the + on the message card and the terminal + need the binary files plan
(a Draft) and stay hidden until the daemon offers `images`; the terminal
paste needs that plan's amendment, which the owner has not yet agreed.
The status stays `Draft` until the first implementing commit (Rule 1.5).

## Amendment — 2026-10-29 00:01 EDT: implementation started; the document is now immutable

Implementation starts with this amendment, in the commit of step 2
(flankr-companion 0.5.2, `Settings::speak_gemini`) on `wt-bottom-bar`:
`status` is `In implementation`, and the body above is frozen (AGENTS.md
Rule 1.5). Departures are recorded in `CHANGES.md`.

## Amendment — 2026-10-29 00:26 EDT: the speaker is a mute toggle; its menu on a 2 s hold

On the owner's explicit instruction (Rule 1.5), 2026-10-29: "tapping
speaker should be a mute toggle, with menu items disabled and checked
status retained (monochrome icon)"; then "yes, write it and build" to the
lead's wording. This replaces the volume button's tap (decision 3: a tap
opens the menu) and its glyph states (the dim speaker with an ✕ when both
options are off); the menu's items, their toggles and Skip stand.

1. **A tap mutes or unmutes.** Muting stops anything being spoken at once
   (as Skip does) and sits above both Speak options: neither is changed.
2. **The menu opens on a 2 s hold**, with the same filling bar as the
   centre and right buttons (`controls/hold.rs`, `HOLD` 2 s, `SLOP` 10 pt).
3. **While muted**, the menu's two Speak items are drawn disabled with their
   checks kept, so the menu shows what unmuting brings back; Skip is not
   shown, since nothing is speaking.
4. **The glyph.** Muted: the plain speaker, monochrome and dim, no waves.
   Unmuted: the speaker at full strength, with waves for the options that
   are on (none when both are off), stepping every 300 ms while speaking.
5. **Kept across restarts**: `muted` is a device setting in
   `settings.toml` (default false), beside `speak_coding` and
   `speak_gemini`; Settings → Sound gets no row for it. Muted, the app
   speaks nothing whatever the two options say; the client is told as if
   both were off.

## Amendment — 2026-10-29 00:43 EDT: skip and stop while speaking; mute clears the queue

On the owner's explicit instruction (Rule 1.5), 2026-10-29: "the volume
button should still skip when actually [speaking], with 1s push to stop but
not mute. mute should stop queued playback (clear queue)"; then "yes, write
it and build, install when done" to the lead's wording. This replaces the
00:26 EDT amendment's item 1 (a tap always mutes) for the time something is
being spoken, and its "muting stops anything being spoken" with the
clearing below; its items 2-5 stand.

1. **While something is being spoken:** a **tap** skips the current reply
   (the next queued one follows, as `Input::Skip`); a **1 s hold** stops:
   the current reply ends and the queue is cleared, mute stays off, so new
   replies are read as they come; a bar fills over the 1 s as for the 2 s
   holds; **holding on to 2 s** still opens the menu, after the stop at 1 s.
2. **While nothing is being spoken:** a tap mutes or unmutes, a 2 s hold
   opens the menu (unchanged).
3. **Muting clears the queue:** it stops the current reply and drops every
   queued reply and spoken notice, so nothing plays after a mute.
4. **flankr-companion, additive:** `Input::StopReading` stops the current
   reply and clears the queued replies and notices; the 1 s stop and mute
   both send it. The crate takes the next free patch number at its commit.

## Amendment — 2026-10-29 00:52 EDT: mute stops all voice output

On the owner's explicit instruction (Rule 1.5), 2026-10-29: "no, mute
stops all voice output", correcting the build's choice (CHANGES.md, the
00:49 EDT addendum, item 2) that muted, the client could still say
confirmations and notices. This adds to the 00:26 and 00:43 EDT
amendments:

1. **Muted, nothing is spoken**: no read-aloud reply, no spoken notice, no
   spoken confirmation or question, no dictation read-back, no Gemini line,
   whatever the Speak options say. Sounds that are not speech (the done
   ding, the record chimes) follow *All sound* as before.
2. Muting stops what is being said and clears the queue (the 00:43 EDT
   amendment, item 3); while muted, anything the client asks to say is
   dropped, not held for later.
3. Since nothing speaks while muted, a press on the speaker while muted
   always unmutes (a tap) or opens the menu (a 2 s hold); the
   skip-and-stop of the 00:43 EDT amendment applies only while unmuted.

## Amendment — 2026-10-29 00:52 EDT: correction to the previous amendment's citation

The previous amendment cites "CHANGES.md, the 00:49 EDT addendum"; the
addendum is headed 2026-10-29 00:50 EDT. Nothing else changes.

## Amendment — 2026-10-29 02:06 EDT: the muted speaker has a slash

On the owner's explicit instruction (Rule 1.5), 2026-10-29: "mute icon
should have slash through it when muted". This adds to the 00:26 EDT
amendment's item 4: the muted glyph is the plain speaker, dim and with no
waves, with a diagonal slash through it from top left to bottom right, in
the same colour and stroke width as the speaker.
