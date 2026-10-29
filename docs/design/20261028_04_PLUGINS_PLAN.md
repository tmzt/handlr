---
title: Built-in plugins, with a namespaced message flow, a title-bar menu and the planning plugin
date: 2026-10-28
status: In implementation
covers:
  - "A plugin model for flankr: plugins are built-in Rust, each in two
    halves (daemon and device) under one reverse-DNS package name that is
    its namespace and id everywhere; flankr's own plugins are named
    `dev.flankr.plugins.*`"
  - "The daemon half: the `Plugin` trait, the plugin host, and `PluginCtx`,
    which gives a plugin the general agent's read-only operations
    (`list_files`, `read_file`, `recent_commits`, the focused project) with
    the same restrictions, and an allow-listed `run_command` declared by
    the plugin in code; a plugin never sends to an agent pane"
  - "A wrapped, limited JSON-RPC on the companion protocol: `plugin.call`,
    `plugin.result`, `plugin.notify`, `plugin.set_enabled`, and the plugins
    listed in `hello` and `server.capabilities` (additive, libflankr_proto
    0.5.2)"
  - "Versioning a plugin on the daemon side (`ns@semver`, or `ns@<hash>`
    before a release) by the crates' semver rules, and the device half's
    supported range"
  - "Enabling: `[plugins.\"<ns>\"] enabled` in the daemon config, off when
    absent, written by the phone's Settings → Plugins toggle and reloaded
    at run time"
  - "A ⋮ popup menu in the title bar of every screen, on a call-driven
    `PopupMenu` control asked of libcontrols, holding plugin items and some
    existing actions"
  - "A plugin action opening a modal or a card"
  - "The first plugin, `dev.flankr.plugins.planning@0.1.0`: read-only,
    disabled by default, listing a project's plans and showing one plan's
    front matter and amendments, from `read_file` and a small reader for
    the critical rules' format; `tpt` designed in as the example
    allow-listed binary, switched on when its query commands exist"
  - "Not covered: plugins loaded at run time (dynamic libraries, WASM,
    processes); plugins that write files or send to agents; plugin tools
    for the general agent; the `tpt` program itself; anything in
    `~/src/tpt` or `~/src/rules`"
sources:
  - "Owner (2026-10-28, through the lead, verbatim): \"start a new plan for
    a 'plugin' model, but for now, the plugins will be built-in rust. they
    should have a context that allows access to the same operations
    available to the general agent (including read file) with the same
    restrictions, as well as allow a limited set of commands (such as
    tpt). the plugin will contribute a menu item (but we'll need a popup
    menu with actions, maybe in the title bar?) and open a card or modal.
    these plugins will be optional and activated through a Plugins option
    in the settings. the first plugin will be the planning extension, using
    the tpt (critical rules) planning format, disabled by default. discuss
    before writing\""
  - "Owner (2026-10-28, through the lead, verbatim): \"plugins get a
    java/android like package name which is their namespace, and a limited
    json-rpc subset (such as wrapped). the UI component is device-side
    rust, passed the message flow from the namespace.\""
  - "Owner (2026-10-28, through the lead, verbatim): \"it could do either,
    if just using read_file is safer we could do that (and it will need
    markdown and front-matter) or it could be an example of executing a
    local binary. version the plugin on the daemon side (@hash or
    @semver), same rules as our semver. if we use tpt, I'll just tell it's
    agent to build the query-only plan. yes, read-only to start\""
  - "Owner (2026-10-28, through the lead, verbatim), to the lead's four
    recommendations: \"approve and impl in worktree, new branch
    (main-plugins)\""
  - "Owner (2026-10-28, through the lead, verbatim), correcting the
    planning plugin's package name: \"dev.flankr.plugins.planning\""
  - "flankr at 062c15f (branch main-plugins, from main-terminal), read
    2026-10-28: the files cited in section 2"
  - "libcontrols at 0bfd5db (0.10.2), read 2026-10-28, read-only:
    docs/design/20261028_02_LIBCONTROLS_SPEC.md (4.6, 6, 8.5, 10 and the
    amendments of 12:50 and 18:53 EDT)"
  - "tpt at 2d90c27, read 2026-10-28, read-only:
    docs/design/20261027_02_TPT_QUERY_PLAN.md (Draft; decisions 4, 5, 6, 9
    and 12); no code exists yet"
related:
  - docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md (In
    implementation; `list_files`, `read_file`, the `read` patterns and the
    refusals that `PluginCtx` reuses)
  - docs/design/20261027_05_PLANNING_FOLLOWUPS_PLAN.md (In implementation;
    `recent_commits` with the `git` binary, the file watcher, and the
    12:35 EDT amendment that keeps planning conventions out of the general
    agent, which this plan keeps)
  - docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (its living section of
    sections and rows, which gets a Plugins entry)
  - docs/design/20261028_03_SESSION_DRAWER_PLAN.md (In implementation;
    flankr-companion's non-exhaustive `Input` and `Event`, and the title
    bars' ≡ and ‹ slots)
  - ~/src/libcontrols/docs/design/20261028_02_LIBCONTROLS_SPEC.md (the
    living spec; `PopupMenu` is asked of libcontrols by its own plan there)
  - ~/src/tpt/docs/design/20261027_02_TPT_QUERY_PLAN.md (Draft; the query
    commands the planning plugin's command path will run)
---

# Built-in plugins, with a namespaced message flow, a title-bar menu and the planning plugin

## 1. Goal

flankr today has one set of features, all always present. The owner wants
optional features that can be switched on one by one from the phone,
starting with a view of a project's plans.

After this plan:

- flankr has **plugins**: built-in Rust code, compiled into the daemon and
  the companion app, each named by a reverse-DNS package name such as
  `dev.flankr.plugins.planning`. That name is the plugin's namespace on the
  wire, its key in the config and its id in the app.
- every plugin is **off until the owner turns it on** in the phone's
  **Settings → Plugins**, which writes the daemon's config;
- a plugin's daemon half reads the focused project **only as the general
  agent may** (the same `read` patterns, refusals and path checks), may run
  **only the commands it declares** (a binary with fixed subcommands and
  argument shapes, no shell), and can never send anything to an agent;
- a plugin's device half is Rust in the app and sees **only its own
  namespace's messages**;
- every screen's title bar has a **⋮** that opens a small popup menu
  holding the enabled plugins' actions and a few existing actions; an
  action opens a **modal** or a **card**;
- the first plugin, **`dev.flankr.plugins.planning@0.1.0`**, lists the
  focused project's plans under the critical rules' format (status, date,
  title, the README's decision) and shows one plan's front matter and its
  amendments with their exact times. It only reads.

## 2. Findings (read 2026-10-28)

### 2.1 The general agent's read-only operations

- **The project's files.** `src/project.rs` holds `ProjectFiles` (`:28`),
  opened on a canonical root with the `read` patterns (`:62`). A path may
  be read when it is not `always_refused` (`:54`: any segment `.private`,
  `.git`, `.env`, `.env.*`, `.`, `..` or empty) and matches a pattern
  (`allowed`, `:76`). `list` (`:82`) walks with `walkdir` without
  following links, skips `.git`, `target` and `node_modules` (`:22`),
  and returns at most `MAX_LISTED` = 500 sorted paths (`:18`). `read`
  (`:117`) refuses a path by name, canonicalises it, refuses it again by
  its real name, requires a regular file and reads at most
  `MAX_READ_BYTES` = 64 KiB of UTF-8 (`:20`). Contents are never logged
  (`:9-10`).
- **The patterns.** `ReadPattern` (`src/llm/planning.rs:25`, `parse` at
  `:41`, `matches` at `:72`) is the limited syntax of plan 20261027_04's
  11:20 EDT amendment, item 4 (`**` as a whole segment, `*` within one,
  everything else literal). `PlanningConfig` (`:114`) holds the valid
  patterns from `[llm.uses.planning] read` (`from_use`, `:124`); empty
  turns the file tools off (`file_tools`, `:146`). There are no built-in
  patterns (plan 20261027_04, 11:22 EDT amendment); `config.default.toml`
  suggests `["*.md", "docs/**/*.md"]`.
- **The executor.** `HerdrToolExecutor` (`src/transport.rs`) opens the
  current project in `current_project` (`:126`) and finds its folder in
  `current_folder` (`:142`): refused over SSH ("flankr reads only its
  own"), refused with no current agent ("No project is open: open a
  session, or focus an agent first"), else the agent's `cwd` from herdr's
  snapshot. `execute_recent_commits` (`:168`), `execute_list_files`
  (`:233`) and `execute_read_file` (`:264`) wrap them.
- **`recent_commits`.** `src/git.rs`: the `git` binary with a fixed
  argument list, no shell, `GIT_CONFIG_NOSYSTEM=1`,
  `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`, stdin closed and a 5 s
  `TIMEOUT` (`:5-6`, `:25`); `DEFAULT_COUNT` 20, `MAX_COUNT` 50,
  `MAX_FILES` 50 (`:19-23`); the path filter is checked by `check_path`
  (`:92`), which calls `crate::commands::check_paths` (`src/commands.rs:244`:
  every path resolved against the root by its text and on disk, bounded
  by `MAX_PATHS`, `MAX_PATH_LEN`, `MAX_PATH_PARTS`, `MAX_PARENT_PARTS`).
  `recent_commits` itself is at `:165`.
- **The focused project.** A companion conversation's `Session` keeps
  `current_agent` (`src/server/companion.rs:165`), set from
  `turn.start`'s `session` when the phone has an agent view open
  (`:494-497`) or by `focus_project`.
- **Who may reach what.** `Principal` (`src/core/principal.rs:5`) has
  `can_approve` (`:12`) and `may_reach(agent)` (`:26`).
- **No planning knowledge in the general agent.** Plan 20261027_05's 12:35
  EDT amendment dropped `list_plans`: "flankr knows nothing of the owner's
  planning conventions", and a hook for external tools was "left to a
  later plan". This plan is that later plan's first part, kept apart from
  the general agent: the conventions live only in an optional plugin that
  is off by default, and the general agent gets no plugin tools.

### 2.2 The protocol and the client

- **The envelope.** `libflankr_proto` is JSON-RPC 2.0, one object per line,
  serde alone (`crates/libflankr_proto/src/lib.rs:1-13`). Error codes are
  in `envelope.rs:138-183` (`NOT_PERMITTED` -32001 … `BUSY` -32020).
- **`hello` and capabilities.** The companion's `HelloResult`
  (`crates/libflankr_proto/src/companion.rs:51`) has optional `llm` and
  `terminal` appended with `#[serde(default)]`; `ServerCapabilities`
  (`:133`) repeats them and is sent whenever they change, from
  `forward_events` (`src/server/conn.rs:1432-1460`). Appending a field to
  these structs is additive by the owner's ruling (a patch bump).
- **The method table.** Companion requests are dispatched by name in
  `src/server/conn.rs` (`:842-1268`); `hello` is at `:524`.
- **Versions.** libflankr_proto 0.5.1 and flankr-companion 0.5.0
  (`VERSIONS.md:560`, `:574`); tags are crate-prefixed
  (`libflankr_proto-v0.5.1`).
- **flankr-companion.** `Input` (`crates/flankr-companion/src/client.rs:34-35`)
  and `Event` (`:519-520`) are `#[non_exhaustive]` since 0.5.0 (plan
  20261028_03, 18:48 EDT amendment, item 6), so new variants are a patch
  bump.

### 2.3 The app

- **Title bars.** The overview's title has the gear at its right
  (`apps/companion/src/overview.rs:21`, placed at `:345`); the agent view
  has a `header` and a back glyph at its left
  (`apps/companion/src/agent.rs:1977`, `:1985`); the drawer plan puts a ≡
  on both and moves ‹ one slot right (its 18:48 EDT amendment, item 1).
- **Settings.** `Section` (`apps/companion/src/settings/screen.rs:122`) is
  Sound, Voice, Display, Background, Notifications, Battery, Remotes,
  Device; copy is in `apps/companion/copy/SETTINGS_COPY.md` (read by
  `src/settings/copy.rs`). The sections and rows are a living section of
  plan 20261024_02 (its 13:03 EDT amendment).
- **Modals.** The remote details modal is a libcontrols `ModalDialog`
  (`apps/companion/src/settings/remote_details.rs:239`); `ui::modal_layout`
  (`apps/companion/src/ui.rs:481`) sizes a modal's text.
- **Cards.** The overview's agent cards are a grid
  (`apps/companion/src/overview/card_grid.rs`); other sessions' cards sit
  at the bottom (`apps/companion/src/session_cards.rs:1-14`).
- **Read aloud** is the agent view's play button (`AgentHit::ReadAloud`,
  `apps/companion/src/agent.rs:305`, handled at `src/lib.rs:1662-1672`).

### 2.4 libcontrols (read-only)

- At 0.10.2 (`0bfd5db`). It has `ModalDialog` and the modal stack
  (spec 4.6), `Drawer`/`DrawerLayout` driven by calls only (spec
  amendment of 12:50 EDT, 6.14), `EditableLabel` (18:53 EDT, 6.15), and no
  popup or menu control.
- The theming standard (spec 8.5): defaults in `ControlTheme.x: XTheme`
  (`#[non_exhaustive]`, `Default`), overrides in `XOptions` (`Option`s,
  builder setters), resolved by `XOptions::with_theme(&XTheme) ->
  XResolved`; appending these is additive (8.5, item 4).

### 2.5 tpt (read-only)

- `~/src/tpt` has plans only (`2d90c27`); its query plan is `Draft`.
- The query commands (its decision 9): `tpt plans list [--status S] …
  [--json]`, `tpt plans show <name> [--body] [--json]`, `tpt plans times
  <name> [--json]`, `tpt changes list`, `tpt check [--json]`; common flags
  `-C <dir>`, `--time-offset`, `--json`. It finds the root by walking up to
  `.git` and never runs `git` (decision 4).
- Its parser (decision 5): the file name
  `^(\d{8})_(\d{2})_([A-Z0-9]+(?:_[A-Z0-9]+)*)\.md$`; the front matter
  between `---` lines, a fixed YAML subset; amendments outside fences as
  `^## Amendment — (\d{4}-\d{2}-\d{2} \d{2}:\d{2} <zone>): (.+)$`.
- Its JSON (decision 12): one object with `"schema": 1`; a plan has
  `file`, `base`, `date_from_name`, `nn`, `name`, `kind`, `title`, `date`,
  `status`, `status_base`, `status_by`, `covers`, `sources`, `related`,
  `living_sections`, `headings`, `amendments` (`index`, `line`,
  `timestamp`, `subject`, `living_section`), `last_amendment`, `readme`
  (`row`, `status`, `decision`), `error`.

## 3. Decisions

### 3.1 Plugins, their names and their two halves

1. **Built-in Rust, two halves under one name.** A plugin is compiled into
   the daemon (its daemon half) and into the companion app (its device
   half). Nothing is loaded at run time. Both halves are named by the same
   **package name**, a reverse-DNS name in the Java/Android style:
   lowercase ASCII segments `[a-z][a-z0-9_]*` joined by `.`, at least three
   segments, at most 128 bytes. It is the plugin's **namespace** (`ns`) on
   the wire, its key under `[plugins]` in the config, and its id in the
   app.
2. **flankr's own plugins are `dev.flankr.plugins.<name>`.** The first is
   `dev.flankr.plugins.planning`. A name outside `dev.flankr.plugins.*` is
   reserved for later (third-party built-ins) and refused by the registry
   in this plan.
3. **The daemon half** is a `Plugin` (`src/plugins/mod.rs`):

   ```rust
   #[async_trait::async_trait]
   pub trait Plugin: Send + Sync + 'static {
       /// The package name, e.g. "dev.flankr.plugins.planning".
       fn ns(&self) -> &'static str;
       /// Shown in Settings → Plugins and as the menu group.
       fn name(&self) -> &'static str;
       /// One sentence for Settings → Plugins.
       fn description(&self) -> &'static str;
       fn version(&self) -> PluginVersion;
       /// The actions it adds to the ⋮ menu.
       fn menu(&self) -> &'static [MenuItem];
       /// The project paths it needs, in ReadPattern syntax; reads are
       /// allowed only where these AND the configured `read` match.
       fn read_patterns(&self) -> &'static [&'static str];
       /// The commands it may run (3.3, item 3); empty for none.
       fn commands(&self) -> &'static [CommandSpec];
       /// One plugin-local method. `params` is the call's JSON.
       async fn call(
           &self,
           method: &str,
           params: serde_json::Value,
           ctx: &PluginCtx,
       ) -> Result<serde_json::Value, PluginError>;
   }

   pub enum PluginVersion { Semver(&'static str), Hash(&'static str) }

   pub struct MenuItem {
       pub id: &'static str,        // "plans", unique within the plugin
       pub label: &'static str,     // "Plans in this project"
       pub opens: Opens,            // Modal or Card
       pub screens: Screens,        // Overview, Agent, or Both
   }

   pub struct PluginError { pub code: i64, pub message: String }
   ```

   The registry is a fixed list built at start
   (`src/plugins/registry.rs`: `pub fn builtin() -> Vec<Arc<dyn Plugin>>`).
   It checks every name (item 1, 2) and that names are unique, and panics
   in tests (not at run time: a bad entry is logged and left out).
4. **The device half** is a `DevicePlugin`
   (`apps/companion/src/plugins/mod.rs`):

   ```rust
   pub trait DevicePlugin {
       fn ns(&self) -> &'static str;
       /// The daemon versions it can talk to (3.4, item 3).
       fn supports(&self) -> &'static str;     // "0.1"
       /// A menu item was chosen; it may send calls and open a view.
       fn action(&mut self, item: &str, out: &mut PluginOut);
       /// A result or notification for its namespace.
       fn message(&mut self, msg: PluginMessage, out: &mut PluginOut);
       /// The modal's or card's content, drawn in the given rect.
       fn draw(&self, view: ViewKind, rect: Rect, ctx: &DrawContext,
               list: &mut DrawList);
       fn tap(&mut self, view: ViewKind, x: f32, y: f32,
              out: &mut PluginOut);
       /// Back or the modal's close.
       fn close(&mut self, view: ViewKind);
   }
   ```

   `PluginOut` collects what it asks of the app: `call(method, params)`
   (the app fills `ns` and the id), `open(ViewKind::Modal | Card(key))`,
   `close`, `redraw`. The app routes each `Event::PluginResult` and
   `Event::PluginNotify` to the device half whose `ns` it names, and to no
   other; a device half cannot name another namespace, since `PluginOut`
   has no `ns`.

### 3.2 The wrapped JSON-RPC

1. **Three new companion methods and notifications** (in
   `libflankr_proto::companion::methods`):
   - `plugin.call` (request, device → daemon), params
     `PluginCall { ns: String, id: Option<u64>, method: String, params:
     serde_json::Value, session: Option<String> }`. The JSON-RPC response
     comes at once: `PluginAccepted {}` when the call was handed to the
     plugin, or an error (item 3). With `id`, the plugin's answer follows
     as `plugin.result`; without, it is a notification to the plugin and
     nothing follows. `session` is the pane id of the agent view the phone
     has open, if any (3.3, item 2).
   - `plugin.result` (notification, daemon → device):
     `PluginResult { ns, id: u64, result: Option<Value>, error:
     Option<ErrorObject> }`, exactly one of `result` and `error`.
   - `plugin.notify` (notification, daemon → device): `PluginNotify { ns,
     method: String, params: Value }`.
   The inner `id` is the device's, unique within the connection and the
   namespace; the outer JSON-RPC id belongs to the connection as for every
   request. `params` (in the crate) is `serde_json::Value`, so
   libflankr_proto gains `serde_json` as a dependency (it is already in
   every user's graph).
2. **The subset.** Requests, results and notifications only: no batches, no
   nested `plugin.*` calls, no access to any host method (a plugin's
   `method` is looked up only among its own). `method` is
   `[a-z][a-z0-9_.]*`, at most 64 bytes. `params`, `result` and a
   notification's `params` are plain JSON values; each serialised object
   is at most **256 KiB** (`PLUGIN_MAX_BYTES = 262_144`). A plugin's
   result over the cap is replaced by an error `PLUGIN_TOO_LARGE`; a call
   over the cap is refused at once. At most 4 calls per connection run at
   once (others: `BUSY`); a call runs at most 10 s (then
   `plugin.result` with `CANCELLED`).
3. **Errors answered at once** to `plugin.call`, as new codes appended in
   `envelope.rs` `codes`: `PLUGIN_UNKNOWN` (-32030) for a namespace not in
   the registry; `PLUGIN_DISABLED` (-32031) for one that is not enabled;
   `PLUGIN_TOO_LARGE` (-32032); `INVALID_PARAMS` for a bad `method` or
   `ns`; `BUSY`. A plugin's own errors come in `plugin.result.error` with
   the plugin's code (plugins use -32100 … -32199) or `INVALID_PARAMS`.
4. **Isolation.** The daemon sends `plugin.result` and `plugin.notify`
   only on the connection that called, tagged with the plugin's own `ns`;
   a plugin is given no handle to other plugins or other connections. In
   0.1.0 a plugin sends a notification only during a call, through
   `PluginCtx::notify(method, params)` (progress); a standing subscription
   is left to a later plan.
5. **The plugins in `hello` and `server.capabilities`.** Both get
   `plugins: Option<Vec<PluginInfo>>`, appended with `#[serde(default,
   skip_serializing_if = "Option::is_none")]` (`None` from an older
   daemon: no plugins):

   ```rust
   pub struct PluginInfo {
       pub ns: String,
       pub name: String,
       pub description: String,
       pub version: String,       // "0.1.0" or "g1a2b3c4"
       pub enabled: bool,
       #[serde(default)]
       pub menu: Vec<PluginMenuItem>,
       /// Why it can't be used now, e.g. "No project is open".
       #[serde(default, skip_serializing_if = "Option::is_none")]
       pub reason: Option<String>,
   }
   pub struct PluginMenuItem {
       pub id: String,
       pub label: String,
       pub opens: String,         // "modal" | "card"
       pub screens: String,       // "overview" | "agent" | "both"
   }
   ```

   Every built-in plugin is listed, enabled or not, so Settings → Plugins
   can show it. `server.capabilities` is sent when the list changes (a
   toggle, a config reload).
6. **`plugin.set_enabled`** (request): `{ ns, enabled: bool }` →
   `{ enabled: bool }`. Allowed only for a principal with `can_approve`
   (else `MAY_NOT_APPROVE`); an unknown `ns` is `PLUGIN_UNKNOWN`. It
   writes the config (3.5) and answers when the write is done.
7. **Versions.** All of item 1-6 are appended types, constants, fields
   and methods: **libflankr_proto 0.5.2**, a patch bump.

### 3.3 `PluginCtx`

1. **What a plugin may do**, and nothing else (`src/plugins/ctx.rs`):

   ```rust
   impl PluginCtx {
       pub fn project(&self) -> Result<&Project, PluginError>; // root, agent name
       pub async fn list_files(&self, glob: Option<&str>) -> Result<Listing, PluginError>;
       pub async fn read_file(&self, path: &str) -> Result<FileText, PluginError>;
       pub async fn recent_commits(&self, count: Option<i64>, path: Option<&str>,
                                   files: bool) -> Result<History, PluginError>;
       pub async fn run_command(&self, name: &str, args: &[&str])
                                -> Result<CommandOutput, PluginError>;
       pub fn notify(&self, method: &str, params: serde_json::Value);
       pub fn config(&self) -> &toml::Table;   // its own [plugins."<ns>"] table
   }
   ```

   There is no method that sends keys, messages or commands to a pane,
   focuses herdr, starts a turn or reads another plugin's state.
2. **The focused project** is, in order: the call's `session` (the agent
   view open on the phone), else the conversation's `current_agent`
   (2.1), else none, and every file and git operation is refused with
   today's message "No project is open: open a session, or focus an
   agent first". The agent must be one the principal `may_reach`, and
   its folder is found as `current_folder` finds it, including the SSH
   refusal. That lookup moves out of `HerdrToolExecutor` into
   `src/project.rs` (`pub async fn agent_folder(client, pane_id) ->
   Result<String, String>`) so both use one function.
3. **The same restrictions.**
   - `list_files` and `read_file` are `ProjectFiles::list` and
     `ProjectFiles::read`, unchanged (500 paths, 64 KiB, the refusals,
     the symlink checks), opened with the patterns that match **both** the
     configured `[llm.uses.planning] read` and the plugin's
     `read_patterns()`: a path is allowed only when one configured pattern
     and one plugin pattern match it. The configured `read` is read
     whether or not `[llm]` or `planning` is enabled: it is the owner's
     statement of what flankr may read. Empty, every read is refused with
     "Reading files is off: [llm.uses.planning] has no read patterns."
   - `recent_commits` is `crate::git::recent_commits` with `check_path`,
     the same clamps and environment.
   - Each call is logged at info with the plugin, project and path, never
     the contents, as the general agent's are.
4. **`run_command`, the allow-list declared in code.** A plugin's
   `commands()` lists `CommandSpec`s:

   ```rust
   pub struct CommandSpec {
       pub name: &'static str,          // "plans_list"
       pub binary: &'static str,        // "tpt", found on PATH at start
       pub fixed: &'static [&'static str],    // ["plans", "list", "--json"]
       pub args: &'static [ArgShape],   // the caller's arguments, in order
       pub root_flag: Option<&'static str>,   // Some("-C"): the project root
       pub max_output: usize,           // ≤ PLUGIN_MAX_BYTES
   }
   pub enum ArgShape { PlanName, Literal(&'static [&'static str]) }
   ```

   - `run_command(name, args)` runs `binary`, then `root_flag` and the
     project's canonical root when set, then `fixed`, then each argument
     checked against its `ArgShape` (`PlanName`: `^\d{8}_\d{2}(_[A-Z0-9]+)+$`;
     `Literal`: one of the listed words). Anything else is refused before
     a process starts.
   - The process: no shell, `Command::new` with an argument vector; stdin
     closed; the working directory the project root; the environment
     cleared except `PATH`, `HOME`, `LC_ALL=C` and `TZ` (if set); a 5 s
     timeout, then killed; stdout read up to `max_output` (more is an
     error), stderr up to 4 KiB for the error message only; a non-zero
     exit is an error carrying the first line of stderr.
   - A binary not on `PATH` at start makes that command unavailable, logged
     once; the plugin is told by the error `COMMAND_UNAVAILABLE` (-32101).
   - **The config may only narrow:** `[plugins."<ns>"] commands = false`
     switches every command off; `commands = ["plans_list"]` keeps only
     those named. A name the plugin does not declare is logged as ignored.
     The config can never add a binary, a subcommand or an argument shape.
5. **A plugin never sends to an agent pane.** This holds by construction
   (item 1), and a test asserts that `PluginCtx` has no herdr client
   capable of writes: it holds only a read-only `ProjectLookup` (the
   snapshot's `cwd`s) built from the herdr client.

### 3.4 Versions of a plugin

1. **Declared by the daemon half.** `PluginVersion::Semver("0.1.0")` for a
   released plugin; `PluginVersion::Hash("1a2b3c4")` while a plugin has
   not had a release, written `ns@g1a2b3c4` (the `g` marks a git hash and
   keeps it from reading as a number). `PluginInfo.version` carries the
   text without `ns@`.
2. **The crates' semver rules apply** (AGENTS.md Rule 5.4, as the owner
   said: "same rules as our semver"). The plugin's public API is its
   methods, their params and results, its notifications and its menu items'
   ids. While `0.x`, a change that is not strictly additive (a method or
   field removed or renamed, a shape changed, behaviour the device half can
   observe changing) bumps the minor version; an additive one bumps the
   patch. Each version change gets a `VERSIONS.md` entry headed
   `## dev.flankr.plugins.planning 0.1.0 — <date> (the commit tagged
   `dev.flankr.plugins.planning-v0.1.0`)`, with the tag made as for the
   crates. A hash version gets no entry.
3. **The device half's range.** `supports()` is a version requirement in
   Cargo's caret meaning: `"0.1"` accepts `0.1.x`; `"1"` accepts `1.x.y`.
   The app reads it with the `semver` crate (1.0.28, already in
   `Cargo.lock`; a new direct dependency of the app only). A hash version
   is accepted only when the range is exactly `"@g<hash>"` for that hash.
   A plugin outside the range, or with a device half missing (an older
   app), is shown in Settings → Plugins as "Needs a newer app" (or "Needs
   a newer flankr") with its toggle off and disabled, and has no menu
   items.

### 3.5 Enabling

1. **The config.** In the daemon's `config.toml`:

   ```toml
   [plugins."dev.flankr.plugins.planning"]
   enabled = true          # absent or false: off
   # source = "read_file"  # or "tpt" (3.8, item 6)
   # commands = false      # narrow or switch off the declared commands
   ```

   A plugin with no table is off. `config.default.toml` gets this table
   commented out, with one line saying what the plugin does.
2. **Reload.** The plugin host re-reads `[plugins]` whenever the file
   watcher (`src/watch_files.rs`) reports the config file changed, as
   `[llm]` is (plan 20261027_05, decision 2). A change applies to the next
   call; a call already running finishes. Each connection is then sent
   `server.capabilities` with the new `plugins`.
3. **The phone's toggle writes it.** `plugin.set_enabled` edits only
   `plugins."<ns>".enabled` with `toml_edit` (0.22, already in
   `Cargo.lock` under `toml` 0.8; a new direct dependency of the daemon),
   keeping the rest of the file as written, comments included. The write
   is to a temporary file in the same folder, then renamed over the config
   (mode kept). A missing config file is created with only that table. A
   config file that does not parse is not written; the call fails with
   `INTERNAL_ERROR` "The config file has an error; fix it first."
4. **Who may.** Only a principal with `can_approve` (3.2, item 6). The
   Settings toggle is shown disabled with "Only a device that can approve
   may change this" for one that can't.

### 3.6 The ⋮ popup menu

1. **Where.** A ⋮ at the right of the title bar on every screen that has
   one: the overview (left of the gear), the agent view (at the header's
   right), and the settings screen (at its title's right). It is a
   42 pt square tap target with the glyph at the theme's text colour,
   drawn dim and taking no taps while the menu would be empty.
2. **What it holds**, top to bottom: the enabled plugins' items for this
   screen (`screens`), grouped under each plugin's `name` as a small
   heading when more than one plugin has items; a separator; the app's own
   items. The app's own items in this plan: **Read aloud** (agent view
   only, a check mark when on; the same as the play button) and
   **Settings** (overview and agent view). Others are added later by
   amendment to this plan.
3. **The control, asked of libcontrols: `PopupMenu`.** A small call-driven
   control, following libcontrols' style (no gestures of its own, like the
   `Drawer`) and its theming standard (spec 8.5). The interface flankr
   needs:

   ```rust
   pub struct PopupMenu { pub state: Arc<PopupMenuState>, pub rect: Rect, /* options */ }
   pub struct PopupItem { pub id: Symbol, pub label: String, pub enabled: bool,
                          pub checked: Option<bool>, pub kind: PopupItemKind }
   #[non_exhaustive] pub enum PopupItemKind { Action, Heading, Separator }
   impl PopupMenuState {
       pub fn open(&self, anchor: Rect, items: Vec<PopupItem>);
       pub fn close(&self);
       pub fn is_open(&self) -> bool;
       pub fn take_events(&self) -> Vec<PopupMenuEvent>;
   }
   #[non_exhaustive] pub enum PopupMenuEvent { Chosen(Symbol), Dismissed }
   // ControlTheme.popup_menu: PopupMenuTheme (#[non_exhaustive], Default):
   //   bg, border, corner_radius, item_height, padding, min_width,
   //   max_width, separator_color, heading_color, disabled_alpha, shadow
   // PopupMenuOptions (Options of the same, builder setters) and
   // PopupMenuOptions::with_theme(&PopupMenuTheme) -> PopupMenuResolved
   ```

   - It opens below its anchor, right-aligned to it, and flips above or
     shifts left to stay inside the viewport; it is pushed on the modal
     stack as a modal with no backdrop, dismissed by an outside tap,
     Escape or Back (`DismissOn`), so it takes input above the screen.
   - A press and release on an enabled `Action` gives `Chosen` and closes
     it; `Heading` and `Separator` take no input; keys Up/Down move a
     highlight and Enter chooses, for desktop builds.
   - The app reads `take_events` each frame and acts.
   This is a libcontrols change, built in libcontrols by its own plan there
   (a patch release, **libcontrols 0.10.3**, by spec 8.5 item 4). flankr
   does not edit libcontrols; step 4 asks for it with this interface.
4. **The app's side.** One `PopupMenu` in `apps/companion/src/menu.rs`,
   shared by the screens. The ⋮ tap builds the items from the current
   `PluginInfo` list and the screen, calls `open`, and on `Chosen(id)`
   either runs the app's action or calls the device half's `action(item)`.
   The ids are `"<ns>/<item>"` for plugins and `"app/read_aloud"`,
   `"app/settings"` for the app's own.

### 3.7 Cards and modals

1. **Each menu item chooses** `opens: modal` or `opens: card`. A modal is a
   libcontrols `ModalDialog` centred as the remote details modal is
   (2.3), with the plugin's name as its title, a close (X), and the device
   half drawing its content; Back closes it. A card is a full-width card
   at the end of the overview's grid, below the agent cards, with the
   plugin's name as its header; a tap calls the device half's `tap`, a
   long press (2 s, as the drawer's remote cards) offers **Unpin**.
2. **Pinning from a modal.** A device half may offer a **Pin** button in
   its modal; it calls `PluginOut::open(ViewKind::Card(key))`, which adds
   a card keyed by `(ns, key)` (at most 4 plugin cards; the oldest goes).
   Pinned cards last for the app's run and are per remote; keeping them
   across restarts is left to a later amendment.
3. **A disabled plugin's views close** at once when `server.capabilities`
   says it is off, and its cards are removed.

### 3.8 The planning plugin, `dev.flankr.plugins.planning@0.1.0`

1. **Name and version.** `dev.flankr.plugins.planning`, version
   `Semver("0.1.0")` from its first commit; disabled by default (no
   config table). Name "Planning", description "Lists this project's
   plans and their amendments, from docs/design. Read-only."
2. **Read-only.** It declares no write, and `PluginCtx` offers none.
   `read_patterns()`: `["README.md", "CHANGES.md", "docs/design/*.md"]`.
   The configured `read` must also match (3.3, item 3); with the
   template's `["*.md", "docs/**/*.md"]` it does.
3. **Menu items.**
   - **Plans in this project** (`plans`, opens a modal, both screens).
   - **Show plan** is not a menu item: it is a row's tap in the modal
     above. The menu has one item in 0.1.0; **Check** is added, as an
     additive patch release (`0.1.x`), when tpt's `check` exists (item 6).
4. **Methods** (plugin-local), with tpt's JSON shapes (2.5) so the two
   sources answer alike:
   - `plans.list {}` → `{ "schema": 1, "root": <project name>, "plans":
     [ { "file", "base", "date_from_name", "nn", "name", "kind", "title",
     "date", "status", "status_base", "status_by", "last_amendment",
     "readme": { "status", "decision" } | null, "error" } ] }`, oldest
     first (date, then `nn`), at most 500 (`list_files`' cap).
     `status_by.ref` only (no `file`/`exists`: the reader resolves
     nothing outside the root).
   - `plans.show { "name": <base name> }` → `{ "schema": 1, "plan": { …as
     above, plus "covers", "sources", "related", "living_sections",
     "headings": [{ "line", "text" }], "amendments": [{ "index", "line",
     "timestamp", "subject", "living_section" }] } }`. `name` must be a
     base name matching `ArgShape::PlanName`; the file is
     `docs/design/<name>.md`.
   - Errors: `-32110` "No docs/design folder in this project", `-32111`
     "No plan named …", a reader error as `error` on the plan, not the
     call.
5. **The reader** (`src/plugins/planning/reader.rs`), no new dependency,
   following tpt's decision 5 exactly so both read the same files the same
   way:
   - the file-name pattern; plans end in `_PLAN`, others are specs;
   - the front matter between `---` lines, the fixed YAML subset (plain or
     double-quoted scalars that may span lines, lists of `- ` items
     continued on lines indented more, `key: []`, `#` comment lines);
     known keys kept, others in `extra`;
   - level-2 headings and amendment headings outside fenced code blocks
     (```` ``` ```` or `~~~`), the amendment regex of 2.5 with `—` U+2014,
     a subject ending `(living section <X>)` naming a living section;
     timestamps kept as written, never re-computed;
   - `README.md`'s `## Architectural decisions` table: each row's
     Document link target matched to a plan's file, giving its Status and
     Decision cells (a Markdown table split on unescaped `|`); a Status
     cell that differs from the front matter is reported as it is, both
     kept, and the device shows the front matter's.
   - A file over 64 KiB is read cut, as `read_file` cuts it, and its plan
     gets `error: "cut at 64 KiB"` when the cut falls before the last
     amendment heading could be seen.
   - The Markdown bodies are not rendered or returned; only titles,
     headings and subjects.
6. **The command path, designed in with tpt.** `commands()` declares, from
   0.1.0:

   ```rust
   CommandSpec { name: "plans_list", binary: "tpt", root_flag: Some("-C"),
                 fixed: &["plans", "list", "--json"], args: &[], max_output: 256 * 1024 },
   CommandSpec { name: "plans_show", binary: "tpt", root_flag: Some("-C"),
                 fixed: &["plans", "show", "--json"], args: &[ArgShape::PlanName],
                 max_output: 256 * 1024 },
   CommandSpec { name: "check", binary: "tpt", root_flag: Some("-C"),
                 fixed: &["check", "--json"], args: &[], max_output: 256 * 1024 },
   ```

   (`plans show` takes the name after `--json`; tpt accepts flags before or
   after positional arguments, its decision 3.) `[plugins."<ns>"] source`
   chooses: `"read_file"` (the default) uses the reader; `"tpt"` runs the
   commands and passes their JSON through after checking `"schema": 1`,
   falling back to the reader with a logged warning when `tpt` is missing
   or fails. The `tpt` source stays undocumented in the template, and the
   **Check** item is not offered, until tpt's query commands exist; turning
   them on is a patch release of the plugin with an amendment here. The
   owner has said he will ask tpt's agent to build the query plan; nothing
   in `~/src/tpt` or `~/src/rules` is changed by this plan.
7. **The device half** (`apps/companion/src/plugins/planning.rs`),
   `supports() = "0.1"`:
   - **Plans in this project** opens the modal and calls `plans.list`;
     the modal shows "Reading plans…", then one row per plan, newest at
     the top: its title (one line, cut with …), and under it the date and
     the status as written in the front matter, with "(README: <status>)"
     when the README differs. The list scrolls (a libcontrols
     `ScrollPane`). A tap on a row calls
     `plans.show` and replaces the list with the plan.
   - **The plan** shows its title, date and status, its `covers` as a
     short list, then **Amendments**: one line each, the exact timestamp
     as written (`2026-10-28 18:48 EDT`) and the subject. A ‹ returns to
     the list; **Pin** pins it as a card.
   - **The pinned card** shows the plan's title, its status and its last
     amendment's timestamp and subject; a tap opens the modal on that
     plan, calling `plans.show` again so it is current.
   - Text uses the app's Markdown-free label drawing; no file content is
     shown beyond these fields.

### 3.9 The Settings → Plugins section

1. A **Plugins** section between Remotes and Device. One row per plugin in
   the active remote's `PluginInfo` list: its name, its version under it
   in the small style (`0.1.0`), and a toggle. An ⓘ shows the
   description. A plugin the app can't show (3.4, item 3) has its toggle
   disabled and the reason on its second line.
2. A toggle sends `Input::SetPluginEnabled { ns, enabled }`; the row shows
   a spinner until `server.capabilities` (or the call's error, shown as a
   notice) settles it.
3. With no plugins listed (an older daemon), the section shows one line:
   "This flankr has no plugins."
4. The copy goes in `apps/companion/copy/SETTINGS_COPY.md`. The settings
   plan's living section gets an entry for the section when step 6 is
   built (by amendment to plan 20261024_02, written then).

### 3.10 flankr-companion

1. **Inputs** (appended): `PluginCall { ns: String, id: Option<u64>,
   method: String, params: serde_json::Value, session: Option<String> }`
   and `SetPluginEnabled { ns: String, enabled: bool }`.
2. **Events** (appended): `Plugins(Vec<PluginInfo>)` after `hello` and on
   each `server.capabilities` that carries `plugins`;
   `PluginResult(PluginResult)` and `PluginNotify(PluginNotify)`; a
   `plugin.call` refused at once is given as a `PluginResult` with that
   error, so the device half sees one path.
3. Calls to an inactive remote are refused in the client (the drawer
   plan's connection manager keeps them connected, but plugins act on the
   active remote only).
4. **Versions:** new variants of non-exhaustive enums: **flankr-companion
   0.5.1**, a patch bump; it takes libflankr_proto 0.5.2.

## 4. Dependencies and order

1. **libflankr_proto 0.5.2** first (step 1): every other step needs its
   types.
2. **The daemon's plugin host and the planning plugin's daemon half**
   (steps 2 and 3) need step 1 only.
3. **libcontrols' `PopupMenu`** (step 4) is built in libcontrols by its own
   plan, written and implemented there by its owner's agent; flankr only
   asks for it with 3.6's interface. Steps 5 and 6 can be built without it
   (the menu falls back to opening on a test hook); step 7 needs it. The
   app also needs a libcontrols revision with `ModalDialog` and
   `ScrollPane` (present at the pinned `52e6c07`) and whatever revision
   the drawer plan has moved it to by then; this plan moves it no
   further than the `PopupMenu` release.
4. **flankr-companion 0.5.1** (step 5) needs step 1.
5. **The app** (steps 6-8) needs steps 4 and 5, and the drawer plan's
   title-bar changes (≡ and ‹), so the ⋮ is placed against them.
6. tpt is not needed; its command path is off until tpt's query commands
   exist.

## 5. Testing

- **libflankr_proto (host):** round trips of `PluginCall` (with and without
  `id` and `session`), `PluginAccepted`, `PluginResult` (result; error),
  `PluginNotify`, `PluginInfo`, `plugin.set_enabled`; `HelloResult` and
  `ServerCapabilities` from an older daemon (no `plugins`) and with an
  empty and a full list; the new codes.
- **The daemon (host):**
  - the registry: a bad name, a duplicate, a name outside
    `dev.flankr.plugins.*` left out;
  - the host: an unknown `ns` (`PLUGIN_UNKNOWN`), a disabled one
    (`PLUGIN_DISABLED`), a bad `method`, a call over 256 KiB, a result
    over 256 KiB, a fifth concurrent call (`BUSY`), a call past 10 s
    (`CANCELLED`), a notification with no `id` giving no result, results
    only on the calling connection and only with its `ns` (a test plugin
    under `dev.flankr.plugins.test`, compiled in tests only);
  - `PluginCtx`: no project (the message), `session` winning over
    `current_agent`, an agent the principal may not reach, SSH refused;
    `read_file` refused where the configured `read` matches but the
    plugin's patterns don't, and the reverse; `.private`, `.env`, `..`,
    a symlink out, refused; the 64 KiB cut; `list_files`' 500 cap;
    `recent_commits` on a temporary repository;
  - `run_command`, with a stub binary on a temporary `PATH`: the argument
    vector exactly `stub -C <root> plans show --json 20261028_04_PLUGINS_PLAN`;
    a bad plan name, an extra argument, a shell metacharacter refused
    before running; the cleared environment; the timeout; output over the
    cap; a non-zero exit; a missing binary; `commands = false` and a
    narrowing list; a config naming an undeclared command ignored;
  - enabling: `toml_edit` keeping comments and other tables; a missing
    file created; a broken file not written; the reload sending
    `server.capabilities`; `plugin.set_enabled` refused without
    `can_approve`;
  - the planning reader, on this repository's own `docs/design/` and
    `README.md` (copied into a temporary folder at a fixed commit) and on
    hand-made files: every plan listed with the front matter's status, the
    README's status and decision matched by link, multi-line quoted
    scalars, `key: []`, amendments inside a fenced block ignored, a
    living-section subject, a cut file, a file with no front matter as
    `error`; `plans.list` and `plans.show` shapes against tpt's decision
    12.
- **flankr-companion (host):** the inputs sent as `plugin.call` and
  `plugin.set_enabled`; `Event::Plugins` from `hello` and capabilities;
  a refused call as `PluginResult` with the error; calls to an inactive
  remote refused.
- **The app (host):** the device-half router (a message reaching only its
  `ns`); `supports` against `0.1.0`, `0.2.0`, `g1a2b3c4` and
  `@g1a2b3c4`; the menu's items per screen and plugin state; the planning
  modal's rows from a fixture list; the Plugins section's rows and
  disabled states.
- **On the S23** (`timothys-s23-ultra`, asked of the owner before each
  run): turn the planning plugin on in Settings → Plugins and see the
  config change on the Mac; open ⋮ on the overview and an agent view; open
  **Plans in this project** in flankr's own project and see this plan
  listed as it is in the README; open it and see its amendments with their
  exact times; pin it and see the card; turn the plugin off and see the
  modal, card and menu item go. Evidence under
  `out/plugins/<YYYYMMDD_NN_name>/` with its log `out/plugins/LOG.md`.

## 6. Steps

1. **libflankr_proto 0.5.2: the plugin envelope.** In
   `crates/libflankr_proto/src/companion.rs`: the method names, `PluginCall`,
   `PluginAccepted`, `PluginResult`, `PluginNotify`, `PluginInfo`,
   `PluginMenuItem`, `PluginSetEnabledParams`/`Result`, `PLUGIN_MAX_BYTES`;
   `plugins` appended to `HelloResult` and `ServerCapabilities`; in
   `envelope.rs`, `PLUGIN_UNKNOWN`, `PLUGIN_DISABLED`, `PLUGIN_TOO_LARGE`;
   `serde_json` as a dependency. `VERSIONS.md` entry, tag
   `libflankr_proto-v0.5.2`. The plan's status becomes `In implementation`
   in this commit.
   **Done when:** the crate's tests (section 5) pass, and an older
   `HelloResult` without `plugins` still reads.
2. **The daemon's plugin host.** `src/plugins/{mod.rs,registry.rs,ctx.rs,command.rs,host.rs}`:
   the `Plugin` trait, the registry, `PluginCtx` over the moved
   `project::agent_folder`, `ProjectFiles` and `git`, `run_command`,
   `[plugins]` read and reloaded on the watcher's tick, `plugin.call`,
   `plugin.set_enabled` (with `toml_edit`) and `plugins` in `hello` and
   `server.capabilities` in `src/server/conn.rs`; the commented table in
   `config.default.toml` (its own commit, Rule 3.1).
   **Done when:** the daemon's host tests (section 5, all but the reader)
   pass with the test plugin, and `flankr ctl` (or a test client) sees an
   empty-but-listed plugin set in `hello`.
3. **The planning plugin's daemon half.** `src/plugins/planning/{mod.rs,reader.rs}`:
   `plans.list`, `plans.show`, the reader, the declared tpt commands with
   `source = "read_file"` the only one switched on; registered in the
   registry; its `VERSIONS.md` entry for `dev.flankr.plugins.planning
   0.1.0` and tag `dev.flankr.plugins.planning-v0.1.0`.
   **Done when:** the reader's tests pass on the fixture copy of this
   repository's plans, and a test client calling `plans.list` on this
   worktree, with `enabled = true` and the template's `read`, lists every
   plan of the README's table with matching statuses.
4. **libcontrols `PopupMenu`, asked of libcontrols.** The lead gives
   libcontrols' agent section 3.6, item 3, as the requested interface; it
   is planned, built and released (0.10.3) in libcontrols under that
   repository's rules. Nothing here edits `~/src/libcontrols`.
   **Done when:** a tagged libcontrols release has `PopupMenu` with that
   interface (or an agreed variant, recorded in `CHANGES.md` here), and
   its spec has the amendment for it.
5. **flankr-companion 0.5.1.** The inputs and events of 3.10, on
   libflankr_proto 0.5.2. `VERSIONS.md` entry, tag
   `flankr-companion-v0.5.1`.
   **Done when:** its tests (section 5) pass.
6. **The app: device halves and Settings → Plugins.**
   `apps/companion/src/plugins/{mod.rs,planning.rs}`: `DevicePlugin`, the
   router by `ns`, the version check with `semver`, the planning device
   half's modal, plan view and pinned card; the Plugins section and its
   copy.
   **Done when:** the app's host tests pass, and with the menu replaced by
   a test hook the planning modal fills from a running daemon on the
   desktop build.
7. **The app: the ⋮ menu.** `apps/companion/src/menu.rs` on libcontrols'
   `PopupMenu`, the ⋮ on the overview, agent view and settings title bars,
   Read aloud and Settings as app items; the app moved to the libcontrols
   release of step 4.
   **Done when:** on the desktop build, ⋮ opens on each screen with the
   right items, Read aloud matches the play button, and choosing the
   planning item opens its modal.
8. **On the S23.** The checks of section 5, asked of the owner first;
   evidence logged.
   **Done when:** each check passes on the phone, and the plan's status
   becomes `Implemented` by amendment listing each step's commits.

## 7. Risks

- **Writing the owner's config.** A phone toggle editing `config.toml`
  could clash with a hand edit in progress. Mitigations: only one key is
  touched, through `toml_edit`, written by rename; a file that doesn't
  parse is never written; the reload logs what changed.
- **Two parsers of one format.** The reader and tpt may disagree on an
  edge case. The reader follows tpt's decision 5 to the letter and uses
  tpt's JSON shapes; when tpt exists, a test runs both on the same fixture
  and compares.
- **Coupling plugins to `[llm.uses.planning] read`.** Plugins read only
  where the general agent may, as the owner asked, so an owner who has not
  set `read` sees "Reading files is off" in the planning modal. The
  message names the key.
- **The menu control lives in another repository.** Step 7 waits on
  libcontrols; steps 1-3, 5 and 6 do not.
- **A frozen general-agent decision.** Plan 20261027_05's 12:35 EDT
  amendment keeps planning conventions out of the general agent. This plan
  does not change that: the general agent gets no plugin tools, and the
  conventions are only in an optional plugin.
- **Message flow growth.** The 256 KiB cap and 4 concurrent calls keep a
  plugin from flooding the link; `plans.list` for 50 plans is about 25 KiB.

## 8. Points this document settled

The owner approved the design; these details were not in the discussion
and were settled here. Each holds unless the owner amends it before the
first code commit:

1. Plugin names outside `dev.flankr.plugins.*` are refused for now.
2. A plugin reads where the configured `[llm.uses.planning] read` and its
   own declared patterns both match, with `read` honoured even while
   `[llm]` is off (rather than a `read` key of its own under
   `[plugins."<ns>"]`).
3. `plugin.call` is answered at once; the plugin's answer is a separate
   `plugin.result`, matched by an inner `id`.
4. Pinned plugin cards last for the app's run only.
5. A plugin's version changes are recorded in the root `VERSIONS.md` and
   tagged `<ns>-v<version>`, as the crates are.
6. The ⋮ is on the overview, agent view and settings screens; the app's
   own items in this plan are Read aloud and Settings.
7. The planning plugin's methods use tpt's JSON shapes (schema 1), so the
   `tpt` source can replace the reader without changing the device half.

## Amendment — 2026-10-28 19:47 EDT: approved; implementation on main-plugins

The owner (2026-10-28), to the lead's four recommendations: "approve and
impl in worktree, new branch (main-plugins)"; and, correcting the planning
plugin's package name: "dev.flankr.plugins.planning", which the body uses
throughout, with `dev.flankr.plugins.*` as the convention for flankr's own
plugins (3.1, item 2).

The plan is approved as written, with the points of section 8. It is built
in the worktree `.claude/worktrees/main-plugins` on the branch
`main-plugins`, from `main-terminal` at `062c15f`. The status stays
`Draft` until the first code commit (step 1, libflankr_proto 0.5.2), which
sets it to `In implementation` and appends the amendment saying the
document is now immutable.

## Amendment — 2026-10-28 20:11 EDT: implementation started; the document is now immutable

Implementation starts with this amendment, in the commit of step 1
(libflankr_proto 0.5.2) on `main-plugins`: `status` is `In implementation`,
and the body above is frozen (AGENTS.md Rule 1.5). Departures are recorded
in `CHANGES.md`.

## Amendment — 2026-10-28 22:03 EDT: title-bar icons, with Settings moving into the ⋮ past four

On the owner's explicit instruction (Rule 1.5), 2026-10-28: "if more than
four title icons are registered Settings moves to extra menu. agent menu,
plan menu (reuse todo icon for now), etc. extra menu is always last";
then, on which icon, "the todo list icon used in the terminal/transcript
switch", then "it could use the checkbox instead"; then "yes" to this
wording. This adds to decision 3.6: its item 1 (where the ⋮ sits) and
item 2's **Settings** item are replaced as below; the rest of 3.6, and the
⋮'s menu itself, stand.

1. **Title icons are registered.** The app and the plugins register
   title-bar icons for a screen. Each has an id, a glyph, a label (used
   when it moves into the ⋮ menu), the screens it shows on (overview,
   agent view, settings) and what a tap does (opens its own popup menu, a
   modal or a card). The drawer's ≡ (plan 20261028_03) is not one of them
   and is not counted.
2. **Order, left to right, ⋮ always last.** The registered icons in this
   order, then the ⋮:
   - the **agent menu**, the ⊙ of the subagents plan
     (docs/design/20261023_07_SUBAGENTS_PLAN.md, its 20:46 and 20:51 EDT
     amendments), on the agent view while the pane has subagents;
   - the **plans menu**, the checkbox ☑ (U+2611, from the terminal face,
     already in its symbol page), registered by the planning plugin while
     it is enabled, on the overview and the agent view;
   - other plugins' icons, in the registry's order;
   - **Settings**, the gear, on the overview and the agent view.
   Each icon is a 42 pt square target, in the title bar's text colour, as
   the ⋮ is (3.6 item 1).
3. **At most four icons before the ⋮.** When more than four are
   registered for a screen, Settings leaves the title bar and becomes the
   ⋮ menu's last item (after a separator). If more than four still
   remain, the last plugin icons, in reverse registry order, follow it into
   the menu as items under their labels, above Settings. While Settings is
   in the title bar it is not also in the ⋮ menu (this replaces 3.6 item
   2's **Settings** item, which is now in the menu only when moved there).
   The ⋮ shows on every screen, dim while its menu would be empty.
4. **A plugin's icon, by name.** `PluginMenuItem` gains `icon:
   Option<String>`, appended, defaulted and left off the wire when `None`
   (libflankr_proto, a patch release), naming one of the app's own icons;
   an item with an icon is a title icon, one without is a ⋮ item as
   before. A plugin cannot send its own glyph; an unknown name is drawn as
   the item's first letter in a ring. Names in this amendment: `plans`
   (☑). `opens` gains `"menu"`: the icon opens a `PopupMenu` of items the
   device half gives (an app unaware of `"menu"` opens it as `"modal"`).
5. **The planning plugin's icon** (its 0.1.1, additive): its `plans` item
   gains `icon: "plans"` and `opens: "menu"`. A tap opens a popup menu,
   "Reading plans…" as a heading until `plans.list` answers, then the ten
   newest plans by title with their status as a dim second word, and a
   last item **All plans…** that opens today's modal; choosing a plan opens
   the modal on that plan (`plans.show`). The device half gains
   `DevicePlugin::menu(item) -> Vec<PopupItem>`, called again on each
   answer while the menu is open.
6. **Steps.** These are built after step 8 as a follow-up: libflankr_proto
   (`icon`), the daemon's planning half (0.1.1), flankr-companion if its
   types change, and the app (the icon slots, the overflow, `opens:
   "menu"`). Departures are recorded in CHANGES.md as usual.

## Amendment — 2026-10-28 23:14 EDT: the plan menu is a table, by last update

The owner (2026-10-28): "plan menu should use a table view, plan index
(the nn part) and mm/dd date, but sorted by update time"; then "yes" to
this wording. This replaces the 22:03 EDT amendment's item 5 on the
popup's rows ("the ten newest plans by title with their status as a dim
second word") and CHANGES.md's departure 2 of that build (the status
after an en space); the rest of item 5 (Reading plans…, All plans…,
choosing a plan) stands.

1. **A table.** Each row of the ☑ popup is one plan in four columns: its
   **NN** (the sequence within its day, from the file name), its date as
   **mm/dd** (from the file name), its **title** (cut with … to the
   column's width), and its **status** in the dim colour (`Draft`,
   `In implementation`, `Implemented`, `Hold`, or the word before
   "(by" for `Superseded` and `Retired`). NN and mm/dd are right-aligned
   in fixed-width columns, so the rows line up.
2. **Sorted by last update**, newest first: a plan's update time is its
   newest amendment's timestamp (`last_amendment`), else its front-matter
   `date`, else the date in its file name. Ten rows, then **All plans…**.
3. **libcontrols.** Columns in the popup need an additive `PopupMenu`
   change: an item may carry cells (text, alignment, a fixed or flexible
   width, dim or not) instead of one label, laid out in shared columns
   across the menu. It is asked of libcontrols by a plan of its own there,
   to be written; until it is released the rows are one label,
   `NN mm/dd title · status` with NN and mm/dd zero-padded, sorted as in
   item 2.
4. **The modal's list** (All plans…) uses the same columns and order.

## Amendment — 2026-10-28 23:14 EDT: the plan table's statuses abbreviated

The owner (2026-10-28): "but abbrv status (Impl, Sucd, Draft)"; then, to
the lead's table of abbreviations, "Succd". This replaces the previous
amendment's item 1 on the status column (the full status, and "the word
before (by"): the column shows the status abbreviated, in the dim colour:

| Status | Shown |
|---|---|
| `Draft` | Draft |
| `Hold` | Hold |
| `In implementation` | InImpl |
| `Implemented` | Impl |
| `Superseded (by …)` | Succd |
| `Retired (by …)` | Rtrd |

Any other status is cut to its first five letters. The modal's list
(item 4) uses the same abbreviations.

## Amendment — 2026-10-28 23:15 EDT: the status column at 0.8 of the row's text size

The owner (2026-10-28): "and 0.8 fontsize", of the plan table's
abbreviated statuses. Adds to the previous amendment: the status column
is drawn at 0.8 of the row's text size, in the dim colour, vertically
centred on the row; the other columns keep the row's size. The same holds
in the modal's list.
