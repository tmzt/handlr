//! Plan 02, step 1: both handshakes and the framing over an in-memory
//! duplex, and the pairing string.

use blake2::{Blake2s256, Digest};
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::*;

const PIPE: usize = 64 * 1024;

fn payload(server: &StaticKeypair, secret: [u8; KEY_LEN]) -> PairingPayload {
    PairingPayload {
        server_key: server.public,
        secret,
        expiry: 1_792_000_000,
        name: "s23".into(),
        addresses: vec!["box.tail1234.ts.net:7347".into(), "127.0.0.1:7347".into()],
    }
}

/// A paired connection: (client stream, server stream).
async fn paired() -> (LinkStream<DuplexStream>, LinkStream<DuplexStream>) {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let secret = generate_secret().unwrap();
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        pair(c, &client.private, &server.public, &secret, None),
        accept(s, &server.private, |_| Some(secret), |_| false),
    );
    (cr.unwrap().0, sr.unwrap().stream)
}

#[tokio::test]
async fn pairing_succeeds_with_the_right_secret() {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let secret = generate_secret().unwrap();
    let p = payload(&server, secret);
    let id = p.pairing_id();
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        pair(
            c,
            &client.private,
            &p.server_key,
            &p.secret,
            Some(b"{\"hello\":1}")
        ),
        accept(
            s,
            &server.private,
            |got| (got == id).then_some(secret),
            |_| panic!("pairing must not consult authorize"),
        ),
    );
    let (mut cs, server_key) = cr.unwrap();
    let mut acc = sr.unwrap();
    assert_eq!(server_key, server.public);
    assert_eq!(cs.peer_key(), &server.public);
    assert_eq!(acc.peer_key, client.public);
    assert_eq!(acc.stream.peer_key(), &client.public);
    assert_eq!(acc.mode, AcceptedMode::Pairing(id));
    assert_eq!(acc.first_message.as_deref(), Some(&b"{\"hello\":1}"[..]));

    cs.send_message(b"ping").await.unwrap();
    assert_eq!(acc.stream.recv_message().await.unwrap(), b"ping");
    acc.stream.send_message(b"pong").await.unwrap();
    assert_eq!(cs.recv_message().await.unwrap(), b"pong");
}

#[tokio::test]
async fn pairing_fails_with_a_wrong_secret() {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let secret = generate_secret().unwrap();
    let wrong = generate_secret().unwrap();

    // The id names no pending pairing: refused before the handshake.
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        pair(c, &client.private, &server.public, &wrong, None),
        accept(
            s,
            &server.private,
            |got| (got == PairingId::from_secret(&secret)).then_some(secret),
            |_| false,
        ),
    );
    assert!(matches!(sr, Err(LinkError::UnknownPairing)));
    assert!(matches!(cr, Err(LinkError::Closed)));

    // The server uses the real secret whatever the id: the PSK differs and
    // the server's read of the last handshake message fails.
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        pair(c, &client.private, &server.public, &wrong, None),
        accept(s, &server.private, |_| Some(secret), |_| false),
    );
    assert!(matches!(sr, Err(LinkError::Noise(_))), "{:?}", sr.err());
    let (mut cs, _) = cr.unwrap();
    assert!(matches!(cs.recv_message().await, Err(LinkError::Closed)));
}

#[tokio::test]
async fn a_wrong_server_key_in_the_payload_aborts_the_client() {
    let server = generate_keypair().unwrap();
    let other = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let secret = generate_secret().unwrap();
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        pair(c, &client.private, &other.public, &secret, None),
        accept(s, &server.private, |_| Some(secret), |_| false),
    );
    assert!(matches!(cr, Err(LinkError::ServerKeyMismatch)));
    // The client never sent its static key; the server saw the stream end.
    assert!(matches!(sr, Err(LinkError::Closed)));
}

#[tokio::test]
async fn reconnect_carries_the_first_message() {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        reconnect(c, &client.private, &server.public, Some(b"hello")),
        accept(
            s,
            &server.private,
            |_| panic!("reconnecting must not look up a pairing"),
            |key| key == &client.public,
        ),
    );
    let mut cs = cr.unwrap();
    let mut acc = sr.unwrap();
    assert_eq!(acc.mode, AcceptedMode::Reconnect);
    assert_eq!(acc.peer_key, client.public);
    assert_eq!(acc.first_message.as_deref(), Some(&b"hello"[..]));
    acc.stream.send_message(b"welcome").await.unwrap();
    assert_eq!(cs.recv_message().await.unwrap(), b"welcome");
    cs.send_message(b"").await.unwrap();
    assert_eq!(acc.stream.recv_message().await.unwrap(), b"");
}

#[tokio::test]
async fn ik_with_a_rejected_key_gets_no_reply() {
    let server = generate_keypair().unwrap();
    let client = generate_keypair().unwrap();

    // Through the client API: the stream ends with no reply.
    let (c, s) = duplex(PIPE);
    let (cr, sr) = tokio::join!(
        reconnect(c, &client.private, &server.public, Some(b"hello")),
        accept(s, &server.private, |_| None, |_| false),
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
    let sr = accept(s, &server.private, |_| None, |_| false).await;
    assert!(matches!(sr, Err(LinkError::Unauthorized)));
    let mut reply = Vec::new();
    c.read_to_end(&mut reply).await.unwrap();
    assert!(reply.is_empty());
}

#[tokio::test]
async fn an_unknown_mode_byte_closes_the_connection() {
    let server = generate_keypair().unwrap();
    let (mut c, s) = duplex(PIPE);
    c.write_all(&[0x03, 0, 0]).await.unwrap();
    let sr = accept(s, &server.private, |_| None, |_| true).await;
    assert!(matches!(sr, Err(LinkError::UnknownMode(0x03))));
    let mut reply = Vec::new();
    c.read_to_end(&mut reply).await.unwrap();
    assert!(reply.is_empty());
}

#[tokio::test]
async fn one_mib_round_trips_in_chunks() {
    let (mut a, mut b) = paired().await;
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
    let (mut a, mut b) = paired().await;
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
    let (a, mut b) = paired().await;
    let (mut ar, mut aw) = a.into_split();
    aw.send_message(b"one").await.unwrap();
    assert_eq!(b.recv_message().await.unwrap(), b"one");
    b.send_message(b"two").await.unwrap();
    assert_eq!(ar.recv_message().await.unwrap(), b"two");
    aw.send_message(b"three").await.unwrap();
    assert_eq!(b.recv_message().await.unwrap(), b"three");
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
    let p = payload(&server, generate_secret().unwrap());
    let s = p.encode().unwrap();
    assert!(s.starts_with("handlr1"));
    assert_eq!(PairingPayload::decode(&s).unwrap(), p);
    assert_eq!(format!(" {s}\n").parse::<PairingPayload>().unwrap(), p);
    assert_eq!(p.pairing_id(), PairingId::from_secret(&p.secret));
    assert!(!format!("{p:?}").contains(&format!("{:?}", p.secret)));
}

#[test]
fn a_changed_character_fails_the_checksum() {
    // Fixed keys, so the string (and the character changed) is the same on
    // every run.
    let server = StaticKeypair {
        private: [1; KEY_LEN],
        public: [2; KEY_LEN],
    };
    let s = payload(&server, [9; KEY_LEN]).encode().unwrap();
    let mid = PAIRING_PREFIX.len() + (s.len() - PAIRING_PREFIX.len()) / 2;
    let c = s.as_bytes()[mid];
    let swapped = if c == b'2' { '3' } else { '2' };
    let mut t = s.clone();
    t.replace_range(mid..mid + 1, &swapped.to_string());
    assert_eq!(
        PairingPayload::decode(&t),
        Err(PairingStringError::BadChecksum)
    );
}

#[test]
fn a_wrong_prefix_is_refused() {
    let server = generate_keypair().unwrap();
    let s = payload(&server, [9; KEY_LEN]).encode().unwrap();
    let t = s.replacen("handlr1", "handlr2", 1);
    assert_eq!(
        PairingPayload::decode(&t),
        Err(PairingStringError::BadPrefix)
    );
    assert_eq!(
        PairingPayload::decode(&s["handlr1".len()..]),
        Err(PairingStringError::BadPrefix)
    );
}

#[test]
fn an_unknown_version_is_refused() {
    let server = generate_keypair().unwrap();
    let mut bytes = payload(&server, [9; KEY_LEN]).to_bytes().unwrap();
    let body = bytes.len() - 4;
    bytes[0] = 2;
    let sum = Blake2s256::digest(&bytes[..body]);
    bytes[body..].copy_from_slice(&sum[..4]);
    let s = format!("handlr1{}", bs58::encode(&bytes).into_string());
    assert_eq!(
        PairingPayload::decode(&s),
        Err(PairingStringError::UnknownVersion(2))
    );
}

#[test]
fn bad_base58_and_short_payloads_are_refused() {
    // '0', 'O', 'I' and 'l' are not in the Bitcoin alphabet.
    assert_eq!(
        PairingPayload::decode("handlr10OIl"),
        Err(PairingStringError::BadBase58)
    );
    assert_eq!(
        PairingPayload::decode("handlr1"),
        Err(PairingStringError::Malformed)
    );
}

#[test]
fn encoding_refuses_oversized_fields() {
    let server = generate_keypair().unwrap();
    let mut p = payload(&server, [9; KEY_LEN]);
    p.name = "x".repeat(256);
    assert_eq!(p.encode(), Err(PairingStringError::NameTooLong));
}
