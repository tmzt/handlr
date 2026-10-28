The session drawer's copy (docs/design/20261028_03_SESSION_DRAWER_PLAN.md,
3.15), compiled into the app at build time with every other file in
`copy/`, in the same layout: one object tagged Copy per piece, its head the
key the code names, in capitals, as `copy::<KEY>`. A key is defined once
across all the files. The `drawer_*` and `remote_card_*` keys
are here.

The drawer (`src/drawer/`) draws these as plain text, not Markdown, cut
with "…" to the room they have.

The menu button (≡) at the left of the overview's and the agent view's
header opens the drawer; the **+** after the last remote card opens the
pairing window's add mode. Their labels are for accessibility.

# [Copy] drawer_menu

- text: Sessions and remotes

# [Copy] drawer_add_remote

- text: Pair another remote

A remote card's second line: the remote in use while its link is up, and
while it is not. The others have none.

# [Copy] remote_card_in_use

- text: in use

# [Copy] remote_card_connecting

- text: connecting…

The session list's one row while the remote in use has no list to show.

# [Copy] drawer_connecting

- [abc] nickname: the remote's shown name, as its card shows it
- text: Connecting to {{ [abc] nickname }}…

# [Copy] drawer_unreachable

- [abc] nickname: the remote's shown name, as its card shows it
- [abc] seconds: whole seconds until the next attempt
- text: Can't reach {{ [abc] nickname }} · retrying in {{ [abc] seconds }} s

# [Copy] drawer_not_paired_here

- [abc] nickname: the remote's shown name, as its card shows it
- text: {{ [abc] nickname }} doesn't know this phone: pair again or remove it in Settings

# [Copy] drawer_no_sessions

- [abc] nickname: the remote's shown name, as its card shows it
- text: No sessions on {{ [abc] nickname }}

A session row's summary while a flankr confirmation waits for its pane,
and an idle row's age under a minute. The ages `1m`, `1h`, `1d` are
formatted, not copy.

# [Copy] drawer_session_approval

- text: Approval needed

# [Copy] drawer_session_idle_now

- text: now
