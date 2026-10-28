//! Plan 02, step 1, as amended at 23:07 EDT: the `IK` handshake and the
//! framing over an in-memory duplex, the pairing string and the device
//! code.

use blake2::{Blake2s256, Digest};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

use super::*;

const PIPE: usize = 64 * 1024;

fn payload(server: &StaticKeypair) -> PairingPayload {
    PairingPayload {
        server_key: server.public,
        addresses: vec!["box.tail1234.ts.net:7347".into(), "127.0.0.1:7347".into()],
    }
}

/// A connected pair: (client stream, server stream).
async fn connected() -> (LinkStream<DuplexStream>, LinkStream<DuplexStream>) {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        reconnect(c, &client, &server.public, None),
        accept(s, &server, |key| key == &client.public),
    );
    (cr.unwrap(), sr.unwrap().stream)
}

#[tokio::test]
async fn reconnect_carries_the_first_message() {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        reconnect(c, &client, &server.public, Some(b"hello")),
        accept(s, &server, |key| key == &client.public),
    );
    let mut cs = cr.unwrap();
    let mut acc = sr.unwrap();
    assert_eq!(acc.peer_key, client.public);
    assert_eq!(acc.stream.peer_key(), &client.public);
    assert_eq!(cs.peer_key(), &server.public);
    assert_eq!(acc.first_message.as_deref(), Some(&b"hello"[..]));
    acc.stream.send_message(b"welcome").await.unwrap();
    assert_eq!(cs.recv_message().await.unwrap(), b"welcome");
    cs.send_message(b"").await.unwrap();
    assert_eq!(acc.stream.recv_message().await.unwrap(), b"");
}

/// A [`StaticKey`] that keeps its key pair to itself, as a key store does,
/// and counts its Diffie-Hellmans; with `fail`, it is a locked store.
#[derive(Clone)]
struct StoreKey {
    pair: StaticKeypair,
    dhs: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    fail: bool,
}

impl StaticKey for StoreKey {
    fn public(&self) -> [u8; KEY_LEN] {
        self.pair.public
    }

    fn dh(&self, peer: &[u8; KEY_LEN]) -> Option<[u8; KEY_LEN]> {
        self.dhs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail {
            return None;
        }
        self.pair.dh(peer)
    }
}

fn store_key(fail: bool) -> StoreKey {
    StoreKey {
        pair: generate_keypair().unwrap(),
        dhs: Default::default(),
        fail,
    }
}

#[tokio::test]
async fn a_key_store_key_connects() {
    use std::sync::atomic::Ordering::SeqCst;
    let server = generate_keypair().unwrap();
    let client = store_key(false);

    // IK: `ss` on the first message and `se` on the reply.
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        reconnect(c, &client, &server.public, Some(b"hello")),
        accept(s, &server, |key| key == &client.pair.public),
    );
    let mut cs = cr.unwrap();
    let mut acc = sr.unwrap();
    assert_eq!(client.dhs.load(SeqCst), 2);
    acc.stream.send_message(b"welcome").await.unwrap();
    assert_eq!(cs.recv_message().await.unwrap(), b"welcome");
}

#[tokio::test]
async fn a_locked_key_store_fails_the_handshake() {
    let server = generate_keypair().unwrap();
    let client = store_key(true);
    let (c, s) = duplex(PIPE);
    let (cr, _) = tokio::join!(
        reconnect(c, &client, &server.public, Some(b"hello")),
        accept(s, &server, |_| true),
    );
    assert!(matches!(cr, Err(LinkError::Noise(_))), "{:?}", cr.err());
}

#[tokio::test]
async fn ik_with_a_rejected_key_gets_no_reply() {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();

    // Through the client API: the stream ends with no reply.
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        reconnect(c, &client, &server.public, Some(b"hello")),
        accept(s, &server, |_| false),
    );
    assert!(matches!(sr, Err(LinkError::Unauthorized)));
    assert!(matches!(cr, Err(LinkError::Closed)));

    // By hand: not a single byte comes back.
    let (mut c, s) = duplex(PIPE);
    let mut prologue = PROLOGUE_PREFIX.to_vec();
    prologue.push(MODE_RECONNECT);
    let mut hs = snow::Builder::new(RECONNECT_PATTERN.parse().unwrap())
        .local_private_key(&client.private)
        .unwrap()
        .remote_public_key(&server.public)
        .unwrap()
        .prologue(&prologue)
        .unwrap()
        .build_initiator()
        .unwrap();
    let mut buf = vec![0u8; MAX_FRAME];
    let n = hs.write_message(b"hello", &mut buf).unwrap();
    let mut head = vec![MODE_RECONNECT];
    head.extend_from_slice(&(n as u16).to_be_bytes());
    head.extend_from_slice(&buf[..n]);
    c.write_all(&head).await.unwrap();
    let sr = accept(s, &server, |_| false).await;
    assert!(matches!(sr, Err(LinkError::Unauthorized)));
    let mut reply = Vec::new();
    c.read_to_end(&mut reply).await.unwrap();
    assert!(reply.is_empty());
}

#[tokio::test]
async fn an_unknown_mode_byte_closes_the_connection() {
    // 0x01 was the pairing handshake (XXpsk3), and is now unknown too.
    for mode in [0x01, 0x03] {
        let server = generate_keypair().unwrap();
        let (mut c, s) = duplex(PIPE);
        c.write_all(&[mode, 0, 0, 0, 0, 0, 32]).await.unwrap();
        c.write_all(&[0; 32]).await.unwrap();
        let sr = accept(s, &server, |_| panic!("no key to authorize")).await;
        assert!(
            matches!(sr, Err(LinkError::UnknownMode(m)) if m == mode),
            "{:?}",
            sr.err()
        );
        let mut reply = Vec::new();
        c.read_to_end(&mut reply).await.unwrap();
        assert!(reply.is_empty());
    }
}

#[tokio::test]
async fn one_mib_round_trips_in_chunks() {
    let (mut a, mut b) = connected().await;
    let msg: Vec<u8> = (0..MAX_MESSAGE).map(|i| (i % 251) as u8).collect();
    assert!(msg.len() > 16 * MAX_CHUNK, "the message spans many frames");
    let (sent, got) = tokio::join!(a.send_message(&msg), b.recv_message());
    sent.unwrap();
    assert_eq!(got.unwrap(), msg);

    // A message of exactly one chunk, and the stream stays in step.
    let one = vec![7u8; MAX_CHUNK];
    let (sent, got) = tokio::join!(b.send_message(&one), a.recv_message());
    sent.unwrap();
    assert_eq!(got.unwrap(), one);
}

#[tokio::test]
async fn one_mib_plus_one_byte_closes_the_connection() {
    let (mut a, mut b) = connected().await;
    let msg = vec![1u8; MAX_MESSAGE + 1];

    // The sender refuses it outright.
    assert!(matches!(
        a.send_message(&msg).await,
        Err(LinkError::MessageTooLarge)
    ));

    // A sender that does not check is cut off by the receiver.
    let (sent, got) = tokio::join!(a.send_unchecked(&msg), b.recv_message());
    sent.unwrap();
    assert!(matches!(got, Err(LinkError::MessageTooLarge)));
    assert!(matches!(b.recv_message().await, Err(LinkError::Closed)));
    assert!(matches!(a.recv_message().await, Err(LinkError::Closed)));
}

#[tokio::test]
async fn split_halves_carry_messages() {
    let (a, mut b) = connected().await;
    let (mut ar, mut aw) = a.into_split();
    aw.send_message(b"one").await.unwrap();
    assert_eq!(b.recv_message().await.unwrap(), b"one");
    b.send_message(b"two").await.unwrap();
    assert_eq!(ar.recv_message().await.unwrap(), b"two");
    aw.send_message(b"three").await.unwrap();
    assert_eq!(b.recv_message().await.unwrap(), b"three");
}

// Plan 20261027_07, step 2, and the 14:23 EDT amendment: the split flag
// byte, flags low and kind high.

#[tokio::test]
async fn a_terminal_frame_through_recv_message_is_bad_frame() {
    let (mut a, mut b) = connected().await;
    a.send_terminal(b"\x00\x00\x00\x01frame").await.unwrap();
    assert!(matches!(b.recv_message().await, Err(LinkError::BadFrame)));
    assert!(matches!(b.recv_message().await, Err(LinkError::Closed)));
}

#[tokio::test]
async fn a_terminal_frame_through_recv_is_terminal_with_the_same_bytes() {
    let (mut a, mut b) = connected().await;
    a.send_terminal(b"diff").await.unwrap();
    let got = b.recv().await.unwrap();
    assert_eq!(got.kind(), Kind::Terminal);
    assert_eq!(got, Message::Terminal(b"diff".to_vec()));

    // More than one chunk (> 65 KB), and the stream stays in step.
    let big: Vec<u8> = (0..3 * MAX_CHUNK + 17).map(|i| (i % 253) as u8).collect();
    assert!(big.len() > 65 * 1024);
    let (sent, got) = tokio::join!(a.send_terminal(&big), b.recv());
    sent.unwrap();
    assert_eq!(got.unwrap(), Message::Terminal(big));
    a.send_terminal(b"").await.unwrap();
    assert_eq!(b.recv().await.unwrap(), Message::Terminal(vec![]));
}

#[tokio::test]
async fn a_json_message_through_recv_is_json() {
    let (mut a, mut b) = connected().await;
    a.send_message(br#"{"jsonrpc":"2.0"}"#).await.unwrap();
    assert_eq!(
        b.recv().await.unwrap(),
        Message::Json(br#"{"jsonrpc":"2.0"}"#.to_vec())
    );
    let big = vec![9u8; 2 * MAX_CHUNK + 1];
    let (sent, got) = tokio::join!(a.send_kind(Kind::Json, &big), b.recv());
    sent.unwrap();
    assert_eq!(got.unwrap().into_bytes(), big);
    // Mixed kinds, one after the other, and recv_message still reads JSON.
    a.send_terminal(b"t").await.unwrap();
    a.send_message(b"j").await.unwrap();
    assert_eq!(b.recv().await.unwrap(), Message::Terminal(b"t".to_vec()));
    assert_eq!(b.recv_message().await.unwrap(), b"j");
}

#[tokio::test]
async fn kinds_are_the_high_nibble_and_json_is_unchanged() {
    // The raw bytes a sender writes are the bytes the old layout wrote
    // for kind 0, and 0x10/0x11 for a terminal frame.
    let (mut a, mut b) = connected().await;
    a.send_raw_frames(&[(0x01, b"ab"), (0x00, b"c")])
        .await
        .unwrap();
    assert_eq!(b.recv_message().await.unwrap(), b"abc");
    a.send_raw_frames(&[(0x11, b"ab"), (0x10, b"c")])
        .await
        .unwrap();
    assert_eq!(b.recv().await.unwrap(), Message::Terminal(b"abc".to_vec()));
    assert_eq!(Kind::Json.nibble(), KIND_JSON);
    assert_eq!(Kind::Terminal.nibble(), KIND_TERMINAL);
    assert_eq!(Kind::from_nibble(KIND_EXTENDED), None);
}

/// Sends `frames` raw and expects `BadFrame` from `recv` (or from
/// `recv_message` when `json_only`), then a closed stream.
async fn expect_bad_frame(frames: &[(u8, &[u8])], json_only: bool) {
    let (mut a, mut b) = connected().await;
    a.send_raw_frames(frames).await.unwrap();
    let got = if json_only {
        b.recv_message().await.map(Message::Json)
    } else {
        b.recv().await
    };
    assert!(
        matches!(got, Err(LinkError::BadFrame)),
        "{frames:?}: {got:?}"
    );
    assert!(matches!(b.recv().await, Err(LinkError::Closed)));
}

#[tokio::test]
async fn reserved_flag_bits_are_bad_frame() {
    for flag in [0x02, 0x04, 0x08, 0x0E, 0x03, 0x12, 0x18] {
        expect_bad_frame(&[(flag, b"x")], false).await;
        expect_bad_frame(&[(flag, b"x")], true).await;
    }
    // On a later frame of a message, too.
    expect_bad_frame(&[(0x01, b"a"), (0x04, b"b")], false).await;
    expect_bad_frame(&[(0x11, b"a"), (0x12, b"b")], false).await;
}

#[tokio::test]
async fn reserved_and_extended_kinds_are_bad_frame() {
    for kind in (2u8..=15).chain([1]) {
        for more in [0u8, 1] {
            let flag = kind << 4 | more;
            if kind != 1 {
                expect_bad_frame(&[(flag, b"x")], false).await;
            }
            expect_bad_frame(&[(flag, b"x")], true).await;
        }
    }
}

#[tokio::test]
async fn a_kind_changing_mid_message_is_bad_frame() {
    expect_bad_frame(&[(0x01, b"a"), (0x10, b"b")], false).await;
    expect_bad_frame(&[(0x11, b"a"), (0x00, b"b")], false).await;
    expect_bad_frame(&[(0x11, b"a"), (0x01, b"b"), (0x00, b"c")], false).await;
    // recv_message refuses the terminal frame that follows.
    expect_bad_frame(&[(0x01, b"a"), (0x10, b"b")], true).await;
}

#[tokio::test]
async fn split_halves_carry_terminal_frames() {
    let (a, b) = connected().await;
    let (mut ar, mut aw) = a.into_split();
    let (mut br, mut bw) = b.into_split();
    aw.send_terminal(b"frame").await.unwrap();
    assert_eq!(
        br.recv().await.unwrap(),
        Message::Terminal(b"frame".to_vec())
    );
    bw.send_message(b"ack").await.unwrap();
    assert_eq!(ar.recv().await.unwrap(), Message::Json(b"ack".to_vec()));
    bw.send_kind(Kind::Terminal, b"t").await.unwrap();
    assert!(matches!(ar.recv_message().await, Err(LinkError::BadFrame)));
    assert!(matches!(ar.recv().await, Err(LinkError::Closed)));
}

#[test]
fn public_key_matches_the_generated_pair() {
    let kp = generate_keypair().unwrap();
    assert_eq!(public_key(&kp.private).unwrap(), kp.public);
    assert_eq!(StaticKeypair::from_private(kp.private).unwrap(), kp);
    assert!(!format!("{kp:?}").contains(&format!("{:?}", kp.private)));
}

#[test]
fn pairing_string_round_trips() {
    let server = generate_keypair().unwrap();
    let p = payload(&server);
    let s = p.encode().unwrap();
    assert!(s.starts_with("flankr1"));
    assert_eq!(PairingPayload::decode(&s).unwrap(), p);
    assert_eq!(format!(" {s}\n").parse::<PairingPayload>().unwrap(), p);
    // Version 2: the key and the addresses, nothing secret.
    let bytes = p.to_bytes().unwrap();
    assert_eq!(bytes[0], 2);
    assert_eq!(&bytes[1..33], &server.public);
    assert_eq!(bytes.len(), 1 + 32 + 1 + (1 + 24) + (1 + 14) + 4);
}

#[test]
fn a_changed_character_fails_the_checksum() {
    // Fixed keys, so the string (and the character changed) is the same on
    // every run.
    let server = StaticKeypair {
        private: [1; KEY_LEN],
        public: [2; KEY_LEN],
    };
    let s = payload(&server).encode().unwrap();
    let mid = PAIRING_PREFIX.len() + (s.len() - PAIRING_PREFIX.len()) / 2;
    assert_eq!(
        PairingPayload::decode(&change_one(&s, mid)),
        Err(PairingStringError::BadChecksum)
    );
}

/// `s` with the character at `at` changed, staying base58.
fn change_one(s: &str, at: usize) -> String {
    let c = s.as_bytes()[at];
    let swapped = if c == b'2' { '3' } else { '2' };
    let mut t = s.to_string();
    t.replace_range(at..at + 1, &swapped.to_string());
    t
}

#[test]
fn a_wrong_prefix_is_refused() {
    let server = generate_keypair().unwrap();
    let s = payload(&server).encode().unwrap();
    let t = s.replacen("flankr1", "flankr2", 1);
    assert_eq!(
        PairingPayload::decode(&t),
        Err(PairingStringError::BadPrefix)
    );
    assert_eq!(
        PairingPayload::decode(&s["flankr1".len()..]),
        Err(PairingStringError::BadPrefix)
    );
}

/// A pairing string of `body` with its checksum recomputed.
fn with_checksum(mut body: Vec<u8>) -> String {
    let sum = Blake2s256::digest(&body);
    body.extend_from_slice(&sum[..4]);
    format!("flankr1{}", bs58::encode(&body).into_string())
}

#[test]
fn an_unknown_version_is_refused() {
    let server = generate_keypair().unwrap();
    let mut bytes = payload(&server).to_bytes().unwrap();
    bytes.truncate(bytes.len() - 4);
    bytes[0] = 3;
    assert_eq!(
        PairingPayload::decode(&with_checksum(bytes)),
        Err(PairingStringError::UnknownVersion(3))
    );

    // A version 1 string, with its secret, expiry and name, is refused.
    let mut v1 = vec![1];
    v1.extend_from_slice(&server.public);
    v1.extend_from_slice(&[9; KEY_LEN]);
    v1.extend_from_slice(&1_792_000_000u32.to_be_bytes());
    v1.extend_from_slice(&[3, b's', b'2', b'3']);
    v1.extend_from_slice(&[1, 14]);
    v1.extend_from_slice(b"127.0.0.1:7347");
    assert_eq!(
        PairingPayload::decode(&with_checksum(v1)),
        Err(PairingStringError::UnknownVersion(1))
    );
}

#[test]
fn bad_base58_and_short_payloads_are_refused() {
    // '0', 'O', 'I' and 'l' are not in the Bitcoin alphabet.
    assert_eq!(
        PairingPayload::decode("flankr10OIl"),
        Err(PairingStringError::BadBase58)
    );
    assert_eq!(
        PairingPayload::decode("flankr1"),
        Err(PairingStringError::Malformed)
    );
    // Trailing bytes after the addresses.
    let server = generate_keypair().unwrap();
    let mut bytes = payload(&server).to_bytes().unwrap();
    bytes.truncate(bytes.len() - 4);
    bytes.push(0);
    assert_eq!(
        PairingPayload::decode(&with_checksum(bytes)),
        Err(PairingStringError::Malformed)
    );
}

#[test]
fn encoding_refuses_oversized_fields() {
    let server = generate_keypair().unwrap();
    let mut p = payload(&server);
    p.addresses = vec!["x".repeat(256)];
    assert_eq!(p.encode(), Err(PairingStringError::AddressTooLong));
    p.addresses = vec!["a:1".to_string(); 256];
    assert_eq!(p.encode(), Err(PairingStringError::TooManyAddresses));
}

#[test]
fn the_pairing_url_names_the_remote_address_and_round_trips() {
    let server = generate_keypair().unwrap();
    let mut p = payload(&server);
    p.addresses = vec!["127.0.0.1:7347".into(), "box.tail1234.ts.net:7347".into()];
    let url = p.url().unwrap();
    assert_eq!(
        url,
        format!(
            "flankr://box.tail1234.ts.net:7347/pair#{}",
            p.encode().unwrap()
        )
    );
    assert!(p.has_remote_address());
    assert_eq!(PairingPayload::decode(&format!("  {url}\n")).unwrap(), p);

    // Loopback alone: the first address, and nothing remote.
    p.addresses = vec![
        "127.0.0.1:7347".into(),
        "[::1]:7347".into(),
        "localhost:7347".into(),
    ];
    assert!(
        p.url()
            .unwrap()
            .starts_with("flankr://127.0.0.1:7347/pair#flankr1")
    );
    assert!(!p.has_remote_address());

    p.addresses.clear();
    assert_eq!(p.url(), Err(PairingStringError::BadUrl));
}

#[test]
fn a_pairing_url_must_match_its_payload() {
    let server = generate_keypair().unwrap();
    let p = payload(&server);
    let s = p.encode().unwrap();
    for bad in [
        format!("flankr://other.ts.net:7347/pair#{s}"),
        format!("flankr://box.tail1234.ts.net:7347/join#{s}"),
        "flankr://box.tail1234.ts.net:7347/pair".to_string(),
        format!(
            "flankr://box.tail1234.ts.net:7347/pair#{}",
            &s[..s.len() - 1]
        ),
    ] {
        assert!(PairingPayload::decode(&bad).is_err(), "{bad}");
    }
    assert_eq!(
        PairingPayload::decode(&format!("flankr://other.ts.net:7347/pair#{s}")),
        Err(PairingStringError::BadUrl)
    );
    assert_eq!(
        PairingPayload::decode(&format!("https://box.tail1234.ts.net:7347/pair#{s}")),
        Err(PairingStringError::BadPrefix)
    );
}

fn device(name: &str) -> DeviceCode {
    DeviceCode {
        key: generate_keypair().unwrap().public,
        name: name.to_string(),
    }
}

#[test]
fn a_device_code_round_trips() {
    let server = generate_keypair().unwrap();
    let d = device("s23-ultra_1.a");
    let code = d.encode(&server.public).unwrap();
    assert!(code.starts_with("flankrkey1"), "{code}");
    // About 60 characters for a short name.
    let short = device("s23").encode(&server.public).unwrap();
    assert!((60..=70).contains(&short.len()), "{} {short}", short.len());
    assert_eq!(DeviceCode::decode(&code, &server.public).unwrap(), d);
    assert_eq!(
        DeviceCode::decode(&format!("  {code}\n"), &server.public).unwrap(),
        d
    );
    // The layout: version 1, the key, the name; the checksum covers the
    // server's key first.
    let bytes = bs58::decode(&code["flankrkey1".len()..])
        .into_vec()
        .unwrap();
    assert_eq!(bytes[0], 1);
    assert_eq!(&bytes[1..33], &d.key);
    assert_eq!(usize::from(bytes[33]), d.name.len());
    assert_eq!(&bytes[34..bytes.len() - 4], d.name.as_bytes());
    let mut sum = Blake2s256::new();
    sum.update(server.public);
    sum.update(&bytes[..bytes.len() - 4]);
    assert_eq!(&bytes[bytes.len() - 4..], &sum.finalize()[..4]);
}

#[test]
fn a_device_code_for_another_server_is_refused() {
    let server = generate_keypair().unwrap();
    let other = generate_keypair().unwrap();
    let code = device("s23").encode(&other.public).unwrap();
    assert_eq!(
        DeviceCode::decode(&code, &server.public),
        Err(PairingStringError::BadChecksum)
    );
}

#[test]
fn a_mistyped_device_code_is_refused() {
    let server = StaticKeypair {
        private: [1; KEY_LEN],
        public: [2; KEY_LEN],
    };
    let code = DeviceCode {
        key: [5; KEY_LEN],
        name: "s23".into(),
    }
    .encode(&server.public)
    .unwrap();
    let mid = DEVICE_CODE_PREFIX.len() + (code.len() - DEVICE_CODE_PREFIX.len()) / 2;
    assert_eq!(
        DeviceCode::decode(&change_one(&code, mid), &server.public),
        Err(PairingStringError::BadChecksum)
    );
    assert_eq!(
        DeviceCode::decode(
            &code.replacen("flankrkey1", "flankrkey2", 1),
            &server.public
        ),
        Err(PairingStringError::BadPrefix)
    );
    // A pairing string is not a device code, nor the other way round.
    let pairing = payload(&server).encode().unwrap();
    assert_eq!(
        DeviceCode::decode(&pairing, &server.public),
        Err(PairingStringError::BadPrefix)
    );
    assert_eq!(
        PairingPayload::decode(&code),
        Err(PairingStringError::BadPrefix)
    );
}

#[test]
fn a_bad_device_name_is_refused() {
    let server = generate_keypair().unwrap();
    for bad in ["", "has space", "slash/", "ünï", &"x".repeat(33)] {
        assert!(!DeviceCode::valid_name(bad), "{bad}");
        assert_eq!(
            device(bad).encode(&server.public),
            Err(PairingStringError::BadName),
            "{bad}"
        );
    }
    assert!(DeviceCode::valid_name(&"x".repeat(32)));

    // A code that carries a bad name, with a good checksum, is refused.
    let key = [7u8; KEY_LEN];
    let mut body = vec![1];
    body.extend_from_slice(&key);
    body.extend_from_slice(&[2, b'a', b' ']);
    let mut sum = Blake2s256::new();
    sum.update(server.public);
    sum.update(&body);
    body.extend_from_slice(&sum.finalize()[..4]);
    let code = format!("flankrkey1{}", bs58::encode(&body).into_string());
    assert_eq!(
        DeviceCode::decode(&code, &server.public),
        Err(PairingStringError::BadName)
    );
}
