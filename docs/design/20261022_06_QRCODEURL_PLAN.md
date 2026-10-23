---
title: qrcodeurl, and pairing by a scanned URL
date: 2026-10-22
status: Draft
covers:
  - "`crates/qrcodeurl`: a small, pure, synchronous crate that turns any
    string into a QR matrix (the `qrcode` crate), a 1-bit grayscale PNG
    (written by hand, deflated with `miniz_oxide`), a `data:image/png;base64,`
    URL, a base64url string and Unicode text for a terminal"
  - "The pairing URL, `handlr://<host>:<port>/pair#handlr1…`, which the QR
    code carries: the server's reachable (Tailscale) address and the
    pairing string, so the phone app connects and begins pairing from one
    scan"
  - "`handlr pair` using both, in place of `src/pair.rs::qr_text`"
  - "Not covered: the pairing string's own format (plan 20261022_02, 11:58
    EDT amendment); scanning with the camera and the Android intent filter
    (plan 20261022_04); any new pairing security mechanism"
sources:
  - "Owner's requests (2026-10-22): \"plan a qrcodeurl crate with a qrcode
    crate/png/deflate/base64url from that base58 string\", and \"this one
    should encode the tailscale url for the mobile app to connect and begin
    pairing\"."
  - "handlr at `16b7ffb`: `src/pair.rs` (`qr_text`, `qrcode` 0.14 `Dense1x2`,
    light on dark, EC level M); `crates/libhandlr_proto/src/link/pairing.rs`
    (`PairingPayload`); `src/server/link/mod.rs` (addresses: the Tailscale
    name first when `tailscale serve` forwards the port, then loopback)."
  - "`Cargo.lock` at `16b7ffb`: `qrcode` 0.14.1, `miniz_oxide` 0.8.9,
    `base64` 0.22.1."
  - "ISO/IEC 18004 (QR), ISO/IEC 15948 (PNG), RFC 1950 (zlib), RFC 2397
    (`data:` URLs), RFC 3986 (URIs), RFC 4648 (base64, base64url)."
related:
  - docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md (the pairing string)
  - docs/design/20261022_04_COMPANION_APP_PLAN.md (the phone scans the code)
---

# qrcodeurl, and pairing by a scanned URL

## 1. Goal

`handlr pair` prints its `handlr1…` string and a QR code of it in the
terminal. This plan makes the QR code carry a **pairing URL** instead: the
address the phone reaches the server at (its Tailscale name), and the
pairing string. The phone app opens the URL from a scan, connects to that
address and pairs, with nothing typed. The same code is also available as
a PNG and a `data:` URL, from a small crate, `qrcodeurl`, that knows
nothing about handlr.

## 2. Decisions

1. **The pairing URL.** `handlr://<host>:<port>/pair#<pairing string>`.
   - `<host>:<port>` is the payload's first address that is not loopback
     (`127.0.0.0/8`, `::1`, `localhost`). With `tailscale serve` detected,
     that is the Tailscale name (`m4.tail1a2b3c.ts.net:7347`). With none,
     the first address is used and `handlr pair` warns that a phone cannot
     reach it.
   - The pairing string goes in the fragment, unchanged. It stays the
     whole of what is trusted: the payload carries every address and is
     checksummed, and the host in the URL must be one of the payload's
     addresses, or the URL is refused. The host is there so the scan shows
     where it goes, and for the phone's intent filter.
   - `libhandlr_proto::link` gains `PAIRING_URL_SCHEME = "handlr"`,
     `PairingPayload::url(&self) -> Result<String, PairingStringError>` and
     `PairingStringError::BadUrl`. `PairingPayload::decode` accepts either
     form, so every client that takes a pasted string takes a scanned URL
     too.
   - Sizes, byte mode at EC level M: a two-address payload is a 184-character
     string, and its URL about 230 characters, version 11 (61 modules). Three
     long addresses reach about 285 characters, version 13 (69 modules).
2. **QR encoding.** Byte mode: base58 has lowercase letters, so alphanumeric
   mode cannot hold it. EC level M, as today. Quiet zone 4 modules
   (ISO/IEC 18004). PNG scale 8 px per module by default, 1 to 32 allowed;
   side `(modules + 2 × quiet zone) × scale`.
3. **PNG, written by hand.** The signature; `IHDR` (width, height, bit depth
   1, colour type 0, compression 0, filter 0, interlace 0); one `IDAT` of
   `miniz_oxide::deflate::compress_to_vec_zlib(raw, 9)`, where `raw` is each
   row as filter byte 0 then the pixels packed MSB first, light = 1 and
   dark = 0, padded to a byte with light bits; `IEND`. CRC-32 (reflected
   `0xEDB88320`) over each chunk's type and data, from a table built in a
   `const fn`. No `png`, `crc32fast` or `image` dependency.
4. **base64.** `data_url` is `data:image/png;base64,` and the standard
   alphabet, padded (RFC 2397). `base64url` is the URL-safe alphabet with no
   padding, for an image carried inside another URL. Both from `base64`
   0.22.
5. **API.** Pure and synchronous; no I/O, no logging, no handlr types, and
   no `qrcode` types in the public API.

   ```rust
   pub enum Ecc { L, M /* default */, Q, H }
   pub struct Png { pub scale: u32, pub quiet_zone: u32 } // non_exhaustive; default 8, 4
   impl Png { pub fn new(scale: u32, quiet_zone: u32) -> Result<Png, Error>; }
   pub enum Terminal { LightOnDark /* default */, DarkOnLight }
   pub struct Qr { .. }  // Debug shows version and size only
   impl Qr {
       pub fn new(text: &str) -> Result<Qr, Error>;              // Ecc::M
       pub fn with_ecc(text: &str, ecc: Ecc) -> Result<Qr, Error>;
       pub fn version(&self) -> u8;
       pub fn modules(&self) -> usize;
       pub fn is_dark(&self, x: usize, y: usize) -> bool;
       pub fn png(&self, png: Png) -> Vec<u8>;
       pub fn data_url(&self, png: Png) -> String;
       pub fn base64url(&self, png: Png) -> String;
       pub fn terminal(&self, style: Terminal) -> String;        // quiet zone 4
   }
   pub fn png(text: &str) -> Result<Vec<u8>, Error>;
   pub fn data_url(text: &str) -> Result<String, Error>;
   pub fn terminal(text: &str) -> Result<String, Error>;
   pub enum Error { TooLong { len: usize, ecc: Ecc }, BadSize, Unencodable } // non_exhaustive
   ```

   Errors and `Debug` output carry lengths, never the text.
6. **The secret travels in the code.** The URL carries the single-use
   secret, which expires after `--ttl` (300 s by default). `qrcodeurl` never
   logs; handlr never logs the string, the URL or the PNG. The data URL and
   the PNG file are produced only on request; a PNG is written with mode
   0600 and never over an existing file; wherever either is printed or
   written, `handlr pair` says "single use, expires in N s". No new
   mechanism: plan 02's single use, expiry and `link.pair_done` remain the
   protection.
7. **Placement.** `crates/qrcodeurl`, version 0.1.0, a workspace member and
   in `default-members`. Dependencies: `qrcode` 0.14 (`default-features =
   false`), `miniz_oxide` 0.8, `base64` 0.22, all already in the lock.
   Dev-dependencies: `png` 0.18 (an independent decoder) and `rqrr` 0.11 (a
   QR reader). handlr depends on it by workspace path; the companion crates
   do not.

## 3. Steps

1. **`crates/qrcodeurl`, with tests.** `src/lib.rs`, `src/png.rs`,
   `tests/roundtrip.rs`. Tests: the CRC check value (`123456789` →
   `0xCBF43926`); the chunk order, every chunk's CRC and `IHDR`'s fields;
   `png` decodes each image as 1-bit grayscale at the expected size, with a
   light quiet zone and a dark finder corner, including a scale whose side
   is not a multiple of 8; `rqrr` reads back pairing-sized strings and URLs
   at L, M, Q and H and at scales 1 and 8; the versions of §2.1; both base64
   forms decode to the PNG; the terminal form's size; `TooLong` and
   `BadSize`, with no input text in any error or `Debug`.
   **Done when:** fmt, clippy (no warnings) and test pass for
   `-p qrcodeurl`.
2. **The pairing URL in `libhandlr_proto`.** Decision 1: `url`, `decode`
   accepting both forms, `BadUrl`. Tests: the URL's host is the first
   non-loopback address; loopback alone gives the first address; a URL
   round-trips to the same payload; a host not in the payload, a wrong
   scheme or path, or a missing fragment is `BadUrl`.
   **Done when:** fmt, clippy and test pass for `-p libhandlr_proto`.
3. **`handlr pair`.** The terminal QR code carries the pairing URL, from
   `qrcodeurl`; the plain string is still printed below it for pasting.
   `--qr-url` prints the PNG of the same code as a `data:` URL;
   `--qr-png <path>` writes it (decision 6). A warning when no address is
   reachable from another machine. `qr_text` and the direct `qrcode`
   dependency are removed.
   **Done when:** fmt, clippy and test pass for `-p handlr`; a test decodes
   the `--qr-png` file with `rqrr` and pairs `handlr-companion` with the
   URL it holds.
4. **The phone (a proposal for plan 04).** The app registers the `handlr`
   scheme, scans with `rqrr` on a camera frame, and hands the URL unchanged
   to `Input::Pair`. Plan 04 is frozen, so this is proposed to the owner as
   an amendment there, not built here.
   **Done when:** the owner has agreed or declined that amendment.
