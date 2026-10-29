//! The byte layouts of decision 4.1.

use crate::{
    CHUNK_MAX, Code, MIME_MAX, NAME_MAX, NS_MAX, REASON_MAX, REQUEST_MAX, SESSION_MAX, TransferId,
};

/// A namespace: ASCII `[a-z0-9_.]`, 1..=128 bytes, not starting or ending
/// with `.` (decision 4.1). The built-ins are one word; a plugin's is its
/// reverse-DNS package name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Namespace(String);

/// Why a namespace is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamespaceError {
    /// Empty, or longer than 128 bytes.
    Length,
    /// A byte outside `[a-z0-9_.]`.
    Character,
    /// It starts or ends with `.`.
    Dot,
}

impl std::fmt::Display for NamespaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NamespaceError::Length => "a namespace is 1 to 128 bytes",
            NamespaceError::Character => "a namespace is lowercase letters, digits, '_' and '.'",
            NamespaceError::Dot => "a namespace may not start or end with '.'",
        })
    }
}

impl std::error::Error for NamespaceError {}

impl Namespace {
    pub fn new(s: &str) -> Result<Namespace, NamespaceError> {
        Self::check(s.as_bytes())?;
        Ok(Namespace(s.to_string()))
    }

    fn check(b: &[u8]) -> Result<(), NamespaceError> {
        if b.is_empty() || b.len() > NS_MAX {
            return Err(NamespaceError::Length);
        }
        if !b
            .iter()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'))
        {
            return Err(NamespaceError::Character);
        }
        if b[0] == b'.' || b[b.len() - 1] == b'.' {
            return Err(NamespaceError::Dot);
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The built-in `images`.
    pub fn images() -> Namespace {
        Namespace(crate::NS_IMAGES.to_string())
    }

    /// The built-in `files`.
    pub fn files() -> Namespace {
        Namespace(crate::NS_FILES.to_string())
    }

    /// The built-in `git`.
    pub fn git() -> Namespace {
        Namespace(crate::NS_GIT.to_string())
    }
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A subcommand's byte (decision 4.1). `0x00` and `0x09`-`0xFF` are
/// reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Subcommand {
    Request = 0x01,
    Offer = 0x02,
    Accept = 0x03,
    Refuse = 0x04,
    Chunk = 0x05,
    End = 0x06,
    Done = 0x07,
    Cancel = 0x08,
}

impl Subcommand {
    pub fn from_byte(b: u8) -> Option<Subcommand> {
        Some(match b {
            0x01 => Subcommand::Request,
            0x02 => Subcommand::Offer,
            0x03 => Subcommand::Accept,
            0x04 => Subcommand::Refuse,
            0x05 => Subcommand::Chunk,
            0x06 => Subcommand::End,
            0x07 => Subcommand::Done,
            0x08 => Subcommand::Cancel,
            _ => return None,
        })
    }
}

/// An OFFER's body: what is about to be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// The pane id of the session (decision 4.8), 1..=128 bytes.
    pub session: String,
    /// A display name, 1..=1024 bytes: never a path on the receiver.
    pub name: String,
    /// The MIME type, ASCII, 1..=127 bytes.
    pub mime: String,
    pub size: u64,
    pub sha256: [u8; 32],
    /// The file's mtime in Unix ms, 0 when unknown.
    pub modified_ms: u64,
}

impl Offer {
    /// An offer of `bytes`, its size and SHA-256 computed.
    pub fn for_bytes(
        session: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
        modified_ms: u64,
    ) -> Offer {
        Offer {
            session: session.to_string(),
            name: name.to_string(),
            mime: mime.to_string(),
            size: bytes.len() as u64,
            sha256: crate::sha256(bytes),
            modified_ms,
        }
    }
}

/// A message's body, by subcommand (decision 4.1). Open: later
/// subcommands are additive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Body {
    /// Receiver-to-be → sender: the namespace's request (`files`: the
    /// path; `git`: decision 6.5), at most `max_size` bytes wanted.
    Request {
        session: String,
        request: String,
        max_size: u32,
    },
    Offer(Offer),
    /// Receiver: send, at most `chunk_max` (1..=983040) bytes per CHUNK.
    Accept {
        chunk_max: u32,
    },
    /// The answer to a REQUEST or OFFER: no.
    Refuse {
        code: Code,
        reason: String,
    },
    /// Sender: `data` (1..=`chunk_max` bytes) at `offset`.
    Chunk {
        offset: u64,
        data: Vec<u8>,
    },
    /// Sender: all `size` bytes are sent.
    End {
        size: u64,
    },
    /// Receiver: stored and verified; the namespace's body (`images`: the
    /// 8-byte reference; `files`, `git`: empty).
    Done {
        body: Vec<u8>,
    },
    /// Either side: the transfer is over.
    Cancel {
        code: Code,
        reason: String,
    },
}

impl Body {
    pub fn subcommand(&self) -> Subcommand {
        match self {
            Body::Request { .. } => Subcommand::Request,
            Body::Offer(_) => Subcommand::Offer,
            Body::Accept { .. } => Subcommand::Accept,
            Body::Refuse { .. } => Subcommand::Refuse,
            Body::Chunk { .. } => Subcommand::Chunk,
            Body::End { .. } => Subcommand::End,
            Body::Done { .. } => Subcommand::Done,
            Body::Cancel { .. } => Subcommand::Cancel,
        }
    }

    /// A CANCEL with `code` and `reason`.
    pub fn cancel(code: Code, reason: impl Into<String>) -> Body {
        Body::Cancel {
            code,
            reason: reason.into(),
        }
    }

    /// A REFUSE with `code` and `reason`.
    pub fn refuse(code: Code, reason: impl Into<String>) -> Body {
        Body::Refuse {
            code,
            reason: reason.into(),
        }
    }
}

/// One kind-2 message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobMessage {
    pub transfer: TransferId,
    pub ns: Namespace,
    pub body: Body,
}

/// Why a message did not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The header cannot be read (shorter than 6 bytes, id 0, a bad
    /// namespace): close the connection, as a bad frame does.
    Fatal,
    /// The header was read but the subcommand is reserved or the body does
    /// not decode: answer once with CANCEL `protocol` for this transfer.
    Answer(TransferId, Namespace),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Fatal => f.write_str("unreadable binary message header"),
            DecodeError::Answer(id, ns) => {
                write!(f, "bad binary message body (transfer {id}, namespace {ns})")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// `s` cut to at most `max` bytes, at a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn header(out: &mut Vec<u8>, sub: Subcommand, transfer: TransferId, ns: &Namespace) {
    out.push(sub as u8);
    out.extend_from_slice(&transfer.0.to_be_bytes());
    out.push(ns.0.len() as u8);
    out.extend_from_slice(ns.0.as_bytes());
}

fn put_str8(out: &mut Vec<u8>, s: &str, max: usize) {
    let s = cut(s, max.min(255));
    out.push(s.len() as u8);
    out.extend_from_slice(s.as_bytes());
}

fn put_str16(out: &mut Vec<u8>, s: &str, max: usize) {
    let s = cut(s, max.min(65535));
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

impl BlobMessage {
    pub fn new(transfer: TransferId, ns: Namespace, body: Body) -> BlobMessage {
        BlobMessage { transfer, ns, body }
    }

    /// The message's bytes. A string over its field's limit (session 128,
    /// request and name 1024, MIME 127, reason 512 bytes) is cut at a
    /// character boundary; the other limits are the caller's to keep.
    pub fn encode(&self) -> Vec<u8> {
        if let Body::Chunk { offset, data } = &self.body {
            return Self::encode_chunk(self.transfer, &self.ns, *offset, data);
        }
        let mut out = Vec::with_capacity(self.encoded_len());
        header(&mut out, self.body.subcommand(), self.transfer, &self.ns);
        match &self.body {
            Body::Request {
                session,
                request,
                max_size,
            } => {
                put_str8(&mut out, session, SESSION_MAX);
                put_str16(&mut out, request, REQUEST_MAX);
                out.extend_from_slice(&max_size.to_be_bytes());
            }
            Body::Offer(o) => {
                put_str8(&mut out, &o.session, SESSION_MAX);
                put_str16(&mut out, &o.name, NAME_MAX);
                put_str8(&mut out, &o.mime, MIME_MAX);
                out.extend_from_slice(&o.size.to_be_bytes());
                out.extend_from_slice(&o.sha256);
                out.extend_from_slice(&o.modified_ms.to_be_bytes());
            }
            Body::Accept { chunk_max } => out.extend_from_slice(&chunk_max.to_be_bytes()),
            Body::Refuse { code, reason } | Body::Cancel { code, reason } => {
                out.extend_from_slice(&code.0.to_be_bytes());
                put_str16(&mut out, reason, REASON_MAX);
            }
            Body::Chunk { .. } => unreachable!("encoded above"),
            Body::End { size } => out.extend_from_slice(&size.to_be_bytes()),
            Body::Done { body } => out.extend_from_slice(body),
        }
        out
    }

    /// The length of [`encode`](Self::encode)'s bytes.
    pub fn encoded_len(&self) -> usize {
        let header = 6 + self.ns.0.len();
        header
            + match &self.body {
                Body::Request {
                    session, request, ..
                } => 1 + cut(session, SESSION_MAX).len() + 2 + cut(request, REQUEST_MAX).len() + 4,
                Body::Offer(o) => {
                    1 + cut(&o.session, SESSION_MAX).len()
                        + 2
                        + cut(&o.name, NAME_MAX).len()
                        + 1
                        + cut(&o.mime, MIME_MAX).len()
                        + 8
                        + 32
                        + 8
                }
                Body::Accept { .. } => 4,
                Body::Refuse { reason, .. } | Body::Cancel { reason, .. } => {
                    2 + 2 + cut(reason, REASON_MAX).len()
                }
                Body::Chunk { data, .. } => 8 + data.len(),
                Body::End { .. } => 8,
                Body::Done { body } => body.len(),
            }
    }

    /// A CHUNK's bytes from borrowed data, for senders.
    pub fn encode_chunk(transfer: TransferId, ns: &Namespace, offset: u64, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(6 + ns.0.len() + 8 + data.len());
        header(&mut out, Subcommand::Chunk, transfer, ns);
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    /// Reads one message. `Err(Fatal)` for an unreadable header (close the
    /// connection), `Err(Answer(id, ns))` for a readable header with a
    /// reserved subcommand or a bad body (CANCEL `protocol`).
    pub fn decode(bytes: &[u8]) -> Result<BlobMessage, DecodeError> {
        if bytes.len() < 6 {
            return Err(DecodeError::Fatal);
        }
        let sub = bytes[0];
        let transfer = TransferId(u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]));
        if !transfer.is_valid() {
            return Err(DecodeError::Fatal);
        }
        let ns_len = usize::from(bytes[5]);
        let ns_bytes = bytes.get(6..6 + ns_len).ok_or(DecodeError::Fatal)?;
        Namespace::check(ns_bytes).map_err(|_| DecodeError::Fatal)?;
        let ns = Namespace(String::from_utf8(ns_bytes.to_vec()).map_err(|_| DecodeError::Fatal)?);
        let rest = &bytes[6 + ns_len..];
        let answer = || DecodeError::Answer(transfer, ns.clone());
        let sub = Subcommand::from_byte(sub).ok_or_else(answer)?;
        let body = decode_body(sub, rest).ok_or_else(answer)?;
        Ok(BlobMessage { transfer, ns, body })
    }
}

/// A cursor over a body.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_be_bytes(b.try_into().expect("4 bytes")))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|b| u64::from_be_bytes(b.try_into().expect("8 bytes")))
    }
    /// A UTF-8 string of `len` bytes, `min..=max`.
    fn str(&mut self, len: usize, min: usize, max: usize) -> Option<String> {
        if len < min || len > max {
            return None;
        }
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
    fn str8(&mut self, min: usize, max: usize) -> Option<String> {
        let len = usize::from(self.u8()?);
        self.str(len, min, max)
    }
    fn str16(&mut self, min: usize, max: usize) -> Option<String> {
        let len = usize::from(self.u16()?);
        self.str(len, min, max)
    }
    fn end(self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

fn decode_body(sub: Subcommand, bytes: &[u8]) -> Option<Body> {
    let mut r = Reader(bytes);
    let body = match sub {
        Subcommand::Request => Body::Request {
            session: r.str8(1, SESSION_MAX)?,
            request: r.str16(1, REQUEST_MAX)?,
            max_size: r.u32()?,
        },
        Subcommand::Offer => {
            let session = r.str8(1, SESSION_MAX)?;
            let name = r.str16(1, NAME_MAX)?;
            let mime = r.str8(1, MIME_MAX)?;
            if !mime.bytes().all(|b| b.is_ascii_graphic()) {
                return None;
            }
            Body::Offer(Offer {
                session,
                name,
                mime,
                size: r.u64()?,
                sha256: r.take(32)?.try_into().expect("32 bytes"),
                modified_ms: r.u64()?,
            })
        }
        Subcommand::Accept => {
            let chunk_max = r.u32()?;
            if chunk_max == 0 || chunk_max > CHUNK_MAX {
                return None;
            }
            Body::Accept { chunk_max }
        }
        Subcommand::Refuse | Subcommand::Cancel => {
            let code = Code(r.u16()?);
            if code.0 == 0 {
                return None;
            }
            let reason = r.str16(0, REASON_MAX)?;
            if sub == Subcommand::Refuse {
                Body::Refuse { code, reason }
            } else {
                Body::Cancel { code, reason }
            }
        }
        Subcommand::Chunk => {
            let offset = r.u64()?;
            let data = r.0;
            if data.is_empty() || data.len() > CHUNK_MAX as usize {
                return None;
            }
            return Some(Body::Chunk {
                offset,
                data: data.to_vec(),
            });
        }
        Subcommand::End => Body::End { size: r.u64()? },
        Subcommand::Done => {
            return Some(Body::Done { body: r.0.to_vec() });
        }
    };
    r.end()?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(s: &str) -> Namespace {
        Namespace::new(s).unwrap()
    }

    fn round(transfer: u32, n: &str, body: Body, golden: &[u8]) {
        let m = BlobMessage::new(TransferId(transfer), ns(n), body);
        let bytes = m.encode();
        assert_eq!(bytes, golden, "{m:?}");
        assert_eq!(m.encoded_len(), golden.len());
        assert_eq!(BlobMessage::decode(&bytes).unwrap(), m);
    }

    #[test]
    fn every_subcommand_has_its_golden_bytes() {
        // Header: sub, transfer 0x00000102, ns "files".
        let h = |sub: u8| {
            let mut v = vec![sub, 0, 0, 1, 2, 5];
            v.extend_from_slice(b"files");
            v
        };
        let mut g = h(0x01);
        g.extend_from_slice(&[4]);
        g.extend_from_slice(b"w1:1");
        g.extend_from_slice(&[0, 9]);
        g.extend_from_slice(b"README.md");
        g.extend_from_slice(&[0, 0x80, 0, 0]);
        round(
            0x102,
            "files",
            Body::Request {
                session: "w1:1".into(),
                request: "README.md".into(),
                max_size: 8_388_608,
            },
            &g,
        );

        let mut g = h(0x02);
        g.extend_from_slice(&[4]);
        g.extend_from_slice(b"w1:1");
        g.extend_from_slice(&[0, 9]);
        g.extend_from_slice(b"README.md");
        g.extend_from_slice(&[13]);
        g.extend_from_slice(b"text/markdown");
        g.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x01, 0x00]);
        g.extend_from_slice(&[0xAB; 32]);
        g.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 7]);
        round(
            0x102,
            "files",
            Body::Offer(Offer {
                session: "w1:1".into(),
                name: "README.md".into(),
                mime: "text/markdown".into(),
                size: 256,
                sha256: [0xAB; 32],
                modified_ms: 7,
            }),
            &g,
        );

        let mut g = h(0x03);
        g.extend_from_slice(&[0, 0x0F, 0, 0]);
        round(0x102, "files", Body::Accept { chunk_max: 983_040 }, &g);

        let mut g = h(0x04);
        g.extend_from_slice(&[0, 3, 0, 2]);
        g.extend_from_slice(b"no");
        round(0x102, "files", Body::refuse(Code::NOT_PERMITTED, "no"), &g);

        let mut g = h(0x05);
        g.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 16]);
        g.extend_from_slice(b"abc");
        round(
            0x102,
            "files",
            Body::Chunk {
                offset: 16,
                data: b"abc".to_vec(),
            },
            &g,
        );

        let mut g = h(0x06);
        g.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 19]);
        round(0x102, "files", Body::End { size: 19 }, &g);

        round(0x102, "files", Body::Done { body: vec![] }, &h(0x07));

        let mut g = h(0x08);
        g.extend_from_slice(&[0, 15, 0, 0]);
        round(0x102, "files", Body::cancel(Code::CANCELLED, ""), &g);

        // An images DONE with its reference, on a daemon-picked id.
        let mut g = vec![0x07, 0x80, 0, 0, 1, 6];
        g.extend_from_slice(b"images");
        g.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        round(
            0x8000_0001,
            "images",
            Body::Done {
                body: vec![1, 2, 3, 4, 5, 6, 7, 8],
            },
            &g,
        );
    }

    #[test]
    fn unreadable_headers_are_fatal() {
        let good = BlobMessage::new(TransferId(1), ns("git"), Body::End { size: 0 }).encode();
        assert!(BlobMessage::decode(&good).is_ok());
        for n in 0..6 {
            assert_eq!(
                BlobMessage::decode(&good[..n]),
                Err(DecodeError::Fatal),
                "{n}"
            );
        }
        // Id 0, and the daemon half's 0.
        for id in [[0, 0, 0, 0], [0x80, 0, 0, 0]] {
            let mut b = good.clone();
            b[1..5].copy_from_slice(&id);
            assert_eq!(BlobMessage::decode(&b), Err(DecodeError::Fatal));
        }
        let with_ns = |n: &[u8]| {
            let mut b = vec![0x06, 0, 0, 0, 1, n.len() as u8];
            b.extend_from_slice(n);
            b.extend_from_slice(&[0; 8]);
            BlobMessage::decode(&b)
        };
        assert_eq!(with_ns(b""), Err(DecodeError::Fatal));
        assert_eq!(with_ns(&[b'a'; 129]), Err(DecodeError::Fatal));
        assert!(with_ns(&[b'a'; 128]).is_ok());
        for bad in [
            &b"Files"[..],
            b"files.",
            b".files",
            b"fi les",
            b"f-x",
            b"\xff",
        ] {
            assert_eq!(with_ns(bad), Err(DecodeError::Fatal), "{bad:?}");
        }
        assert!(with_ns(b"dev.flankr.plugins.planning").is_ok());
        // A namespace longer than the bytes left.
        assert_eq!(
            BlobMessage::decode(&[0x06, 0, 0, 0, 1, 9, b'a']),
            Err(DecodeError::Fatal)
        );
        assert_eq!(Namespace::new(""), Err(NamespaceError::Length));
        assert_eq!(Namespace::new("A"), Err(NamespaceError::Character));
        assert_eq!(Namespace::new("a."), Err(NamespaceError::Dot));
    }

    #[test]
    fn bad_bodies_and_reserved_subcommands_are_answered() {
        let answer = DecodeError::Answer(TransferId(5), ns("images"));
        let with = |sub: u8, body: &[u8]| {
            let mut b = vec![sub, 0, 0, 0, 5, 6];
            b.extend_from_slice(b"images");
            b.extend_from_slice(body);
            BlobMessage::decode(&b)
        };
        for sub in [0x00, 0x09, 0x7F, 0xFF] {
            assert_eq!(with(sub, &[]), Err(answer.clone()), "{sub}");
        }
        // Trailing bytes are refused, except in CHUNK and DONE.
        assert_eq!(with(0x06, &[0; 9]), Err(answer.clone()));
        assert_eq!(with(0x03, &[0, 0, 1, 0, 0]), Err(answer.clone()));
        assert!(with(0x05, &[0; 20]).is_ok());
        // Short bodies.
        assert_eq!(with(0x06, &[0; 7]), Err(answer.clone()));
        assert_eq!(with(0x01, &[]), Err(answer.clone()));
        // An empty CHUNK, an ACCEPT of 0 or over CHUNK_MAX.
        assert_eq!(with(0x05, &[0; 8]), Err(answer.clone()));
        assert_eq!(with(0x03, &[0, 0, 0, 0]), Err(answer.clone()));
        assert_eq!(with(0x03, &983_041u32.to_be_bytes()), Err(answer.clone()));
        // Code 0; a reason over 512 bytes; bad UTF-8 in a session.
        assert_eq!(with(0x08, &[0, 0, 0, 0]), Err(answer.clone()));
        let mut long = vec![0, 3, 0x02, 0x01];
        long.extend_from_slice(&[b'a'; 513]);
        assert_eq!(with(0x08, &long), Err(answer.clone()));
        let mut bad = vec![1, 0xff];
        bad.extend_from_slice(&[0, 1, b'x', 0, 0, 0, 1]);
        assert_eq!(with(0x01, &bad), Err(answer.clone()));
        // An empty session or request.
        assert_eq!(
            with(0x01, &[0, 0, 1, b'x', 0, 0, 0, 1]),
            Err(answer.clone())
        );
        assert_eq!(with(0x01, &[1, b's', 0, 0, 0, 0, 0, 1]), Err(answer));
    }

    #[test]
    fn long_strings_are_cut_when_encoded() {
        let m = BlobMessage::new(
            TransferId(1),
            ns("git"),
            Body::refuse(Code::FAILED, "é".repeat(400)),
        );
        let back = BlobMessage::decode(&m.encode()).unwrap();
        let Body::Refuse { reason, .. } = back.body else {
            panic!()
        };
        assert_eq!(reason.len(), 512);
        assert_eq!(m.encoded_len(), m.encode().len());
    }

    /// A small xorshift, so the property test needs no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn random_bytes_never_panic_decode() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for i in 0..200_000 {
            let len = (rng.next() % 200) as usize;
            let mut b: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            // Half of them with a plausible header, so bodies get exercised.
            if i % 2 == 0 && b.len() > 12 {
                b[0] = (rng.next() % 10) as u8;
                b[1..5].copy_from_slice(&[0, 0, 0, 1]);
                b[5] = 5;
                b[6..11].copy_from_slice(b"files");
            }
            let _ = BlobMessage::decode(&b);
        }
    }
}
