---
title: A settings screen, from a gear on the overview
date: 2026-10-24
status: In implementation
covers:
  - "A gear at the right of the overview's header opening a settings
    screen, in sections, on libcontrols 0.8.0's scroll pane"
  - "Sound: all sound on or off, and a ding when an agent is done"
  - "Voice: all microphone input on or off, and the general agent
    (Gemini) on or off, each disabling its UI throughout the app"
  - "Battery: Android's battery optimization page, and the vendor's
    battery page (Samsung's app sleeping) when the phone has one"
  - "Unpair this device and delete the app's data, in red, last, each
    behind a confirming modal; a `link.unpair` method so the daemon
    forgets the device too"
  - "Not covered: the foreground service's own notification controls
    (libmobile/libmorph); per-agent settings; settings for the desktop
    build beyond what applies there"
sources:
  - "Owner (2026-10-24): \"plan a settings screen from a gear in the right
    on overview. add Battery Settings (Android) Battery Settings (Samsung)
    (or vendor, if supported) Unpair device (red text) and delete data
    (red text) (both with modal confirm) as last option, write new plan.
    also include toggle for all sound, ding on done, toggle for all mic
    input, general agent (Gemini). organize in sections with basic
    libcontrols scrollpane\"; \"audio/mic toggles disable corresponding ui
    throughout app\""
  - "libmobile 0.2.3 (`2c50e9d`), `src/battery.rs` and
    `src/android/battery.rs`: `state() -> BatteryState
    { ignoring_optimizations, background_restricted, manufacturer }`,
    `BatteryState::is_samsung`, `open_settings()`, `open_app_details()`,
    `open_vendor_battery() -> Result<Page, BatteryError>`; reached through
    libmorph's `service::battery` wrapper (libmorph plan 20261023_01,
    approved, not yet built)"
  - "libcontrols 0.8.0 (`a7d0fd9`), `src/scroll.rs`: `ScrollPane::new(
    viewport, axis)`, `with_state`, `with_content_size`, `with_indicator`,
    `ScrollState`. The app pins libcontrols `5cad90f` today"
  - "handlr at `7b28958`: `apps/companion/src/{lib.rs, overview.rs,
    controls/}`; the app's `read_aloud`; `ctx.data_dir()`; the pairing
    screen (`ui.rs`) and `pair.txt`; the daemon's paired clients in
    `~/.config/handlr/auth.toml` (`src/server/link/mod.rs`); the link
    methods `link.pair_begin`, `link.pair_add`, `link.pair_done`"
related:
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (the overview's header)
  - docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md (read aloud, which "all sound" silences)
  - docs/design/20261023_08_SIGNED_PAIRING_PLAN.md (pairing; unpairing undoes it)
  - docs/design/20261023_09_HANDLR_DATA_PLAN.md (the phone's store, which "delete data" clears)
---

# A settings screen, from a gear on the overview

## 1. Goal

The companion has no settings. Sound, the microphone and Gemini are on or
off only through scattered controls, the battery pages that keep the link
alive have to be found by hand, and a paired phone can only be unpaired by
editing the daemon's `auth.toml`. After this plan, a gear on the overview
opens one settings screen that holds all of it, in sections, with the
destructive actions last and confirmed.

## 2. Decisions

1. **The gear.** A gear icon at the right end of the overview's header, a
   44 pt touch target, in the header's dim colour. Tapping it opens the
   settings screen. The header's "‹" and the system Back close it, back
   to the overview.
2. **The screen** is one vertical libcontrols `ScrollPane` (0.8.0) holding
   sections, each a small dim caps title and its rows. Rows are 52 pt
   high: a label on the left, and a toggle or a "›" on the right. The app
   moves to libcontrols 0.8.0 for this (step 1); the agent view's own
   scrolling is not changed by this plan.
3. **The sections and rows, in order:**
   - **Sound**
     - *All sound* (toggle, on): off silences everything the app plays,
       reading aloud and dings included, and disables the sound UI
       everywhere (decision 4).
     - *Ding when an agent is done* (toggle, on): a short chime when a
       Finished card appears for any agent, foreground or not. Dim and
       inactive while *All sound* is off.
   - **Voice**
     - *Microphone* (toggle, on): off stops all microphone input: no wake
       or dictation listening runs, and the microphone UI is disabled
       everywhere (decision 4).
     - *General agent (Gemini)* (toggle, on): off sends no turns to
       Gemini. The overview's Gemini strip and overlay are hidden, and
       with the microphone on, a recording goes as a message to the agent
       in view, or does nothing on the overview. Sent to the daemon in
       `client.options` (decision 7), so it also stops spoken summaries.
   - **Battery** (Android only; hidden elsewhere)
     - *Battery optimization* ("›", subtitle "Exempt" or "Optimized" from
       `BatteryState.ignoring_optimizations`): opens Android's page,
       `open_settings()`, or `request_ignore_optimizations()` when not yet
       exempt.
     - *<Vendor> battery* ("›"): shown when the phone has a vendor page.
       On Samsung it reads *Samsung battery (app sleeping)* and opens
       `open_vendor_battery()`. If that call answers that no vendor page
       exists, the row is hidden from then on.
     - A dim note row under them when `background_restricted` is true:
       "Battery use is Restricted: handlr can't stay connected in the
       background".
   - **Device** (last)
     - *Unpair this device* (red text): a modal, "Unpair this phone from
       <daemon name>? You will need to pair again to use handlr.",
       buttons *Cancel* and *Unpair* (red). Unpair calls `link.unpair`
       (decision 6), deletes the phone's link key and pairing record from
       the key store, and shows the pairing screen. If the daemon is
       offline, the phone forgets the pairing anyway, and the modal says
       so first: "handlr is offline: it will still list this phone until
       it is removed from auth.toml".
     - *Delete data* (red text): a modal, "Delete handlr's data on this
       phone? Settings, stored scrollback and pairing are removed. Voice
       models stay.", buttons *Cancel* and *Delete* (red). Delete unpairs
       as above, then removes everything in the app's data directory
       except the voice models, and restarts the app on the pairing
       screen.
4. **A toggle that is off disables its UI throughout the app.** Disabled
   controls are drawn at 35% opacity, take no taps, and show nothing
   "live". Each control reads the setting, so no screen needs its own
   switch.
   - **All sound off:** the agent view's play (read aloud) button and its
     skip state, the "speaking" indicator, and any spoken-summary or ding
     control. Nothing is queued to be read while it is off.
   - **Microphone off:** the record button on the overview and in the
     agent view, a card's reply (message) button, the dictation card,
     and the voice level and "listening" indicators. Typed input, where
     the app has it, still works. A card's Yes and No still work.
   - **General agent off:** the overview's Gemini strip and overlay are
     hidden, not just dimmed, because there is nothing of Gemini's to
     show.
5. **Settings are stored** in `settings.toml` in the app's data directory
   (`ctx.data_dir()`), written when a toggle changes, read at start.
   Missing or unreadable means the defaults above. Desktop builds keep the
   same file in their data directory and hide the Battery section.
6. **`link.unpair`** (libhandlr_proto, a new method; no params, result
   `{}`): over a paired link, the daemon removes the calling device's
   entry from `auth.toml` and closes the link. Not offered on the local
   socket. A patch release, by the owner's rulings.
7. **`client.options` gains `general_agent: bool`** (default true,
   appended, so a patch release with decision 6): while false, the daemon
   starts no Gemini turn and makes no spoken summary for this client.
8. **Controls, one per file** under `apps/companion/src/settings/`:
   `screen.rs` (the screen and its sections), `toggle_row.rs`,
   `link_row.rs` (a "›" row), `danger_row.rs` (red text), `confirm.rs`
   (the modal). The gear is `overview/gear.rs`.

## 3. Steps

1. **libcontrols 0.8.0.** The app's pin moves to `a7d0fd9`, with the
   lock file, building against it without changing what any screen does.
   **Done when:** the app builds for desktop and Android and its tests
   pass unchanged.
2. **The protocol:** `link.unpair` and `ClientOptionsParams.general_agent`,
   libhandlr_proto's next patch release with its VERSIONS.md entry.
   **Done when:** both round-trip, and options without the field read as
   `general_agent: true`.
3. **The daemon:** `link.unpair` removes the device from `auth.toml` and
   closes its link; `general_agent: false` stops Gemini turns and spoken
   summaries for that client.
   **Done when:** daemon tests unpair a paired test device (its key no
   longer connects), refuse `link.unpair` on the local socket, and show
   no Gemini turn or summary for a client with `general_agent: false`.
4. **The client:** inputs for the settings (`Input::Settings(Settings)`),
   sound and microphone gates, `general_agent` in `client.options`,
   `Input::Unpair` (calls `link.unpair`, then forgets the pairing), and a
   ding event on a new Finished card.
   **Done when:** client tests show each toggle's effect, the ding once per
   Finished card, and unpairing online and offline.
5. **The app:** the gear, the screen, the rows, the modals, `settings.toml`,
   the battery rows through libmorph's battery wrapper, and delete data.
   **Done when:** app tests place the gear and the sections in order, open
   and cancel each modal, show every control of decision 4 disabled
   (drawn dim, taking no taps) with its toggle off and the Gemini strip
   hidden, read and write `settings.toml`, and hide the
   Battery section off Android; and on the S23, the gear opens the screen,
   both battery pages open, *Ding when an agent is done* chimes, and
   *Unpair this device* returns to the pairing screen and the daemon's
   `auth.toml` no longer lists the phone (the owner re-pairs after).

Each step is one crate or subproject, one commit (Rule 3.1). 1 needs
nothing; 2 needs nothing; 3 needs 2; 4 needs 2; 5 needs 1 and 4, and its
battery rows need libmorph's battery wrapper (libmorph plan 20261023_01,
step 5). Until that is released, the Battery section is built and tested
behind a stub and shown as "Battery settings need an app update".

## 4. Open questions for the owner

1. **Delete data and the voice models:** keep the models (about the size
   of the speech model pushed by `push-model.sh`), as written, or delete
   them too?
2. **The ding:** a short built-in chime, or the system notification
   sound?
3. **General agent off, on the overview:** should the record button do
   nothing (as written), or send to the agent herdr has in focus?

## Amendment — 2026-10-24 11:32 EDT: a Display section with the scrollbar's visibility

The owner (2026-10-24): "we should have a Scrollbar visibility with the
arrow-based option selector in libcontrols (left/right arrows select
options) Disabled, Hidden, Always"; then "yes". This adds to decision 3,
between Voice and Battery:
- **Display**
  - *Scrollbar*: libcontrols' `ArrowLabel` (‹ Hidden ›). The arrows step
    through *Disabled*, *Hidden* (default) and *Always*, wrapping at both
    ends. Its meaning is in plan 20261022_03's L3 amendment of the same
    time.

It is saved in `settings.toml` (decision 5) and its control lives in
`settings/option_row.rs`. Step 5's tests also step the arrows both ways
and keep the value.

## Amendment — 2026-10-24 11:37 EDT: the general agent toggle stops user-directed Gemini turns only

The owner (2026-10-24): "for now, turning off general agent stops all
user-directed Gemini turns (summaries are the daemon's concern)", with the
agent split planned in plan 20261024_03 (the general agent as a realtime
chat). This replaces, in decision 3, "so it also stops spoken summaries",
and in decision 7, "and makes no spoken summary for this client":
`general_agent: false` stops only the Gemini turns the user starts. Spoken
summaries go on as the daemon decides.

It also answers open question 3: with the general agent off, the record
button sends to the agent in view as a message, and on the overview it is
disabled, drawn dim (decision 4). Once plan 20261024_03 is built, the same
toggle hides its centre button and refuses `chat.open`.

## Amendment — 2026-10-24 11:46 EDT: approved, to be built now

The owner approved building the plan on 2026-10-24 ("Do the basic
settings screen"). Open questions 1 and 2 take these values until the
owner says otherwise:
- *Delete data* keeps the voice models;
- the ding is a short built-in chime, made by the app's own audio
  output, not the system notification sound.

Open question 3 was answered by the 11:3x EDT amendment. The battery rows
are built behind the stub of step 5 until libmorph's battery wrapper is
released. The status becomes `In implementation` with step 1's commit.

## Amendment — 2026-10-24 11:47 EDT: a correction to the previous amendment's reference

The previous amendment names "the 11:3x EDT amendment" for open question
3. It means the amendment of 2026-10-24 11:37 EDT, "the general agent
toggle stops user-directed Gemini turns only".

## Amendment — 2026-10-24 11:49 EDT: implementation has started; the document is now immutable

Implementation has started with step 1 (the app's move to libcontrols
0.8.0, `a7d0fd9`). The status is `In implementation`, and from here the
document changes only on the owner's explicit instruction (Rule 1.5).

## Amendment — 2026-10-24 12:12 EDT: Clear cache, in the Device section

The owner (2026-10-24): "add a Clear cache (n MB) black text in the same
section of settings, no confirm, just cached sessions not models". This
adds a row to decision 3's Device section, before *Unpair this device*:
- *Clear cache (n MB)*, in the normal text colour (not red), with no
  confirmation. A tap empties the cached sessions: the closed views kept
  in memory (plan 20261022_03 L3, the amendment of the same time), and,
  once plan 20261023_09 is built, the stored scrollback. Voice models,
  settings and pairing are untouched. *n* is the cache's size in MB, one
  decimal under 10. It is shown as "Clear cache (0 MB)" and does nothing
  when empty.

The row is built with the session cache, in the full-text build after
this plan's steps.

## Amendment — 2026-10-27 11:27 EDT: Tab follows focus, in the Voice section

The owner (2026-10-27), on plan 20261027_04: "the focus was meant to be
split, not change herdr's focused tab, but that would be an option in the
settings tab_follows_focus"; then "the tab_follows_focus part should also be
added to the appropriate plan". This adds a row to decision 3's Voice
section, after *General agent (Gemini)*:
- *Tab follows focus* (toggle, off): on, when Gemini focuses a project
  (`focus_project`, plan 20261027_04), herdr's focused tab on the desktop
  moves to that agent's tab too; off, Gemini's focus stays its own and the
  desktop is left alone. Dim and inactive while *General agent (Gemini)* is
  off.
- Sent to the daemon in `client.options` as `tab_follows_focus: bool`
  (default false), a field appended to `ClientOptionsParams` beside
  `general_agent` (decision 7), in `libflankr_proto` 0.3.11.
- Its copy is `[Copy] tab_follows_focus` in `SETTINGS_COPY.md`, naming
  herdr's tab focus and the `focus` tool it does not affect.

The row is built by plan 20261027_04's step 6, once that plan is approved.

## Amendment — 2026-10-28 13:03 EDT: the sections and rows become living; Remotes replaces Unpair this device

The owner (2026-10-28), answering
`docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md`'s open question 8: "yes,
an amendment, settings becomes living".

1. **Living section.** The settings screen's list of sections and their
   rows is a living section from now on: a later plan that adds, moves or
   removes a section or row records it here by amendment, naming the plan,
   and the entry is fixed once implemented.
2. **Remotes.** A **Remotes** section, between Battery and Device, lists
   the paired daemons with an X each and a + to pair another
   (`20261028_01_MULTIPLE_REMOTES_PLAN.md`).
3. **`Unpair this device` is removed** from the Device section when that
   plan is built; each remote's X replaces it.

## Amendment — 2026-10-28 17:46 EDT: a remote's row opens its details (living section)

The owner (2026-10-28): "click the remote in settings should show the
details, when the pairing occurred and offer the Remove option if the user
missed the X"; asked to queue it, "yes". A new entry for the living list of
sections and rows (the 2026-10-28 amendment that made it living):

1. **A tap on a Remotes row** (anywhere but its X) opens a details modal in
   the settings style (opens and closes at once, no fade, no
   transparency): the remote's name and status ("in use", "in use ·
   offline", or not in use); its hostname (name and `host:port`); its full
   fingerprint (the six fingerprint words and the 64-hex BLAKE2s
   fingerprint in groups of four, in the mono font); when it was paired
   (date and time, or "paired before this list"); and **Remove** (the same
   confirm modal as the X) and **Close**.
2. The companion crate's `RemoteInfo` gains the full fingerprint (hex and
   words), additively, for this.

## Amendment — 2026-10-28 17:48 EDT: the Remotes section is standard rows only (living section)

The owner (2026-10-28): "I'm not sure what the 'already paired' line is
supposed to mean, but the settings rows should be standardized, not extra
text after them"; asked to queue the change, "yes".

1. **No note under the Remotes rows.** The section is standard rows only
   (name and status, the details line, the X); the note line that showed
   "Removing…", "Removed", "Removed on this phone only", "Paired with…" and
   "Already paired…" is removed.
2. **Each message moves to where the action happened:** "Paired with…" and
   "Already paired with <name>; addresses updated" (a scan of a daemon
   already listed, whose stored addresses were refreshed) are the pairing
   window's success state; removal results show in the remove confirm
   modal ("Removing…" until the daemon answers, then it closes; "removed on
   this phone only" stays in the modal until it is closed); full details
   are in the row's details modal (the previous amendment).
3. **Empty and full:** with no remotes the section shows one standard row,
   "No remotes paired"; with 8 remotes the + is dimmed and its ⓘ says why.

## Amendment — 2026-10-28 17:56 EDT: a remote's nickname, set in its details; the fingerprint collapsible (living section)

The owner (2026-10-28), about the drawer's small remote cards: "they will
probably have a new 'nickname' which defaults to the base hostname and
optional port, and should be settable in the remote details modal (the
fingerprint should be an expandable)". This adds to the 17:46 EDT entry
for the remote details modal:

1. **Nickname.** Each remote has a nickname, stored on the phone per remote
   (an additive `nickname: Option<String>` on the client's record and
   `RemoteInfo`; `None` means the default). The default is the base
   hostname (the host's first DNS label, e.g. `m4` for
   `m4.tail1a2b3c.ts.net`; an IP address whole) plus `:port` when the port
   is not 7347. The details modal has a field to set or clear it; the
   Remotes row's first line and the drawer's remote cards show it.
2. **The fingerprint in the details modal is collapsible:** a "Fingerprint"
   section, collapsed by default, that expands to the six words and the
   64-hex fingerprint.

## Amendment — 2026-10-28 17:57 EDT: the default nickname adds the port only on a conflict (living section)

The owner (2026-10-28): "port if there's a conflict between two with the
same hostname, not just if non-default". This replaces the previous
amendment's item 1 default: a remote's default nickname is the base
hostname alone; `:port` is added only when two or more listed remotes
would otherwise show the same nickname (then each of them shows
`host:port`), whatever the port.

## Amendment — 2026-10-28 20:43 EDT: Settings → Plugins (living section)

docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.9, built in its step 6
(approved by the owner with that plan, its 19:47 EDT amendment). A new
entry for the living list of sections and rows (the 2026-10-28 13:03 EDT
amendment):

1. **A Plugins section, between Remotes and Device.** One standard
   two-line row per plugin the daemon in use lists (its `PluginInfo`), in
   the daemon's order: the plugin's name, with an ⓘ after it that opens its
   description, package name, version and config key; under it, small and
   dim, its version as the daemon gives it (`0.1.0`); and a toggle at the
   right.
2. **A plugin the app can't show** (no device half, or a daemon version
   outside the half's range) has its toggle off and disabled, and its
   second line says *Needs a newer app* or *Needs a newer flankr*.
3. **A toggle** asks the daemon to write `[plugins."<ns>"] enabled`; the
   switch is replaced by a spinner until the daemon's list
   (`server.capabilities`) settles it. A refusal (for example a device that
   may not approve) opens a notice, "<name> was not changed: <reason>",
   with OK.
4. **With no plugins listed** (an older daemon), the section is one
   standard row, *This flankr has no plugins.*
5. The copy is in `apps/companion/copy/SETTINGS_COPY.md`.

## Amendment — 2026-10-28 21:29 EDT: Photos, converted or unmodified (living section)

docs/design/20261028_05_BINARY_FILES_PLAN.md, its 2026-10-28 21:29 EDT amendment, item 8,
agreed with the owner on 2026-10-28 ("this can be an option with
selectable label (convert photos to jpeg) (send unmodified)", then
"yes"). A new entry for the living list of sections and rows (the
2026-10-28 13:03 EDT amendment):

1. **A Photos row in the Device section**, a standard row with a
   `SelectableLabel` choosing between **Convert photos to JPEG** (the
   default) and **Send unmodified**, and an ⓘ that explains both: the
   first keeps PNGs (screenshots) as they are and turns HEIC, rotated or
   large photos into JPEG at 2048 px, dropping their location data; the
   second sends files byte for byte, EXIF included, except HEIC, which is
   still converted to PNG.
2. The choice belongs to the phone, stored with the device settings.
3. The copy is in `apps/companion/copy/SETTINGS_COPY.md`.

## Amendment — 2026-10-29 00:18 EDT: the two speak rows in Sound (living section)

Plan 20261028_06 (the bottom bar updated), decision 6, approved by the owner
on 2026-10-28. A new entry for the living list of sections and rows (the
2026-10-28 13:03 EDT amendment):

1. **Sound gains two toggle rows after *Ding when an agent is done***:
   **Speak coding responses** (off by default; the agent's replies read
   aloud) and **Speak agent (Gemini) responses** (on by default; Gemini's
   lines spoken). Both are inactive while *All sound* is off; the second is
   also inactive while the general agent is off or not offered.
2. They are device settings in `settings.toml` (`speak_coding`,
   `speak_gemini`), kept across restarts, the same values as the volume
   menu's two items. They replace the in-memory read-aloud toggle and the ⋮
   menu's Read aloud.
3. The copy is in `apps/companion/copy/SETTINGS_COPY.md` (`speak_coding`,
   `speak_gemini`).

## Amendment — 2026-10-29 00:26 EDT: mute is a device setting without a row (living section)

docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md, its 2026-10-29 00:26 EDT amendment: the
speaker button's tap mutes or unmutes; `muted` (default false) is kept in
`settings.toml` beside `speak_coding` and `speak_gemini`. Sound's two speak
rows (the 00:18 EDT amendment's item 1) stay; while muted they keep their
values. There is no Mute row.
