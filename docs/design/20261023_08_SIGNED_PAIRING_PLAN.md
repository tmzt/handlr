---
title: Pairing by signed challenge tokens, and a key store crate for the daemon
date: 2026-10-23
status: Draft
covers:
  - "The pairing URL and QR code carry a short-lived, single-use **pairing
    token** signed by the daemon, not the daemon's static Noise key. The
    daemon's keys are delivered and pinned inside the pairing handshake,
    after the token checks out."
  - "A pairing handshake, `Noise_XX` with mode byte `0x03`, in which the
    phone presents the token, the daemon proves it minted the token and
    binds its static key to it, and the daemon learns the phone's key. The
    pasted device code (`handlrkey1…`) is removed."
  - "The human step becomes a six-digit comparison: `handlr pair` shows the
    number the phone shows, and the owner accepts or rejects. Nothing is
    written to `auth.toml` before that."
  - "`crates/handlr-keys`: libdevice's key store engine, copied as it is
    designed to be, with a file vault (default for the daemon), the macOS
    Keychain vault (opt-in) and a memory vault (tests), and the two key
    adapters handlr needs: the X25519 Noise static and an Ed25519 signing
    key. The daemon's `link_key` file moves into it."
  - "Reconnects (`IK`, mode `0x02`), `auth.toml` and revocation are
    unchanged. Already paired phones keep working."
  - "Not covered: rotating the daemon's static key; using the signing key
    for anything but pairing; libmobile's (Android) vault, which the
    companion app keeps using through libmorph."
sources:
  - "Owner (2026-10-23): \"design a better plan, signed challenge tokens
    instead of exposing the key. we might move the libdevice keychain into a
    shared crate in this repo for now\"."
  - "handlr at `64b77f2`: `crates/libhandlr_proto/src/link/{pairing,
    handshake, keys, mod, resolver, stream}.rs`; `src/pair.rs`;
    `src/server/link/{mod, auth, keyfile}.rs`;
    `crates/handlr-companion/src/{keys, scan, net}.rs`;
    `apps/companion/src/keys.rs` (libmorph's key store, `NoiseStatic`,
    `VaultStaticKey`)."
  - "libdevice at `b46963b` (0.1.1): `src/key_store.rs` (the engine, \"the
    same, byte for byte, in libmobile and libdevice\", with
    `ENGINE_SOURCE`), `src/key_store/vault.rs` (login Keychain, memory),
    `src/secure_store.rs`, `docs/design/20261022_01_SECURE_STORE_AND_KEY_STORE_PLAN.md`."
  - "The Noise Protocol Framework, revision 34 (`XX`, `IK`, handshake
    hash, prologue); RFC 8032 (Ed25519); RFC 7748 (X25519); ISO/IEC 18004
    (QR capacity: version 10 at level M holds 213 bytes in byte mode,
    version 11 holds 251)."
related:
  - "docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md: decision 1
    (the handshake), decision 4 (`link_key`), decision 6 (`auth.toml`) and
    the 23:07 EDT amendment (pairing by public keys), whose pairing string
    and device code this plan replaces. That plan stays frozen; this one
    names what it replaces."
  - "docs/design/20261022_06_QRCODEURL_PLAN.md: the URL
    `handlr://<host>:<port>/pair#…` and its QR code, kept; only the
    fragment changes."
  - "docs/design/20261022_04_COMPANION_APP_PLAN.md, decision 5 (the phone's
    key store), which step 7 extends."
---

## 1. Goal

Today the QR code on the Mac's screen is the daemon's long-term identity:
`handlr1…` holds its static X25519 public key, so the code is the same every
run and anyone who photographs it keeps it. Pairing then needs a 60-character
device code pasted back into `handlr pair`.

After this plan the QR code holds a **token**: 16 random bytes, the minute it
was issued, the addresses, and the daemon's Ed25519 signature over them. It
is good for ten minutes and for one connection. The phone presents it in a
`Noise_XX` handshake; the daemon checks its own signature, proves inside the
handshake that it holds the key that signed the token, and learns the
phone's static key. The owner then compares a six-digit number on the phone
with the one `handlr pair` prints and presses `y`. Only then is the phone
written to `auth.toml`, and only then has the phone pinned the daemon's
static key. Every later connection is `IK` with that pinned key, as now.

## 2. Decisions

### 2.1 Keys: a separate Ed25519 signing key

- The daemon keeps its X25519 Noise static key (`handlr.noise.x25519.v1`)
  and gains an Ed25519 **signing key** (`handlr.sign.ed25519.v1`), 32-byte
  seed, public key 32 bytes, signatures 64 bytes (`ed25519-dalek` 2, which
  already shares `curve25519-dalek` 4 with the companion).
- They are separate, not derived from one another: the key store lends one
  key of one kind for one call and never exports it, so a derivation would
  have to run outside the store; and one compromised key must not give the
  other. Deriving an Ed25519 key from the X25519 secret was considered and
  rejected for those two reasons.
- The phone never needs a signing key. It verifies only (`verify_strict`,
  which refuses small-order points).
- `Signer`, in `libhandlr_proto::link`, beside `StaticKey`:

  ```rust
  pub trait Signer: Send + Sync + 'static {
      fn public(&self) -> [u8; 32];
      /// Ed25519 over `msg`, or `None` if the key could not be used.
      fn sign(&self, msg: &[u8]) -> Option<[u8; 64]>;
  }
  ```

  `SigningKeypair` (seed in memory) implements it for tests; the daemon's
  implementation borrows the seed from the key store for each call, as
  `VaultStaticKey` does for X25519.

### 2.2 The pairing token

The payload, as bytes, in this order:

| Field | Size | Notes |
|---|---|---|
| version | 1 | `3` |
| token id | 16 | from the OS random source |
| issued | 4 | unsigned big-endian seconds since the Unix epoch |
| addresses | 1-byte count, then each a 1-byte length and `host:port` in UTF-8 | as today; loopback addresses are included only when there is no other |
| signature | 64 | Ed25519 over `b"handlr.pair/1 token"` then every field above |

- The text form is `handlr1` and base58 (Bitcoin alphabet), as today; the
  version byte tells the two apart, and a version 2 string (the old key
  string) is refused as an unknown version. The URL is unchanged in shape,
  `handlr://<host>:<port>/pair#handlr1…`.
- Size: with one Tailscale address the payload is about 113 bytes, the
  string about 155 characters, the URL about 200. In byte mode that is QR
  version 10 or 11 at level M (today: about 160, version 9). `qrcodeurl`
  needs no change.
- **Validity**: from `issued` to `issued + 600` seconds. The daemon also
  refuses a token whose `issued` is before the daemon's own start time
  (`Link::started`), so a token outlives neither ten minutes nor the
  process that minted it, and a restart cannot be replayed against; and one
  more than 60 seconds in the future (clock skew).
- **Single use**: the daemon keeps `consumed: HashSet<[u8; 16]>` of token
  ids that reached message 3 of the handshake, pruned of ids older than
  the validity window. A second presentation is refused.
- The token holds no secret: the signature can be verified by anyone who
  has the public key, and reveals the public key to nobody (Ed25519 hashes
  the public key into the challenge, so it cannot be recovered from a
  signature). What it gives an observer is a ten-minute, one-shot right to
  *ask* to pair; see section 3.
- **Mint**: `PairingToken::mint(signer, addresses, now) -> PairingToken`;
  `encode`, `url`, `decode` (checks shape only), `verify(public, now,
  not_before) -> Result<(), TokenError>` with `Expired`, `NotYetValid`,
  `BadSignature`, `UnknownVersion`, `Malformed`.

### 2.3 The pairing handshake: `XX`, mode `0x03`

`Noise_XX_25519_ChaChaPoly_BLAKE2s`, prologue `b"handlr.link/1"` then
`0x03`, framed as every link message is (2-byte length). The phone is the
initiator.

1. **→ `e`**, payload (cleartext; it was on a screen): the token's payload
   bytes, as decoded from the QR.
   The daemon decodes and verifies the token with its own public key
   (2.2). A token that fails, is spent, or arrives while the daemon has no
   `handlr pair` waiting is dropped with no reply, exactly as an
   unauthorized `IK` key is today.
2. **← `e, ee, s, es`**, payload (encrypted): the daemon's signing public
   key `A` (32 bytes) and the **bind signature** (64 bytes), Ed25519 by
   `A` over `b"handlr.pair/1 bind"`, the token id, the phone's ephemeral
   `e_I`, the daemon's ephemeral `e_R` and the daemon's static `s_S`.
   The phone checks, in order: `A` verifies the token's signature it
   scanned (so `A` is the key that minted what was on the owner's screen);
   `A` verifies the bind signature; and `s_S` in the bind signature is the
   remote static the handshake delivered. Any failure closes the connection
   and the app says the code was not this server's.
3. **→ `s, se`**, payload (encrypted): version byte `1`, then the device's
   name (1-byte length, 1 to 32 bytes of ASCII letters, digits, `-`, `_`
   or `.`, as the device code's name is today).
   The daemon marks the token id consumed and now holds the phone's static
   key and name as a **pending pairing**.
4. Both sides compute the **SAS**: the first 20 bits of BLAKE2s-256 over
   `b"handlr.pair/1 sas"` and the handshake hash, as six decimal digits
   (leading zeros kept, shown as `482 913`). The handshake hash covers both
   statics, both ephemerals, the token and the name, so the two numbers
   agree only on the one pair of endpoints that ran this handshake.
5. The connection waits, in transport mode, for the owner (2.4). The daemon
   sends one message, `paired` (`{name, protocols}` as JSON) on acceptance;
   on rejection or after 120 seconds it closes with no message. The phone
   closes after `paired` and connects again with `IK` (mode `0x02`), as
   every later connection does.

Why the SAS when the handshake already resists an attacker in the middle:
the token is public for ten minutes, and whoever presents it first becomes
the pending device. The number lets the owner see that the pending device
is the phone in their hand, which shows the same digits, and not a device
that raced it, whose digits the owner's phone cannot show.

Functions, in `libhandlr_proto::link`:

```rust
pub struct PairedClient<S> { pub stream: LinkStream<S>, pub server_key: [u8; 32],
                             pub signing_key: [u8; 32], pub sas: String }
pub async fn pair<S, K: StaticKey>(io: S, local: &K, token: &PairingToken, name: &str)
    -> Result<PairedClient<S>, LinkError>;

pub struct PendingPair<S> { pub stream: LinkStream<S>, pub peer_key: [u8; 32],
                            pub name: String, pub token_id: [u8; 16], pub sas: String }
pub async fn accept_pair<S, K: StaticKey, G: Signer, V>(io: S, local: &K, signer: &G, verify: V)
    -> Result<PendingPair<S>, LinkError>
where V: FnOnce(&PairingToken) -> Result<(), TokenError>;
```

`accept` (the existing server entry point) reads the mode byte and hands
`0x03` to `accept_pair`; `0x01` stays unknown. New `LinkError` variants:
`Token(TokenError)`, `NotThisServer` (the phone's checks in message 2
failed), `PairingClosed` (the daemon closed without `paired`).

### 2.4 `handlr pair` and the daemon's API

`handlr pair [--name <name>] [--protocols companion,harness] [--replace]
[--accept <digits>] [--qr-png <file>]`. `--code` is removed.

1. **`link.pair_begin {}`** mints a token and returns `{url, pairing,
   addresses, expires_in: 600, warnings}`. The daemon remembers the token
   id as *open* (waiting) for its validity; `handlr pair` exiting closes it
   (`link.pair_cancel {}`), and the daemon drops a token that is not open.
2. **It prints** the URL and the QR code's data URL (plan 06) and waits.
3. **`link.pair_pending {name, sas, fingerprint}`** is sent to the
   `handlr pair` connection when a handshake reaches message 3.
   `fingerprint` is the first 8 hex digits of BLAKE2s-256 of the phone's
   static key, for the log. `handlr pair` prints:

   ```
   s23 asks to pair. The phone shows a number; is it 482 913? [y/N]
   ```

   `--accept 482913` answers without the prompt, and only if it matches;
   any other value is a rejection with exit code 1.
4. **`link.pair_accept {sas, name?, protocols?, replace?}`** checks `sas`
   against the pending pairing, then appends to `auth.toml` as
   `link.pair_add` does today (name: the device's own or `--name`;
   protocols: `["companion"]` by default; `--replace` and the name and key
   rules of the 23:07 EDT amendment, unchanged), sends `paired` and closes
   the pairing connection. Result `{name, protocols}`.
   **`link.pair_reject {}`** closes it with nothing.
5. **`link.pair_done {name, paired}`** follows the first `IK` hello, as
   today. `handlr pair` prints "<name> connected" and exits 0.
6. One pending pairing at a time per open token; a second `0x03` for the
   same token is a replay and is dropped. Who may call: the daemon socket
   only, as today (-32001 for link clients).

### 2.5 The phone

1. `scan::read` accepts the URL, the data URL or the bare string, and now
   decodes a token (it also checks `issued` against the phone's clock and
   says "this code has expired" early rather than failing silently).
2. With no server record, `net.rs` runs `pair` (2.3) with the device's name,
   shows **"Confirm 482 913 on your Mac"** while pending, and on `paired`
   stores the server record and connects with `IK`. Without `paired` within
   120 seconds it says the Mac did not accept, and offers to scan again.
3. `ServerRecord` gains `signing_key: Option<[u8; 32]>` (base64 in
   `server.toml` and the secure-store blob, optional so old records read).
   It is stored and not yet used; a later plan may let a daemon whose
   static key changed re-introduce itself under the same signing key.
4. The device-code screen and its Copy button are removed.

### 2.6 `crates/handlr-keys`: the key store, for now

The daemon needs a store that lends keys of two kinds without exporting
them. libdevice has exactly that engine, and it is written to be copied
("the same, byte for byte, in libmobile and libdevice"). The companion app
cannot take it from here: on Android it must keep libmobile's Keystore
vault through libmorph. So the crate serves the daemon and the desktop
companion fallback, and keeps its engine identical to libdevice's.

- `src/engine.rs`: libdevice's `key_store.rs` unchanged (`SecretKey`,
  `SealToken`, `generate`, `public`, `with_key`, `delete`, kinds and
  migration). A test, behind a dev-dependency on libdevice pinned by git
  revision (`b46963b…`, features `secure-store`), asserts the file equals
  `libdevice::key_store::ENGINE_SOURCE`, so the copy cannot drift unnoticed.
- `src/vault.rs`: `Vault` with three backends behind one API
  (`put/get/delete`, `supported_flags`):
  - **file** (default): `<dir>/<name>.key`, mode 0600, directory 0700,
    contents `[flags][envelope]`, written through a temporary file and a
    rename; refused if group or others have any access or the owner is not
    this uid (the checks `keyfile.rs` makes today);
  - **keychain** (macOS, feature `keychain`, opt-in by
    `[link] key_store = "keychain"`): libdevice's login-Keychain vault,
    service `handlr.keys`. It is not the default because a daemon started
    over SSH or from an agent's shell cannot answer the Keychain prompt
    (`errSecInteractionNotAllowed`, -25308), and a rebuilt ad-hoc-signed
    binary prompts again;
  - **memory** (feature `test-vault`): for tests.
- `src/adapters.rs`: `NoiseStatic` (`handlr.noise.x25519.v1`, the type in
  `apps/companion/src/keys.rs` today, moved here unchanged) and
  `SigningSeed` (`handlr.sign.ed25519.v1`, 32-byte seed behind a `Box`,
  zeroized on drop); `VaultStaticKey: StaticKey` and `VaultSigner: Signer`,
  each borrowing its key for one call.
- Names in the daemon's vault: `link.static` and `link.sign`, directory
  `~/.config/handlr/keys/`. On first start, an existing `link_key` file is
  imported as `link.static` and then renamed `link_key.imported`, so the
  pinned key on every paired phone stays valid; the import is logged.
- Dependencies: `zeroize`, `curve25519-dalek`, `ed25519-dalek`, `getrandom`,
  `log`, `libhandlr_proto` (path, in-workspace), `security-framework`
  (macOS, optional). No sibling path dependency outside the workspace;
  libdevice only as a pinned git dev-dependency.
- **The path back.** When libdevice's key store has settled and libmorph
  pins it by revision, handlr-keys keeps only `vault.rs`'s file backend and
  the adapters, re-exports `libdevice::key_store` as its engine, and
  `engine.rs` and its drift test are deleted; the daemon's and the
  companion's code see the same names. The adapters themselves can move to
  libmorph (as its key store plan's section 7 already describes handlr's
  use) once they have two users there. Each of those is its own plan.

### 2.7 Versions (Rule 5)

- `libhandlr_proto` 0.3.0 → **0.4.0** (**Breaking**: `PairingPayload`
  becomes `PairingToken` with new fields; `DeviceCode`,
  `DEVICE_CODE_PREFIX`, `DEVICE_CODE_VERSION` removed; `PAIRING_VERSION`
  3; new `Signer`, `pair`, `accept_pair`, `LinkError` variants).
- `handlr-companion` 0.1.0 → **0.2.0** (**Breaking**: `ServerRecord` gains
  a field; `scan::Pairing` carries a token).
- `handlr-keys` new at **0.1.0**.
- `handlr` and `companion` are applications; manifests and lock file only.

## 3. Threat model

An observer who photographs the screen, reads the terminal's scrollback, or
sees the link in a screen share:

| | Before | After |
|---|---|---|
| Learns the daemon's long-term Noise identity | yes, for good; the same every run | no: the QR holds a token, and the keys arrive encrypted in message 2 |
| Correlates the daemon across runs, networks, screenshots | yes, by the key | no: new random id and signature every `handlr pair`; neither reveals the public key |
| Can connect and probe the port with a valid first message | yes, at any time (dropped silently unless in `auth.toml`) | only within ten minutes of the token, once; afterwards the mode-`0x03` message is dropped like any bad key |
| Can pair without the owner | no: the owner must paste a device code | no: the owner must compare and accept a number |
| Can race the owner's phone to the token | n/a | yes, once; it then appears as a pending device whose number the owner's phone does not show; the owner rejects, the token is spent, and `handlr pair` is run again |
| An attacker in the middle on the tailnet during pairing | cannot: the phone has the daemon's key from the QR | cannot: the bind signature ties `s_S` and both ephemerals to a token only the daemon could sign, and the phone checks the signer is the token's signer |
| After pairing | unchanged: `IK` against a pinned key; revocation by deleting the `auth.toml` entry | the same |

Residual risks, unchanged by this plan: a compromised Mac or phone; an owner
who accepts without looking at the phone (the prompt names the device and
asks for the number, and `--accept` must carry it); the Tailscale network
itself, which decides who can reach the port at all.

## 4. Steps

1. **`crates/handlr-keys`** (section 2.6): engine copy, file and memory
   vaults, the two adapters, `VaultStaticKey` and `VaultSigner`, the drift
   test, the workspace entry and `default-members` line; the Keychain vault
   behind `keychain`, built on macOS only.
   **Done when:** `cargo test -p handlr-keys` passes, including the drift
   test against libdevice `b46963b…`; a key generated in a file vault is
   refused at mode 0644; `VaultStaticKey` agrees with snow on a shared
   secret and `VaultSigner`'s signatures verify with `ed25519-dalek`.
2. **`libhandlr_proto`: `Signer` and the token** (sections 2.1 and 2.2):
   `PairingToken` replaces `PairingPayload`; `DeviceCode` is removed;
   `SigningKeypair` for tests; URL and base58 as before; version 0.4.0 with
   its `VERSIONS.md` entry and tag.
   **Done when:** a minted token round-trips through its URL; a byte
   changed anywhere fails `verify`; a token is `Expired` at `issued + 601`,
   `NotYetValid` at `issued - 61`, and refused when issued before
   `not_before`; a version 2 string is `UnknownVersion(2)`; the URL for one
   Tailscale address is under 213 bytes.
3. **`libhandlr_proto`: the `XX` pairing handshake** (section 2.3): `pair`,
   `accept_pair`, the bind signature, the SAS, `accept` routing `0x03`.
   **Done when:** over an in-memory duplex, a phone with a fresh key pairs,
   both sides show the same six digits and the phone holds the daemon's
   static and signing keys; a relay that substitutes its own static in
   message 2 fails on the phone with `NotThisServer`; a token presented
   twice is dropped the second time; a bad token gets no reply.
4. **The daemon** (sections 2.4 and 2.6): the signing key and the static key
   in a handlr-keys file vault, with the `link_key` import; `pair_begin`
   mints and opens a token; the acceptor handles `0x03`; pending pairings,
   `link.pair_pending`, `link.pair_accept`, `link.pair_reject`,
   `link.pair_cancel`; the 120-second pending timeout; `link.pair_done`
   as today.
   **Done when:** `tests/link.rs` runs the whole exchange against a daemon
   with a temporary config directory: begin, scan the PNG back, `pair`,
   `pair_pending` with matching digits, `pair_accept`, `paired`, then an
   `IK` hello answered `paired: true`; `pair_accept` with the wrong digits
   writes nothing; a daemon started with an old `link_key` serves the same
   static key after the import and the file is renamed.
5. **`handlr pair`** (section 2.4): the new prompt, `--accept`, `--code`
   removed, `Pasted` removed.
   **Done when:** the command pairs a desktop companion interactively and
   with `--accept`; a wrong `--accept` exits 1 and the entry is not added;
   Ctrl-C cancels the token.
6. **`handlr-companion`** (section 2.5): `scan::read` for tokens, the
   pairing path in `net.rs`, the pending state in `client.rs` with its
   "Confirm … on your Mac" output, `ServerRecord.signing_key` in
   `FileKeyStore`; version 0.2.0 with its `VERSIONS.md` entry and tag.
   **Done when:** the desktop companion, given the URL from step 5, reaches
   `paired: true` after acceptance, reconnects with `IK`, and reads an
   old `server.toml` without a signing key.
7. **`apps/companion`** (sections 2.5 and 2.6): `NoiseStatic` taken from
   handlr-keys on the desktop and left as is for Android through libmorph
   (the adapter is the same bytes); the server blob gains `signing_key`;
   the pending screen shows the digits; the device-code screen goes.
   **Done when:** the S23 scans the QR code, shows the digits, and after
   `y` on the Mac is in `auth.toml` and connected; a second scan of the
   same code on another device is refused.
8. **Documents**: this plan's row in the README table; the `CHANGES.md`
   addendum naming this plan; plan 02's 23:07 EDT amendment is left as it
   is, this plan naming what it replaces.
   **Done when:** the README table lists this plan with its status, and
   `CHANGES.md` has the addendum.

## 5. Open questions

1. **The key store's default for the daemon.** This plan keeps keys in
   0600 files by default and makes the Keychain opt-in, because of the
   prompt a daemon under SSH or an agent cannot answer. Should the daemon
   instead try the Keychain first and fall back to files, or should the
   Keychain be the default once the binary is signed?
2. **Token lifetime and pending timeout**: 600 and 120 seconds. Shorter
   (300 and 60) tightens the race window; longer suits a phone fetched from
   another room.
3. **Six digits or four.** Six matches common numeric-comparison pairing;
   four reads faster aloud. The number guards identity, not secrecy, so
   four would do.
4. **Multiple phones from one `handlr pair`**: this plan is one token, one
   phone. Should `handlr pair --count 2` mint a token usable twice, each
   use confirmed, for pairing a watch and a phone at once?
5. **Should the phone use the stored signing key** to accept a daemon whose
   static key changed, without a new scan? Left for a later plan.
6. **Where the adapters end up**: libmorph's key store plan (section 7)
   already describes handlr's use of them. Should `NoiseStatic` and
   `SigningSeed` go to libmorph now rather than live in handlr-keys, which
   would leave the daemon depending on libmorph?
7. **Sibling repository rule**: the drift test depends on libdevice by git
   revision. If that revision is not pushed, the test is `#[ignore]` until
   it is, or the drift check is a script in `tools/` instead.
