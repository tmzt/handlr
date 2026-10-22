---
title: handlr link pairing and the companion protocol
date: 2026-10-22
status: Draft
covers:
  - handlr link: Noise over TCP, via the `snow` crate, with QR-code pairing
    and a stored key on each side
  - `~/.config/handlr/auth.toml`, the paired clients, in the manner of
    `authorized_keys`
  - The `handlr.companion` protocol: conversational turns for the phone,
    with an Esc-like interrupt, structured confirmations and notices
  - The turn service: Gemini, through libinfer, calling the policy core on
    the phone's behalf
  - Not covered: the phone app (a voicexn plan), secure key storage on the
    phone (a libmobile plan), streaming replies by sentence (a later plan)
sources:
  - Owner's discussion (2026-10-22): Noise (snow) with QR activation and a
    private key stored in secure preferences (libmorph/libmobile); auth in
    an `auth.toml`, like `authorized_keys`; the phone owns voice;
    turn-based, with Esc-like interruptions.
  - The Noise Protocol Framework, revision 34: the `XX` and `IK`
    handshakes, the `psk3` modifier, 65535-byte messages, the prologue.
  - voicexn at `5b2d7e5`: on-device Kyutai STT, Phonon TTS, the earshot
    tap-to-talk gate, `TtsRequest::Stop`, half-duplex capture.
  - handlr at `9ce1a5f`: the affirmative list in `main.rs`, the Gemini
    tools (`focus`, `run_command`, `watch_pane`), `HerdrContextContributor`,
    libinfer's `SessionManager`, the notice rules in `src/notify.rs`.
related:
  - docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md (this depends on
    it)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (also carried by the
    link)
---

# handlr link pairing and the companion protocol

## 1. Goal

A phone is paired once by scanning a QR code that `handlr pair` shows.
From then on it connects to the daemon over an encrypted, mutually
authenticated link, and holds a conversation with it in turns: it sends
what the user said, it speaks what handlr answers, it is asked to confirm
commands, and it hears notices about watched agents. A tap while handlr is
working or speaking interrupts the turn, as Esc does in a coding agent. No
audio crosses the link.

## 2. Decisions

### The link

1. **Noise, with `snow`**, in `libhandlr_proto::link` behind a `link`
   feature (`snow` with its default pure-Rust resolver):
   - Pairing: `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s`, with the QR code's
     pairing secret as the PSK. Both static keys are exchanged and both
     sides are authenticated in one handshake; without the secret, nobody
     can pair.
   - Reconnecting: `Noise_IK_25519_ChaChaPoly_BLAKE2s`. The phone already
     knows the server's static key, so the handshake is one round trip and
     the first JSON-RPC message (`hello`) travels in it.
   - The prologue is `b"handlr.link/1"` followed by the mode byte (decision
     2), so a handshake cannot be replayed as the other kind.

2. **Framing.** Over TCP. The client's first byte is the mode: `0x01`
   pairing, `0x02` reconnecting; anything else closes the connection. After
   it, every Noise message is a frame: a 2-byte big-endian length and the
   message, at most 65535 bytes. After the handshake, each frame's
   plaintext is one flag byte (`0x00` last, `0x01` more) and a chunk of a
   JSON-RPC message. A message reassembled past 1 MiB closes the
   connection.

3. **Listening.** `[link] listen = ["<addr>:<port>", …]` in `config.toml`,
   default port 7347 when only an address is given. There is no default
   listener: the link is off until an address is configured, such as a
   Tailscale address. The QR code carries the configured addresses.

4. **Keys.**
   - The server's static key is `~/.config/handlr/link_key`: the 32-byte
     X25519 private key, base64, created by `serve` on first start. `serve`
     refuses to start if the file is readable by anyone but its owner.
   - The phone generates its own static key on first launch and keeps it
     in secure storage (the libmobile plan). Only its public key ever
     leaves the phone.

5. **Pairing.** `handlr pair --name <name> [--protocols companion]
   [--ttl 300]` asks the running daemon (common-method style, over the
   daemon socket: `link.pair_begin`) for a pending pairing: a 32-byte
   random secret, single use, expiring after `--ttl` seconds (default
   300). It prints a QR code in the terminal (the `qrcode` crate's Unicode
   renderer) and the same URI as text:

   ```text
   handlr://pair?v=1&n=<name>&k=<server public key, base64url>&s=<secret, base64url>&a=<host:port>[,<host:port>…]
   ```

   The phone connects with mode `0x01`, completes `XXpsk3`, and aborts if
   the server's static key differs from `k`. The daemon appends the
   phone's entry to `auth.toml` (decision 6), burns the secret, and
   answers the phone's `hello` with `paired: true`. `handlr pair` prints
   the paired name and exits.

6. **`~/.config/handlr/auth.toml`**, kept apart from `config.toml`, mode
   0600:

   ```toml
   [[client]]
   name = "s23"
   key = "x25519:<public key, base64>"
   paired = "2026-10-22T11:30:00-04:00"
   protocols = ["companion"]       # "companion", "harness"
   agents = ["claude", "codex"]    # optional; narrows [[agents]], never widens
   approve = true                  # optional; default true for companion, false for harness
   ```

   On a reconnect (mode `0x02`), the daemon learns the phone's static key
   from the first handshake message and looks it up. An unknown key closes
   the connection with no reply. The file is read again on every new
   connection, and checked every 60 s against open connections, which are
   closed if their entry is gone. Revoking a phone is deleting its entry.
   Names are unique; `pair` refuses a name already in the file.

7. **Liveness.** The client sends `ping` every 20 s. The daemon closes a
   link connection silent for 60 s. The phone reconnects with backoff (1,
   2, 4 … 30 s) and sends `hello` with the `session_id` it had.

### The companion protocol

8. **`handlr.companion` version 1.** JSON-RPC 2.0, over the link or over
   the daemon socket, adding to the common methods of plan 01:

   | Direction | Method | Params | Result / meaning |
   |---|---|---|---|
   | phone → | `hello` | `{protocol: "handlr.companion", version: 1, client, session_id?}` | `{…, session_id, vocabulary: [string], paired?}` |
   | phone → | `turn.start` | `{turn_id, text}` | answered when the turn ends: `{outcome: "done" \| "interrupted" \| "error", ran: [{pane_id, command}]}` |
   | phone → | `turn.interrupt` | `{turn_id}` | `{}` |
   | phone → | `confirm.reply` | `{confirm_id, approve}` | as in plan 01 |
   | → phone | `turn.say` | `{turn_id, seq, text}` | speak this |
   | → phone | `confirm.requested` | plan 01's fields plus `text` | the question to speak |
   | → phone | `confirm.resolved` | `{confirm_id, outcome}` | |
   | → phone | `notice` | `{id, kind: "agent_status", pane_id, text}` | speak when free |
   | → phone | `vocabulary.changed` | `{vocabulary}` | |

   `turn_id` is chosen by the phone, unique within the session.
   `vocabulary` is every agent name, project name and workspace label in
   the herdr snapshot, for the phone's STT vocabulary boost.

9. **Turns.** One turn at a time per session; `turn.start` during another
   gets `-32020` (busy). A turn runs the Gemini engine (libinfer) with
   today's system prompt, tools and `HerdrContextContributor`, with every
   tool call going through the policy core as this principal. Version 1
   sends the whole reply as one `turn.say` with `seq: 0`; streaming by
   sentence is a later plan, and needs a libinfer API change.

10. **Interrupt.** `turn.interrupt` (and `cancel` on the `turn.start`
    request id) aborts the turn's task at once:
    - the Gemini call or tool loop in flight is dropped; a tool call
      already sent to herdr is not undone, and is listed in `ran`;
    - confirmations this turn asked for are cancelled;
    - the session history gets the user's text, any reply already said,
      and a model entry `[interrupted by user]`;
    - the `turn.start` response, `outcome: "interrupted"`, is sent within
      500 ms.

    It never sends anything to a pane. Stopping an agent is a spoken
    command like any other. The phone stops its own playback without
    waiting for the reply.

11. **Confirmations by voice.** While a confirmation this principal asked
    for is pending, the next `turn.start` text is matched against the
    affirmative list (moved from `main.rs` to `commands.rs`). A match
    approves it and the turn ends with `turn.say` of the outcome. Anything
    else denies it, and the text is then handled as a new turn. The phone
    may instead send `confirm.reply` from a button. The question in
    `confirm.requested.text` is built by the daemon: `Send "<command>" to
    <spoken agent name>?`, using `speakable_name`.

12. **Notices.** The daemon withholds `notice` while a confirmation is
    pending for this principal, then sends what is held, oldest first,
    with the cap and dropping rule of today's `Announcer`. The phone queues
    notices behind its own playback and while it is listening. Wording is
    `notify::announcement`.

13. **Sessions.** `session_id` is `companion:<name>`, stored by libinfer's
    `SessionManager`, so a reconnecting phone continues its conversation.

## 3. Steps

1. **The link layer.** `libhandlr_proto::link`: framing, chunking and both
   handshakes over any `AsyncRead + AsyncWrite`, with tests over an
   in-memory duplex: pairing succeeds with the right secret and fails with
   a wrong one; a wrong server key in the URI aborts the client; `IK` with
   a key the server rejects gets no reply; a 1 MiB message round-trips in
   chunks; 1 MiB plus one byte closes the connection.
   **Done when:** `cargo test -p libhandlr_proto --features link` passes.

2. **Keys and `auth.toml`.** In handlr: `link_key` creation and the
   permission check, `auth.toml` loading and appending, name uniqueness,
   narrowing of `agents`, the `approve` defaults.
   **Done when:** `cargo test -p handlr` covers each, including a refusal
   to start with `link_key` at mode 0644.

3. **Pairing and reconnecting.** The TCP listener, `link.pair_begin`,
   `handlr pair` with the QR code, the revocation check, and an example
   client, `cargo run -p libhandlr_proto --features link --example
   link_client -- <uri or auth file>`, that pairs, stores its key in a
   file, reconnects and runs `agents.list`.
   **Done when:** on the desktop, the example pairs from the printed URI,
   `auth.toml` gains its entry, a second run reconnects with `IK` and
   lists the agents, an expired or reused secret is refused, and deleting
   the entry closes the open connection within 60 s.

4. **The companion protocol and turn service.** The `companion` types in
   `libhandlr_proto`, decisions 8–13 in handlr, and a text mode in the
   example client: each typed line is a `turn.start`, a line `!` is
   `turn.interrupt`, `y` and `n` answer the last `confirm.requested`.
   **Done when:** over the link, "send continue to claude" gives a
   `confirm.requested` whose text names the command; "yes" sends it once
   and `ran` lists it; "no" sends nothing; `!` during a turn gets
   `outcome: "interrupted"` within 500 ms and the session file ends with
   `[interrupted by user]`; a notice that fires while a confirmation is
   pending arrives only after it is resolved.

## 4. Open questions

1. Port 7347 is arbitrary. Should the link also listen on the daemon's
   loopback by default, for testing?
2. Should the phone's `ping` interval be longer to save battery, with the
   OS keeping the socket alive in the background, or does the app hold
   the link only in the foreground?
3. Should a harness client paired over the link (T3 Code on another
   machine) get its own pairing flow without a QR code, such as a URI
   pasted into a file?

## Amendment — 2026-10-22 11:58 EDT: pairing string, liveness and listening

Agreed with the owner. This amendment replaces decisions 3, 5 (the URI and
how the phone connects with it) and 7, and the parts of steps 1 and 3 that
depend on them. It settles open questions 1 and 3. Where it disagrees with
the body, this amendment wins.

**Pairing string (replaces the URI in decision 5).** The QR code and a
pasted string carry the same payload, as bytes:

| Field | Size |
|---|---|
| version | 1 byte, `1` |
| server public key | 32 bytes |
| secret | 32 bytes |
| expiry | 4 bytes, unix seconds, big-endian |
| name | 1-byte length, then UTF-8 |
| addresses | 1-byte count, then each as a 1-byte length and `host:port` in UTF-8 |
| checksum | the first 4 bytes of BLAKE2s-256 over every field above |

The text form is `handlr1` followed by the payload in base58 (the Bitcoin
alphabet). The QR code encodes that same string. `handlr pair` prints the
QR code and the string. Any client can pair from the string alone: the
phone by scanning or pasting it, a harness on another machine by taking it
as an argument. A string with a bad prefix, a bad base58 character, a bad
checksum or an unknown version is refused as an invalid pairing string,
before any connection is made. Everything else in decision 5 stands: the
secret is single use and expires, the client aborts if the server's static
key differs from the payload's, and the daemon appends to `auth.toml`.

**Liveness (replaces decision 7).** The daemon has no application idle
timeout. It sets `SO_KEEPALIVE` on every link socket with the operating
system's own timers, and closes a connection only on a socket error or end
of stream. `ping` stays a method, but is optional. A client pings at
whatever interval its OS and network allow, or not at all. On waking or a
network change, the phone reconnects with `IK` and its `session_id`, with
backoff of 1, 2, 4 … 30 s.

**Listening (replaces decision 3).**

1. By default the link binds only `127.0.0.1:7347`.
2. To reach it from other devices, Tailscale forwards to that address with
   `tailscale serve --bg --tcp 7347 tcp://127.0.0.1:7347`. handlr never
   runs this itself. At start, and in `handlr pair`, it reads `tailscale
   serve status --json`. If Tailscale is running and the forward is
   missing, it logs a warning with the exact command to run. If the forward
   is there, the pairing payload's addresses are the machine's MagicDNS
   name from `tailscale status --json` (`Self.DNSName`, without the final
   dot) with port 7347, followed by `127.0.0.1:7347`.
3. An explicit `[link] listen = ["<addr>:<port>", …]` in `config.toml`
   replaces the default. It turns off the Tailscale check, and its
   addresses are the ones in the pairing payload.

Nothing is reachable from another machine unless Tailscale or an explicit
address exposes it. Connections forwarded by `tailscale serve` arrive from
loopback, so the link relies on the Noise keys, never on the peer address.

**Steps.** Step 1 adds tests for the pairing string: encode and decode
round-trip; a changed character fails the checksum; a wrong prefix and an
unknown version are refused. Step 3's example client takes the pairing
string instead of the URI. Its **Done when:** also requires that with
Tailscale running and no forward, `handlr serve` logs the `tailscale serve`
command, and that with the forward in place, the example client pairs from
a second machine on the tailnet using the printed string.

**Open questions.** Questions 1 (the default listener) and 3 (pairing
without a QR code) are settled above. Question 2 (pinging in the
background) is settled by the liveness rule.

## Amendment — 2026-10-22 12:06 EDT: voice confirmations are made by the client

Agreed with the owner. This amendment replaces decision 11 and part of
decision 8's table, and changes step 4. It follows plan 01's amendment of
the same time (approval phrases, action labels and lifetimes). Where it
disagrees with the body, this amendment wins.

**Decision 11, replaced.** A voice confirmation checks that the command
handlr is about to type is the one the user meant. The client does the
listening, and handlr keeps the authority.

- `confirm.requested.text` is `Approve sending "<command>" to <spoken agent
  name>?`, using `speakable_name`. When the request has a label, it is
  followed by ` For <label, with "-" read as spaces>.`
- The companion client (voicexn) speaks the text. While the confirmation
  is pending, the client matches the next utterance itself. If the
  utterance is "approve" or "approved" (case-insensitive, ignoring
  punctuation and surrounding space), it sends `confirm.reply {confirm_id,
  approve: true}`. Any other utterance sends `approve: false`, and is then
  sent as a new `turn.start`. A button may send the same replies.
- handlr does not match affirmatives in turn text. A `turn.start` that
  arrives while a voice-turn confirmation of that session is still pending
  ends that confirmation as denied (plan 01's lifetime rule).
- The affirmative list in `main.rs` is not moved. It stays with the
  `local-voice` code until that code is deleted.

**Decision 8.** `confirm.requested`, as sent to a companion principal,
carries `phrase` and `label` in addition to plan 01's fields. The phone may
show the phrase, so the user can approve from another client, such as
`handlr ctl`.

**Step 4.** In the example client's text mode, `approve` and `deny` answer
the last `confirm.requested`; `y` and `n` are removed. In its **Done
when:**, "yes" becomes "approve", and "no" becomes any other line. Also
required: a request with a label is spoken with the label, and a typed
line other than "approve" while a confirmation is pending denies it and is
then handled as a turn.

## Amendment — 2026-10-22 16:23 EDT: step 4 checked with the companion crate, Tokio I/O

Agreed with the owner. This amendment changes steps 1, 3 and 4. Where it
disagrees with earlier text, this amendment wins.

- **I/O traits.** The link layer works over Tokio's `AsyncRead` and
  `AsyncWrite` (`libhandlr_proto`'s `link` feature adds `tokio` with
  `io-util`). The daemon and `handlr-companion` both run on Tokio.
- **Step 3's example client** in `libhandlr_proto` stays as it is, for
  pairing, reconnecting and revocation at the link level.
- **Step 4's text mode** is removed from that example. Step 4's **Done
  when:** is checked with `cargo run -p handlr-companion --example text`
  (plan 20261022_04, step 2), whose typed lines are utterances. "approve"
  approves, and an empty line is the tap that interrupts. No step of this
  plan needs libvoicexn or any audio model.

## Amendment — 2026-10-22 17:52 EDT: naming agents aloud, by project and tab

Agreed with the owner. This amendment replaces how spoken names are built
(`notify::speakable_name`, used by decisions 11 and 12). It adds tabs to
the agent list and to the vocabulary, and lets the common methods address
a pane by its tab. Where it disagrees with earlier text, this amendment
wins.

**What herdr gives.** The snapshot has a top-level `tabs` list. Each tab
has `tab_id`, `workspace_id`, `number` (unique within its workspace) and
`label`. Panes point at their tab by `tab_id`. The owner names tabs with
short codes, because tabs appear as tabs on a phone screen. When the
owner types a number it becomes a prefix (`17-hr`, `16-stt`, `5-kp`);
otherwise herdr keeps only the code (`-mb`, `-ad`, `ctrls`). An unnamed
tab's label is just a number. handlr reads tabs only when they are nested
under workspaces, which this herdr does not do, so today it sees none. On
2026-10-22 all 11 Claude panes shared one workspace label (`cc-highbay`),
and every spoken name fell through to the pane id ("pane pZ").

**Agents (`core::agents::AgentRef`, `agents.list`).** Each agent gains
`tab_label` and `tab_number`, taken from the top-level `tabs` (and still
from nested tabs where a herdr nests them). `common::AgentInfo` gains the
same two optional fields; adding them is backwards-compatible on the wire.

**The project (replacing `project_for`).** The project is the workspace
label, unless more than one workspace shares that label or (as here) the
panes being compared all share it. In that case it is the basename of the
pane's cwd. Usually one Claude instance runs per project directory, which
this matches. Others may run several in one directory, which the tab
covers.

**The spoken tab.** A tab's spoken form is its label without a leading
number and without leading or trailing `-`, with any remaining `-` read as
a space: `17-hr` → "hr", `-mb` → "mb", `5-kp` → "kp", `ctrls` → "ctrls".
A label that is only a number, or empty, is "tab <number>". The TTS
engine decides how a code is pronounced.

**The ladder (replacing `speakable_name`'s four rungs).** Each rung is
used only when the earlier ones are not unique among the current agents:

1. The agent: "Claude".
2. The project: "the handlr agent".
3. The agent in the project: "Claude in libmorph".
4. The tab: "Claude in libmorph, tab mb". This is the label's spoken form,
   or "tab <number>" when the label is only a number.
5. The tab number, when spoken labels collide: "Claude in libmorph, tab 24".

A pane id is never spoken, and never shown: it means nothing to the
user. Everything a client presents carries a `name`, the result of this
ladder, and clients show that instead of `pane_id`. The `pane_id` stays
in messages only as a handle for later requests. The fields:

- `common::AgentInfo.name`;
- `ConfirmRequested.name`;
- `notice.name`;
- the `ran` entries of a turn's result.

`handlr ctl` prints the names. The `confirm <id>` lines `serve` prints
name the agent and its tab, not the pane.

**Vocabulary (decision 8).** `vocabulary` and `vocabulary.changed` gain
every tab label as written and its spoken form, alongside agent names,
projects (now including cwd basenames) and workspace labels.

**Addressing by tab.** Wherever a common method takes `pane_id` or
`target_id` (`agents.read`, `agents.send`, `agents.focus`, `watch.add`
and `watch.remove`), the value may also be a tab label as written, a
label's spoken form, or a tab number, resolved within the snapshot in that
order. A value that matches more than one pane gets `-32002`, and the
error lists the candidates. This extends plan 20261022_01's resolution
without changing it for pane ids. A companion turn's `run_command` tool
resolves the same way, so "send continue to hr" works.

**Steps.** A new step 0, before step 1, implements this:
- tabs in `core::agents`;
- the project rule;
- the spoken tab and the ladder in `notify`;
- resolution by tab, used by the daemon's methods;
- the `AgentInfo` fields.

Its tests are built from the 2026-10-22 snapshot's shape:
- the 11 Claude panes each get a name without a pane id;
- the two libmorph panes are named by tab ("mb", "ad");
- "hr" and "17-hr" resolve to `w2:pZ`;
- an ambiguous tab label gets `-32002`.

**Done when:** `HANDLR_SNAPSHOT=<captured snapshot> cargo test --test
live_naming -- --nocapture` prints no name containing a pane id for that
snapshot, and `handlr ctl agents.read '{"pane_id":"hr","lines":5}'` reads
the handlr pane.

## Amendment — 2026-10-22 17:55 EDT: spelling out codes that do not read as English

Agreed with the owner. This amendment adds to the spoken forms of the
amendment before it (naming agents aloud, by project and tab), and changes
nothing else.

**Which words are spelled.** Each word of a spoken name (a tab's spoken
form, a project, an agent name) is checked on its own. A word is spelled
out, letter by letter, when it is at most 6 letters long and has no vowel
(`a`, `e`, `i`, `o`, `u` or `y`), because it does not read as English.
Digits are left as they are. Examples:
- `hr` → "aitch ar";
- `mb` → "em bee";
- `kp` → "kay pee";
- `stt` → "ess tee tee";
- `ctrls` → "see tee ar el ess";
- `ad`, `bubble`, `sigsp` and `libmorph` are read as written.

**How letters are spelled.** By their English letter names, written so the
TTS says the name and not a sound. These are not the military alphabet:

| a | b | c | d | e | f | g | h | i | j | k | l | m |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| ay | bee | see | dee | ee | ef | jee | aitch | eye | jay | kay | el | em |

| n | o | p | q | r | s | t | u | v | w | x | y | z |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| en | oh | pee | cue | ar | ess | tee | you | vee | double you | ex | why | zee |

**Vocabulary.** For each spelled word, the vocabulary also gets its
capitals (`HR`, `MB`, `CTRLS`), the form STT writes spelled letters in. A
heard "HR" then resolves to the tab `17-hr`, like "hr" does.

**Step 0** gains tests:
- each example above, and all 26 letter names;
- a 7-letter word with no vowel is read as written;
- "HR" resolves to `w2:pZ`.

## Amendment — 2026-10-22 17:59 EDT: naming moves to its own plan

Agreed with the owner. Naming is not the core of this plan, so it moves to
[Naming agents aloud and on screen](20261022_05_SPOKEN_NAMES_PLAN.md).
This amendment replaces the amendments of 17:52 EDT (naming agents aloud,
by project and tab) and 17:55 EDT (spelling out codes that do not read as
English). It also removes the step 0 they added. Their content, with
configuration added, is in that plan.

Here, decisions 8 (`vocabulary`), 11 (the question in `confirm.requested`)
and 12 (`notice`) take names and vocabulary from plan 20261022_05. Where
they say `speakable_name`, read that plan's spoken name. The rest of this
plan is unchanged.

## Amendment — 2026-10-22 18:03 EDT: an interrupt also stops the coding agent

Agreed with the owner. This amendment replaces decision 10's "It never sends
anything to a pane". It adds a per-agent interrupt key, and adds
`agents.interrupt` to the common methods. Where it disagrees with earlier
text, this amendment wins.

**Why.** A companion session is about active coding agents. Gemini is a
layer that turns speech into requests and the JSON results into speech.
Interrupting a turn therefore means stopping the work the user is
listening to, and that work is the agent's, not only Gemini's.

**The current agent.** Each companion session tracks its *current agent*:
the agent the conversation last addressed, by a tool call or by name.
Plan 20261022_05's resolution decides which agent a name refers to.

**What `turn.interrupt` does,** in addition to decision 10's ending of the
turn:
1. It sends the interrupt key to every pane this turn sent a command to
   (`ran`), whatever its status. The agent may not yet report `working`
   for a command it has just received.
2. It sends the interrupt key to the session's current agent, if herdr
   reports it `working` and it is not already covered by step 1. An idle
   agent is left alone.

The keys go through herdr's `pane.send_keys`. They go only to agents
with an `[[agents]]` entry that the principal can reach. They are not
gated by confirmation: an interrupt that waited for an approval would not
be one, and the key only stops work. The turn's result gains
`interrupted: [{pane_id, name, spoken}]`, so the phone can say what it
stopped ("stopped the handlr agent").

**The interrupt key.** `[[agents]]` entries gain `interrupt_key`, default
`"escape"` (herdr's name for the key is to be checked against herdr when
this is implemented). Other agents may use `"ctrl+c"`. An empty string
turns interrupting off for that agent.

**`agents.interrupt {target}`,** a common method in both protocols. It
sends the target's interrupt key, under the same rules, and returns
`{interrupted: bool, name, spoken}`. A harness, such as a T3 Code plugin
or an MCP agent keeping agents on track, can stop an agent that is
looping. This extends plan 20261022_01's methods without changing them.
Its wire types go in `libhandlr_proto::common`.

**Steps.** Step 4 gains these checks over the mock:
- `turn.interrupt` after a turn sent `continue` sends the interrupt key to
  that pane;
- with nothing sent, it reaches the current agent only when it is working;
- an agent with `interrupt_key = ""` gets nothing;
- `agents.interrupt` from a harness client sends the key.

These are checked by the mock's keystroke log, never against a live agent.

## Amendment — 2026-10-22 18:05 EDT: the interrupt key is a keystroke, not text

This clarifies the previous amendment ("an interrupt also stops the coding
agent"), raised by the owner. It replaces that amendment's paragraph "The
interrupt key". Nothing else changes.

**The interrupt key.** `[[agents]]` entries gain `interrupt_key`. It is
the name of one key, in herdr's key-name vocabulary, sent as a keystroke
with herdr's `pane.send_keys`, as if the key were pressed in that pane.
herdr is the multiplexer, which delivers the key to the pane's terminal;
the agent running in the pane is what reads it and stops.

- It is never typed as text: that is `pane.send_input`, which carries
  commands such as `continue`.
- It is never spoken, and is not a command a user says.

The default is herdr's name for the Esc key. That is expected to be
`escape`, and the name is checked against herdr when this is implemented.
When `[[agents]]` is loaded, a value that is not a single key name herdr
accepts is rejected. An empty string turns interrupting off for that
agent.

The previous amendment offered `"ctrl+c"` as an alternative, and that
suggestion is withdrawn. In Claude Code, Ctrl+C clears the input or
cancels, and pressed twice it exits the session, so two interrupts in a row
could close it. It is not an equivalent of Esc. `interrupt_key` is for an
agent whose interrupt key really is a different key, and is set only when
that is known.

## Amendment — 2026-10-22 18:07 EDT: "Esc-like" corrected

Raised by the owner. The front matter's "with an Esc-like interrupt" and
the goal's "interrupts the turn, as Esc does in a coding agent" are out of
date since the amendment "an interrupt also stops the coding agent". The
interrupt is not merely like Esc. It ends the turn, and, through herdr,
sends the coding agents involved their interrupt key (Esc by default).
Read both passages that way. Nothing else changes.
