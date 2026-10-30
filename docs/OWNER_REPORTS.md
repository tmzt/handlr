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
