The settings screen's copy, compiled into the app at build time
(docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decision 6). Each piece
is one object tagged Copy, its head the key the code names, in capitals,
as `copy::<KEY>`. `title` is optional; `text` is Markdown, drawn with
handlr-markdown, and its `{{ [abc] name }}` slots are declared as members.
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

# [Copy] microphone

- text: Microphone

# [Copy] general_agent

- title: General agent (Gemini)
- text: |
    What you ask by voice on the overview (and, later, from the chat
    button) goes to **Gemini**, the general agent. Off, the overview's
    record button is disabled and no voice turn is sent to Gemini. The
    daemon's spoken summaries of agents' replies are separate: they are
    set in the daemon's `config.toml`, under `[summary]`.

# [Copy] scrollbar

- text: Scrollbar

# [Copy] scrollbar_none

- text: None

# [Copy] scrollbar_on_scroll

- text: On Scroll

# [Copy] scrollbar_always

- text: Always

# [Copy] background

- title: Stay connected in the background
- text: |
    Once handlr leaves the screen, Android's **cached apps freezer**
    suspends its process, and **Doze** and **App Standby** defer its
    network. With this on, handlr runs a `specialUse` **foreground
    service** while it's connected, which keeps the link to the daemon
    open. Android requires that service's ongoing notification; without
    `POST_NOTIFICATIONS` it shows only in the running apps list. Doze
    can still delay the network unless *Battery optimization* is off.
    [Foreground services](https://developer.android.com/develop/background-work/services/fgs)

# [Copy] background.samsung

- text: |
    One UI adds its own freezer, **Freecess**, and its sleeping apps
    lists: the service keeps handlr out of Freecess while it runs, but a
    sleeping app is still stopped. See *Samsung battery (app sleeping)*.

# [Copy] alerts

- title: Alert when an agent needs you
- text: |
    While handlr isn't on screen, an agent's **question**, an
    **approval** it waits on, or a **finished turn** posts a
    notification; tapping it opens that agent. This needs Android's
    `POST_NOTIFICATIONS` permission (Android 13 and later), which handlr
    asks for only here, when you turn this on. Alerts come over the
    background connection, so they need *Stay connected in the
    background*.
    [Notification permission](https://developer.android.com/develop/ui/views/notifications/notification-permission)

# [Copy] alerts_needs_background

- text: Needs *Stay connected in the background*

# [Copy] needs_update

- text: Needs an app update

# [Copy] battery_update

- text: Battery settings need an app update

# [Copy] battery_restricted

- text: |
    Battery use is **Restricted**: handlr can't stay connected in the
    background

# [Copy] battery_optimization

- title: Battery optimization
- text: |
    Exempting handlr from battery optimization lifts **App Standby**'s
    limits and some of **Doze**'s network limits. It doesn't stop Doze:
    while the phone is idle, network access can still be deferred to
    maintenance windows. It doesn't override **Restricted** battery use,
    which stops background work entirely.
    [Doze and App Standby](https://developer.android.com/training/monitoring-device-state/doze-standby)

# [Copy] battery_exempt

- text: Exempt

# [Copy] battery_optimized

- text: Optimized

# [Copy] samsung_battery

- title: Samsung battery (app sleeping)
- text: |
    One UI keeps its own lists. **Sleeping apps** are frozen in the
    background by its freezer, and **Deep sleeping apps** don't run at
    all until opened; an app unused for a while is added on its own. Add
    handlr to **Never sleeping apps** under *Battery*, *Background usage
    limits*. Android's battery optimization exemption doesn't cover
    these lists.
    [dontkillmyapp.com/samsung](https://dontkillmyapp.com/samsung)

# [Copy] unpair

- title: Unpair this device
- [abc] daemon: the paired daemon's name
- text: |
    Unpair this phone from {{ [abc] daemon }}? You will need to pair
    again to use handlr.

# [Copy] unpair_offline

- text: |
    handlr is offline: it will still list this phone until it is removed
    from `auth.toml`.

# [Copy] unpair_action

- text: Unpair

# [Copy] delete_data

- title: Delete data
- text: |
    Delete handlr's data on this phone? Settings, stored scrollback and
    pairing are removed. Voice models stay.

# [Copy] delete_action

- text: Delete

# [Copy] cancel

- text: Cancel

# [Copy] ok

- text: OK

The foreground service's notification (CHANGES.md, 2026-10-24 12:59 EDT,
item 3). TODO(libmorph 0.4.0): these are filled and shown once libmorph
0.4.0's foreground service and `NotificationContent` are wired in; until
then nothing reads them. The app's label stays "handlr".

# [Copy] notification_title_several

- [#] needs: how many agents need you
- [#] working: how many agents are working
- text: |
    handlr · {{ [#] needs }} needs you, {{ [#] working }} working

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
