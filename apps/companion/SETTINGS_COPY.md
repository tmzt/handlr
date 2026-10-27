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

# [Copy] unpair

- title: Unpair this device
- [abc] daemon: the paired daemon's name
- text: |
    Unpair this phone from {{ [abc] daemon }}? You will need to pair
    again to use flankr.

# [Copy] unpair_offline

- text: |
    flankr is offline: it will still list this phone until it is removed
    from `auth.toml`.

# [Copy] unpair_action

- text: Unpair

# [Copy] delete_data

- title: Delete data
- text: |
    Delete flankr's data on this phone? Settings, stored scrollback and
    pairing are removed. Voice models stay.

# [Copy] delete_action

- text: Delete

# [Copy] cancel

- text: Cancel

# [Copy] ok

- text: OK

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
