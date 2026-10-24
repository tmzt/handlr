---
title: The session outline by index, with section contents delivered separately on the same link
date: 2026-10-24
status: Draft
covers:
  - "A contiguous, stable `index` for every section of a session file,
    assigned by the daemon alongside the serial"
  - "An outline mode for `session.subscribe`: deliveries carry light
    slots (index, serial, kind, size) for the whole session, and section
    contents are fetched by index range with `session.content`, on the
    same connection"
  - "Gap detection and range requests by index (`gt`, `lt`, `from`,
    `to`)"
  - "The companion's ribbon on the outline: every slot present from the
    start, heights estimated from `size` until laid out, contents fetched
    near the window"
  - "Not covered: what a section holds, the card rules, redaction and
    serials (plan 20261023_03, unchanged); the stores (plan 20261023_09,
    which takes the index as its key for the outline)"
sources:
  - "Owner (2026-10-23/24), after the S23 scrollback check: \"the protocol
    should transfer sections by index and allow requesting missing ranges
    or gt/lt\"; \"right but didnt we design the serial as such an index
    already?\"; \"the ribbon would be a sequence of structs with the serial
    and height in pixels plus the monotonic index\"; \"the actual contents
    are delivered out of band in the same proto\"; then \"yes\" to this
    plan"
  - "Plan 20261023_03, amendment of 14:54 EDT, A2: the serial is derived
    from the session file (name hash, line, slot), not counted, so it is
    monotonic and stable but has gaps, and a client cannot tell a missed
    section from a skipped line"
  - "Plan 20261022_03, L3 amendment of 2026-10-23 23:54 EDT: the
    scrollback as a ribbon of sections in serial order, being built at the
    time of writing with `Slot { index, serial, height }` and the index
    assigned by the client"
  - "handlr at `b207c9d`: `crates/libhandlr_proto/src/common.rs`
    (`Section`, `Serial`, `SessionSubscribeParams`, `SessionDelivery`,
    `SessionHistoryParams`, `SECTIONS_MAX` 200), `src/sessions/{hub.rs,
    sections.rs}`, `src/server/subscriptions.rs`,
    `crates/handlr-companion/src/{client.rs, view.rs}`"
related:
  - docs/design/20261023_03_SESSION_SUBSCRIPTIONS_PLAN.md (sections, serials, subscriptions and history; this adds to its interface)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (living section L3, the agent view and its ribbon)
  - docs/design/20261023_09_HANDLR_DATA_PLAN.md (the stores; the outline is kept by index, contents by serial)
---

# The session outline by index, with section contents delivered separately on the same link

## 1. Goal

Today a subscription sends whole sections, and a client knows only the
sections it has been sent. Serials have gaps by design, so the client
cannot tell a section it missed from a line that made none. It also does
not know how long the session is, so the agent view's scrollback grows
upward as history pages land, and scrolling back is uneven.

After this plan, the daemon numbers a session's sections 0, 1, 2… with
no gaps, and a subscription can send the session's **outline**: one small
slot per section, for the whole session, with no contents. The contents
come separately, on the same link, for the index ranges the client asks
for. The phone's ribbon has every slot from the start, at its estimated
height, and fills in contents near the window.

## 2. Decisions

1. **The index.** `Section.index: u64` is the section's position among the
   sections of its session file, counted from 0 by the daemon as it builds
   sections in file order.
   - **Contiguous:** every index from 0 to the newest has a section, or
     is marked removed.
   - **Stable:** the same file gives the same indexes on every read,
     after a reconnect, and after a daemon restart, because the daemon
     counts as it builds sections from the start of the file (the hub
     already scans the whole file for its line index).
   - **Replaced in place:** a section rebuilt with new content (a card
     resolved, a tool result arriving) keeps its index and serial.
   - **Never reused:** a removed section's index stays removed. The
     daemon keeps a set of removed indexes per tailer.
   - **Per file:** a new session file (after `/clear`) starts again at 0,
     with a reset, as serials do.

   The serial stays the section's identity and its place in the file. An
   index and a serial always name the same section within one file.
2. **The slot.** The outline is made of:

   ```rust
   pub struct Slot {
       pub index: u64,
       pub serial: Serial,
       pub kind: SectionKind,
       /// The content's size: its lines (after the readers' line cuts)
       /// and characters, for the client's height estimate.
       pub lines: u32,
       pub chars: u32,
       /// Bumped each time the section is replaced, from 0.
       pub rev: u32,
   }
   ```

   On the wire a slot is a JSON object of these six fields, about 60 bytes.
   A session of 5,000 sections is an outline of about 300 KB, which the
   daemon sends in deliveries of at most `SLOTS_MAX = 2000` slots each.
3. **Outline mode.** `SessionSubscribeParams` gains
   `outline: bool` (default false) and `gt: Option<u64>`. With
   `outline: true`:
   - the first delivery (and each reset) carries the slots from index 0,
     or from `gt + 1` when `gt` is given and the file is the one the
     client holds (judged by the serial prefix of `after`, which the
     client also sends); further deliveries continue it until the outline
     is complete;
   - later deliveries carry new and replaced slots, and `removed_index:
     Vec<u64>`, in place of `sections`;
   - each delivery carries `first: u64` and `last: Option<u64>`, the
     session's lowest and highest index (`last` is null for no sections).
     A client that sees a slot index above the newest it holds plus one,
     or `last` above it, has missed slots and asks for them (decision 4);
   - **live contents ride along:** a delivery also carries
     `sections: Vec<Section>` for the slots it adds or replaces at the
     session's end (the newest `LIVE_INLINE = 8` indexes), so following the
     bottom needs no second round trip. Older replaced slots arrive as
     slots only.

   Without `outline`, deliveries carry full sections exactly as today,
   so `handlr session tail` and older clients do not change.
4. **`session.content`.** One method for contents and for missing
   outline ranges:

   ```rust
   pub struct SessionContentParams {
       pub subscription_id: u64,
       /// Strictly after / before this index.
       pub gt: Option<u64>,
       pub lt: Option<u64>,
       /// Inclusive range; used instead of gt/lt when given.
       pub from: Option<u64>,
       pub to: Option<u64>,
       /// 1 to SECTIONS_MAX (200); default 50.
       pub limit: usize,
       /// Slots only, no contents (to fill a gap in the outline).
       pub slots_only: bool,
   }
   pub struct SessionContentResult {
       /// In index order. Each section carries its index.
       pub sections: Vec<Section>,
       /// When slots_only.
       pub slots: Vec<Slot>,
       /// Indexes in the asked range that were removed.
       pub removed_index: Vec<u64>,
       pub first: u64,
       pub last: Option<u64>,
       /// The range held more than `limit`; ask again past the last one.
       pub more: bool,
   }
   ```

   With only `gt`, it returns the next `limit` sections after it; with
   only `lt`, the `limit` sections before it, oldest first; with both, or
   with `from`/`to`, the sections in that range. A range whose session
   file has changed answers `SESSION_CHANGED` (a new error code), and a
   reset delivery follows. `session.history` stays, by serial, unchanged.
5. **Redaction and limits** are as for sections today: contents are
   redacted before they are sent, and `session.content` counts against the
   same per-subscription limits as `session.history`.
6. **The client.** `handlr-companion` subscribes with `outline: true`.
   - It keeps the outline in a `BTreeMap<u64, Slot>` by index, and contents in
     the existing `Sections` map by serial, each with the `rev` it was
     fetched at.
   - It asks `session.content` for the window's slots plus `KEEP_SCREENS`
     screens on each side whose contents are missing or older than the
     slot's `rev`, in ranges of at most 200, and at most two requests in
     flight.
   - It fills outline gaps with `slots_only` requests.
   - It forgets contents more than 20 screens away from the window (the
     outline is always kept), so memory stays bounded in long sessions.
   - The app's event carries the slots and the contents held.
7. **The ribbon on the outline.** The ribbon's `Slot { index, serial,
   height }` takes the daemon's index in place of the client-assigned one.
   - Before a slot's contents are laid out, its height is estimated from
     `lines`, `chars` and the width. The estimate is the wrapped line
     count at the view's font, as the layout would wrap plain text, and a
     card's fixed parts.
   - When contents arrive, the real height replaces the estimate. The
     anchor (serial and distance into its block) keeps the view still.
   - A slot without contents is drawn as a dim placeholder of its height,
     with a short "…" row.
   - The "loading earlier…" row is no longer needed in outline mode. The
     fixed marker row shows "start of session" when the window reaches
     index 0.
8. **Versions.** `Section.index` (serde default 0, skipped when the
   subscription is not in outline mode), `Slot`, the new subscribe
   fields, the new delivery fields, `session.content`, its types and
   `SESSION_CHANGED` are all additions. By the owner's rulings (appended
   struct fields, new items) this is a patch release: libhandlr_proto
   0.3.6. `SectionKind` is unchanged.

## 3. Steps

1. **The protocol.** `Section.index`, `Slot`, `SLOTS_MAX`, `LIVE_INLINE`,
   the subscribe and delivery fields, `session.content` with its params,
   result and method name, `SESSION_CHANGED`; libhandlr_proto 0.3.6 with
   its VERSIONS.md entry and tag.
   **Done when:** every new type round-trips through serde; a delivery and
   a subscribe without the new fields read as before; the existing tests
   pass unchanged.
2. **The daemon's index.** The hub counts indexes as it builds sections,
   keeps removed indexes, and fills `Section.index`.
   **Done when:** hub tests show indexes contiguous from 0 across a
   session's sections; the same file read twice, and after a simulated
   restart, gives the same indexes; a replaced section keeps its index
   and bumps `rev`; a removed index is not reused; a new file starts at 0.
3. **The daemon's outline mode and `session.content`.** In
   `src/server/subscriptions.rs`.
   **Done when:** daemon tests show:
   - an outline subscription's deliveries cover every index, in chunks of
     at most `SLOTS_MAX`;
   - `gt` resumes after the given index;
   - live contents ride along for the newest slots only;
   - `session.content` answers `gt`, `lt`, both, and `from`/`to`, with
     `more` and `removed_index` set correctly;
   - `slots_only` returns slots;
   - a changed file answers `SESSION_CHANGED`;
   - a subscription without `outline` delivers sections exactly as before.

   `handlr session tail --outline` prints the slots, and a range command
   prints their contents.
4. **The client.** Decision 6 in `handlr-companion`.
   **Done when:** client tests show:
   - the outline held complete across chunked deliveries;
   - a gap detected and filled with `slots_only`;
   - contents asked for only around the window, at most two requests in
     flight;
   - a replaced slot's stale contents fetched again;
   - far contents forgotten while the outline is kept;
   - a reset dropping both.
5. **The app.** Decision 7 in `apps/companion`, on the ribbon of the L3
   amendment.
   **Done when:** app tests show:
   - the ribbon's height set from the outline before any contents
     arrive;
   - an estimate replaced by the real height without moving the view;
   - placeholders drawn for missing contents;
   - "start of session" at index 0.

   On the S23, scrolling back through a long session to its start moves
   at the finger's speed with no jumps, and the scrollback's length is
   right from the start.
6. **Live, read-only.** `handlr session tail --outline` against real
   herdr, logged under `out/sessions/YYYYMMDD_NN_outline_live/`.
   **Done when:** the log shows a complete outline of a long live session
   with contiguous indexes, and a content range fetched by `gt`, `lt` and
   `from`/`to`.

Each step is one crate or subproject, one commit (Rule 3.1).
- Step 1 needs nothing.
- Step 2 needs 1.
- Step 3 needs 2.
- Step 4 needs 1.
- Step 5 needs 4 and the L3 ribbon.
- Step 6 needs 3.

## 4. Open questions for the owner

1. **How far contents are kept** on the phone: the window ± `KEEP_SCREENS`
   (5) for fetching, and forgotten past 20 screens. Are those the right
   distances? Once plan 20261023_09's redb store is on the phone, forgotten
   contents can be read back from it in place of the daemon.
2. **`LIVE_INLINE`:** contents ride along for the newest 8 indexes. Should
   that be more, or only while the client says it is following?

## Amendment — 2026-10-24 00:03 EDT: one inclusive range, `from` and `to`, each open when `None`

The owner (2026-10-24): "gt/lt are just from/to with None"; then "yes"
to this amendment. This replaces `gt` and `lt` everywhere in the plan:
decision 3's `SessionSubscribeParams.gt`, decision 4's
`SessionContentParams` fields and the paragraph after it, and the
ranges named in steps 3 and 6.

1. **`session.content` takes one range:**

   ```rust
   pub struct SessionContentParams {
       pub subscription_id: u64,
       /// Inclusive bounds; None is open on that side.
       pub from: Option<u64>,
       pub to: Option<u64>,
       /// 1 to SECTIONS_MAX (200); default 50.
       pub limit: usize,
       pub slots_only: bool,
   }
   ```

   - `from` only: the next `limit` sections from `from` on;
   - `to` only: the `limit` sections ending at `to`, oldest first;
   - both: the range, cut at `limit` from `from`, with `more` set when
     cut;
   - neither: the newest `limit` sections.

   `SessionContentResult` is unchanged.
2. **`session.subscribe` takes `from: Option<u64>`** in place of `gt`:
   the outline starts at that index (the client's newest held index plus
   one, to resume), and at 0 when it is `None` or the file is not the
   one the client holds.
3. **Steps.** Step 3's tests answer the four range shapes above, in place
   of "`gt`, `lt`, both, and `from`/`to`", and `from` resumes in place of
   `gt`. Step 6's log fetches a content range by `from` only, `to` only
   and both.
