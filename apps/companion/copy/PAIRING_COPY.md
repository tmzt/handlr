The pairing window's copy, compiled into the app at build time with
`SETTINGS_COPY.md` and every other file in `copy/`
(docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decision 6), in the
same layout: one object tagged Copy per piece, its head the key the code
names, in capitals, as `copy::<KEY>`. A key is defined once across all the
files. The `pairing_*` and `remote_add*` keys are here.

The pairing screen (`src/ui.rs`, laid out by `src/remotes.rs`): its lines
are drawn as plain text, not Markdown. In add mode (**+** in Remotes) its
title is `remote_add`'s. It is quiet (plan 20261028_01, the 14:42 EDT
amendment): while a pairing waits it shows only the six pairing words,
large, one line from `pairing_confirm` (or, in the device-code fallback,
`pairing_fallback` or `pairing_fallback_no_words`), and Cancel; then
`pairing_paired` or the reason it was not. Everything else is behind its
Details button.

# [Copy] remote_add

- title: Pair another remote
- text: |
    Scan the QR code shown by **flankr pair**.

`remote_add_paste` is `remote_add`'s text where there is no camera (the
desktop and the web, plan 20261028_02, decision 21): the link is pasted.

# [Copy] remote_add_paste

- text: |
    Run **flankr pair** on the machine whose daemon you're adding and paste
    the pairing URL it prints below. flankr stays connected to the
    remote in use meanwhile.

# [Copy] pairing_confirm

- [abc] server: the daemon's name, from the pairing URL
- text: |
    If **flankr pair** on {{ [abc] server }} shows these six words, type yes
    there.

The device-code fallback (the 14:37 EDT amendment, item 2): the daemon
could not be asked directly, so this phone's code is pasted into
`flankr pair`, which then prints the words for it and asks for yes. A
link from an older `flankr pair` has no offer, and so no words.

# [Copy] pairing_fallback

- [abc] server: the daemon's name, from the pairing URL
- text: |
    {{ [abc] server }} couldn't be asked directly: paste this phone's code
    from Details into **flankr pair** there, then type yes if it shows these
    words.

# [Copy] pairing_fallback_no_words

- [abc] server: the daemon's name, from the pairing URL
- text: |
    {{ [abc] server }} couldn't be asked directly: paste this phone's code
    below into **flankr pair** there.

The pairing window's states around the six words (the owner,
2026-10-28: "it should give more feedback after the scan succeeded, same
when pairing is otherwise completed"): once a scanned code is accepted the
camera stops and the area shows a check, `pairing_read` and
`pairing_connecting` until the words come. The words show in a modal
titled `pairing_confirm_title`, with Details and Cancel. Once paired, the
window shows a check, `pairing_paired` (or `pairing_already`, with
`pairing_addresses_updated` when they changed), `pairing_reached` dim
under it, and Details and `pairing_done`; it stays until Done.

# [Copy] pairing_read

- text: Code read

# [Copy] pairing_connecting

- [abc] host: the host in the pairing link
- text: |
    Connecting to {{ [abc] host }}…

# [Copy] pairing_confirm_title

- text: Confirm on your Mac

# [Copy] pairing_paired

- [abc] name: the remote's name
- text: |
    Paired with {{ [abc] name }}

# [Copy] pairing_already

- [abc] name: the remote's name
- text: |
    Already paired with {{ [abc] name }}

# [Copy] pairing_addresses_updated

- text: |
    Its addresses were updated.

# [Copy] pairing_reached

- [abc] address: the daemon's host:port
- [abc] fingerprint: the remote's short fingerprint, `3f9a 12c4`
- text: |
    {{ [abc] address }} · fingerprint {{ [abc] fingerprint }}

# [Copy] pairing_done

- text: Done

# [Copy] pairing_cancelled

- text: |
    Pairing cancelled: nothing was added.

The Details button and what it shows: the daemon's fingerprint words, its
`host:port`, the time the link has left, and the device-code fallback's
code.

# [Copy] pairing_details

- text: Details

# [Copy] pairing_details_hide

- text: Hide details

# [Copy] pairing_fingerprint

- text: Daemon fingerprint

The whole fingerprint in Details: BLAKE2s-256 of the daemon's key, as 64
hex digits in groups of four, under this label.

# [Copy] pairing_fingerprint_full

- text: BLAKE2s-256 fingerprint

# [Copy] pairing_address

- [abc] server: the daemon's name, from the pairing URL
- [abc] address: the daemon's host:port
- text: |
    {{ [abc] server }} at {{ [abc] address }}

# [Copy] pairing_expires

- [#] seconds: the seconds the link has left
- text: |
    The link expires in {{ [#] seconds }} s.

# [Copy] pairing_expired

- text: |
    The link has expired: run **flankr pair** again.

# [Copy] pairing_no_words

- text: |
    This link is from an older **flankr pair** and has no offer, so there are
    no pairing words to compare: check the daemon fingerprint instead.

# [Copy] pairing_paste

- [abc] server: the daemon's name, from the pairing URL
- text: |
    Paste this code into **flankr pair** on {{ [abc] server }}:

`pairing_replace` is shown above `pairing_paste` when the remote is
already listed and this phone's key for it was lost: the daemon still lists
the old key under this phone's name, and refuses a second entry of that
name without `--replace` (plan 20261028_01, the 14:37 EDT amendment,
item 4).

# [Copy] pairing_replace

- [abc] server: the daemon's name, from the pairing URL
- text: |
    This phone's key for {{ [abc] server }} was lost, and {{ [abc] server }}
    still lists the old one under this phone's name: run
    **flankr pair --replace** there to paste this code over it.

The pairing window's camera (docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md,
decision 21; `src/scanner.rs`, Android only): the window opens on a
placeholder reading `pairing_tap_to_scan` (or `pairing_scan_again` after a
pairing failed), with `pairing_camera_why` under it; the camera starts only
on a tap. While it runs the line under the viewfinder is
`pairing_scan_hint`, `pairing_scan_ignored` for 3 s after a code that is
not a pairing link, or `pairing_scan_unreadable` for a pairing code that
cannot be used. A refusal shows `pairing_camera_denied`, or
`pairing_camera_denied_settings` with the `pairing_open_settings` button;
other errors `pairing_camera_failed`. `pairing_paste_link` shows the field
instead; `pairing_retry` is the line after a failed pairing.

# [Copy] pairing_tap_to_scan

- text: Tap to scan

# [Copy] pairing_scan_again

- text: Tap to scan again

# [Copy] pairing_camera_why

- text: |
    Scan the QR code **flankr pair** shows. The camera opens only when you tap,
    and Android asks for the Camera permission the first time. Frames are
    decoded on the phone, never saved or sent, and the camera closes when
    you leave this screen.

# [Copy] pairing_scan_hint

- text: |
    Point at the QR code **flankr pair** shows. Pinch to zoom.

# [Copy] pairing_scan_ignored

- text: |
    Not a flankr pairing code.

# [Copy] pairing_scan_unreadable

- [abc] why: why the pairing code was refused
- text: |
    That flankr pairing code can't be used: {{ [abc] why }}.

# [Copy] pairing_camera_denied

- text: |
    Camera permission was refused, so the code can't be scanned. Tap to be
    asked again, or paste the link instead.

# [Copy] pairing_camera_denied_settings

- text: |
    Camera permission is refused and Android won't ask again. Allow it
    under Settings → Apps → flankr → Permissions → Camera, or paste the
    link instead.

# [Copy] pairing_open_settings

- text: Open Settings

# [Copy] pairing_camera_failed

- [abc] why: the camera's error
- text: |
    The camera couldn't start: {{ [abc] why }}.

# [Copy] pairing_paste_link

- text: Paste a link

# [Copy] pairing_retry

- [abc] why: why the pairing failed
- text: |
    {{ [abc] why }}. Run **flankr pair** again, then scan its new code.
