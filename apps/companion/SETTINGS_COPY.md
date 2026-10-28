The settings screen's copy, compiled into the app at build time
(docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decision 6). Each piece
is one object tagged Copy, its head the key the code names, in capitals,
as `copy::<KEY>`. `title` is optional; `text` is Markdown, drawn with
flankr-markdown, and its `{{ [abc] name }}` slots are declared as members.
A `<key>.<vendor>` object (the vendor lower-cased) is appended after its
key's text on that vendor's phones.

# [Copy] settings_title

- text: Settings

# [Copy] section_sound

- text: SOUND

# [Copy] section_voice

- text: VOICE

# [Copy] section_display

- text: DISPLAY

# [Copy] section_background

- text: BACKGROUND

# [Copy] section_notifications

- text: NOTIFICATIONS

# [Copy] section_battery

- text: BATTERY

# [Copy] section_remotes

- text: REMOTES

# [Copy] section_device

- text: DEVICE

# [Copy] sound

- text: All sound

# [Copy] ding

- text: Ding when an agent is done

# [Copy] voice

- title: Voice
- text: |
    The voices are **Phonon**'s: each is a checkpoint in the
    `phonon-31m-en/voices/` folder of flankr's data directory, put on
    the phone with `scripts/push-model.sh`. *Default* is Phonon's own
    voice. Each step says a short sample in the voice chosen. A voice
    that is no longer installed is spoken as *Default*.

# [Copy] voice_default

- text: Default

# [Copy] voice_preview_default

- text: This is the default voice.

# [Copy] voice_preview

- [abc] voice: the voice's name, as the row shows it
- text: This is {{ [abc] voice }}.

# [Copy] microphone

- text: Microphone

# [Copy] general_agent

- title: General agent (Gemini)
- text: |
    What you ask by voice on the overview (and, later, from the chat
    button) goes to **Gemini**, the general agent. Off, the overview's
    record button is disabled and no voice turn is sent to Gemini. The
    daemon's own use of models, the general agent and its summaries of
    agents' replies alike, is set in the daemon's `config.toml`, under
    `[llm]`.

# [Copy] general_agent_unavailable

- [abc] reason: why the daemon offers no general agent, as it says
- text: |
    **The daemon offers no general agent:** {{ [abc] reason }}. Until it
    does, this switch has no effect and the centre button is hidden. It is
    turned on with `enabled = true` under `[llm]` in the daemon's
    `config.toml`; the daemon picks the change up within about 5 seconds.

# [Copy] tab_follows_focus

- title: Tab follows focus
- text: |
    On, when Gemini focuses a project (its `focus_project` tool), herdr's
    focused tab on the desktop moves to that agent's tab too. Off, Gemini's
    project focus is its own: it sets which agent Gemini's commands and
    file reads go to, and the desktop is left alone. Gemini's `focus` tool,
    used when you ask to be shown a pane, moves herdr's focus either way;
    this switch does not change it. Needs *General agent (Gemini)* on.

# [Copy] scrollbar

- text: Scrollbar

# [Copy] scrollbar_none

- text: None

# [Copy] scrollbar_on_scroll

- text: On Scroll

# [Copy] scrollbar_always

- text: Always

# [Copy] appearance

- title: Appearance
- text: |
    With *System*, the theme follows the phone's dark mode: the *Light
    theme* while it is light, and the *Dark theme* while it is dark. Until the app can
    read the phone's setting, *System* is dark. *Always light* and
    *Always dark* keep one. More themes can be added in a `themes.toml`
    beside `settings.toml` in flankr's data directory; it is read each
    time Settings opens.

# [Copy] appearance_system

- text: System

# [Copy] appearance_light

- text: Always light

# [Copy] appearance_dark

- text: Always dark

# [Copy] light_theme

- text: Light theme

# [Copy] dark_theme

- text: Dark theme

# [Copy] background

- title: Stay connected in the background
- text: |
    Once flankr leaves the screen, Android's **cached apps freezer**
    suspends its process, and **Doze** and **App Standby** defer its
    network. With this on, flankr runs a `specialUse` **foreground
    service** while it's connected, which keeps the link to the daemon
    open. Android requires that service's ongoing notification; without
    `POST_NOTIFICATIONS` it shows only in the running apps list. Doze
    can still delay the network unless *Battery optimization* is off.
- link: |
    [Android documentation](https://developer.android.com/develop/background-work/services/fgs)

# [Copy] background.samsung

- text: |
    One UI adds its own freezer, **Freecess**, and its sleeping apps
    lists: the service keeps flankr out of Freecess while it runs, but a
    sleeping app is still stopped. See *Samsung battery (app sleeping)*.

# [Copy] alerts

- title: Alert when an agent needs you
- text: |
    While flankr isn't on screen, an agent's **question**, an
    **approval** it waits on, or a **finished turn** posts a
    notification; tapping it opens that agent. This needs Android's
    `POST_NOTIFICATIONS` permission (Android 13 and later), which flankr
    asks for only here, when you turn this on. Alerts come over the
    background connection, so they need *Stay connected in the
    background*.
- link: |
    [Android documentation](https://developer.android.com/develop/ui/views/notifications/notification-permission)

# [Copy] alerts_needs_background

- text: Needs *Stay connected in the background*

# [Copy] needs_update

- text: Needs an app update

# [Copy] battery_update

- text: Battery settings need an app update

# [Copy] battery_restricted

- text: |
    Battery use is **Restricted**: flankr can't stay connected in the
    background

# [Copy] battery_optimization

- title: Battery optimization
- enabled_text: Exempt: Android doesn't hold back flankr's network
- disabled_text: Optimized: Android may hold back flankr's network when idle
- text: |
    Exempting flankr from battery optimization lifts **App Standby**'s
    limits and some of **Doze**'s network limits. It doesn't stop Doze:
    while the phone is idle, network access can still be deferred to
    maintenance windows. It doesn't override **Restricted** battery use,
    which stops background work entirely.
- link: |
    [Android documentation](https://developer.android.com/training/monitoring-device-state/doze-standby)

# [Copy] battery_enable

- text: Enable

# [Copy] samsung_battery

- title: Samsung battery (app sleeping)
- text: |
    One UI keeps its own lists. **Sleeping apps** are frozen in the
    background by its freezer, and **Deep sleeping apps** don't run at
    all until opened; an app unused for a while is added on its own. Add
    flankr to **Never sleeping apps** under *Battery*, *Background usage
    limits*. Android's battery optimization exemption doesn't cover
    these lists.
- link: |
    [dontkillmyapp.com/samsung](https://dontkillmyapp.com/samsung)

# [Copy] clear_cache

- [abc] size: the cache's size in MB
- text: Clear cache ({{ [abc] size }} MB)

# [Copy] delete_data

- title: Delete data
- text: |
    Delete flankr's data on this phone? Every remote is removed first:
    each daemon is asked to forget this phone (`link.unpair`; the one in
    use over its open link, the others over one short connection each,
    all at once, up to 15 seconds), and this phone's key, record and
    `remotes/<id>/` state for each are deleted whether it answers or not.
    Then settings, stored scrollback and the rest of the data directory
    go. Voice models stay.

# [Copy] delete_action

- text: Delete

# [Copy] cancel

- text: Cancel

# [Copy] ok

- text: OK

The Remotes section (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md,
3.10 and 3.11, with its 13:03 EDT amendment), drawn in
`src/settings/screen.rs` and `src/settings/remote_row.rs`. `flankr` has no
command that removes a paired device: a daemon forgets one by
`link.unpair`, or by its `[[client]]` entry being deleted from its
`auth.toml`, which it reads again on change.

# [Copy] remotes

- title: Remotes
- text: |
    Each remote is a flankr daemon this phone is paired with, known by
    its link static key (X25519). The phone pins that key and reconnects
    with `Noise_IK`. For each remote it holds a separate static key of
    its own in libmorph's key store (Keystore-sealed on Android, the
    login Keychain on macOS), so removing one never touches another, and
    two daemons can't tell from their `auth.toml` that they list the same
    phone. The fingerprint is the first 8 hex digits of BLAKE2s-256 over
    the daemon's key, as `flankr pair` prints it.

    flankr connects to one remote at a time, the one marked *in use*.
    **+** pairs another beside it without dropping that connection;
    choosing which one is in use comes later. Removing the one in use
    moves to the most recently paired of the rest. Each remote's state is
    kept in its own `remotes/<id>/state.redb` in flankr's data directory,
    and goes with it. The phone keeps at most 8 remotes.

# [Copy] remote_in_use

- text: in use

# [Copy] remote_in_use_offline

- text: in use · offline

# [Copy] remote_detail

- [abc] host: the remote's first address, host:port
- [abc] fingerprint: the remote's fingerprint, as 3f9a 12c4
- [abc] date: when it was paired, as 28 Oct 2026
- text: |
    {{ [abc] host }} · {{ [abc] fingerprint }} · paired {{ [abc] date }}

# [Copy] remote_detail_undated

- [abc] host: the remote's first address, host:port
- [abc] fingerprint: the remote's fingerprint, as 3f9a 12c4
- text: |
    {{ [abc] host }} · {{ [abc] fingerprint }} · paired before this list

# [Copy] remotes_empty

- text: |
    No remote paired. Tap **+** to pair with a flankr daemon: `flankr
    pair` on that machine prints the URL to paste.

# [Copy] remotes_full

- text: |
    8 remotes is the most this phone keeps: remove one to pair another.

# [Copy] remove_remote

- title: Remove remote
- [abc] name: the remote's name
- [abc] host: the remote's first address, host:port
- [abc] fingerprint: the remote's fingerprint
- text: |
    Remove **{{ [abc] name }}** (`{{ [abc] host }}`, {{ [abc] fingerprint }})
    from this phone? This phone's key for that pairing, its record, and
    its stored state and cache on this phone (`remotes/<id>/`) are
    deleted. To use it again, pair it again with `flankr pair`.

# [Copy] remove_remote_tell

- [abc] name: the remote's name
- [abc] device: the name this phone paired as
- text: |
    flankr asks {{ [abc] name }} over the open link to forget this phone
    (`link.unpair`, waiting up to 5 seconds), which deletes its
    `[[client]]` entry for this phone (`{{ [abc] device }}`) from its
    `auth.toml`.

# [Copy] remove_remote_try

- [abc] name: the remote's name
- [abc] device: the name this phone paired as
- text: |
    flankr connects to {{ [abc] name }} once, with this phone's key for
    it (`IK`, up to 15 seconds), to ask it to forget this phone
    (`link.unpair`). If it can't be reached, its `auth.toml` keeps the
    `[[client]]` entry for this phone (`{{ [abc] device }}`) until you
    delete it on that machine (`~/.config/flankr/auth.toml` by default).

# [Copy] remove_remote_offline

- [abc] name: the remote's name
- [abc] device: the name this phone paired as
- text: |
    **{{ [abc] name }} is offline, so it won't be told.** Its `auth.toml`
    keeps the `[[client]]` entry for this phone (`{{ [abc] device }}`).
    The key that entry names is deleted here, so nothing can connect
    with it; to clear it, delete the entry on that machine
    (`~/.config/flankr/auth.toml` by default). `flankr` has no command
    for it; the daemon reads the file again when it changes.

# [Copy] remove_remote_next

- [abc] next: the remote that will be in use after
- text: |
    It's the remote in use: flankr connects to {{ [abc] next }} next.

# [Copy] remove_remote_last

- text: |
    It's the last remote: flankr returns to the pairing screen.

# [Copy] remove_remote_action

- text: Remove

# [Copy] remote_removing

- [abc] name: the remote's name
- text: |
    Removing {{ [abc] name }}: asking it to forget this phone…

# [Copy] remote_removed

- [abc] name: the remote's name
- text: |
    Removed {{ [abc] name }}: it answered `link.unpair` and no longer
    lists this phone.

# [Copy] remote_removed_here

- [abc] name: the remote's name
- [abc] device: the name this phone paired as
- text: |
    Removed {{ [abc] name }} on this phone only: it didn't answer
    `link.unpair`, so its `auth.toml` lists `{{ [abc] device }}` until
    that entry is deleted on that machine.

# [Copy] remote_paired

- [abc] name: the remote's name
- text: |
    Paired with {{ [abc] name }}. flankr stays connected to the remote in
    use.

# [Copy] remote_already

- [abc] name: the remote's name
- text: |
    Already paired with {{ [abc] name }}: its addresses are updated.

The pairing screen (`src/ui.rs`, laid out by `src/remotes.rs`): its lines
are drawn as plain text, not Markdown. In add mode (**+** in Remotes) its
title is `remote_add`'s.

# [Copy] remote_add

- title: Pair another remote
- text: |
    Run flankr pair on the machine whose daemon you're adding and paste
    the pairing URL it prints below. flankr stays connected to the
    remote in use meanwhile.

# [Copy] pairing_paste

- [abc] server: the daemon's name, from the pairing URL
- text: |
    Paste this code into flankr pair on {{ [abc] server }}:

# [Copy] pairing_confirmation

- text: Confirmation code

# [Copy] pairing_confirmation_check

- [abc] server: the daemon's name, from the pairing URL
- text: |
    Check that it matches the confirmation code flankr pair printed on
    {{ [abc] server }}. Both come from the daemon's static key; if they
    differ, the URL names another daemon: tap Cancel.

The foreground service's notification (CHANGES.md, 2026-10-24 12:59 EDT,
item 3), filled in `src/notify.rs` and shown with libmorph's
`NotificationContent`. With one agent, the title is the agent's own name;
with none to name, "flankr". The app's label stays "flankr".

# [Copy] notification_title_several

- [#] needs: how many agents need you
- [#] working: how many agents are working
- text: |
    flankr · {{ [#] needs }} needs you, {{ [#] working }} working

# [Copy] notification_text_question

- [abc] question: the question, cut to one line
- text: |
    question: {{ [abc] question }}

# [Copy] notification_text_approval

- [abc] approval: what it asks to run, cut to one line
- text: |
    approval: {{ [abc] approval }}

# [Copy] notification_text_working

- [abc] line: the agent's newest line
- text: |
    working: {{ [abc] line }}

# [Copy] notification_text_done

- [abc] summary: the finished turn's summary
- text: |
    done: {{ [abc] summary }}

# [Copy] notification_text_idle

- [#] megabytes: the link's traffic since connecting, in MB
- text: |
    All agents idle · {{ [#] megabytes }} MB this session

Other sessions' cards at the bottom of the screen (plan 20261022_03, the
L3 amendments of 2026-10-24 15:56, 16:00 and 16:01 EDT), drawn in
`src/session_cards.rs`: the header is in the session's colour and names
its agent; several collapse to the newest, with "+N" for the rest.

# [Copy] session_asking

- [abc] agent: the agent's shown name
- text: |
    {{ [abc] agent }} is asking

# [Copy] session_approval

- [abc] agent: the agent's shown name
- text: |
    {{ [abc] agent }} needs approval

# [Copy] session_finished

- [abc] agent: the agent's shown name
- [abc] line: the last reply's first line
- text: |
    {{ [abc] agent }} finished: {{ [abc] line }}

# [Copy] session_more

- [#] count: the cards collapsed under the newest
- text: |
    +{{ [#] count }}

The prompt card (plan 20261027_03, `src/prompt/`): the general agent's and
a message's card, pinned or expanded to an overlay with its history, and
the general agent's two bars while its card is hidden.

# [Copy] prompt_gemini

- text: ✦ Gemini

# [Copy] prompt_message

- [abc] agent: the session's agent, as shown
- text: |
    Message to {{ [abc] agent }}

# [Copy] prompt_message_untargeted

- text: Message

# [Copy] prompt_listening

- text: Listening…

# [Copy] prompt_ask_gemini

- text: Ask Gemini…

# [Copy] prompt_type_message

- text: Type your message

# [Copy] prompt_no_messages

- text: No messages sent to this session yet.
