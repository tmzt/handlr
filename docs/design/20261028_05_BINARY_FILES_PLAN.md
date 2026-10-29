---
title: Binary transfers over the link, for images to a session and read-only files and git output to the phone, with a built-in preview
date: 2026-10-28
status: Draft
covers:
  - "Link kind 2, `KIND_BINARY`, in libflankr_proto (additive): every kind-2
    message is one command type with subcommands (REQUEST, OFFER, ACCEPT,
    REFUSE, CHUNK, END, DONE, CANCEL) under a namespace, all transfer
    control in kind 2 and none in JSON-RPC; transfers interleave with
    JSON-RPC and terminal traffic"
  - "A new crate, `crates/libflankr_blob`: the byte encoding, namespaces,
    refusal codes, transfer ids, the sender and receiver state machines,
    MIME sniffing by magic bytes and SHA-256"
  - "Three built-in namespaces: `images` (phone to daemon, written only to
    a fixed, protected temp folder, the path pasted into the message in the
    image's place), `files` (daemon to phone, read-only, exactly the
    general agent's `read_file` path rules) and `git` (daemon to phone,
    read-only: `show <rev>`, `diff <rev>..<rev>` and `diff`, optionally for
    one path, as a fixed argv with no shell, not limited by the `read`
    patterns); later plugin namespaces by their reverse-DNS names"
  - "Paths and commit hashes in transcript cards drawn as links that open
    the preview"
  - "Every transfer names the selected session (the pane of the open agent
    view)"
  - "Sending a message with images: `agents.send` gains `parts`; an
    image-only send needs the agent's `[[agents]]` entry and reach, not
    `allow_arbitrary_commands` (nor `allow_messages`); the user's own text
    keeps its existing checks"
  - "Image sources in the app: the camera and the gallery through
    libmobile's media request, and Share → flankr from other apps"
  - "A built-in full-screen preview in the app: Markdown (flankr-markdown),
    images, plain text and diffs, with a share icon offering Open in…
    (ACTION_VIEW) and Share (ACTION_SEND); entry points in transcript cards
    and the planning plugin's plan rows"
  - "Not covered: transfers over the local Unix socket; resuming a
    transfer after a reconnect; writing project files; video; images to
    the general agent; voice-initiated sends with images; partial path
    references and a full file review mode (section 16, later); the
    libmobile plan this depends on (named in 10.6, written in libmobile)"
sources:
  - "Owner (2026-10-28, verbatim): \"plan a binary file protocol with
    namespace (files, images, etc), selected session. images are written to
    a defined temp folder and path pasted in it's place, files are readonly
    and limited to readable paths (same list as read_file). this also
    enables the basic markdown and image preview as builtin, with an open
    in/share icon (using intents). binary commands are one command type
    with subcommands, like terminal. discuss first\""
  - "Owner's answer to the lead's proposal (2026-10-28, verbatim): \"yes,
    except the allow arbitrary commands, and writes to the protected temp
    folder. blob is fine\"; the lead's proposal settled: kind 2, the header
    `[subcommand u8][transfer u32][namespace u8 len + ASCII][body]`, OFFER /
    ACCEPT / REFUSE / CHUNK / END / CANCEL, all control in kind 2, `files`
    and `images` built in, a 20 MiB image cap, 7 days and 500 MiB of
    retention, an 8 MiB file cap, Markdown, images, plain text and diffs in
    the preview, a content URI from the app's cache, a new libmobile plan,
    and the crate named `libflankr_blob`"
  - "Owner (2026-10-28, through the lead): add a third built-in namespace
    `git` and a REQUEST subcommand (phone to daemon: namespace, session,
    body) used by both `files` and `git`; `git` request bodies `show <rev>`,
    `diff <rev>..<rev>` and `diff`, each optionally for one path; a fixed
    argv `git -C <root> show|diff --no-color --no-ext-diff …`, revs only a
    full or short hash or `HEAD~N`, no options from the phone, a cleared
    environment as the plugins plan's `CommandSpec`, a 5 s timeout, output
    cut at 8 MiB with a marker instead of failing; previewed as a diff;
    entry points commit hashes in the transcript, `recent_commits` results,
    later the planning plugin's amendment list"
  - "Owner (2026-10-28, verbatim), on the `read` patterns and `git`: \"the
    limit is mostly there so gemini doesnt try coding, it doesnt need to be
    enforced on git paths called from user tapping a git hash (for
    instance)\""
  - "Owner (2026-10-28, verbatim), on whether user-tapped `files` requests
    are exempt too: \"for now, it's mostly for reading docs, we could
    expand to a full file review addon/mode later\""
  - "Owner (2026-10-28, verbatim), on links: \"also, it would detect unique
    paths by first enumerating the allowed paths so partial file references
    and things like plan xx work, but that should be in the plugin most
    likely. actual paths can be supported now as links\""
  - "`src/git.rs` (`recent_commits`: `git -C <root> --no-pager -c
    core.fsmonitor=false …`, `GIT_CONFIG_NOSYSTEM`, `GIT_TERMINAL_PROMPT=0`,
    `GIT_LITERAL_PATHSPECS=1`, `LC_ALL=C`, stdin closed, a 5 s `TIMEOUT`,
    the environment otherwise inherited, output read whole by
    `Command::output`; `check_path`: a path checked as a message's paths
    are and passed as `./<relative>` after `--`) and `src/llm/planning.rs`
    (`ReadPattern`)"
  - "`crates/libflankr_proto/src/link/mod.rs` (the flag byte: low nibble
    flags, bit 0 *more*, bits 1-3 reserved; high nibble kind: 0 JSON-RPC, 1
    terminal, 2-14 reserved, 15 extended; `MAX_FRAME` 65535, `MAX_CHUNK`
    65518, `MAX_MESSAGE` 1 MiB); `link/stream.rs` (`Kind` and `Message`,
    both `#[non_exhaustive]`; `send_kind`; `read_message` takes kind 1
    only from `recv`; `write_message` writes a message's frames back to
    back, so messages interleave whole, never frame by frame)"
  - "`crates/libflankr_proto/Cargo.toml` (0.5.1 on `main-terminal`) and
    the `main-plugins` worktree's (0.5.2, plan 20261028_04's step 1, tag
    `libflankr_proto-v0.5.2`); `crates/flankr-companion/Cargo.toml` (0.5.0
    here, 0.5.1 on `main-plugins`)"
  - "`docs/design/20261027_07_TERMINAL_MODE_PLAN.md` (decision 3 and the
    14:23 EDT amendment: the kind nibble; decision 6: the gates) and
    `crates/libflankr_term/src/frame.rs` (one big-endian layout per
    message, `encode`/`decode`, `DecodeError`)"
  - "`src/server/conn.rs:68-71` (`Outgoing::{Line, Terminal}`), `:149`
    (`send_terminal`, link only), `:313-360` (`handle_link`: one writer
    task on an unbounded queue; a client kind other than JSON closes the
    connection), `:1011-1017` (`agents.send` → `send_message`),
    `:1312` (`tapped`), `:1328-1420` (`send_message`: `check_message`,
    then `run_command` when tapped, else a confirmation)"
  - "`crates/flankr-companion/src/net.rs:1619` (the client routes
    `Message::Terminal` and ignores other kinds)"
  - "`src/core/policy.rs` (`reach`, `check`, `check_message`: entry and
    reach, `allow_messages`, `validate_message`, `check_paths`; a message
    is always confirmed) and `src/commands.rs` (`validate_message`:
    no control characters, first character not `/`, `!` or `#`, at most
    4000 characters; `check_paths`: every path in the text must resolve
    inside the agent's project; `validate_arbitrary_command`)"
  - "`src/project.rs` (`ProjectFiles`: `always_refused`, `allowed`,
    `read` with canonicalisation and the UTF-8 rule) and
    `docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md`, decision 4 and
    the amendments of 11:20 and 11:22 EDT (the `[llm.uses.planning] read`
    patterns, no built-in patterns, an empty list turns the file tools off)"
  - "AGENTS.md section 3 (per-agent permissions and the confirmation gate)"
  - "`crates/flankr-markdown/src/lib.rs` (`lay_out`), and
    `apps/companion/src/markdown_view.rs` (`MarkdownView`, links)"
  - "`apps/companion/src/prompt/` (the message card, `PromptInput.text`, a
    libcontrols `TextArea` with a cursor) and
    `crates/flankr-companion/src/client.rs` (`Input` and `Event`,
    `#[non_exhaustive]`; `SetText`, `Submit`)"
  - "`crates/libflankr_proto/src/common.rs` (`AgentsSendParams`,
    `AgentInfo.cwd`, `Entry { kind, tool, text }`) and
    `crates/libflankr_proto/src/companion.rs` (`HelloResult`,
    `ServerCapabilities`, `TerminalInfo`)"
  - "`~/src/libmobile/docs/design/20261028_02_MEDIA_REQUEST_PLAN.md`
    (Draft: `request_media`, `MediaKind::Mime`, `CameraEntry::Photo`,
    `Picker`, `Delivery::Bytes`, `MediaFileProvider`) and
    `20261028_01_CAMERA_CAPTURE_AND_QR_PLAN.md` (In implementation), read
    only; `~/src/libmobile/src/intent.rs` (`IntentData`: action, data
    string and extras stringified, so a shared stream's content URI
    arrives as text but its bytes cannot be read without new code)"
  - "`.claude/worktrees/main-plugins/docs/design/20261028_04_PLUGINS_PLAN.md`
    (read only: reverse-DNS package names, at least three segments, at most
    128 bytes; `PluginCtx`'s `read_file`; the planning plugin's modal and
    plan rows; libcontrols' `PopupMenu`)"
related:
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (the link)
  - docs/design/20261027_07_TERMINAL_MODE_PLAN.md (kind 1, the pattern
    this follows)
  - docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md (the `read`
    rules the `files` namespace reuses)
  - docs/design/20261023_02_BULK_INPUT_PLAN.md (messages to an agent)
  - docs/design/20261027_03_UNIFIED_PROMPT_CARD_PLAN.md (the message card)
  - docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md (flankr-markdown)
  - docs/design/20261028_04_PLUGINS_PLAN.md (on `main-plugins`; plugin
    namespaces, the planning plugin's plan rows, `PopupMenu`)
  - "~/src/libmobile/docs/design/20261028_02_MEDIA_REQUEST_PLAN.md (the
    camera and gallery source)"
  - "A libmobile share-and-view plan, to be written in libmobile (section
    6): the dependency for Open in…, Share and Share → flankr"
---

# Binary transfers over the link, for images to a session and read-only files and git output to the phone, with a built-in preview

## 1. Goal

From the message card of a session, the owner attaches a photo (from the
camera, the gallery, or shared into flankr from another app); it is
uploaded over the paired link into a fixed, protected folder on the
daemon's machine, and when the message is sent the image's absolute path
takes its place in the text the agent receives. In the other direction,
a tap on a file path in a transcript card (an agent's Read, Write or Edit)
or on a plan row of the planning plugin fetches that file read-only, under
exactly the rules the general agent's `read_file` obeys, and shows it in a
full-screen preview (Markdown, image, plain text or diff) with a share icon
that hands it to another app. A tap on a commit hash shows that commit's
`git show` in the same preview, and the session's uncommitted changes can
be shown as `git diff`.

Both ride one new link kind with one command type and subcommands, as the
terminal rides kind 1.

## 2. Findings (read 2026-10-28)

- **The flag byte has room.** Kind 2 is reserved and today's `recv` and
  `recv_message` refuse it with `BadFrame`, which closes the connection.
  So a phone must never send kind 2 to a daemon that has not said it reads
  it, and the daemon must never send kind 2 to a phone that has not sent
  one: the same rule kind 1 follows (the daemon sends terminal frames only
  after `terminal.open`).
- **Messages interleave whole, not frame by frame.** `write_message`
  writes all frames of one message before the next message, and the
  daemon's link writer is one task on an unbounded queue
  (`conn.rs:313-337`). A 960 KiB CHUNK holds the writer for 16 frames; at
  a phone's 2 Mbit/s uplink that is about 4 s in which no JSON-RPC goes
  out. Interleaving therefore needs smaller chunks by default and a writer
  that prefers control traffic (decision 4.5).
- **`Kind` and `Message` are `#[non_exhaustive]`**, so `Kind::Binary` and
  `Message::Binary` are additive. The daemon's `handle_link` closes the
  connection on any client kind but JSON (`conn.rs:345-350`); the client
  ignores unknown kinds (`net.rs:1630`).
- **Version clash.** `main-terminal` has libflankr_proto **0.5.1** and
  flankr-companion **0.5.0**; `main-plugins` has already released
  libflankr_proto **0.5.2** (tag `libflankr_proto-v0.5.2`) and
  flankr-companion **0.5.1**. Tags are per crate and per repository, so a
  second 0.5.2 from this branch would collide (section 9).
- **Messages are gated by `allow_messages`, not `allow_arbitrary_commands`.**
  Since plan 20261023_02 (as amended) and plan 20261027_04, decision 1, a
  typed or dictated message goes through `policy::check_message`: the
  agent's entry and reach, `allow_messages`, `validate_message` and
  `check_paths`. `allow_arbitrary_commands` gates only free-form text sent
  as a *command* (`policy::check`). The owner's exception ("except the
  allow arbitrary commands") is honoured by the rule in decision 7.2: an
  image-only send needs neither.
- **`check_paths` would refuse the image's path.** Every `/`-word of a
  message must resolve inside the agent's project; the temp folder is
  outside it. So the daemon must insert the path *after* checking the
  user's text, never run the composed text through `check_paths`
  (decision 7.3).
- **A message's first character may not be `/`** (`COMMAND_PREFIXES`), and
  an absolute path starts with one. A message that starts with an image
  needs a rule (decision 7.4).
- **`read_file` serves UTF-8 text only** (`ProjectFiles::read`: "anything
  that isn't a regular UTF-8 file" is refused) and cuts at 64 KiB. The
  `files` namespace keeps every *path* rule exactly but must also serve
  images whole, up to 8 MiB (decision 6.2).
- **`AgentInfo.cwd` is on the wire**, so the app can turn a tool call's
  absolute `file_path` into the root-relative path `read_file` expects.
- **Tool calls carry their input in `Entry.text`** as `<Tool> {json}`
  (for example `Edit {"file_path":"src/a.rs"}`; `agent.rs:5625`).
- **The message card's text is a libcontrols `TextArea`** with a cursor
  (`prompt/card.rs:376,533-536,589`), and `flankr-companion` owns the
  draft (`Input::SetText`, `Input::Submit`). It has no inline objects, so
  an image in the text is a text token, and its thumbnail chip sits in a
  row above the text (decision 9.2).
- **Decoders.** `png` 0.18 and `sha2` 0.10 are already in `Cargo.lock`.
  There is no JPEG, WebP or GIF decoder; the preview and the thumbnails
  need them (decision 10.4).
- **flankr already runs git read-only** (`src/git.rs`, for
  `recent_commits`): a fixed argv, no shell, a 5 s timeout and
  `check_path` for a pathspec. Two things differ from what `git` here
  needs: the environment is inherited apart from four variables (the
  owner asks for a cleared one, as the plugins plan's `CommandSpec`), and
  `Command::output` reads stdout whole, so the 8 MiB cut needs a reader
  that stops and kills the process (decision 6.8).
- **The `read` patterns are the daemon's alone** (`src/llm/planning.rs`
  `ReadPattern`, `src/project.rs` `always_refused`). To draw only readable
  paths as links, the app needs the patterns and the same matcher
  (decision 10.1).
- **libmobile.** The media request plan (Draft) gives the camera and the
  gallery as bytes with a MIME type. Nothing in libmobile yet offers a
  file to another app (ACTION_VIEW / ACTION_SEND with a content URI) or
  receives ACTION_SEND: `IntentData` stringifies extras, so a shared
  `EXTRA_STREAM` arrives as its URI text, unreadable without a content
  resolver call and the grant. `MediaFileProvider` serves only
  `<cacheDir>/libmobile-media/` for the camera to write into.

## 3. Decisions: the wire (libflankr_proto)

1. **`KIND_BINARY = 0x2`.** Frames of a binary message have the flag byte
   `0x20` (last) or `0x21` (more). `Kind::Binary`, `Message::Binary(Vec<u8>)`,
   `LinkStream::send_binary(&[u8])` and `LinkWriter::send_binary(&[u8])`
   are added. `recv` (both halves) accepts kinds 0, 1 and 2;
   `recv_message` is unchanged and still `BadFrame` on anything but kind 0.
   Kinds 3-14 stay reserved; 15 stays extended.
2. **Only to a peer that reads it.** The phone sends kind 2 only to a
   daemon whose `hello` (or `server.capabilities`) carries `blob`; the
   daemon sends kind 2 only on a connection on which the phone has already
   sent a kind-2 message. Local Unix-socket connections carry no kind 2 and
   get `blob: None`.
3. **The capability, appended and serde-defaulted** to `HelloResult` and
   `ServerCapabilities` (`companion.rs`), as `terminal` was:

   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub struct BlobInfo {
       /// The binary protocol's version: 1.
       pub version: u32,
       /// The namespaces this daemon serves now, e.g. ["images", "files",
       /// "git"]. "files" is listed only while `[llm.uses.planning] read`
       /// is non-empty; "git" only when a `git` binary was found at start.
       pub namespaces: Vec<String>,
       /// The `[llm.uses.planning] read` patterns as configured (the
       /// limited syntax of plan 20261027_04, 11:20 EDT), so the app can
       /// draw only readable paths as links (10.1). Not a secret: the
       /// daemon still checks every REQUEST.
       pub read: Vec<String>,
       /// Largest image accepted, bytes (20 MiB).
       pub image_max: u64,
       /// Largest file served, bytes (8 MiB).
       pub file_max: u64,
       /// Largest `git` output served before it is cut, bytes (8 MiB).
       pub git_max: u64,
       /// Largest CHUNK data the daemon accepts, bytes (983040).
       pub chunk_max: u32,
       /// Most images in one `agents.send` (4).
       pub images_per_send: u32,
   }
   // HelloResult and ServerCapabilities:
   #[serde(default, skip_serializing_if = "Option::is_none")]
   pub blob: Option<BlobInfo>,
   ```

   This is negotiation, not transfer control: no transfer is started,
   answered or ended in JSON-RPC.
4. **`agents.send` gains `parts`** (decision 7), appended to
   `AgentsSendParams`:

   ```rust
   /// A message in parts: its text and the images in their places (plan
   /// 20261028_05). When non-empty, `command` must be "" and `kind`
   /// `message`. Left out of the wire when empty.
   #[serde(default, skip_serializing_if = "Vec::is_empty")]
   pub parts: Vec<SendPart>,

   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(rename_all = "snake_case")]
   #[non_exhaustive]
   pub enum SendPart {
       /// Text the user typed or dictated: `{"text": "..."}`.
       Text(String),
       /// An image stored by an `images` transfer, by the 16 lowercase hex
       /// digits of its DONE's reference: `{"image": "9f3c..."}`.
       Image(String),
   }
   ```

   An older daemon ignores `parts` and would see an empty `command`, which
   `validate_message` refuses ("Message cannot be empty"), so nothing
   half-sent can happen; the app only sends `parts` to a daemon with
   `blob.namespaces` holding `images` anyway.

## 4. Decisions: the binary protocol (libflankr_blob)

1. **One command type, eight subcommands.** Every kind-2 message is, in
   big-endian:

   ```text
   u8   subcommand
   u32  transfer            (0 is invalid)
   u8   ns_len              (1..=128)
   ns_len bytes namespace   (ASCII [a-z0-9_.], not starting or ending with '.')
   ...  body                (by subcommand, below)
   ```

   The header is at most 134 bytes. Every message of a transfer carries
   the transfer's namespace; a different one is `protocol`.

   | Value | Subcommand | Sent by | Body |
   |---|---|---|---|
   | `0x01` | REQUEST | receiver-to-be (the phone, in this plan) | `u8 session_len, session (UTF-8, 1..=128)`; `u16 request_len, request (UTF-8, 1..=1024)`: the namespace's request (`files`: the path, 6.1; `git`: 6.5); `u32 max_size` |
   | `0x02` | OFFER | sender | `u8 session_len, session`; `u16 name_len, name (UTF-8, 1..=1024)`; `u8 mime_len, mime (ASCII, 1..=127)`; `u64 size`; `[32] sha256`; `u64 modified_ms` (Unix ms, 0 unknown) |
   | `0x03` | ACCEPT | receiver | `u32 chunk_max` (1..=983040) |
   | `0x04` | REFUSE | answerer of REQUEST or OFFER | `u16 code`; `u16 reason_len, reason (UTF-8, 0..=512)` |
   | `0x05` | CHUNK | sender | `u64 offset`; data (the rest of the message, 1..=`chunk_max` bytes) |
   | `0x06` | END | sender | `u64 size` |
   | `0x07` | DONE | receiver | namespace body: `images` `[8] reference`; `files` empty |
   | `0x08` | CANCEL | either | `u16 code`; `u16 reason_len, reason (UTF-8, 0..=512)` |

   `0x00` and `0x09`-`0xFF` are reserved. A message whose header cannot be
   read (shorter than 6 bytes, a bad namespace) closes the connection, as
   a bad frame does. A message with a readable header but a reserved
   subcommand, or a body that does not decode, is answered once with
   CANCEL `protocol` for that transfer and otherwise ignored. Bytes left
   after a body are `protocol`, except in CHUNK, whose body runs to the
   end.
2. **The flows.**

   ```text
   images (phone → daemon)            files, git (daemon → phone)
   phone  OFFER  ─►                   phone  REQUEST ─►
          ◄─ ACCEPT | REFUSE                 ◄─ OFFER | REFUSE
   phone  CHUNK… END ─►               phone  ACCEPT | REFUSE ─►
          ◄─ DONE(reference) | CANCEL        ◄─ CHUNK… END
                                      phone  DONE | CANCEL ─►
   ```

   A transfer ends at REFUSE, DONE or CANCEL; its id is then retired.
   CHUNKs come only after ACCEPT, in order: each `offset` equals the bytes
   received so far, else `protocol`. END's `size` must equal OFFER's and
   the bytes received; the receiver then checks the SHA-256 against
   OFFER's and answers DONE, or CANCEL `hash_mismatch` / `size_mismatch`.
   The phone REFUSEs a file's OFFER with `have_it` when its preview cache
   already holds that SHA-256, and uses the cached copy.

   REQUEST and DONE are added to the six subcommands the proposal named:
   REQUEST because a file or a git output is asked for by the phone and
   sent by the daemon, so the phone needs a way to start it inside kind 2
   (the owner then asked for REQUEST for both `files` and `git`); DONE
   because after END the sender must learn that the receiver stored and
   verified the bytes (and, for an image, the reference a send names).
3. **Transfer ids.** The side that sends a transfer's first message
   (REQUEST or OFFER) picks its id. Phone-picked ids have the top bit
   clear (`0x0000_0001..=0x7FFF_FFFF`), daemon-picked ids have it set
   (`0x8000_0001..=0xFFFF_FFFF`), so both may start transfers without
   collision. Each side counts up from 1 per connection and never reuses an
   id on that connection. A REQUEST or OFFER with an id already live, or of
   the other side's half, is REFUSE `protocol`. A CHUNK or END for an
   unknown id is answered with CANCEL `unknown_transfer` once per id; a
   REFUSE, DONE or CANCEL for an unknown id is ignored. The daemon starts
   no transfers in this plan; its half is for plugin namespaces later.
4. **Limits** (constants in `libflankr_blob`):

   | Constant | Value |
   |---|---|
   | `CHUNK_MAX` (accepted) | 983 040 bytes (960 KiB; header + body stay under `MAX_MESSAGE`) |
   | `CHUNK_DEFAULT` (sent) | 262 144 bytes (256 KiB) |
   | `IMAGE_MAX` | 20 971 520 bytes (20 MiB) |
   | `FILE_MAX` | 8 388 608 bytes (8 MiB) |
| `GIT_MAX` | 8 388 608 bytes (8 MiB), the cut marker included |
| `DAEMON_GIT_RUNNING` | 2 `git` processes at once, across connections |
   | `IMAGE_MIN` | 16 bytes |
   | `ACTIVE_PER_SIDE` | 4 live transfers each side starts, per connection |
   | `DAEMON_IMAGES_ACTIVE` | 8 incoming images at once, across connections |
   | `OFFER_TIMEOUT` | 30 s from REQUEST or OFFER to its answer |
   | `IDLE_TIMEOUT` | 60 s without a CHUNK or END after ACCEPT |
   | `DONE_TIMEOUT` | 30 s from END to DONE or CANCEL |
   | `TRANSFER_MAX_TIME` | 10 min from OFFER to END |

   Over `ACTIVE_PER_SIDE` is REFUSE `busy`. A timeout is CANCEL `timeout`
   from whichever side notices. A connection that closes ends all its
   transfers; nothing resumes (a partial image file is deleted).
5. **Interleaving and backpressure.**
   - Senders send `CHUNK_DEFAULT` (256 KiB, about 1 s at 2 Mbit/s), never
     more than the peer's ACCEPT `chunk_max` nor `CHUNK_MAX`.
   - **The daemon's link writer gets two queues:** control (JSON-RPC
     lines, terminal frames, and every kind-2 message but CHUNK) and bulk
     (CHUNKs). It drains control first (`tokio::select!` with `biased`),
     and takes one bulk message only when control is empty, so JSON-RPC
     and terminal frames wait at most one CHUNK. The phone's writer does
     the same.
   - **Each transfer has at most 2 CHUNKs queued** to its writer. The
     sender queues the next only when the writer has written the one
     before last (a oneshot from the writer, as `Conn::written` counts
     lines). Two transfers' CHUNKs alternate in the bulk queue.
   - **The receiver applies each CHUNK before reading the next message**
     (the daemon writes it to the open file, at most 256 KiB, through
     `spawn_blocking`; the phone appends it to a buffer allocated to
     OFFER's `size`), so a slow receiver slows the TCP stream rather than
     buffering without bound. There is no window or credit message.
6. **Refusal and cancel codes** (`u16`; 0 reserved; 1000-65535 for a
   namespace's own codes). The reason text is for logs and the app's
   detail line, never contents.

   | Code | Name | Meaning | App's line |
   |---|---|---|---|
   | 1 | `unsupported` | namespace or subcommand not served | "This daemon can't take that." |
   | 2 | `no_session` | the session's pane is gone | "That session has closed." |
   | 3 | `not_permitted` | the agent has no `[[agents]]` entry or is out of reach, or the path's rules refuse it | "Not allowed for this session." |
   | 4 | `too_large` | over `IMAGE_MAX`, `FILE_MAX` or REQUEST's `max_size` | "Too large (over 20 MB)." / "(over 8 MB)." |
   | 5 | `bad_type` | MIME not allowed, magic bytes disagree, or text not UTF-8 | "Not a PNG, JPEG, WebP or GIF image." / "No preview for this kind of file." |
   | 6 | `busy` | over a concurrency limit | "Busy, try again in a moment." |
   | 7 | `no_space` | the store cannot make room | "No room on the computer for images." |
   | 8 | `not_found` | no such file | "File not found." |
   | 9 | `files_off` | `read` is empty | "File access is off on this daemon." |
   | 10 | `hash_mismatch` | SHA-256 differs from OFFER's | "The upload was damaged; try again." |
   | 11 | `size_mismatch` | END or the bytes differ from OFFER's size | as 10 |
   | 12 | `timeout` | a timeout of 4.4 | "Timed out." |
   | 13 | `io` | a write or read failed | "The computer couldn't save it." |
   | 14 | `protocol` | out of order, bad body, wrong namespace or id | "Something went wrong (protocol)." |
   | 15 | `cancelled` | the user cancelled | (no line) |
   | 16 | `have_it` | receiver already holds this SHA-256 | (no line) |
   | 17 | `unknown_transfer` | no such live transfer | (no line) |
| 18 | `bad_request` | a `git` request that is not one of 6.5's forms, or a rev that 6.6 refuses | "That isn't a commit flankr can show." |
| 19 | `not_a_repository` | the session's folder is not in a git work tree | "This session's folder isn't a git repository." |
| 20 | `failed` | `git` exited non-zero (the reason is its stderr's first line, at most 200 characters) | "git couldn't show that: <reason>" |

7. **Namespaces.** `files`, `images` and `git` are built in. A plugin namespace
   is its reverse-DNS package name (plan 20261028_04, 3.1: lowercase
   `[a-z][a-z0-9_]*` segments joined by `.`, at least three, at most 128
   bytes), so it can never equal a one-word built-in. A namespace this
   daemon does not serve is REFUSE `unsupported`. How a plugin registers a
   namespace is left to a later amendment of the plugins plan.
8. **The selected session.** REQUEST's and OFFER's `session` is the pane
   id of the open agent view (`AgentInfo.pane_id`). The daemon resolves it
   against a fresh herdr snapshot as `agents.send` does
   (`resolve_target`); a pane it cannot find is `no_session`. Every check
   of decisions 5 and 6 is against that pane's agent and that pane's `cwd`.
9. **The crate's API.** Pure state, no I/O, no async, depending on
   `sha2` only (as libflankr_term depends on `serde` and libflankr_proto):

   ```rust
   pub const NS_FILES: &str = "files";
   pub const NS_IMAGES: &str = "images";
   pub const NS_GIT: &str = "git";

   pub struct Namespace(/* validated ASCII, 1..=128 */);
   impl Namespace { pub fn new(s: &str) -> Result<Namespace, NamespaceError>; pub fn as_str(&self) -> &str; }

   #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
   pub struct TransferId(pub u32);
   impl TransferId { pub fn started_by_daemon(self) -> bool; }

   /// Hands out a side's ids (4.3).
   pub struct Ids { /* next */ }
   impl Ids { pub fn phone() -> Ids; pub fn daemon() -> Ids; pub fn next(&mut self) -> TransferId; }

   #[derive(Clone, Debug, PartialEq, Eq)]
   #[non_exhaustive]
   pub enum Body {
       Request { session: String, request: String, max_size: u32 },
       Offer(Offer),
       Accept { chunk_max: u32 },
       Refuse { code: Code, reason: String },
       Chunk { offset: u64, data: Vec<u8> },
       End { size: u64 },
       Done { body: Vec<u8> },
       Cancel { code: Code, reason: String },
   }
   pub struct Offer { pub session: String, pub name: String, pub mime: String,
                      pub size: u64, pub sha256: [u8; 32], pub modified_ms: u64 }
   pub struct BlobMessage { pub transfer: TransferId, pub ns: Namespace, pub body: Body }
   impl BlobMessage {
       pub fn encode(&self) -> Vec<u8>;
       pub fn encoded_len(&self) -> usize;
       /// `Err(Fatal)` for an unreadable header (close), `Err(Answer(id, ns))`
       /// for a readable header with a bad body (CANCEL `protocol`).
       pub fn decode(bytes: &[u8]) -> Result<BlobMessage, DecodeError>;
       /// CHUNK without copying the data, for senders.
       pub fn encode_chunk(transfer: TransferId, ns: &Namespace, offset: u64, data: &[u8]) -> Vec<u8>;
   }

   #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
   pub struct Code(pub u16);   // with the consts of 4.6: Code::NOT_PERMITTED, ...

   /// The receiving side of one transfer: feed it messages and the clock,
   /// it says what to send or do.
   pub struct Receiving { /* state, bytes, hasher, deadlines */ }
   pub struct Sending   { /* state, offset, deadlines */ }
   // Both: fn on_message(&mut self, body: &Body, now: Instant) -> Step;
   //       fn on_tick(&mut self, now: Instant) -> Step;
   // Receiving::chunk(&mut self, offset, &[u8]) -> Result<(), Code> checks
   // order and size and feeds the hasher; Receiving::finish(size) checks
   // size and hash. Sending::next_chunk(&mut self, len) -> Option<(u64, usize)>.

   /// The image type by magic bytes, from at least the first 12 bytes.
   #[derive(Clone, Copy, Debug, PartialEq, Eq)]
   pub enum ImageType { Png, Jpeg, WebP, Gif }
   impl ImageType {
       pub fn sniff(head: &[u8]) -> Option<ImageType>;
       pub fn mime(self) -> &'static str;      // image/png, image/jpeg, image/webp, image/gif
       pub fn ext(self) -> &'static str;       // png, jpg, webp, gif
       pub fn from_mime(mime: &str) -> Option<ImageType>;
   }

   /// A `git` request (6.5), parsed and validated; `Display` writes it back.
   pub enum GitRequest {
       Show { rev: Rev, path: Option<String> },
       DiffRange { from: Rev, to: Rev, path: Option<String> },
       DiffWorkingTree { path: Option<String> },
   }
   impl GitRequest { pub fn parse(s: &str) -> Result<GitRequest, Code>; }
   /// A rev of 6.6: a hash or HEAD~N.
   pub enum Rev { Hash(String), Head(u16) }

   /// The `read` pattern syntax and the always-refused names, moved here
   /// from `src/llm/planning.rs` and `src/project.rs` so the daemon and the
   /// app match paths with one piece of code (10.1).
   pub struct ReadPattern { /* segments */ }
   impl ReadPattern { pub fn parse(s: &str) -> Result<ReadPattern, String>; pub fn matches(&self, path: &str) -> bool; }
   pub fn always_refused(path: &str) -> bool;
   ```

   Magic bytes: PNG `89 50 4E 47 0D 0A 1A 0A`; JPEG `FF D8 FF`; GIF
   `GIF87a` or `GIF89a`; WebP `RIFF` at 0 and `WEBP` at 8. The daemon's
   `ReadPattern` and `always_refused` become re-exports of these, with the
   same tests.

## 5. Decisions: `images`, phone to daemon

1. **The protected temp folder is fixed and not configurable:**
   `<temp>/flankr-<uid>/images/<pane>/`, where `<temp>` is
   `std::env::temp_dir()` read once at daemon start (on macOS the per-user
   `$TMPDIR`, `/var/folders/…/T/`; on Linux `$TMPDIR` or `/tmp`), `<uid>`
   the daemon's numeric uid, and `<pane>` the pane id with every character
   outside `[A-Za-z0-9_-]` replaced by `_`, cut to 64 characters. No config
   key, flag or environment variable of flankr's names it.
2. **Created safely** (with `rustix`, on the daemon's thread pool):
   - each of `flankr-<uid>`, `images` and `<pane>` is opened with
     `openat(parent, name, O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`, and on
     `ENOENT` created with `mkdirat(parent, name, 0o700)` and opened again;
   - each opened directory is `fstat`ed: it must be owned by the daemon's
     uid and its mode `0o700` exactly; otherwise the store is off for the
     daemon's run, every image OFFER is REFUSE `io`, and one warning is
     logged ("the image folder <path> is not private: owner <uid>, mode
     <mode>; images are refused");
   - the file is created as `<name>.part` with `openat(pane_dir, …,
     O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)`;
   - at END, after the size and SHA-256 check, it is `fsync`ed and
     `linkat`ed to its final name (which fails rather than replaces if the
     name exists), and the `.part` is `unlinkat`ed. The final file is never
     opened for writing again.
3. **The name:** `<YYYYMMDD-HHMMSS>-<hash8>.<ext>`, the daemon's local time
   at END, the first 8 hex digits of the SHA-256, and the extension of the
   sniffed type. If that name exists with the same size, it is the same
   image (same second, same hash): the new `.part` is deleted and the
   existing file is used. The phone's display name is never used in a path
   and never logged.
4. **The type is checked twice:** OFFER's `mime` must be one of
   `image/png`, `image/jpeg`, `image/webp`, `image/gif` (else REFUSE
   `bad_type`), and the first CHUNK's first 12 bytes must sniff to that
   type (else CANCEL `bad_type`, `.part` deleted). The daemon never decodes
   an image.
5. **OFFER is accepted when** the namespace is served, `size` is
   `IMAGE_MIN..=IMAGE_MAX`, the session resolves (`no_session`), its agent
   has an `[[agents]]` entry this principal may reach (`policy::reach`;
   else `not_permitted`), the limits of 4.4 hold (`busy`), and the store
   has room after cleanup (`no_space`). ACCEPT carries `chunk_max =
   CHUNK_MAX`.
6. **DONE's reference** is 8 random bytes (`getrandom`, already a
   dependency through `snow`), shown in `parts` as 16 lowercase hex digits.
   The daemon keeps `reference → (path, pane id, principal id, stored at)`
   in memory for 30 min (`IMAGE_REF_TTL`), keyed by the link entry's
   principal rather than the connection, so a reconnect between upload and
   send keeps it. A reference is used by at most one send, then forgotten
   (the file stays until cleanup).
7. **Cleanup** runs at daemon start, every hour, and before each ACCEPT:
   - `.part` files older than 1 h are deleted;
   - files whose mtime is older than 7 days are deleted;
   - while the folder's total exceeds 500 MiB less the incoming size, the
     oldest files are deleted, skipping files a live reference names; if
     it still cannot fit, REFUSE `no_space`;
   - empty `<pane>` folders are removed.
   The walk does not follow symlinks; anything that is not a regular file
   or a directory is deleted with `unlinkat`, logged once.

## 6. Decisions: `files` and `git`, daemon to phone

### `files`

1. **Exactly `read_file`'s path rules.** REQUEST's `request` is the path,
   relative to the session's project root (the pane's `cwd`,
   canonicalised; a leading `/` means that root, as for `read_file`). It
   is allowed when it matches a `[llm.uses.planning] read` pattern and
   none of `always_refused`'s names (`.private`, `.git`, `.env`, `.env.*`,
   `.`, `..`, empty segments); the canonical path must stay inside the
   root and be allowed by its own name too (so a symlink out, or onto a
   refused name, is refused); and it must be a regular file. An empty
   `read` is REFUSE `files_off`, and `files` is then not listed in
   `blob.namespaces`. The agent must have an `[[agents]]` entry this
   principal may reach (`not_permitted`), as the terminal's read-only mode
   requires.
2. **The one difference: bytes, not UTF-8 text.** `ProjectFiles` gains
   `read_bytes(path, max) -> Result<FileBytes, ReadRefusal>` sharing one
   private `resolve(path)` with `read`, so the path rules are one piece of
   code. In place of `read`'s UTF-8 rule, a file is served only when it is
   a previewable type: UTF-8 text (`text/markdown` for `.md` and
   `.markdown`, `text/x-diff` for `.diff` and `.patch`, `text/plain` for
   any other UTF-8 file), or an image whose magic bytes sniff to PNG,
   JPEG, WebP or GIF. Anything else is REFUSE `bad_type`. Files are not
   cut: over `min(FILE_MAX, max_size)` is REFUSE `too_large`.
3. **Read once, sent from memory.** At REQUEST the daemon reads the whole
   file (at most 8 MiB) on the blocking pool, hashes it, and OFFERs it;
   CHUNKs come from that buffer, so a file changing during the transfer
   cannot break the hash. `modified_ms` is the file's mtime. OFFER's
   `name` is the root-relative path as resolved. User-tapped `files`
   requests get no exemption from these rules (the owner: "for now, it's
   mostly for reading docs"; section 16).
4. **The phone's cache.** A received file is kept in
   `cache_dir()/libmobile-share/fl-<hash16>.<ext>` (the directory the
   libmobile plan's provider serves, 10.6), keyed by SHA-256, at most
   64 MiB in all, least recently viewed deleted first. A `git` output is
   kept the same way, as `.diff`.

### `git`

5. **Requests**, REQUEST's `request` in namespace `git`, exactly one of
   (single spaces, nothing else):

   | Request | argv after the common prefix (6.7) |
   |---|---|
   | `show <rev>` | `show --no-color --no-ext-diff --no-textconv --end-of-options <rev>` |
   | `show <rev> -- <path>` | the same, then `-- ./<path>` |
   | `diff <rev>..<rev>` | `diff --no-color --no-ext-diff --no-textconv --end-of-options <rev>..<rev>` |
   | `diff <rev>..<rev> -- <path>` | the same, then `-- ./<path>` |
   | `diff` | `diff --no-color --no-ext-diff --no-textconv HEAD` (the working tree and the index against `HEAD`; untracked files are not shown) |
   | `diff -- <path>` | the same, then `-- ./<path>` |

   Anything else is REFUSE `bad_request`. Nothing from the phone becomes
   an option: revs are validated (6.6), `--end-of-options` stops option
   parsing before them, and the path goes after `--` as `./<relative>`
   with `GIT_LITERAL_PATHSPECS=1`, from `git::check_path` against the
   root (inside the project, no `~`, no spaces or control characters, the
   bounds of `check_paths`), else `not_permitted`.
6. **Revs:** a hash, `^[0-9a-f]{7,40}$`, or `HEAD` or `HEAD~N` with `N`
   in `0..=999`, written without leading zeros. Nothing else: no branch or
   tag names, `^`, `@`, `:` or ranges other than 6.5's single `..`.
7. **The command.** `git` is the binary `git::binary()` found on `PATH`
   at start (none: `git` is not in `blob.namespaces`). Every run is
   `git -C <root> --no-pager -c core.fsmonitor=false -c
   log.showSignature=false -c diff.external= -c core.pager=cat` then 6.5's
   argv, where `<root>` is the session's project root as `recent_commits`
   uses it (the pane's `cwd`). No shell. The environment is **cleared**,
   then only `PATH`, `HOME`, `TZ` (if set in the daemon's), `LC_ALL=C`,
   `GIT_CONFIG_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0`,
   `GIT_LITERAL_PATHSPECS=1` and `GIT_OPTIONAL_LOCKS=0` (so a `diff` does
   not write the index). stdin is closed, `kill_on_drop`, at most
   `DAEMON_GIT_RUNNING` (2) at once (else `busy`), and a 5 s timeout
   (REFUSE `timeout`, the process killed).
8. **The 8 MiB cut.** stdout is read in pieces into a buffer; at
   `GIT_MAX` less the marker's length the process is killed and the marker
   `\n[flankr: cut at 8 MiB]\n` appended. A cut output is served, not
   refused. A non-zero exit that was not the cut is REFUSE `failed` with
   stderr's first line; exit 128 with "not a git repository" is
   `not_a_repository`. stderr is read up to 4 KiB and never sent beyond
   that first line.
9. **Who may.** The session must resolve and its agent have an
   `[[agents]]` entry this principal may reach (`not_permitted`), as for
   any reading of a session today. The `read` patterns do **not** apply
   to `git`, and an empty `read` does not turn it off (the owner: "the
   limit is mostly there so gemini doesnt try coding, it doesnt need to be
   enforced on git paths called from user tapping a git hash"). Only the
   fixed command, rev validation, the path check of 6.5, the session's own
   repository on this machine, and the caps apply. `git` is never offered
   to the general agent by this plan.
10. **The OFFER.** The whole output (at most 8 MiB) is buffered, then
    OFFERed as for `files`: `name` is the request as normalised
    (`show 1a2b3c4d`, `diff HEAD~3..HEAD -- src/a.rs`), `mime`
    `text/x-diff`, `modified_ms` 0. A `show` or `diff` with no output (no
    changes) is OFFERed as the empty text `(no changes)\n`.

## 7. Decisions: sending a message with images

1. **`parts` (3.4)**, at most 16 parts and `images_per_send` (4) images,
   each image part a reference this principal holds for this `pane_id`
   (else `NOT_PERMITTED`, "That image was not uploaded for this session";
   an expired one, "That image has expired; attach it again").
2. **Who may send.** The owner's exception, precisely:
   - **an image-only send** (no text part with a letter or digit) needs
     only `policy::reach`: the agent has an `[[agents]]` entry and the
     principal may reach it. It does **not** need `allow_arbitrary_commands`,
     and does not need `allow_messages`;
   - **a send with text** checks the text exactly as today:
     `check_message` (entry and reach, `allow_messages`,
     `validate_message`, `check_paths`);
   - a new `policy::check_parts(principal, agents, agent, pane_id, parts,
     root) -> Result<Checked, Denied>` implements both, returning
     `require_confirmation: true` as `check_message` does.
3. **Composition, after the checks.** The text parts are joined with one
   space and checked as one message (so the first-word rule, the 4000
   character limit and `check_paths` apply to the user's words as a
   whole). Each text part is then cleaned as `validate_message` cleans
   (runs of space folded, surrounding space trimmed). The line sent is the
   parts in order, joined with single spaces, each image part replaced by
   its file's absolute path. The composed line is **not** run through
   `check_paths` (its image paths are outside the project by design), and
   `/` is not loosened for the user's text: a path the user typed into the
   temp folder is still refused as outside the project. Only paths the
   daemon made itself, from a live reference, are inserted.
4. **A leading image.** A line may not start with `/` (`COMMAND_PREFIXES`).
   When the first part is an image, its path is written in double quotes,
   `"/…/20261028-210400-9f3c1a2b.png"`, which agents read as a quoted path
   (open question 2). Paths never contain spaces or quotes: the folder is
   under the daemon's `temp_dir()` and the file name is daemon-made; if
   `temp_dir()` itself contains a space or a quote the store is off, as in
   5.2.
5. **Confirmation, unchanged.** A companion's tap (`confirmed: true`,
   `tapped`) sends at once, as for typed messages today; otherwise the
   composed line goes through the confirmation broker as a message
   (`request_message`), and the confirmation card shows it with each image
   path shortened to `[image N]`. The general agent's `send_agent_command`
   takes no images in this plan, so a voice-initiated send never carries
   one and the voice confirmation gate (AGENTS.md section 3.B) is
   unchanged.
6. **Logged:** "Message for pane '<pane>' with <n> image(s) confirmed by
   the tap (<chars> characters)", as today's line plus the count.

## 8. Decisions: what the daemon logs

Never file contents, image bytes, the phone's display names, or full
hashes. At info: a transfer's start (connection id, transfer id,
namespace, pane, MIME, size, hash8), its end (bytes, milliseconds, DONE or
the code), every REFUSE and CANCEL (code and the reason text, which never
holds contents), the stored image's file name, the `files` path as
resolved (as `read_file` logs its path), each `git` request as
normalised with its exit status, milliseconds, output bytes and whether it
was cut (never the output, never stderr beyond the first line), and each
cleanup that deletes anything (count and bytes). At debug: ACCEPT's `chunk_max`. At trace: each
CHUNK's offset and length. Warnings: the store turned off (5.2), a
connection closed for an unreadable kind-2 header.

## 9. Decisions: the app's message card and image sources

1. **Sources, all three.**
   - **Attach** (a paperclip glyph at the left of the message card's input
     row, shown when `blob.namespaces` has `images`): libmobile's
     `request_media` with `kind: MediaKind::Mime(["image/png",
     "image/jpeg", "image/webp", "image/gif"])`, `camera:
     CameraEntry::Photo`, `picker: Picker::Chooser`, `max_items: 4`,
     `max_bytes: 20 MiB`, `delivery: Delivery::Bytes`: Android's chooser
     with the gallery, files and a camera entry, as the owner described
     the media request.
   - **Share → flankr**: flankr is an ACTION_SEND / ACTION_SEND_MULTIPLE
     target for those four MIME types (the libmobile plan, section 10.6).
     A share opens a **Send to…** sheet listing the active remote's
     sessions as the drawer lists them, the session last viewed first and
     highlighted; a tap opens that agent view with its message card open
     and the images attached. More than 4 images: the first 4, and a note
     "Only 4 images can go in one message." Another type: a toast "flankr
     takes PNG, JPEG, WebP and GIF images."
   - On the desktop build, Attach opens nothing and is hidden.
2. **In the card.** Each attached image is numbered 1-4 and inserted at
   the text cursor as the token `[image N]` with a space either side; a
   **chip row** above the text shows a 48 dp square thumbnail for each,
   with its number in the corner and a ✕. Deleting a token from the text
   detaches its image; the ✕ removes both. The thumbnail chip is the image
   in the card; the token marks its place. (A thumbnail drawn inline in
   the `TextArea` itself would need an inline-object feature of
   libcontrols; open question 4.)
3. **Upload at attach time.** Each image starts its `images` transfer
   when attached, to the session of the open agent view. The chip shows a
   progress ring (bytes the phone's writer has written over `size`,
   redrawn at most 10 times a second), a ✓ on DONE, and on REFUSE or
   CANCEL a red ! with the code's line (4.6) and a tap to retry. Send is
   disabled with "Uploading 1 of 2…" while any chip is uploading, and with
   the error line while any failed. A connection that drops restarts
   uploads that had no DONE; a DONE's reference survives a reconnect (5.6).
4. **On Submit**, `flankr-companion` splits the draft at tokens
   `[image N]` whose image is attached and done, into `parts`, and sends
   `agents.send { pane_id, command: "", kind: message, parts, confirmed:
   true }`. A draft with no images sends as today. The sent history shows
   the text with `[image N]` and the thumbnails.
5. **Thumbnails** are decoded on a worker thread from the image bytes:
   images over 8192 px on a side or 40 megapixels are refused before any
   allocation (a ! chip, "Too large to show"); the thumbnail is the image
   box-filtered to at most 96×96 px, uploaded once as a texture. GIF uses
   its first frame. The bytes are kept until the message is sent or the
   card is cleared, at most 4 × 20 MiB.

## 10. Decisions: the built-in preview

1. **Entry points.**
   - **Paths in transcript cards, as links, now** (the owner: "actual
     paths can be supported now as links"). In every card's shown text
     (text entries and tool calls, including a tool call's `file_path`),
     a word that `path_words` would take as a path, written either
     relative to the session's `AgentInfo.cwd` or absolute inside it, is
     a candidate, exactly as the card text shows it (no guessing at bare
     names). It is **drawn as a link only when it would be readable by its
     name**: made relative to `cwd`, it is not `always_refused` and
     matches one of `blob.read`'s patterns, with libflankr_blob's
     `ReadPattern` (the daemon's own matcher). Otherwise it is plain text,
     so a link never leads to a certain refusal. Whether the file exists,
     and the symlink rule, are known only to the daemon: such a tap
     opens the preview, which shows the refusal's line (`not_found`,
     `not_permitted`). A tap opens the preview through `files`. With
     `files` not served, no path is a link.
   - **Commit hashes, as links:** in a card's shown text, a word of 7-40
     lowercase hex digits holding at least one digit and one of `a-f`,
     bounded by non-alphanumerics, opens `git` `show <hash>`; in the
     general agent's chat, each commit of a `recent_commits` result does
     the same. A word that is not a commit gets `failed` ("unknown
     revision") in the preview. With `git` not served, no hash is a link.
   - **The session's changes:** the agent view's ⋮ menu (or, before the
     plugins branch lands, a long press on the session's title) gets
     **Show uncommitted changes**, `git` `diff`.
   - **Later, the planning plugin's amendment list** may link an
     amendment's commits the same way (an amendment there, not written
     now).
   - **The planning plugin's plan rows** (plan 20261028_04, when
     `main-plugins` lands): a row gets an **Open** action that previews the
     plan's file through `files`. This is a small amendment to that plan's
     living section, to be proposed when it lands; nothing is changed there
     by this plan.
   - **An attached image's chip:** a tap previews the local bytes (no
     transfer).
2. **The modal.** A full-screen libcontrols `ModalDialog`: a 56 dp title
   bar with ✕ at the left, the file's name (middle-ellipsised) over its
   folder and size in the dim colour (for `git`, the request, `show
   1a2b3c4d`, over the session's project name; a cut output says "cut at
   8 MB" beside the size), and at the right the **share icon**
   (Android only), which opens a `PopupMenu` (libcontrols 0.10.3, from the
   plugins branch) with **Open in…** and **Share**. While the transfer
   runs the body shows "Loading…" and a progress bar (bytes received over
   `size`); a REFUSE or CANCEL shows its line (4.6) and **Try again**.
3. **Bodies.**
   - **Markdown** (`text/markdown`): `MarkdownView` on `flankr-markdown`,
     in a `ScrollPane`, at the transcript's text size. `http(s)` links open
     with `open_link`; a relative link to a `.md` file resolves against the
     file's folder and opens another preview through `files`; other links
     do nothing. Over 512 KiB, the file is shown as plain text, with a note.
   - **Plain text** (`text/plain`): `NOTO_SANS_MONO_ATLAS` at 12 pt,
     wrapped to the width with a dim ↩ at each soft wrap, at most 20 000
     lines, then "Cut at 20 000 lines; Share to see it all."
   - **Diff** (`text/x-diff`, every `git` output): as plain text, with lines starting `+` in
     the theme's added colour, `-` in its removed colour, `@@` in the
     accent, and `diff `, `index `, `--- `, `+++ ` dim (the transcript's
     diff colours).
   - **Image:** decoded on a worker thread (the 8192 px / 40 MP refusal of
     9.5), downscaled by an integer factor to at most 4096 px on its long
     side, one texture; fit to width; pinch zoom 1×-8×; pan; double tap
     toggles fit and 1:1. GIF: its first frame.
4. **Decoders.** The app adds `zune-jpeg`, `image-webp` and `gif`
   (pure Rust) beside `png` (open question 5).
5. **Open in… and Share** write nothing new: the preview's bytes are
   already in `cache_dir()/libmobile-share/` (6.4) and the libmobile calls
   take that path and the MIME type. A `git` output is shared as a file,
   `<request with spaces and dots as ->.diff`, `text/x-diff`. An attached
   image's chip previews its bytes, and its share writes them there first.
6. **The libmobile dependency (a new plan, to be written in libmobile, not
   here):** a *share-and-view* plan that adds:
   - a read-only content provider for `<cacheDir>/libmobile-share/<name>`
     (`<name>` `^[A-Za-z0-9._-]{1,96}$`), authority
     `${applicationId}.libmobile.share`, `exported="false"`,
     `grantUriPermissions="true"`, beside media request's
     `MediaFileProvider` (feature `share-out`);
   - `view_file(path, mime)` (`ACTION_VIEW` in `Intent.createChooser`,
     `FLAG_GRANT_READ_URI_PERMISSION`) and `share_file(path, mime, title)`
     (`ACTION_SEND` with `EXTRA_STREAM`, in a chooser);
   - receiving `ACTION_SEND` and `ACTION_SEND_MULTIPLE` (feature
     `share-target`, an `<intent-filter>` with the app's MIME list),
     copying each `EXTRA_STREAM` through the content resolver under a size
     cap, as `MobileEvent::Shared { items }` with media request's item type;
   - and libmorph surfacing these and media request to the app, with a
     libmorph rev bump in the app.

## 11. Testing

- **libflankr_proto:** a binary message round-trips as `Message::Binary`
  with flag bytes `0x20`/`0x21`; a 1 MiB binary message spans frames;
  `recv_message` is `BadFrame` on kind 2; kinds 3-14 and 15 stay
  `BadFrame` on `recv`; a kind change mid-message is `BadFrame`;
  `BlobInfo` and `parts` round-trip, are left out when absent, and an old
  `HelloResult`/`AgentsSendParams` JSON decodes with `None`/empty.
- **libflankr_blob:** every subcommand encodes and decodes to the byte
  layouts of 4.1 (golden bytes for each); headers shorter than 6 bytes, a
  0-length, 129-byte, upper-case or `.`-ended namespace, and id 0 are
  `Fatal`; a bad body or reserved subcommand is `Answer`; trailing bytes
  are refused; CHUNK out of order, past `size`, or over `chunk_max` is
  `protocol`; END with a short count is `size_mismatch`; a flipped byte is
  `hash_mismatch`; the timeouts of 4.4 fire from `on_tick`; `Ids` never
  repeats and keeps to its half; `sniff` of each type's magic, of 11
  bytes, and of a PNG named JPEG; a property test that random bytes never
  panic `decode`.
- **flankr (daemon):** in a temp dir standing in for `temp_dir()`: an
  upload lands at `<temp>/flankr-<uid>/images/<pane>/<time>-<hash8>.png`
  with modes 0700/0600; a pre-made symlink at `flankr-<uid>`, `images` or
  the pane folder, a folder with mode 0755, or one owned by another uid
  (where the test can make one) turns the store off; a `.part` is never
  left after a CANCEL or a dropped connection; cleanup's age, size and
  live-reference rules; 5 concurrent uploads on one connection get `busy`
  on the fifth; a JSON-RPC `ping` sent during a 20 MiB upload is answered
  before the upload's next CHUNK is written (the writer's priority).
  `files`: the `read_file` test matrix again (`README.md` and
  `docs/a.md` served with `read = ["*.md", "docs/**/*.md"]`; `src/main.rs`,
  `.env`, `.private/key`, `../x.md`, a symlink out and a symlink onto
  `.env` refused; empty `read` is `files_off`), plus a PNG served as
  `image/png`, a binary file `bad_type`, and a 9 MiB file `too_large`.
  `git`, in a temp repository with three commits: `show <hash>`,
  `show HEAD~1 -- a.txt`, `diff HEAD~2..HEAD`, `diff` with an edited and a
  staged file; refused with `bad_request`: `show --output=/tmp/x`,
  `show main`, `show HEAD^`, `show HEAD~1000`, `show abc` (too short),
  `diff a..b..c`, two spaces, a trailing space; `show <hash> -- ../x`
  and `-- ~/x` `not_permitted`; a path *not* matching `read` (with `read`
  empty) served; a repository whose `.git/config` sets `diff.external`
  and a textconv driver runs neither; the child's environment holds only
  6.7's variables (checked with a `git` stand-in on `PATH` that prints its
  environment); a 9 MiB diff is cut with the marker and served; a
  stand-in that sleeps 6 s is `timeout` and killed; a third concurrent run
  is `busy`; a folder outside any repository is `not_a_repository`.
  `agents.send`: image-only to an agent with `allow_messages = false` and
  `allow_arbitrary_commands = false` is sent; text + image to that agent
  is refused; a typed `/tmp/…` path in the text is still refused; the
  composed line has each path in its place, the leading one quoted;
  another pane's or principal's reference is refused; a reference is
  single-use.
- **flankr-companion:** Submit splits `[image N]` tokens into `parts`;
  a deleted token detaches its image; uploads restart after a reconnect;
  a file's OFFER whose hash is cached gets REFUSE `have_it`.
- **companion app:** the preview's Markdown, text, diff and image bodies
  laid out at 412 dp; the 8192 px refusal; path links for
  `docs/design/x.md` and `<cwd>/docs/design/x.md` with
  `read = ["docs/**/*.md"]`, and none for `src/main.rs`, `.env`,
  `../x.md`, a path outside `cwd`, or a bare `x.md`; hash links for
  `1a2b3c4` and none for `1234567`, `deadbeefcafe` without a digit, or
  `abc12`.
- **On the S23** (owner's device, by adb, as other plans): an image from
  the camera, one from the gallery, and one shared from Samsung Gallery,
  each sent to a Claude Code session, which reads the image; a plan opened
  from a transcript card and shared to Drive; a commit hash in a card
  opened as `git show`.

## 12. Steps

1. **libflankr_proto 0.5.3: kind 2, `BlobInfo`, `parts`.** In
   `crates/libflankr_proto`: `KIND_BINARY`, `Kind::Binary`,
   `Message::Binary`, `send_binary` on both writers, `recv` reading kind 2
   (3.1); `BlobInfo` on `HelloResult` and `ServerCapabilities` (3.3);
   `AgentsSendParams.parts` and `SendPart` (3.4); the module header's kind
   list updated; the tests of section 11. `VERSIONS.md` entry and tag
   `libflankr_proto-v0.5.3`. This commit sets this plan's `status` to `In
   implementation` and appends the amendment that says so.
   **Done when:** `cargo test -p libflankr_proto` passes with the new
   tests, and the existing link tests pass unchanged.
2. **libflankr_blob 0.1.0.** `crates/libflankr_blob` added to the
   workspace `members` in this commit (the one exception of Rule 3.1):
   decision 4.9's API, the layouts of 4.1, codes of 4.6, limits of 4.4,
   `sniff`, `GitRequest`, `ReadPattern` and `always_refused` (moved from
   the daemon, with their tests), the state machines, and its tests. `VERSIONS.md` entry, tag
   `libflankr_blob-v0.1.0`.
   **Done when:** `cargo test -p libflankr_blob` passes, including the
   golden bytes and the no-panic property test.
3. **flankr: the link's kind 2 and the `images` store.** `src/server/conn.rs`:
   `Outgoing::Binary { bytes, bulk: bool, written: Option<oneshot::Sender<()>> }`,
   the two-queue writer (4.5), kind 2 from the client routed to
   `src/server/blob/mod.rs` (the per-connection transfer table, ids,
   timeouts on a 1 s tick, unknown namespaces refused), `blob` in `hello`
   and `server.capabilities` for link connections; `src/server/blob/images.rs`
   (section 5: the folder, safe creation, `.part` and `linkat`, the
   references, cleanup at start, hourly and before ACCEPT); logging of
   section 8.
   **Done when:** a test client over a real link uploads a PNG and gets
   DONE with a reference, the file is at the 5.1 path with 0700/0600, and
   the daemon tests of section 11 for images and the writer pass with
   `cargo test -p flankr`.
4. **flankr: the `files` namespace.** `src/project.rs`: `resolve` shared
   by `read` and the new `read_bytes` (6.2); `ReadPattern` and
   `always_refused` re-exported from libflankr_blob (4.9); `src/server/blob/files.rs`
   (REQUEST, OFFER from memory, CHUNKs, `files` in `blob.namespaces` only
   while `read` is non-empty).
   **Done when:** the `files` matrix of section 11 passes, and the
   existing `read_file` tests pass unchanged.
5. **flankr: the `git` namespace.** `src/git.rs`: a `run_capped` that
   clears the environment (6.7), reads stdout to `GIT_MAX` and kills on
   the cut (6.8), used by the new `show` and `diff` builders; `recent_commits`
   is unchanged. `src/server/blob/git.rs`: REQUEST parsing with
   `GitRequest::parse`, the checks of 6.9, the OFFER of 6.10, `git` in
   `blob.namespaces` when the binary was found.
   **Done when:** the `git` tests of section 11 pass, and the existing
   `recent_commits` tests pass unchanged.
6. **flankr: `agents.send` with `parts`.** `src/core/policy.rs`:
   `check_parts` (7.2); `src/server/conn.rs`: `send_message` composes
   (7.3, 7.4), consumes references, shortens paths on the confirmation
   card (7.5), logs (7.6).
   **Done when:** the `agents.send` tests of section 11 pass, and every
   existing policy and send test passes unchanged.
7. **flankr-companion 0.5.2: transfers in the client.** `net.rs` routes
   `Message::Binary` and gets the two-queue writer; new `Input`s
   (`AttachImage { bytes, mime }`, `DetachImage(u8)`, `RetryImage(u8)`,
   `RequestFile { session, path }`, `RequestGit { session, request }`,
   `CancelTransfer(u32)`) and `Event`s
   (`TransferProgress { id, done, size }`, `ImageReady { n }`,
   `FileReady { id, path, mime, name, size }`, `TransferFailed { id, code,
   line }`), additive since both enums are `#[non_exhaustive]`; Submit's
   `parts` (9.4); the preview cache (6.4). `VERSIONS.md` entry, tag
   `flankr-companion-v0.5.2`.
   **Done when:** `cargo test -p flankr-companion` passes, including an
   end-to-end test against a test daemon that uploads two images, sends
   `parts`, fetches a Markdown file and a `git show`.
8. **companion: the preview modal.** `apps/companion/src/preview/`
   (section 10.2-10.4; the decoders added to `apps/companion/Cargo.toml`),
   the transcript cards' path and hash links and **Show uncommitted
   changes** (10.1), the share icon wired to the
   libmobile calls when present and hidden otherwise.
   **Done when:** a tap on a Read card's path in a running app shows the
   file's Markdown, a `.png` path shows the image, a commit hash shows its
   `git show` as a diff, and the layout tests of section 11 pass.
9. **companion: attaching images.** The paperclip, the chip row, the
   tokens and progress (9.1-9.3, 9.5), Share → flankr's **Send to…**
   sheet, on the libmobile share-and-view plan and media request as
   released by then.
   **Done when:** on the S23, an image from each of the three sources is
   sent to a Claude Code session and the agent reads it (section 11).
10. **README** (its own commit): a short paragraph under the features on
    sending images and previewing files and commits.
    **Done when:** the README describes both, and the decision table's row
    matches this plan's status.

## 13. Versioning (Rule 5)

| Crate | From | To | Why |
|---|---|---|---|
| `libflankr_proto` | 0.5.1 here, 0.5.2 on `main-plugins` | **0.5.3** | additive (`BlobInfo` is new, so its `read` and `git_max` fields are part of it from the start): a `#[non_exhaustive]` `Kind`/`Message` variant, `send_binary`, appended serde-defaulted fields, a new `#[non_exhaustive]` enum `SendPart` |
| `libflankr_blob` | — | **0.1.0** | new crate |
| `flankr-companion` | 0.5.0 here, 0.5.1 on `main-plugins` | **0.5.2** | additive: `#[non_exhaustive]` `Input`/`Event` variants; takes libflankr_proto 0.5.3 (a compatible patch) |
| `flankr` (daemon), `companion` (app) | — | no bump | applications; manifests and lock only |

The numbers assume `main-plugins` (libflankr_proto 0.5.2, flankr-companion
0.5.1) is merged into this branch before step 1. If this lands first, step 1
takes the next number free across both branches at that moment, so no tag
name is used twice; the number is fixed in the amendment that starts
implementation.

## 14. Risks

- **A shared temp folder on Linux.** With `$TMPDIR` unset, `<temp>` is
  `/tmp`, where another user could pre-create `flankr-<uid>`. The ownership
  and mode checks (5.2) turn the store off rather than writing into it.
- **The agent must be able to read the path.** Claude Code reads a pasted
  image path itself; another agent may ask for permission to read outside
  its project, or not take images at all. The send still succeeds; the
  agent's behaviour is its own (open question 2).
- **`git` shows what `files` would refuse.** By the owner's ruling (6.9),
  `git show` of a commit that added a `.env` or a file under `.private`
  shows its contents, which `read_file` would never serve. The rule is
  the owner's; it is listed so it stays a known choice.
- **Photos carry EXIF**, location included. The daemon stores them as
  sent; stripping is open question 3.
- **Large uploads on a slow link** still delay JSON-RPC by up to one 256
  KiB CHUNK (about 1 s at 2 Mbit/s). Smaller chunks cost more frames and
  hashing calls but nothing else; `CHUNK_DEFAULT` is the knob.
- **Memory on the phone:** 4 attached images (80 MiB at most) plus a file
  being received (8 MiB) and decoded textures (4096² × 4 = 64 MiB for one
  preview image). The decode limits keep a hostile file from asking for
  more.

## 15. Open questions for the owner

1. **Image-only sends and `allow_messages`.** As written, an image-only
   send needs only the agent's entry and reach (7.2), so an agent with
   `allow_messages = false` can still be sent an image. Should it also
   need `allow_messages`?
2. **A leading image's form.** The path is quoted when the message starts
   with an image (7.4). Should it rather be moved after the first text
   part, or should messages that start with an image be refused? (To be
   checked on the S23 with Claude Code, Codex and omp.)
3. **EXIF.** Leave photos as they are, or strip JPEG `APP1` (Exif, XMP)
   in the app before upload? Stripping also drops the orientation tag, so
   the app would have to rotate the pixels, which means re-encoding.
4. **Inline thumbnails.** The chip row and `[image N]` tokens (9.2) need
   nothing new. A thumbnail drawn in the text itself needs an inline-object
   feature in libcontrols' `TextArea`: ask libcontrols for it now, or
   later?
5. **New decoder crates in the app:** `zune-jpeg`, `image-webp` and `gif`
   (10.4). Agreed, or decode through Android's `ImageDecoder` in the
   libmobile plan instead (which would also cover HEIC)?
6. **Images to the general agent.** Out of scope here; a later plan could
   let Gemini see an attached image. Wanted?

## 16. Later (not steps of this plan)

- **A full file review addon or mode** may widen `files` beyond
  `read_file`'s rules (other types, paths outside the `read` patterns),
  by its own plan (the owner: "we could expand to a full file review
  addon/mode later").
- **Partial references** (a bare file name, "plan 04"), resolved by first
  enumerating the readable paths and linked only when the match is unique,
  most likely belong to the planning plugin, as a later amendment or plan
  there (the owner: "that should be in the plugin most likely"). This plan
  links only paths written out in full (10.1).
- **The planning plugin's plan rows and amendment list** opening `files`
  and `git` previews (10.1), proposed to that plan when `main-plugins`
  lands.

## Amendment — 2026-10-28 21:29 EDT: attachments in a grid, paths by paste, permissions, conversion and the Photos setting

Agreed with the owner on 2026-10-28, in order: "images can be added as
small thumbnails above the text as attachments"; "the tokens are fine
but the actual images are kept together in a small grid with x to remove
in top right (like a message app)"; "what's allow_messages? if these are
connected to a sent message the same permission applies, token can also
be used with herdr if the protocol matches (using a path instead of
paste)"; "should we convert heic to png and bake rotation?"; "what about
screenshots?"; "this can be an option with selectable label (convert
photos to jpeg) (send unmodified)"; then "yes" to the whole of this
wording. It answers section 15's questions 1-6 and replaces the text
named in each item.

1. **A grid of attachments (replaces 9.2's chip row; answers question
   4).** The `[image N]` tokens stay in the text as 9.2 says. The images
   themselves are kept together in a small grid above the text, as a
   messaging app shows attachments: up to 4 square thumbnails of 64 dp,
   in one row of up to 4 (two rows of 2 when the card is narrower than
   4 × 64 dp plus gaps), each with its number in the bottom-left corner
   and an ✕ in its top-right corner. The ✕ removes the image and its
   token; deleting a token from the text removes its image. 9.3's
   progress ring, ✓ and ! are drawn on the thumbnail. No libcontrols
   change is needed; inline thumbnails in the `TextArea` are not wanted.
2. **Each token is replaced in place, by a paste where herdr allows it
   (adds to 7.3).** When the daemon's herdr attachment matches (endpoint
   generation 1, herdr protocol 22, plan 20261027_08), the composed
   message is delivered in order: each text run as today, and each
   image's path as a herdr `Paste { text }` client message on the
   attachment, so herdr pastes it (bracketed when the pane has bracketed
   paste on) and an agent that turns a pasted image path into an
   attachment (Claude Code's `[Image #N]`) does so; then the submit key as
   today. The path is flankr's own file in its protected folder (5.2);
   herdr's `ClipboardImage` (which would stage a second copy in herdr's
   folder) is not used. Without a matching attachment (an older herdr,
   such as 0.8.0), the path is typed in place of its token, as 7.3 says.
3. **A leading image is quoted (answers question 2).** 7.4 stands: when
   the message starts with a token, that path is written in double
   quotes, in a typed message and in a paste alike.
4. **Images need `allow_messages` (replaces 7.2's first bullet; answers
   question 1).** Images are part of a sent message, so the message's
   permission applies: every send with images, image-only or not, needs
   the agent's entry, reach and `allow_messages`. An image-only send does
   not need `allow_arbitrary_commands`. The user's text is checked as 7.2's
   second bullet says, and the daemon still inserts the paths after the
   checks (7.3), so `/` is not loosened for the user's text.
5. **EXIF (answers question 3).** Nothing is stripped on its own: a
   file sent unchanged keeps its EXIF. A converted photo (item 7) is
   re-encoded and carries none, so its location goes with it. 14's EXIF
   risk stands for files sent unchanged.
6. **Decoding through Android (replaces 10.4's decoders; answers
   question 5).** The app adds no `zune-jpeg`, `image-webp` or `gif`.
   Thumbnails, the preview's images and conversion decode through
   Android's `ImageDecoder` (it applies the EXIF orientation and reads
   HEIC/HEIF), and conversion encodes with `Bitmap.compress`, both through
   the libmobile share-and-view plan this plan already depends on, which
   gains `decode_image(bytes, max_side) -> Rgba` and
   `encode_image(rgba, Jpeg { quality } | Png) -> Vec<u8>`. The desktop
   build previews PNG only, through the `png` crate it already has.
7. **Conversion on the phone, before upload (new).** Under **Convert
   photos to JPEG** (item 8's default):
   - **PNG stays PNG**, so screenshots arrive as taken: a PNG under the
     20 MiB cap is sent byte for byte at full size (an S23 Ultra
     screenshot, 1440 × 3088, is about 1-3 MB); a PNG over the cap is
     scaled down by halves until it fits and stays PNG, never JPEG.
   - **A photo is converted** when it is HEIC or HEIF, has an EXIF
     orientation other than 1, or is longer than 2048 px on its long
     side: decoded with the rotation applied, scaled to at most 2048 px on
     the long side, and encoded as JPEG at quality 90, or PNG when it has
     transparency. The 2048 px limit is for photos only.
   - **A JPEG or WebP** with no rotation, at most 2048 px and under the
     cap, is sent byte for byte.
   - **A GIF** is never re-encoded (its animation is kept); one over the
     cap is refused with `TOO_LARGE` and its line.
   - The daemon is unchanged: it checks the first bytes against the four
     types (5.3), never sees HEIC, and never decodes or rewrites an image.
8. **The Photos setting (new).** Settings → Device gains a **Photos** row
   with a libcontrols `SelectableLabel` choosing between **Convert photos
   to JPEG** (default) and **Send unmodified**, with an ⓘ:
   - **Send unmodified:** every PNG, JPEG, WebP or GIF is sent byte for
     byte, its rotation tag and EXIF included; one over 20 MiB is refused
     with `TOO_LARGE` instead of scaled. A HEIC or HEIF photo, which no
     agent reads and the daemon refuses, is still converted, losslessly
     to PNG with its rotation applied; the ⓘ says so.
   - The choice is the phone's own (each device keeps its own), stored
     with the app's other device settings, and read when an image is
     attached; the copy is in `apps/companion/copy/SETTINGS_COPY.md`. It
     is entered in the settings plan's living section
     (docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md) by its own
     amendment.
9. **No images to the general agent (answers question 6).** Not in this
   plan; a later plan may let it see an attached image.

Step 9 (the app's attaching) covers items 1, 7 and 8; its tests add the
grid's layout, the ✕ and token removal both ways, each conversion rule
with fixture images (a PNG screenshot unchanged, a PNG over the cap
scaled and still PNG, a rotated JPEG and a HEIC converted, a GIF over the
cap refused) and both settings. Step 6 (`agents.send` with `parts`)
covers items 2-4; its tests add an image-only send refused with
`allow_messages = false`, and paste delivery over herdr's mock
attachment beside typed delivery without one.

## Amendment — 2026-10-28 23:00 EDT: the preview scrolls with libcontrols' Scrollbar, labelled by section

Agreed with the owner on 2026-10-28: "we should backport the advanced
scrollbar to libcontrols and use it for the file viewer, with the optional
features like labels"; then "Yes, write the libcontrols plan and the binary
files amendment". This adds to section 10.3 (the bodies) and step 8; the
rest of 10.3 stands.

1. **The control.** Every scrolling body of the preview (Markdown, plain
   text, diff) scrolls in a libcontrols `ScrollPane` with its `Scrollbar`
   (`ScrollPane::with_scrollbar`), the agent view's scrollbar backported
   to libcontrols by its own plan there
   (~/src/libcontrols/docs/design/20261028_08_SCROLLBAR_PLAN.md, a patch
   release, 0.10.5 as drafted): a draggable thumb that scrubs, and
   optional section labels shown as translucent pills while the thumb is
   dragged, the one nearest the thumb brighter.
2. **Visibility** follows the app's *Scrollbar* setting (None, On Scroll,
   Always) as the agent view's does, except that a plain-text or diff
   body longer than three screens shows the thumb **Always** whatever the
   setting, since there is no other sense of place in it.
3. **Labels, by body:**
   - **Markdown:** one label per `#` and `##` heading, its text cut to
     40 characters, at the heading's offset;
   - **diff and `git` output:** one label per file, at its `diff --git`
     line, the file's name (its last path component; the folder when two
     share a name), and for `git show` a first label "commit <hash8>" at
     the top;
   - **plain text:** no labels, the thumb only;
   - **an image:** no scrollbar (it pans and zooms, 10.3).
4. **Order of building.** Step 8 needs the libcontrols release with
   `Scrollbar`; the app's pin moves to it in step 8's commit. If step 8 is
   built first, the bodies scroll in a `ScrollPane` with its plain
   indicator, and the scrollbar comes in a later commit recorded in
   CHANGES.md.
5. **Tests** (step 8) add: the labels' texts and offsets from fixture
   Markdown, a two-file diff and a `git show`, the duplicate-name folder
   rule, no labels for plain text, and the Always rule past three screens.
6. **Later, not this plan:** the agent view moves onto the same libcontrols
   `Scrollbar`, keeping its Finished-card labels, by its own amendment or
   plan.

## Amendment — 2026-10-28 23:43 EDT: the + of the bottom bar plan, terminal.paste, and the version numbers

The owner (2026-10-28): "yes, agree to the binary files amendment", to
the amendment the bottom bar plan
(docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md, its decision on the
terminal + and its interface block) asks of this plan. It replaces the
text named in each item.

1. **The paperclip is the bottom bar plan's + (replaces 9.1's Attach
   paperclip).** The message card's attach control is the + at the left of
   the card's new row: a tap opens the media chooser, a 2 s hold opens a
   `PopupMenu` with **Paste text** and **Attach image…**, as that plan's
   decision 8 says. The terminal keys row has the same + at its left. Both
   are hidden while the daemon does not offer `images` (`BlobInfo`). 9.2's
   tokens and grid (the 21:29 EDT amendment, item 1) are unchanged.
2. **A terminal paste beside `agents.send`'s `parts` (adds to section 7).**
   `terminal.paste { terminal_id, text | images }` as the bottom bar plan's
   interface block gives it (`TerminalPasteParams`, one of `text` and 1-4
   `images` references, result `{}`; `NOT_PERMITTED`, `UNKNOWN_AGENT`,
   `HERDR_UNAVAILABLE`, `INVALID_PARAMS`; an older daemon's
   `METHOD_NOT_FOUND` shown as "This flankr can't paste into the
   terminal."):
   - the daemon pastes the images' paths, joined by single spaces, as one
     herdr `Paste { text }` on the terminal's attachment (bracketed when
     the pane has bracketed paste on), else as literal keystrokes in
     64-byte batches; the references are consumed as a send consumes them;
   - text is at most 8192 bytes of UTF-8, `\r\n` and lone `\r` made
     `\n`, other control characters but `\t` and `\n` refused, and a
     multi-line text refused without the attachment;
   - it needs what `terminal.input` needs (`[terminal] input`, the
     agent's `terminal_input`, a principal that may approve, the terminal
     opened on this connection, the live arm), not `allow_messages` or
     `allow_arbitrary_commands`; it is logged as a `terminal_input` notice
     of kind `paste` with the agent and the byte count, never the text or
     paths;
   - flankr-companion gains `Input::TerminalPasteText(String)` and
     `Input::TerminalPasteImages(Vec<u8>)` on the `#[non_exhaustive]`
     `Input`.
   Steps 1 (libflankr_proto: `TERMINAL_PASTE`, `TerminalPasteParams`), 6
   (the daemon's handler beside `agents.send`'s `parts`) and 7 (the
   client's inputs) carry it.
3. **Version numbers (replaces section 13's table and steps 1 and 7's
   numbers).** `libflankr_proto` 0.5.3 went to the plugins plan's title
   icons and flankr-companion may take 0.5.2 for the bottom bar plan's
   `speak_gemini`: each crate here takes the next free patch number when
   its step is committed (libflankr_proto 0.5.4 as of this amendment), by
   release order, and its VERSIONS.md entry names the number it took.
