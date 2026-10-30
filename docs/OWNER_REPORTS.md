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
