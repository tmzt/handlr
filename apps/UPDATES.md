# Companion app updates

What has changed in the flankr companion app, most recent last.

From now on, each change you can notice ships with an app version. The app is version 0.1.0 today. A change that adds a feature, or changes something you would notice, raises the middle number (0.2.0). A fix or a small polish raises the last number (0.1.1). Entries from then on are headed with the version and the date.

# History before versioning

These changes came before the app had version numbers.

## 2026-10-22

**Added**
- The companion app: a voice screen on your phone that talks with your flankr agents, speaks replies and listens for your voice.

## 2026-10-23

**Added**
- A confirmation card: when an agent asks you to confirm something, the request appears as a card with its buttons.
- A dictation card: dictate a message to the agent you are viewing.
- The agent view: a session's conversation as a scrollable view, with priority cards and a pinned card.
- A back button in the agent view's header, and a suggestion row above the message button at the bottom right.
- Play reads a reply aloud. While it is read, Play becomes Skip, and a tap skips it.
- The record button turns into Send at the bottom right, with Play beside it.
- "Continue" is suggested after 30 seconds of quiet, or straight away when something has gone wrong.
- A message you send shows as typed text in a box until the agent has it.
- The overview is the home screen: a card for each agent, in a grid.
- Cards use the whole screen, and Gemini's replies appear in a strip that opens an overlay.
- Tapping a cut-off question on the pinned card expands it; tapping again folds it.
- Plan questions offer Approve, Discuss and Implement. Links in them can be tapped.
- The agent view shows sections, lets you page back, and names the agent it shows.
- A pairing link opens the app and pairs it.

**Changed**
- Once paired, the pairing field and the Pair button are hidden.
- Context lines are a dim line after an amber dot, not a card.
- Plan text is plain, and button labels shrink to fit.
- "(loading session)" shows until the session answers.
- The pinned card is shown once, at the bottom.
- The overview remembers where you scrolled to.

**Fixed**
- When you have scrolled back, the agent view holds its place as its top entry changes.

## 2026-10-24

**Added**
- The agent view's scrollback is in sections, with a jump line below that goes to the end and says what is there.
- Grey bars, shimmering faintly, show where content not yet loaded will go.
- Two-finger scrolling moves the agent view five times as far.
- A scrollbar appears while you scroll, and you can tap or drag it.
- A scrollbar setting: None, On Scroll, or Always.
- A settings screen, opened from a gear on the overview.
- Settings explains background running and notifications, with links to Android's help pages.
- A notification keeps the app running in the background, and the app asks permission to show notifications.
- A battery row in Settings with an Enable button. It updates once you have allowed the app.
- A voice choice in Sound: step through the phone's voices with a spoken preview.
- Full message text, folded tool calls and diffs, summary cards, pull to refresh, section labels, and a Clear cache option.
- A new launcher icon: a magenta circuit dog.
- Appearance in Display: Light theme or Dark theme, applied at once. A themes file can add your own.
- Each session has its own colour, and other sessions' cards appear at the bottom.

**Changed**
- Dialogs open and close at once, without a fade, and are no longer see-through.
- A harder flick scrolls further, and the scroll follows the speed of your finger on release.

**Fixed**
- The agent view no longer snaps back during a drag when your moves arrive together.
- A settings row's status wraps short of its button, and the row grows.
- An expanded pinned card stays expanded while you scroll.
- Pull to refresh leaves the pinned card whole.

## 2026-10-25

**Added**
- The app is now called flankr. It installs as a separate app from the earlier build.
- Typed messages: a keyboard card with a placeholder and a shaded text area. The text area shows at least three lines, grows to several lines, shrinks the font to fit, and has a cursor.
- The keyboard appears when you tap the text area, unless you dismiss it or an external keyboard is in use.
- A keyboard button to the left of the record button.
- A dark navy default theme, with brighter session colours, and a subtle panel texture on the overview.

**Changed**
- The "OVERVIEW - 2-COLUMN" label above the cards is gone.
- A pinned card collapses when you take a message, and remembers your choice.

**Fixed**
- The on-screen keyboard no longer covers the screen; the content moves up above it.
- The overview panel's texture lines up with the card tiles.

## 2026-10-26

**Added**
- A centre button in the bottom bar talks to the general agent: tap to talk, or hold for 2 seconds to open it. It replaces tap-to-talk on the overview.
- A general agent card, pinned, with an overlay and a text entry area.
- The centre button pulses while your voice is being recognised.
- Voice starts at once when you press the centre button, and Gemini's card appears on press.
- Replies that use Markdown (headings, lists, bold, code) show as formatted text.
- Voice input ends sooner after a pause in your speech.

**Changed**
- The centre button starts idle, and is lit only while pressed.
- The general agent's card, overlay and state look and behave the same on every screen.

**Fixed**
- Reactivating the general agent after a turn, and interrupting a turn, work cleanly.

## 2026-10-27

**Added**
- A terminal view for a session, opened from a button between Play and the centre button. Pinch to zoom, pan, and hold to type.
- In the terminal, a tap starts typing and brings up the keyboard, and a vertical drag scrolls.
- A question's suggested answers appear, and tapping one answers the question.
- A "Tab follows focus" setting in Settings' Voice section, off by default. When on, voice goes to the agent whose view is open.

**Changed**
- Finished cards show one line.
- Plan answers name the plan by its number, as in "Approve plan 03".
- Tapping a suggestion, or the Submit button, sends at once with no confirmation card. A message sent after silence still asks you first.
- One prompt card serves the general agent and typed messages on both screens. The centre button's 2-second hold opens Gemini's card pinned.
- The centre button, the Gemini strip and the Settings toggle follow the agent's setup. They are hidden or dimmed when no general agent is available, and Settings says why.

## 2026-10-28

**Added**
- Terminal: the cursor is drawn, the terminal fits above the keyboard, and taps and drags work in programs that use the mouse.
- A keys row above the keyboard: Esc, Tab, Ctrl, Alt and the arrows. Ctrl and Alt apply to the next key you press.
- Shell panes are listed and open straight into the terminal.
- A Remotes section in Settings lists the flankr computers you have paired. Add another with +, remove one with its X after a confirmation, or use "Unpair this device".
- Each remote's details show its nickname and fingerprint, and the nickname can be edited.
- Six-word pairing: the computer shows six words, and you confirm they match. Details shows the full fingerprint, the address and the time left, with a device-code fallback.
- Pairing by QR code: tap to scan, pinch to zoom, and refusals show their reason.
- Pairing shows its progress: "Code read", the words, then "Paired with ..." until you tap Done. A failure stays on screen with its reason.
- A session drawer with cards for each remote and a row for each session, with a spinner for busy ones. You can switch between remotes from it.
- Plugins: a Plugins section in Settings, a menu (⋮) on the overview, agent view and Settings, title-bar icons for plugins, and a planning popup that lists plans.

**Changed**
- Typing over an armed terminal shows only a coloured frame, with no countdown.
- Typing in the terminal no longer times out.
- Terminal input notices are no longer spoken.
- The pairing screen has no tap-to-listen. A pairing link opened while already paired opens the add mode.
- The pairing screen is quieter: its placeholder says only to scan the QR code, and there is no status line while idle.
- Two computers with the same address are told apart by the first part of their fingerprint.

## 2026-10-29

**Added**
- A speaker button that mutes and unmutes. The mute is kept across restarts. While muted, nothing is spoken, and a slash shows through the speaker.
- A 2-second press on the speaker opens its menu, with speak options.
- Tap the speaker while it is speaking to skip. Hold it for 1 second to stop and clear what is queued, without muting.
- A volume menu with speak switches, and a text row with + and a keyboard toggle in the bottom bar.
- A "Speak Gemini replies" setting. When off, a Gemini reply is shown but not spoken.
- You can send images into a session. You can open read-only files and git output on the phone, with previews of Markdown, text and diffs. Paths and commit links in cards can be tapped, and "Show uncommitted changes" is available. Paste sends text from the clipboard into the terminal.
- Choice questions: a live question card offers the agent's choices, with details, multi-select, an Other option, a "1 of N" count and a review, and results.
- Subagents: a ⊙ menu lists running and finished subagents. You can open one from the flow, and the overview counts them. "All agents…" opens the full list.
- The planning popup has a plan table, and "All plans…" lists every plan.
- A microphone on the text cards: a tap goes on by voice and keeps the text you typed.

**Changed**
- The centre and right buttons open their card typed on a 500 ms hold. The volume button and + keep the 2-second hold.

## 2026-10-30

**Changed**
- The session list shows at once on the overview and in the drawer, online or not. "Looking for agents" and "Connecting" lines are gone, and switching remotes shows that remote's last list.
- A terminal error shows as a small "Terminal error" pill for 2 seconds, not as the error text across the screen.

**Fixed**
- Dragging across the drawer's session rows no longer opens a session. Only a tap does.
- The overview's flick follows your finger's speed, so a drag stays where you leave it and the grid no longer runs to the top or the end.
- Nothing is spoken over voice input. Speech waits, and plays once the voice input ends.

## 0.2.0 — 2026-10-30

**Changed**
- After you finish speaking, replies that came in while you talked are no longer read one by one: you hear one short update on where things stand now, in light of what you just said. If no update can be made within a few seconds, the replies are read as before.
- Spoken summaries are shorter: they leave out which files changed, branches, log entries and test counts, and keep what was done, what was found and any question for you.

## 0.2.1 — 2026-10-30

**Fixed**
- The record button works again after a reply with a dash, an ellipsis, an arrow or a tick mark is read aloud. Reading such a reply on the phone stopped its link to the computer: nothing more was said, pulling to refresh stayed on refreshing, and taps on record did nothing.

## 0.3.0 — 2026-10-30

**Added**
- After you speak, the update on where things stand also shows as a pinned card, "While you spoke", even when muted. Tap its ✕ to dismiss it; the next update replaces it.
- In a session the card sits above the buttons, where the pinned card goes. A waiting question, approval or choice comes first, and the card shows once you answer it.

## 0.3.1 — 2026-10-30

**Fixed**
- Replies are spoken and summarized on their cards again.

## 0.4.0 — 2026-10-30

**Added**
- Drag the card above the buttons left or right to put it away; let go early and it springs back. Scrolling up and down still scrolls the session.
- A Finished card dragged away stays as one line, "Finished · " and the reply's first line; tap the line to see it whole. Its reading aloud is dropped.
- Something broke and other spoken cards dragged away are gone, with what was waiting to be said about them.
- An approval dragged away is denied, and a yes or no question is answered No; a dim line in the session says so. A question about a plan is not answered: it stays as one line.
- The "While you spoke" card can be dragged away too, which also stops its update being read.

**Changed**
- A tap on the card above the buttons no longer folds or expands it. A long card shows More at its bottom right, and Less once open; only those change its size.

**Fixed**
- Links in that card's text open when tapped.

## 0.5.0 — 2026-10-30

**Added**
- Settings has *At startup*, beside *Tab follows focus*: *Overview* (as before) or *Last session*. With *Last session* the app opens on the session you last had open, on the remote you last used; if that session is gone, on the most recent session of another remote, and otherwise on the overview.
- The session you return to shows at once as you left it, scrolling and links working, with "refreshing…" at the bottom until it has caught up.
- A session that no longer exists when you come back to it says "This session has ended", with a button back to the overview.

**Changed**
- Going back from a session to the overview makes the overview what opens next time, with *Last session* on.

## 0.6.0 — 2026-10-30

**Added**
- A question with options from Claude can be dragged away like the other cards: it is dismissed as pressing Esc at the Mac would, and a dim line in the session says "Dismissed the question". If it can't be dismissed, the card springs back and says "Couldn't dismiss the question".
