# Owner reports

Reports from the owner using the companion app, and what was done about
them. Entries are appended under dated headings, never edited; a
correction is a later entry that names the one it corrects.

## 2026-10-30 13:17 EDT — Owner: a drag in the drawer opened a session

Device: Galaxy S23 Ultra, the build installed 2026-10-29 15:18 EDT.

**Report.** "tap and drag in the left sidebar drawer should not register
as a tap on the menu items. So when holding down the [finger] and dragging
more than the height of the menu item, that should register as a drag.
When tapping, releasing within a very small slop, that should count as a
tap."

**Cause.** The drawer's remote cards ignored a release after the finger
had moved past the 10 pt slop, but the session rows did not: the release
always sent a click, so a drag that scrolled the list could open the row
under the finger
([Session drawer](design/20261028_03_SESSION_DRAWER_PLAN.md), 3.11).

**Done.** `afd9474`: a press that moved past the 10 pt slop is a drag
and never opens a row; a release within the slop is a tap. Installed on
the S23 2026-10-30 13:14 EDT.

## 2026-10-30 13:17 EDT — Owner: the session list should show at once, without "Looking for agents"

**Report.** "for the remote session list, that should be stored persisted
and restored immediately. It shouldn't have a looking for agents message
in places that both in the drawer and the overview screen. What should
happen instead is it should merge the up list as it comes in".

**Seen.** The overview shows *Looking for agents…* until the daemon's
first list arrives, and the drawer shows *Connecting to <nickname>…* in
place of the rows while the remote is offline; nothing is kept between
starts.

**Done.** Recorded as the session drawer plan's 2026-10-30 13:15 EDT
amendment (`ec4df3c`): each remote's last list is kept in its
`state.redb`, shown at once at start and on a switch, with no
looking-for message, and each new list merged in by pane. Being built.

## 2026-10-30 13:19 EDT — Owner: a drag on the overview snaps back

Device: Galaxy S23 Ultra, the build installed 2026-10-30 13:14 EDT.

**Report.** "dragging on the overview screen snaps back to the beginning
or end it doesn't persist it doesn't persist where you drag to so if you
have" (the report was cut off there).

**Seen.** Not yet looked into; handed to a subagent in its own worktree.

## 2026-10-30 13:20 EDT — Owner: the rest of the overview drag report

This completes the entry before ("a drag on the overview snaps back"),
which was cut off.

**Report.** "more if you have more tiles then fit on the screen like more
than more than three screens or whatever sometimes it won't let you it
won't let you navigate to the center tiles".

**Seen.** With several screens of cards, the drag's snap back to the
beginning or the end means the cards in the middle can't be reached.
Passed to the same subagent.

## 2026-10-30 13:29 EDT — Owner: a terminal error fills the screen with the daemon's JSON

Device: Galaxy S23 Ultra, the build installed 2026-10-30 13:29 EDT.

**Report.** "on the terminal screen with an incompatible terminal when you
attempt to drag or do something else that causes an invalid event it's
the entire screen with it looks like it's from the JSON instead that
should like a small maybe centered of the terminal just as it you know
it's only there for a couple of seconds it's out and just just a short
message like like like terminal arrow or something like that".

**Seen.** Not yet looked into; handed to a subagent in its own worktree.
Read as: when the terminal can't take an input (an incompatible terminal,
a drag or other event it rejects), the whole screen shows the raw error,
apparently the daemon's JSON. It should be a short message such as
*Terminal error*, small and centred over the terminal, gone after a couple
of seconds.

## 2026-10-30 13:30 EDT — Owner: the overview drag report, done

This follows the two entries on the overview's drag snapping back.

**Cause.** The overview grid's scroll took each move's distance over the
time since the move before as the finger's speed. The S23 delivers moves
in bursts a fraction of a millisecond apart, so even a slow drag measured
tens of thousands of points a second, and on release that became a fling
that ran the grid to the top or the end. The agent view had fixed the
same fault earlier; the overview's scroll never had the fix.

**Done.** `3da50c6`: the grid keeps the moves' samples and flings at the agent
view's release speed (a fit over the last 100 ms, capped), as the overview
plan's A1 already says. A drag now stays where the finger leaves it; a
hard flick still carries on. The daemon's re-list was checked and does not
move the scroll. Installed on the S23 the same day.

## 2026-10-30 13:34 EDT — Owner: spoken messages interrupt voice input

Device: Galaxy S23 Ultra, the build installed 2026-10-30 13:31 EDT.

**Report.** "spoken messages shouldn't interrupt an interactive voice
input".

**Seen.** Not yet looked into; handed to a subagent in its own worktree.
Read as: while the owner is giving voice input (dictating a message or
talking to Gemini, the mic armed or hearing), a reply read aloud or a
spoken notice starts playing over it. Speech should wait until the voice
input ends.

## 2026-10-30 13:39 EDT — Owner: the terminal error report, done

This follows the entry on a terminal error filling the screen.

**Cause.** When a terminal call failed (opening, arming, input, a scroll
or mouse event, as an older herdr refuses), the client kept the daemon's
error text, and the agent view's status line drew it as *The daemon
refused the terminal: …*, wrapped across the whole terminal. The same
text also went to the overview's status line.

**Done.** The terminal mode plan's 2026-10-30 13:31 EDT amendment and
`9d35f5e`: a failure shows a small *Terminal error* pill centred over the
terminal for 2 s (restarted by a further failure), with the terminal
still drawn under it; the full error goes only to the log. A failed open
says *The terminal could not open.* Installed on the S23 the same day.

## 2026-10-30 13:46 EDT — Owner: the voice input report, done

This follows the entry on spoken messages interrupting voice input.

**Cause.** Four ways speech reached voice input: queued notices and
read-aloud replies were let out in the gap between dictated sentences,
when the mic goes idle before the next is armed; Gemini's spoken lines
were said at once, even while a message was being dictated; a tapped
Gemini turn's "stopped" sentence spoke over the new prompt; and speech
already playing when voice input started was never stopped.

**Done.** The dictated messages plan's 2026-10-30 13:37 EDT amendment and
flankr-companion 0.6.0 (`5fd5b66`): while a message or Gemini's prompt is
taken by voice, speech is held in order and plays as soon as the voice
input ends (sent, cleared or timed out); starting voice input stops what
is being said. Still spoken at once: the dictation's own read-back, its
empty, full and send-failure lines, replays you tap, and confirmation
questions (the next sentence answers them). The done ding still plays.
Installed on the S23 the same day.

## 2026-10-30 15:12 EDT — Owner: the record button does nothing

Device: Galaxy S23 Ultra, the build installed 2026-10-30 14:47 EDT (app
0.2.0, the voice catch-up).

**Report.** "tapped the record button and got no response".

**Seen.** The app's log (`adb logcat -s flankr`, read 15:12 EDT) has five
taps on record between 15:10:45 and 15:11:05, each a release logged as
`hold: pressed Message` then `agent_hit: Message`, so the tap reached the
app and `StartDictation` was sent. The client logged nothing after any of
them: no dictation started, no refusal. Before them: 15:09:21 a tap on an
answer chip; 15:10:16 the viewed agent finished and a reply was spoken;
15:10:21 "no answer in time; reading the reply here". Handed to a subagent
in its own worktree.

## 2026-10-30 15:22 EDT — Owner: stuck on refreshing, and no summary

Device: Galaxy S23 Ultra, the build installed 2026-10-30 14:47 EDT.

**Report.** "app is currently showing --refreshing...-- and doesn't have
the latest updates, it also didn't speak/pin that last summary".

**Seen.**
- **The connection.** At 15:10:21 the phone logged "no answer in time;
  reading the reply here", and the real daemon logged the phone's
  connection closed at the same second. The phone's client has logged
  nothing since: no disconnect, no reconnect. So every later input (the
  record taps in the entry before, pull to refresh) goes nowhere. Very
  likely the same fault as the record button; passed to that subagent.
- **The summary.** Every summary since the restart failed: the key's model
  list includes `gemini-2.5-flash-lite`, which *oldest* picked, but
  generating with it is refused: "This model models/gemini-2.5-flash-lite
  is no longer available to new users. Please update your code to use
  models/gemini-3.5-flash-lite". A model can be listed and still refused
  to a new key, so *oldest* can pick one that fails.

## 2026-10-30 15:27 EDT — Owner: the record button and the stuck refresh, done

This follows the two entries before ("the record button does nothing"
and "stuck on refreshing, and no summary"); both had one cause.

**Cause.** The rules that read a reply aloud split a word at a byte
offset when checking for "am" or "pm", which crashed on a word ending in
a character such as —, …, → or ✓. The daemon crashed on it while
measuring the reply for a summary, so the summary was never answered;
after 10 s the phone read the reply itself, crashed the same way, and the
crash ended its connection thread without a word. From then on every tap
and refresh went nowhere.

**Done.** `0d1cb5e` and the commits after it: such a word is read as it
is; the phone's connection thread now logs a crash that ends it and every
connection that ends, and a tap that does nothing says why in the log.
App 0.2.1 and daemon 0.2.1. The summary's model refusal is a separate
fix, in progress.

## 2026-10-30 22:05 EDT — Owner: a Gemini turn shows no sign it is working

Device: Galaxy S23 Ultra, the build installed 2026-10-30 17:05 EDT (app
0.6.0).

**Report.** "The problem was it it didn't seem to do anything. It didn't
have any any kind of thinking Feedback or anything else".

**Seen.** At 21:59:57 a spoken question to Gemini was sent and the turn
started; Gemini called `recent_commits` and was answering with its result
when, at 22:00:03, a second tap on the centre button interrupted the turn,
so no answer came. For those six seconds the phone showed nothing: the
client keeps the turn internally but gives the app no event for it, and
the daemon sends nothing while Gemini thinks or runs a tool. Handed to a
subagent to amend the general agent chat plan and build the feedback.

## 2026-10-30 22:11 EDT — Owner: Gemini should ask the coding agent and summarize its answer

**Report.** "I asked the general agent what we shipped today. I would
expect it to use a tool call to pass that request to you Get the response
back and summarize it as a list of changes".

**Seen.** At 21:59:57 Gemini answered the question itself: it called
`recent_commits` (5 commits) instead. It has `send_agent_command`, which
types a command into a coding agent's pane after the owner approves it,
but no tool that waits for the agent's reply and gives it back, so it
cannot ask an agent and summarize what it says. Handed to a subagent to
amend the general agent planning plan and build it.
