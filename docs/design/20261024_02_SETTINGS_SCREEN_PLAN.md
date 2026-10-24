---
title: A settings screen, from a gear on the overview
date: 2026-10-24
status: Draft
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
