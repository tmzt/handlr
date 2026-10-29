---
title: Screen drivers, one per agent, for answering choice questions by number and text and reading usage on demand
date: 2026-10-29
status: Draft
covers:
  - "A `ScreenDriver` trait per agent beside `SessionReader`, chosen by the
    pane's agent: `recognise(&Screen)`, `answer(&ChoiceSet,
    &[ChoiceAnswer])`, `usage()`; Claude Code's driver first, the trait and
    fixture folders ready for Codex, Gemini CLI and agy"
  - "One `Screen` type over two sources: the cell grid of the daemon's
    libherdr_term attachment when it shows the pane, else herdr's JSON
    `pane.read` text; per-driver anchor tables, the focused row and cursor,
    and a settle rule (no change for 150 ms, with a timeout) before every
    check, failing safe"
  - "Answering a choice question with two actions only: a single choice is
    its number; everything else (multi-select, Other) is the Other row's
    number, the answer typed as numbered text, then Enter. No arrows, Tab or
    Esc. This replaces the key table of the choice questions plan's
    decision 5"
  - "Reading Claude Code's `/usage` only on demand: the 5-hour and weekly
    percentages and reset times, as `UsageInfo` on the agent's entry, the
    screen closed with the agent's own key"
  - "An agent-commands menu (libcontrols `PopupMenu`) in the agent view's
    input row, listing the agent's allowed commands and the driver's actions
    (**Usage**); the title bar as the alternative (open question)"
  - "Permission, logging and evidence: `allow_messages` for typing an
    answer, the allowed commands for sending a command, every key and check
    logged without screen contents, recorded fixtures, an S23 check per
    driver"
  - "Not covered: drivers for Codex, Gemini CLI and agy (later, on the same
    trait); Claude Code's status-line route to the rate limits (named, not
    built); answering by voice"
sources:
  - "Owner (2026-10-29, verbatim), on the choice questions plan's driver:
    \"could it navigate to the last input and type the responses with
    numbered prefixes?\", then \"or does typing the number select the
    input?\""
  - "Owner (2026-10-29, verbatim): \"sorry, use the number based select and
    enter text method with heuristics on the terminal update either in json
    or via cell grid. this will need an advanced plan and driver per agent
    (like session reader), it should probably support /usage as well\""
  - "Owner (2026-10-29, verbatim), to the lead's proposal (the trait, the
    two actions, the two screen sources with the grid preferred, `/usage`
    on demand, safety as decision 15): \"yes, write it as a draft. only on
    demand, probably with a command menu in the input row in agent view
    (maybe in titlebar?)\""
  - "flankr at `46a2215` (branch `main-terminal`), read 2026-10-29:
    - `src/sessions/choices.rs` (the current driver: `Step`, `plan`,
      `unmet`, `fold`, the `Pane` trait, `Timing`, `drive`, `check`,
      `compare`, `split_multi`, `await_result`, `tested`;
      `STEP_GAP` 150 ms, `CHECK_POLL` 100 ms, `CHECK_FOR` 1 s);
    - `src/server/choice.rs` (`choice.answer`: `HerdrPane` over
      `pane.read` visible, `pane.send_keys`, `pane.send_text` as
      `HerdrRequest::Custom`; `Answering`, the per-pane lock);
    - `src/sessions/mod.rs:339-375` (`SessionReader`), `:607-631`
      (`Sessions::reader_for`: `claude`, `codex`, `agy`), `:655-690`
      (`OpenChoice`, `open_choice`);
    - `src/server/herdr_attach.rs` (`HerdrAttach`, `Published { generation,
      view, stale }`, `Published::shows`; active only while a phone terminal
      is open; `pane.focus` claims the attachment's view and geometry);
      `src/server/terminal.rs:313-331` (`attached_grid`,
      `published_grid`); `src/core/policy.rs` (`check`, `check_answer`,
      `check_message`, `is_allow_listed`); `src/config.rs:274-305`
      (`AgentConfig`: `allowed_commands`, `allow_messages`)"
  - "`crates/libherdr_term` 0.2.0 (`SurfaceView`, `PaneView { pane, cells,
    cursor }`, herdr's `CellData`; endpoint protocol generation 1, herdr
    protocol 22 at the submodule's pin) and `crates/libflankr_term` 0.1.1
    (`Grid { cols, rows, lines, cursor }` with `Hash` and serde, `Run`,
    `Cursor`, `parse_ansi`, `Grid::text_lines`)"
  - "The shipped Claude Code client 2.1.312
    (`~/.local/share/claude/versions/2.1.312`), read-only, for the usage
    screen's strings: rows titled `Current session` (`rate_limits.five_hour`),
    `Current week (all models)` (`seven_day`), `Current week (Sonnet only)`
    (`seven_day_sonnet`, Max and Team plans) and `Current week (<model>)`
    (`model_scoped`), each with `% used` and a dim `Resets …` line; the data
    from `/api/oauth/usage`. The question screen's strings are the choice
    questions plan's findings 3 and 4"
related:
  - docs/design/20261029_01_CHOICE_QUESTIONS_PLAN.md (In implementation;
    this plan replaces its driver, decision 5's key table and screen checks
    and the driver half of step 4. To be recorded there, once this plan is
    approved, by an amendment named "Amendment — <time>: the driver
    replaced by the screen drivers plan", which the owner's approval of this
    plan authorises; not written yet)
  - docs/design/20261023_06_SESSION_READERS_PLAN.md (Implemented; the
    agnostic `SessionReader` trait and per-agent registry this trait sits
    beside)
  - docs/design/20261027_07_TERMINAL_MODE_PLAN.md (In implementation; the
    terminal frames and the `pane.read` fallback)
  - docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md (In
    implementation; libherdr_term, the daemon's attachment, herdr endpoint
    generation 1, protocol 22; the grid source)
  - docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md (In implementation;
    decision 8's input row with the + and the keyboard toggle, where the
    commands menu goes)
  - docs/design/20261028_04_PLUGINS_PLAN.md (In implementation; the
    title-bar icon slots of its 22:03 EDT amendment, the menu's
    alternative place)
---

# Screen drivers, one per agent, for answering choice questions by number and text and reading usage on demand

## 1. Goal

The choice questions plan answers Claude Code's `AskUserQuestion` screen
with Claude's own keys, including ↓ presses counted from the first row for
multi-select. The owner asked for a simpler, sturdier method: select by
number, and where a number alone cannot say the answer, go to the Other row
by its number and type the answer as numbered text. The owner also asked
for this to be a driver per agent, the way session readers are, reading the
terminal either as herdr's JSON text or as the attached cell grid, and for
the same driver to read `/usage`.

After this plan:

1. Each agent with a driver has one `ScreenDriver`, chosen by the pane's
   agent as `Sessions::reader_for` chooses a reader. Claude Code's is the
   first.
2. Every answer uses only digits, literal text and Enter. A single choice is
   its number. Anything else is the Other row's number, the answer typed as
   numbered text (`1. Red; 3. Blue`), and Enter.
3. Before and after every action the driver reads the screen, waits for it
   to settle and checks it with its own anchor table. A screen it does not
   recognise stops the answer with nothing more sent.
4. On demand, from a commands menu in the agent view, the driver sends
   `/usage`, reads the 5-hour and weekly use and their reset times, closes
   the screen and shows the numbers on the agent's card.

## 2. Findings that shape the plan

1. **The current driver** (`src/sessions/choices.rs`) mixes three things:
   agent-agnostic checks of the answer (`check`, `compare`, `split_multi`,
   `await_result`), Claude Code's key plan (`plan`, cases A-F, `down` ×N
   for multi-select) and a text-only screen check (`unmet` over `fold`ed
   `pane.read` text, `Pane`, `drive`). `src/server/choice.rs` supplies the
   `Pane` from herdr's JSON API only.
2. **Claude Code's question screen** (choice questions plan, findings 3
   and 4): in single-select, a digit on an option picks it and advances; a
   digit on the empty Other row focuses its input (placeholder `Type
   something.`), and Enter there submits non-empty text. In multi-select a
   digit **toggles** its row without moving focus, typing in the Other row
   selects it, and Enter in the Other row toggles it. Single-select
   questions with previews have no Other row: a digit focuses, Enter picks.
   The review's `1` submits.
3. So, for the two-action method:
   - single-select options and the review are fully covered by digits;
   - single-select Other is the digit, the text and Enter, as now;
   - previews need the digit and Enter;
   - **multi-select is the uncertain case**: the Other row's digit may
     toggle rather than focus the input, and Enter in the input may toggle
     Other rather than go on. Neither is confirmed on a real screen. The
     driver therefore checks each one on the screen (decision 8) and stops
     safely when the screen does not show what the step needs.
4. **The grid.** The daemon's `HerdrAttach` holds one libherdr_term
   attachment while herdr is local and `[terminal] attach` is on. It is
   *active* (draws the pane) only while a phone terminal views a pane, and
   `Published::shows(pane_id)` tells whether the current surface has that
   pane. `attached_grid` already cuts a pane into a `libflankr_term::Grid`
   with styles and the cursor. Opening a viewer or sending `pane.focus`
   only to read a screen would claim the tab's geometry and move the
   attachment's view, so a driver must not do it.
5. **The text.** `pane.read { source: "visible" }` gives the frame's text
   with no styles or cursor; `parse_ansi` turns it into a `Grid` with
   `cursor: None`. Every heuristic must therefore have a text-only form,
   with the grid adding the cursor and styles as extra evidence.
6. **The usage screen** (2.1.312): rows `Current session`, `Current week
   (all models)`, on Max and Team plans `Current week (Sonnet only)`, and
   per-model `Current week (<model>)`, each with `<n>% used` and a dim
   `Resets <when>` line. The data is fetched from `/api/oauth/usage` when
   the screen opens, so it can take a moment to fill. It is Claude Code's
   settings dialog, closed by Esc.
7. **Commands today.** `allowed_commands` is an exact list per agent
   (`policy::check`); `allow_arbitrary_commands` never admits a leading
   `/`. A tapped `Answer { sends, command: true }` already sends an
   allowed command (`Input::Suggested`). The app has no list of the
   agent's allowed commands.
8. **The status-line route.** Claude Code passes its status-line command a
   JSON object with `rate_limits.five_hour.used_percentage`,
   `rate_limits.seven_day.used_percentage` and their reset times. Reading
   it would need the owner's `statusLine` setting changed to write a file
   flankr reads, and it updates only while the session is drawn. Named as
   an alternative; not built (decision 12).

## 3. Decisions

### The trait

**1. `ScreenDriver`, beside `SessionReader`** (`src/sessions/drivers/mod.rs`,
new):

```rust
pub trait ScreenDriver: Send + Sync {
    /// The same kind as the agent's reader: `claude_code`.
    fn kind(&self) -> &'static str;
    /// The `ChoiceSet::source`s it answers: `["claude_code/AskUserQuestion"]`.
    fn sources(&self) -> &'static [&'static str];
    /// What the screen shows, by this driver's anchors; `None` when it is
    /// nothing the driver knows.
    fn recognise(&self, screen: &Screen) -> Option<Recognised>;
    /// The steps that give `answers` to `set`, with what the session must
    /// record for each question.
    fn answer(&self, set: &ChoiceSet, answers: &[ChoiceAnswer])
        -> Result<AnswerPlan, PlanError>;
    /// How to read usage, when the agent has a usage screen.
    fn usage(&self) -> Option<UsagePlan> { None }
    /// The usage on `screen`, once `recognise` says it is the usage screen.
    fn read_usage(&self, _screen: &Screen) -> Option<UsageInfo> { None }
    /// Whether `version` (from the session) is one the anchors were
    /// written against; others are driven and logged as untested.
    fn tested(&self, _version: Option<&str>) -> bool { false }
}

pub struct AnswerPlan {
    pub steps: Vec<Step>,
    /// One per question: what its recorded answer must be.
    pub recorded: Vec<Recorded>,
}

pub enum Recorded {
    /// A label or a typed text, compared after trimming.
    Exact(String),
}
```

`answer` is the agreed `answer(&ChoiceSet, &[ChoiceAnswer]) -> Vec<Step>`
with two additions the code needs: the `PlanError` of today's `plan`
(`Invalid`, `Unsupported`), and `recorded`, because typed numbered text
changes what Claude records (decision 7).

**2. The registry.** `Drivers` in `src/sessions/drivers/mod.rs`, held by
`Sessions` next to the readers:

```rust
impl Sessions {
    /// `claude` is Claude Code; any other agent has none (yet).
    pub fn driver_for(&self, agent: &str) -> Option<&dyn ScreenDriver>;
}
```

Codex (`codex`), Gemini CLI and agy (`agy`) return `None` until their
drivers land; each gets a fixture folder now (decision 15) so a driver can
be written against recorded screens.

**3. What a driver recognises** (`#[non_exhaustive]`, inside the daemon):

```rust
pub enum Recognised {
    /// The agent's own input, with whether it is empty.
    Prompt { empty: bool },
    /// The agent is working (its spinner or interrupt hint).
    Busy,
    /// A choice question screen.
    Question(QuestionScreen),
    /// The review before submitting several answers.
    Review,
    /// The usage screen; `loaded` once its first row shows a percentage.
    Usage { loaded: bool },
}

pub struct QuestionScreen {
    /// The question's text as shown, folded (decision 5), first 80 chars.
    pub text: String,
    /// The numbered rows seen, in order.
    pub rows: Vec<RowSeen>,
    /// The focused row's number (the `❯` pointer; with the grid, also the
    /// row under a visible cursor).
    pub focused: Option<u8>,
    /// The Other row, when the question has one.
    pub other: Option<OtherRow>,
    /// Check boxes are drawn: a multi-select question.
    pub multi: bool,
}

pub struct RowSeen { pub number: u8, pub label: String, pub checked: Option<bool> }

pub struct OtherRow {
    pub number: u8,
    /// Its placeholder shows: the input is empty.
    pub empty: bool,
    /// The input has focus: the pointer is on it, and with the grid the
    /// cursor is visible on its row.
    pub focused: bool,
    /// The first 40 characters of its text, folded, when not empty.
    pub text: Option<String>,
}
```

### The screen

**4. One `Screen`, two sources** (`src/sessions/drivers/screen.rs`):

```rust
pub enum ScreenSource { Grid, Text }

pub struct Screen {
    pub source: ScreenSource,
    /// The pane's cells: from the attachment with styles and the cursor,
    /// or `parse_ansi` of `pane.read` text with no cursor.
    pub grid: libflankr_term::Grid,
    /// `Grid::text_lines`, kept.
    pub lines: Vec<String>,
}

impl Screen {
    pub fn from_grid(grid: Grid) -> Screen;
    pub fn from_text(text: &str) -> Screen;
    /// Each line folded: box-drawing characters as spaces, runs of
    /// whitespace as one space (today's `fold`, moved here).
    pub fn folded(&self) -> Vec<String>;
    /// The first row, from the bottom up, whose folded text contains `needle`.
    pub fn find(&self, needle: &str) -> Option<usize>;
    /// The row under the cursor, when the source has one and it is visible.
    pub fn cursor_row(&self) -> Option<usize>;
    /// Whether the run at (`row`, `col`) is dim, bold or inverse; `None`
    /// for text.
    pub fn style_at(&self, row: usize, col: usize) -> Option<RunStyle>;
    /// A hash of rows `rows` (`Grid` derives `Hash`), for settling.
    pub fn block_hash(&self, rows: Range<usize>) -> u64;
}
```

**5. Which source** (`src/server/screen.rs`, new, the daemon's
`PaneIo`, replacing `HerdrPane`):

- **The grid when it is up for the pane:** `HerdrAttach::current()` has a
  view that `shows(pane_id)`, the attachment is not `stale`, and herdr is
  local. The pane is cut with `attached_grid` (moved from
  `src/server/terminal.rs` into `src/server/screen.rs` and shared). Each
  read takes the latest `Published`; a wait wakes on the `watch` channel's
  change, at most every 50 ms.
- **Else the text:** `pane.read { source: "visible" }` (200 lines asked;
  the visible frame comes back), polled every 100 ms.
- The driver **never** opens a viewer, sends `pane.focus` or resizes to get
  the grid (finding 4). A source change in the middle of an answer (a phone
  terminal opened or closed) is allowed: each check reads whichever is up
  and logs it.
- Keys and text go by herdr's JSON API as today: `pane.send_keys` for a key
  and `pane.send_text` (`HerdrRequest::Custom`) for literal text, one call
  each.

```rust
#[async_trait]
pub trait PaneIo: Send + Sync {
    async fn screen(&self) -> Result<Screen, String>;
    /// Resolves when the screen may have changed (the watch for the grid,
    /// a 100 ms tick for text).
    async fn changed(&self);
    async fn key(&self, key: &str) -> Result<(), String>;
    async fn text(&self, text: &str) -> Result<(), String>;
}
```

**6. Settling and checking** (`src/sessions/drivers/run.rs`, today's
`drive` rewritten). A step's check passes only on a **settled** screen:

- the block the driver names (for Claude Code: from the first anchor row it
  found to the screen's last row; otherwise the whole pane) has had the
  same `block_hash` for `SETTLE_QUIET` = 150 ms;
- after a key or text was sent, the first screen counted is one read at
  least 150 ms after the send, and a check that needs a change
  (`Advanced`, `OtherFocused`, `Typed`) does not accept a block equal to
  the one before the send;
- each check waits at most `EXPECT_TIMEOUT` = 2 s (`USAGE_OPEN_TIMEOUT` =
  8 s for the usage screen, which fetches its numbers); a busy screen that
  keeps changing never settles and times out;
- between two sends there are at least `STEP_GAP` = 150 ms, as now.

A check that fails or times out stops the run where it is: nothing more is
sent, the error is `SCREEN_MISMATCH` naming the step ("question 2's Other
row did not take focus"), and the screen is left for the Mac.

```rust
pub enum Step {
    /// Wait for a settled screen that `recognise`s as this.
    Expect(Expect),
    /// One key, by herdr's name. Answers may use only `1`-`9` and `enter`
    /// (checked by the runner, decision 8).
    Key(String),
    /// Literal text, no Enter.
    Text(String),
}

pub enum Expect {
    /// Question `index` (0-based): its text's needle found, the hint shown.
    Question { index: usize, needle: String },
    /// Row `number` of the current question is focused (previews).
    Focused { number: u8 },
    /// The Other row (`number`) has focus and is empty.
    OtherFocused { number: u8 },
    /// The Other row shows the first 40 characters of what was typed.
    Typed { needle: String },
    /// Question `index` is gone: the next question, the review, or the
    /// agent's prompt or busy screen.
    Advanced { index: usize },
    Review,
    Usage,
    Prompt { empty: bool },
}
```

### Answering with two actions

**7. The Claude Code answer table** (`src/sessions/drivers/claude.rs`).
N is a question's option count, so its Other row is `N+1`; k is a 1-based
option number:

| Case | Steps, per question | Recorded |
|---|---|---|
| A. Single-select, option k | `Expect Question`; `k`; `Expect Advanced` | the label |
| B. Single-select, Other text T | `Expect Question`; `N+1`; `Expect OtherFocused`; T; `Expect Typed`; `enter`; `Expect Advanced` | T |
| C. Multi-select, any mix of options and Other | `Expect Question`; `N+1`; `Expect OtherFocused`; the numbered text M; `Expect Typed`; `enter`; `Expect Advanced` | M |
| E. Single-select with previews (no Other row), option k | `Expect Question`; `k`; `Expect Focused { k }`; `enter`; `Expect Advanced` | the label |
| F. After the last question, with two or more questions or one multi-select | `Expect Review`; `1`; `Expect Advanced` | — |

(Case D of the choice questions plan, multi-select with Other, is case C
here.) **The numbered text M** is each picked option as `<k>. <label>`, in
the order picked, joined with `"; "`, then, when there is Other text,
`"; Other: <text>"` (only `Other: <text>` when nothing else was picked):
`1. Red; 3. Blue; Other: warm orange`. M goes through `validate_message`
(one line, no leading `/`, `!` or `#`, at most 4000 characters; a label
starting with one of those is safe because M starts with a digit) and is
typed exactly as checked. Claude records it as the question's free-text
answer, so the result check (today's `compare`) compares the recorded
string with M exactly, and `split_multi` is no longer needed for answers
flankr sends (it stays for reading answers given on the Mac).

A question with only one way to answer and no Other row that is not a
preview question does not occur in Claude Code's tool (finding 2 of the
choice questions plan: every non-preview question has Other); `answer`
refuses one with `PlanError::Unsupported`.

**8. The guard rails of the two actions.**

- The runner refuses any answer step whose key is not `1`-`9` or `enter`,
  whatever the driver asks: no arrows, Tab, Shift+Tab, Esc or Space ever
  reach a question screen. A tenth row (N+1 > 9) cannot occur (at most 4
  options in Claude's tool, so Other is at most `5`); a driver plan with a
  digit above 9 is refused.
- **Multi-select's Other digit** (finding 3). `Expect OtherFocused` needs
  all of: the pointer `❯` on the row numbered `N+1`, the multi-select
  placeholder `Type something` on that row, and with the grid a visible
  cursor on that row. If the digit toggled instead (focus still on row 1,
  or a check mark appeared on the Other row), the check fails and nothing
  is typed.
- **Multi-select's Enter.** `Expect Advanced` after the Enter needs the
  question gone. If the same question is still showing (Enter toggled
  Other instead of going on), the run stops with `SCREEN_MISMATCH` "Claude
  kept question <n> after Enter; finish it on the Mac", the typed text left
  in place for the user at the Mac. Open question 2 asks whether one ↓ may
  be allowed for this case if the S23 check shows it is needed.
- An Other row that already holds text (`OtherRow::empty` false) is refused
  before its digit, since the digit would submit that text (today's rule).
- The first key of an answer is sent only when the screen is a `Question`
  whose needle is question 1's; any other screen (the prompt, busy, a
  permission dialog, another question) refuses the answer before any key.

### Heuristics

**9. Anchor tables, per driver.** Each driver keeps its strings and rules
in one `const` table, so a new client version is a table change with new
fixtures. Claude Code's (`CLAUDE_ANCHORS` in `claude.rs`):

| Screen | Anchors (text route) | Grid adds |
|---|---|---|
| Question | the hint `to select` (`Enter to select · …`); rows `❯ <n>.` / `  <n>.`; the question's needle | the focused row's colour; a visible cursor in the Other input |
| Other row | `Type something.` (single), `Type something` (multi) on row N+1 | the placeholder drawn dim |
| Multi-select | check boxes on the option rows (the client's `checkboxOn` / `checkboxOff` glyphs) | — |
| Review | `Review your answers` and `Ready to submit your answers?` | — |
| Usage | `Current session` and `% used` (`loaded`), or `Current session` alone (loading) | — |
| Busy | the interrupt hint `esc to interrupt` | the spinner's row changing |
| Prompt | the input box between two rules with its `>` prompt; empty when nothing follows the prompt | dim text after the prompt is a placeholder, so still empty |

The strings come from the client (finding 6 and the choice questions
plan's findings); the S23 check (step 7) confirms them on real screens and
records them as fixtures. On the text route a prompt whose line holds any
text counts as **not** empty, since a placeholder cannot be told from a
draft without styles (fail safe for usage, decision 10).

### Usage, on demand

**10. Reading usage.** Only when the owner asks, from the commands menu
(decision 13); never on a timer, on connect or after a turn.

```rust
pub struct UsagePlan {
    /// Sent as a command: `/usage`.
    pub command: &'static str,
    /// The key that closes the usage screen: `escape` for Claude Code.
    pub close_key: &'static str,
    pub open_timeout: Duration, // USAGE_OPEN_TIMEOUT, 8 s
}
```

The run (`src/server/usage.rs`, new):

1. **Permission:** the pane's agent has an `[[agents]]` entry the principal
   reaches, and `UsagePlan::command` is on its `allowed_commands`
   (`policy::check` with the command; `allow_arbitrary_commands` cannot
   admit it). Otherwise `NOT_PERMITTED`: "add /usage to the agent's
   allowed_commands".
2. **Only when idle:** no choice question open (`open_choice`), herdr's
   status not `working` or `blocked`, the turn not `Asking`, and the screen
   recognised as `Prompt { empty: true }`. Otherwise `BUSY` with the
   reason ("Claude is working", "Claude is asking a question", "the prompt
   has a draft"). One run per pane at a time (the `Answering` lock,
   renamed `Driving`, shared with `choice.answer`).
3. **Open:** the command through `run_command`'s path (herdr's
   `pane.send_input { text: "/usage", keys: ["Enter"] }`).
4. **Read:** `Expect Usage` until `loaded` and settled, at most 8 s; then
   `read_usage`. Claude Code's reads each titled row and the next two
   lines: the integer before `% used` and the text after `Resets `.
5. **Close:** `close_key`, sent only once `recognise` says `Usage` (loaded
   or not), so Esc can never reach any other screen; then `Expect Prompt`
   at most 2 s. If the usage screen never appeared, nothing is sent after
   the command and the error says what the screen was ("the usage screen
   did not appear").

**11. `UsageInfo`** (libflankr_proto, additive):

```rust
#[non_exhaustive]
pub struct UsageInfo {
    /// At most USAGE_WINDOWS_MAX (6).
    pub windows: Vec<UsageWindow>,
    /// Milliseconds since it was read, on the daemon's clock, as the
    /// message was made (an age, like `idle_ms`).
    pub age_ms: u64,
    /// `claude_code/usage`.
    pub source: String,
}

#[non_exhaustive]
pub struct UsageWindow {
    /// `five_hour`, `seven_day`, `seven_day_sonnet`, or `model:<name>`.
    pub key: String,
    /// As shown: `Current session`, `Current week (all models)`.
    pub label: String,
    /// 0-100.
    pub used_percent: u8,
    /// As shown after `Resets `, at most USAGE_RESETS_MAX (64) characters.
    pub resets: Option<String>,
}
```

`AgentInfo::usage: Option<UsageInfo>` is appended (defaulted, left off
the wire when `None`). The daemon keeps the last read per pane in memory
(`Daemon::usage`, keyed by pane id and session id), lost on restart, and
gives it on that agent's entry with its age. The phone shows it on the
agent's card (decision 14).

**12. The status-line alternative, not built.** Claude Code's status-line
JSON carries `rate_limits.five_hour.used_percentage` and
`rate_limits.seven_day.used_percentage` with reset times (finding 8). It
needs no screen driving, but it needs the owner's `statusLine` setting
changed and is fresh only while the session is drawn. Left as a later
option; `UsageInfo::source` keeps room for it (`claude_code/status_line`).

### The commands menu

**13. An agent-commands menu in the agent view's input row**
(recommended; open question 1 has the title bar as the alternative).

- **Where:** the message card's row of the bottom bar plan's decision 8
  (the **+** at the left, the keyboard toggle at the right) gains a
  **commands** button just right of the +: a 44 × 44 pt target drawn as a
  `/` in a rounded square, 2 pt stroke, `theme().accent`
  (`AgentHit::PromptCommands`). On the message card only, not Gemini's.
- **The menu:** a tap opens a libcontrols `PopupMenu` anchored to it.
  It asks the daemon `agent.commands` (decision 14) as it opens, showing
  "Reading…" until the answer. Items, in order:
  1. the driver's actions: **Usage** (when the driver has a usage plan and
     its command is allowed);
  2. a separator;
  3. each of the agent's `allowed_commands` as written (`/compact`,
     `continue`), except one a driver action already covers (`/usage`
     is not listed beside **Usage**, since sent raw it would leave the
     usage screen open on the Mac).
  Items the daemon marks unavailable (the agent working or asking) are
  drawn dim with the reason as a second line, and take no taps.
- **Choosing a command** sends it as a tapped allowed command does today
  (`Input::Suggested` with `Answer { sends, command: true }`:
  `agents.send`, kind `command`, confirmed by the tap), under every
  existing rule, including the refusal while a choice question is open.
- **Choosing Usage** sends `agent.usage`; the button shows the drawer's
  progress bar along its bottom while it runs; the result opens a small
  read-only popup at the same anchor (one dim row per window:
  `Session 42% · resets 3pm`, `Week 18% · resets Nov 2, 9am`), and the
  agent's card shows the line from then on (decision 14). A failure shows
  its reason in the same popup.

**14. Protocol and the app's model** (all additive):

- libflankr_proto: `methods::AGENT_COMMANDS` = `agent.commands` and
  `methods::AGENT_USAGE` = `agent.usage`, companions only (others get
  `METHOD_NOT_FOUND`, as for `choice.answer`):

  ```rust
  pub struct AgentCommandsParams { pub pane_id: String }
  pub struct AgentCommandsResult {
      /// The agent's allowed commands, at most AGENT_COMMANDS_MAX (32).
      pub commands: Vec<AgentCommand>,
      /// The driver's actions: `usage`.
      pub actions: Vec<AgentCommand>,
  }
  #[non_exhaustive]
  pub struct AgentCommand {
      /// What is sent (`/compact`), or the action's id (`usage`).
      pub id: String,
      /// The menu's label: the command as written, or `Usage`.
      pub label: String,
      /// Why it cannot be chosen now, when it cannot.
      pub unavailable: Option<String>,
  }
  pub struct AgentUsageParams { pub pane_id: String }
  pub struct AgentUsageResult { pub usage: UsageInfo }
  ```

  Errors: `NOT_PERMITTED`, `BUSY`, `SCREEN_MISMATCH`, `HERDR_UNAVAILABLE`,
  `UNKNOWN_AGENT`; no new codes.
- flankr-companion: `Input::AgentCommands` (for the open view's pane) →
  `Event::AgentCommands { pane_id, commands, actions }`;
  `Input::ReadUsage` → `Event::UsageRead { pane_id, usage:
  Option<UsageInfo>, error: Option<String> }`, the request's timeout at
  least 15 s; the agent's `UsageInfo` from `AgentInfo::usage` kept on its
  view model.
- The app: the menu of decision 13; the agent's session card
  (`session_cards.rs`) and the agent view's title area show one dim line
  `5h 42% · week 18% · 12 min ago` while the daemon has a read.

### Safety

**15. Rules, logging and evidence.**

- **Typing into a screen follows `allow_messages`** (`policy::check_answer`
  and, for typed text, `check_message`), as the choice questions plan's
  decision 6 has it; the tap is the confirmation.
- **Sending a command follows the agent's allowed commands**
  (`policy::check`), the usage command included.
- **Logged:** every step and check, at `info`: the driver, the pane, the
  step (`key "3"`, `text (24 chars)`, `expect OtherFocused(5)`), the
  source (`grid` or `text`), pass or fail and how long it took to settle.
  **Never** the screen's contents, the typed text or the usage numbers.
- **Fixtures:** `src/sessions/drivers/fixtures/<kind>/<name>.grid.json`
  (a serialised `libflankr_term::Grid`) and `<name>.txt` (the `pane.read`
  text of the same screen), recorded in step 7 with made-up questions, the
  usage numbers edited; empty `codex/`, `gemini/` and `agy/` folders with
  a `README` line saying what to record. Every anchor rule has a test over
  both forms of at least one fixture.
- **An S23 check per driver** before it is called confirmed
  (`ScreenDriver::tested` widened to the version checked).

### How the existing code moves

**16. The move**, in steps 2 and 3:

| Today | After |
|---|---|
| `choices::check`, `compare`, `await_result`, `parse_version`, `ChoiceOutcome` use | stay in `src/sessions/choices.rs` (agent-agnostic); `compare` takes `AnswerPlan::recorded` |
| `choices::split_multi`, `quoted_end` | stay, for answers given on the Mac (resolved cards) |
| `choices::plan`, `is_preview`, `needle`, `Step::Question`/`Review` | `src/sessions/drivers/claude.rs` (`ClaudeCodeDriver::answer`, decision 7) |
| `choices::fold`, `unmet`, the anchor strings | `screen.rs` (`fold`) and `CLAUDE_ANCHORS` / `recognise` |
| `choices::Pane`, `Timing`, `drive`, `DriveError` | `src/sessions/drivers/run.rs` (`PaneIo` in its trait form, `Timing` with the settle numbers, `run`) |
| `choices::supported`, `TESTED_FIRST`/`LAST`, `tested` | `ScreenDriver::sources` and `ClaudeCodeDriver::tested` |
| `server/choice.rs`'s `HerdrPane` | `server/screen.rs`'s `HerdrPaneIo` (grid or text, decision 5) |
| `terminal.rs`'s `attached_grid`, `published_grid` | `server/screen.rs`, used by both |
| `server/choice.rs`'s `answer` | unchanged in shape: `driver_for(agent)` instead of `choices::supported`, then `answer`, `run`, the result |

## 4. Steps

Each step is one crate (Rule 3.1) and passes `cargo fmt -p <crate> --
--check`, `cargo clippy -p <crate> --all-targets` with no new warnings and
`cargo test -p <crate>`. Before step 1, two documentation commits: the
amendment to the choice questions plan named in the front matter, and a
`CHANGES.md` addendum announcing this plan's work.

1. **libflankr_proto: usage and agent commands.** Decision 11's
   `UsageInfo`, `UsageWindow`, constants (`USAGE_WINDOWS_MAX` 6,
   `USAGE_RESETS_MAX` 64, `AGENT_COMMANDS_MAX` 32); `AgentInfo::usage`
   appended; decision 14's methods and types; the `VERSIONS.md` entry and
   tag. This commit sets this plan to `In implementation` with its
   amendment.
   **Done when:** round-trip tests pass for an `AgentInfo` with and without
   `usage` (an old entry reads as before, `usage` left off the wire when
   `None`), a `UsageInfo` of three windows with and without `resets`, and
   each params and result type; the method names are tested.
2. **flankr: the screen, the trait and Claude Code's driver.** Decisions
   1-4 and 6-9: `src/sessions/drivers/{mod,screen,run,claude}.rs`,
   `Sessions::driver_for`, the fixtures folders, Claude Code's anchors,
   `recognise`, `answer` and the runner with its settle rule and key
   guard; the code of decision 16 moved, not yet wired to the server.
   **Done when:** fixture tests in both forms (grid and text) recognise a
   single-select question, a multi-select one, its Other row empty, focused
   and typed, a preview question, the review, the usage screen loading and
   loaded, the prompt empty and with a draft, and busy; `answer` gives the
   exact steps for cases A, B, C, E and F (including a three-question mix
   and a single multi-select question) and M for `[0, 2]` plus Other; a
   scripted `PaneIo` shows: a run that matches sends every step; a screen
   that keeps changing times out with nothing more sent; a multi-select
   whose Other digit toggles stops before typing; one whose Enter keeps
   the question stops after it; a step asking for `down` or `escape` is
   refused before anything is sent.
3. **flankr: `choice.answer` through the driver, with the grid.** Decision
   5 and 16: `src/server/screen.rs` (`HerdrPaneIo`, the shared
   `attached_grid`), `server/choice.rs` using `driver_for`, the
   `Driving` lock, `compare` with `recorded`.
   **Done when:** against herdr's mock transport with scripted `pane.read`
   frames, and against libherdr_term's fake server with scripted surfaces
   for the grid route, an answer of each case returns `Answered` when a
   fixture result is appended and `Mismatch` when it differs; the grid is
   used while a viewer shows the pane and the text otherwise, with no
   `pane.focus` or resize sent by a driver; the choice questions plan's
   step-4 tests that still apply pass.
4. **flankr: usage and the commands list.** Decisions 10, 11 and 14's
   daemon half: `src/server/usage.rs`, `agent.commands`, `agent.usage`,
   `Daemon::usage`, `AgentInfo::usage` filled.
   **Done when:** with a scripted pane, `/usage` not allowed is
   `NOT_PERMITTED` with nothing sent; a working agent, an open question or
   a draft in the prompt is `BUSY` with nothing sent; a run sends the
   command, reads the fixture's windows, sends `escape` only after the
   usage screen was recognised, and the next `agents.list` carries the
   usage with its age; a usage screen that never appears sends nothing
   after the command; `agent.commands` lists **Usage** and the allowed
   commands without `/usage`, marked unavailable while the agent works.
5. **flankr-companion: commands and usage in the client.** Decision 14's
   `Input`s and `Event`s, the view model's usage; the `VERSIONS.md` entry
   and tag.
   **Done when:** tests show `Input::AgentCommands` and `Input::ReadUsage`
   send their params for the open view's pane; each result and error gives
   its event; an older daemon (no such method) gives an error event, not a
   hang; `AgentInfo::usage` reaches the view model.
6. **companion: the commands menu and the usage line.** Decision 13 and
   14's app half: the button in the input row, the `PopupMenu`, Usage's
   progress and result popup, the card's line.
   **Done when:** layout tests at 411 pt show the row with +, the commands
   button and the keyboard toggle; the menu lists Usage, a separator and
   the allowed commands; a dim item takes no tap; choosing a command gives
   `Input::Suggested` with `command: true`; choosing Usage gives
   `Input::ReadUsage` and its result popup; the agent's card shows the
   usage line with its age.
7. **The Mac and S23 check for Claude Code.** In a herdr pane on the Mac,
   with and without a phone terminal open (both sources): `AskUserQuestion`
   calls for cases A, B, C (options only, options with Other, Other only),
   E and F, answered from the S23 (adb `timothys-s23-ultra`, after asking
   the owner), and one Usage read. Each screen recorded as a fixture pair;
   findings in `out/drivers/<YYYYMMDD_NN_name>/` and the area's log
   `out/drivers/LOG.md`; a screen that differs from the anchors is a
   `CHANGES.md` addendum and a table change in `claude.rs`.
   **Done when:** every case returns `Answered` with its `tool_result`
   matching, or, for multi-select, stops safely with the reason the
   evidence shows (then open question 2 is put to the owner); the usage
   read gives the windows the Mac shows and leaves the prompt as it was;
   the fixtures are committed; `tested` covers the version checked.

Order: 1; 2 and 3 in that order; 4 after 1 and 3; 5 after 1; 6 after 5;
7 last.

## 5. Versioning (Rule 5)

| Crate | From | To | Why |
|---|---|---|---|
| `libflankr_proto` | 0.5.5 | the next free patch at its commit | additive: a field appended to the plain struct `AgentInfo` (defaulted, left off the wire), new `#[non_exhaustive]` types, two method names and constants; no new error codes |
| `flankr-companion` | 0.5.5 | the next free patch at its commit | additive: variants on the `#[non_exhaustive]` `Input` and `Event`, a field on the view model |
| `libflankr_term`, `libherdr_term`, `libherdr` | 0.1.1, 0.2.0, 0.3.1 | no change | `Grid`, `SurfaceView` and `pane.send_text` as `Custom` are used as they are |
| `libcontrols` | the pin then | no change | `PopupMenu` with dim rows is released |
| `flankr` (daemon), `companion` (app) | — | no bump | applications: manifest and lock only |

## 6. Risks

- **Multi-select may not take the two actions.** If the Other digit
  toggles, or Enter in the input toggles instead of going on, multi-select
  cannot be finished without an arrow. The checks stop safely and step 7
  shows which; open question 2 is the way forward.
- **Numbered text is free text to Claude.** Claude reads `1. Red; 3. Blue`
  rather than structured labels. It is plain to read, and the tool's
  free-text result tells Claude to read the answer carefully; the phone's
  resolved card shows it as recorded.
- **The text route is weaker.** Without styles or the cursor, focus is
  judged by the `❯` pointer alone and a dim placeholder looks like a draft.
  The rules fail safe on it (refuse rather than guess); the grid is used
  whenever a phone terminal already shows the pane.
- **The user at the Mac at the same time.** A key typed on the Mac between
  steps changes the screen; the settle rule and checks catch it, and the
  result check reports what was recorded.
- **A client update** moves strings or layout. The anchors are one table,
  the version is logged, untested versions are driven with the checks, and
  step 7 is repeated with new fixtures.
- **Usage closes with Esc.** Esc on any other screen could reject a
  question or clear a draft; it is sent only after `recognise` says the
  usage screen is up, and usage runs only from an empty, idle prompt.

## 7. Open questions for the owner

1. **The menu's place.** The input row of the agent view's message card
   (decision 13), or a title icon through the plugins plan's icon slots
   (its 22:03 EDT amendment: a registered icon such as `commands`, before
   Settings, which moves into the ⋮ past four)? The row is next to typing,
   where commands belong, but shows only while the message card is open;
   the title bar is always visible but competes with the agent and plans
   icons. Recommended: the input row.
2. **Multi-select if the two actions fall short.** If step 7 shows that
   Claude's multi-select needs its Next button after the typed text, either
   leave multi-select to the Mac (the run stops after typing, the text in
   place), or allow one `down` and `enter` for that case only, after a
   check that the Other row holds the typed text. Recommended: leave it to
   the Mac until the owner chooses, as the plan is written.
3. **First-version agents.** Claude Code only, with the trait and fixture
   folders ready for Codex, Gemini CLI and agy? Recommended: Claude Code
   only.
4. **Usage per pane or per account.** Claude's limits belong to the
   account, so one read could be shown on every Claude pane on the
   machine; but two panes may use different accounts. Recommended: per pane,
   as written, until accounts can be told apart.
5. **The numbered text's form.** `1. Red; 3. Blue; Other: warm orange`
   (labels included, `"; "` between) or numbers only (`1, 3`)? Labels make
   the answer readable in the transcript and on the Mac. Recommended: with
   labels, as written.
6. **The status-line route** (decision 12): leave for later, as written, or
   plan it now as a second, passive usage source? Recommended: later.

## Amendment — 2026-10-29 02:19 EDT: approved with the open questions' recommendations

The owner approved the plan on 2026-10-29 ("approve drivers plan", then
"Both" when asked which). Section 7's questions take their
recommendations: (1) the command menu in the message card's input row;
(2) multi-select left to the Mac if the two actions fall short, until the
owner chooses; (3) Claude Code only in the first version, the trait and
fixture folders ready for the others; (4) usage per pane; (5) the numbered
text with labels, `"; "` between; (6) the status-line route later. The
choice questions plan gets the amendment this plan names when its first
step is committed. The status stays `Draft` until the first implementing
commit (Rule 1.5).
