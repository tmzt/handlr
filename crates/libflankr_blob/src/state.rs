//! One transfer's two sides (decisions 4.2-4.5): fed messages and the
//! clock, they say what to send or do. No I/O.

use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::{
    Body, CHUNK_MAX, Code, DONE_TIMEOUT, IDLE_TIMEOUT, OFFER_TIMEOUT, Offer, TRANSFER_MAX_TIME,
};

/// What a side does after a message or a tick.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Step {
    /// Nothing to send.
    Continue,
    /// [`Receiving`] after a REQUEST: the OFFER arrived. The caller decides
    /// and answers with [`Receiving::accept`] or [`Receiving::refuse`].
    Offered(Offer),
    /// [`Sending`]: ACCEPT arrived; send CHUNKs of at most `chunk_max`.
    Accepted { chunk_max: u32 },
    /// [`Receiving`]: the CHUNK is in order, within the size, and hashed;
    /// store its data.
    Store,
    /// [`Receiving`]: END arrived and the size and SHA-256 match. Finish
    /// storing and answer with [`Receiving::done`].
    Complete,
    /// [`Sending`]: DONE arrived, with its namespace body. The transfer is
    /// over.
    Done(Vec<u8>),
    /// Send this CANCEL; the transfer is over (a protocol error, a size or
    /// hash mismatch, or a timeout).
    Abort(Body),
    /// The peer REFUSEd or CANCELled; the transfer is over.
    Ended { code: Code, reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RState {
    /// REQUEST sent, waiting for OFFER (or REFUSE).
    AwaitOffer,
    /// OFFER in hand; the caller decides.
    Deciding,
    /// ACCEPTed; CHUNKs and END come.
    Receiving,
    /// END checked; waiting for the caller's DONE.
    Verified,
    Over,
}

/// The receiving side of one transfer.
#[derive(Clone, Debug)]
pub struct Receiving {
    state: RState,
    offer: Option<Offer>,
    received: u64,
    hasher: Sha256,
    chunk_max: u32,
    /// When the OFFER was sent or arrived, for `TRANSFER_MAX_TIME`.
    offered_at: Option<Instant>,
    deadline: Option<Instant>,
}

impl Receiving {
    fn with(state: RState, offer: Option<Offer>, now: Instant) -> Receiving {
        Receiving {
            state,
            offered_at: offer.as_ref().map(|_| now),
            offer,
            received: 0,
            hasher: Sha256::new(),
            chunk_max: CHUNK_MAX,
            deadline: Some(now + OFFER_TIMEOUT),
        }
    }

    /// After sending a REQUEST: waits `OFFER_TIMEOUT` for the OFFER.
    pub fn requested(now: Instant) -> Receiving {
        Self::with(RState::AwaitOffer, None, now)
    }

    /// An OFFER arrived first (`images`): the caller decides, within
    /// `OFFER_TIMEOUT`.
    pub fn offered(offer: Offer, now: Instant) -> Receiving {
        Self::with(RState::Deciding, Some(offer), now)
    }

    /// The OFFER, once there is one.
    pub fn offer(&self) -> Option<&Offer> {
        self.offer.as_ref()
    }

    /// Bytes received and hashed so far.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Whether the transfer is over.
    pub fn is_over(&self) -> bool {
        self.state == RState::Over
    }

    /// Whether the OFFER has been accepted and bytes may come.
    pub fn is_receiving(&self) -> bool {
        self.state == RState::Receiving
    }

    /// ACCEPT the OFFER, CHUNKs at most `chunk_max` (kept to
    /// `1..=CHUNK_MAX`). `None` unless an OFFER is waiting for a decision.
    pub fn accept(&mut self, chunk_max: u32, now: Instant) -> Option<Body> {
        if self.state != RState::Deciding {
            return None;
        }
        self.chunk_max = chunk_max.clamp(1, CHUNK_MAX);
        self.state = RState::Receiving;
        self.deadline = Some(now + IDLE_TIMEOUT);
        Some(Body::Accept {
            chunk_max: self.chunk_max,
        })
    }

    /// REFUSE the OFFER; the transfer is over.
    pub fn refuse(&mut self, code: Code, reason: impl Into<String>) -> Body {
        self.state = RState::Over;
        Body::refuse(code, reason)
    }

    /// CANCEL the transfer; it is over.
    pub fn cancel(&mut self, code: Code, reason: impl Into<String>) -> Body {
        self.state = RState::Over;
        Body::cancel(code, reason)
    }

    /// One CHUNK's data: in order (`offset` equals the bytes so far), at
    /// most `chunk_max`, not past OFFER's size; else
    /// [`Code::PROTOCOL`]. Fed to the hasher.
    pub fn chunk(&mut self, offset: u64, data: &[u8], now: Instant) -> Result<(), Code> {
        let size = self.offer.as_ref().map_or(0, |o| o.size);
        if self.state != RState::Receiving
            || offset != self.received
            || data.is_empty()
            || data.len() > self.chunk_max as usize
            || self.received + data.len() as u64 > size
        {
            return Err(Code::PROTOCOL);
        }
        self.hasher.update(data);
        self.received += data.len() as u64;
        self.deadline = Some(now + IDLE_TIMEOUT);
        Ok(())
    }

    /// END: `size` and the bytes received must equal OFFER's size
    /// ([`Code::SIZE_MISMATCH`]), then the SHA-256 OFFER's
    /// ([`Code::HASH_MISMATCH`]). On success, the hash; the caller then
    /// answers [`done`](Self::done).
    pub fn finish(&mut self, size: u64) -> Result<[u8; 32], Code> {
        let Some(offer) = self
            .offer
            .as_ref()
            .filter(|_| self.state == RState::Receiving)
        else {
            return Err(Code::PROTOCOL);
        };
        if size != offer.size || self.received != offer.size {
            return Err(Code::SIZE_MISMATCH);
        }
        let hash: [u8; 32] = self.hasher.clone().finalize().into();
        if hash != offer.sha256 {
            return Err(Code::HASH_MISMATCH);
        }
        self.state = RState::Verified;
        self.deadline = None;
        Ok(hash)
    }

    /// DONE, with the namespace's body; the transfer is over.
    pub fn done(&mut self, body: Vec<u8>) -> Body {
        self.state = RState::Over;
        Body::Done { body }
    }

    fn abort(&mut self, code: Code, reason: &str) -> Step {
        self.state = RState::Over;
        Step::Abort(Body::cancel(code, reason))
    }

    /// A message for this transfer.
    pub fn on_message(&mut self, body: &Body, now: Instant) -> Step {
        match (self.state, body) {
            (RState::Over, _) => Step::Continue,
            (_, Body::Cancel { code, reason }) => {
                self.state = RState::Over;
                Step::Ended {
                    code: *code,
                    reason: reason.clone(),
                }
            }
            (RState::AwaitOffer, Body::Refuse { code, reason }) => {
                self.state = RState::Over;
                Step::Ended {
                    code: *code,
                    reason: reason.clone(),
                }
            }
            (RState::AwaitOffer, Body::Offer(offer)) => {
                self.offer = Some(offer.clone());
                self.offered_at = Some(now);
                self.state = RState::Deciding;
                self.deadline = Some(now + OFFER_TIMEOUT);
                Step::Offered(offer.clone())
            }
            (RState::Receiving, Body::Chunk { offset, data }) => {
                match self.chunk(*offset, data, now) {
                    Ok(()) => Step::Store,
                    Err(code) => self.abort(code, "chunk out of order or too large"),
                }
            }
            (RState::Receiving, Body::End { size }) => match self.finish(*size) {
                Ok(_) => Step::Complete,
                Err(code) => self.abort(code, "the bytes do not match the offer"),
            },
            _ => self.abort(Code::PROTOCOL, "unexpected message"),
        }
    }

    /// The clock: a timeout of decision 4.4 aborts with
    /// [`Code::TIMEOUT`].
    pub fn on_tick(&mut self, now: Instant) -> Step {
        if self.state == RState::Over {
            return Step::Continue;
        }
        let max = self
            .offered_at
            .filter(|_| self.state == RState::Receiving)
            .is_some_and(|at| now >= at + TRANSFER_MAX_TIME);
        if max || self.deadline.is_some_and(|d| now >= d) {
            return self.abort(Code::TIMEOUT, "timed out");
        }
        Step::Continue
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SState {
    /// OFFER sent, waiting for ACCEPT or REFUSE.
    AwaitAccept,
    /// Sending CHUNKs.
    Sending,
    /// END sent, waiting for DONE or CANCEL.
    AwaitDone,
    Over,
}

/// The sending side of one transfer.
#[derive(Clone, Debug)]
pub struct Sending {
    state: SState,
    size: u64,
    offset: u64,
    chunk_max: u32,
    offered_at: Instant,
    deadline: Option<Instant>,
}

impl Sending {
    /// After sending `offer`: waits `OFFER_TIMEOUT` for ACCEPT.
    pub fn new(offer: &Offer, now: Instant) -> Sending {
        Sending {
            state: SState::AwaitAccept,
            size: offer.size,
            offset: 0,
            chunk_max: 0,
            offered_at: now,
            deadline: Some(now + OFFER_TIMEOUT),
        }
    }

    /// Bytes handed out by [`next_chunk`](Self::next_chunk) so far.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn is_over(&self) -> bool {
        self.state == SState::Over
    }

    /// Whether ACCEPT has come and bytes are still to send.
    pub fn is_sending(&self) -> bool {
        self.state == SState::Sending
    }

    /// The next CHUNK to send, of at most `len` bytes (and the peer's
    /// `chunk_max`): its offset and length, the offset then advanced.
    /// `None` before ACCEPT or once every byte is handed out; then send
    /// [`end`](Self::end).
    pub fn next_chunk(&mut self, len: usize) -> Option<(u64, usize)> {
        if self.state != SState::Sending {
            return None;
        }
        let left = self.size - self.offset;
        let n = (len as u64).min(u64::from(self.chunk_max)).min(left) as usize;
        if n == 0 {
            return None;
        }
        let at = self.offset;
        self.offset += n as u64;
        Some((at, n))
    }

    /// END, once every byte is handed out; then waits `DONE_TIMEOUT` for
    /// DONE. `None` otherwise.
    pub fn end(&mut self, now: Instant) -> Option<Body> {
        if self.state != SState::Sending || self.offset != self.size {
            return None;
        }
        self.state = SState::AwaitDone;
        self.deadline = Some(now + DONE_TIMEOUT);
        Some(Body::End { size: self.size })
    }

    /// CANCEL the transfer; it is over.
    pub fn cancel(&mut self, code: Code, reason: impl Into<String>) -> Body {
        self.state = SState::Over;
        Body::cancel(code, reason)
    }

    fn abort(&mut self, code: Code, reason: &str) -> Step {
        self.state = SState::Over;
        Step::Abort(Body::cancel(code, reason))
    }

    /// A message for this transfer.
    pub fn on_message(&mut self, body: &Body, _now: Instant) -> Step {
        match (self.state, body) {
            (SState::Over, _) => Step::Continue,
            (_, Body::Cancel { code, reason }) => {
                self.state = SState::Over;
                Step::Ended {
                    code: *code,
                    reason: reason.clone(),
                }
            }
            (SState::AwaitAccept, Body::Refuse { code, reason }) => {
                self.state = SState::Over;
                Step::Ended {
                    code: *code,
                    reason: reason.clone(),
                }
            }
            (SState::AwaitAccept, Body::Accept { chunk_max }) => {
                self.chunk_max = (*chunk_max).clamp(1, CHUNK_MAX);
                self.state = SState::Sending;
                self.deadline = None;
                Step::Accepted {
                    chunk_max: self.chunk_max,
                }
            }
            (SState::AwaitDone, Body::Done { body }) => {
                self.state = SState::Over;
                Step::Done(body.clone())
            }
            _ => self.abort(Code::PROTOCOL, "unexpected message"),
        }
    }

    /// The clock: a timeout of decision 4.4 aborts with
    /// [`Code::TIMEOUT`].
    pub fn on_tick(&mut self, now: Instant) -> Step {
        if self.state == SState::Over {
            return Step::Continue;
        }
        let max = self.state == SState::Sending && now >= self.offered_at + TRANSFER_MAX_TIME;
        if max || self.deadline.is_some_and(|d| now >= d) {
            return self.abort(Code::TIMEOUT, "timed out");
        }
        Step::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn offer_of(bytes: &[u8]) -> Offer {
        Offer::for_bytes("w1:1", "a.png", "image/png", bytes, 0)
    }

    /// Runs a whole transfer between the two machines.
    fn transfer(bytes: &[u8], chunk: usize) -> (Receiving, Sending) {
        let t = Instant::now();
        let offer = offer_of(bytes);
        let mut s = Sending::new(&offer, t);
        let mut r = Receiving::offered(offer, t);
        let accept = r.accept(chunk as u32, t).unwrap();
        assert_eq!(
            s.on_message(&accept, t),
            Step::Accepted {
                chunk_max: chunk as u32
            }
        );
        while let Some((at, n)) = s.next_chunk(usize::MAX) {
            let body = Body::Chunk {
                offset: at,
                data: bytes[at as usize..at as usize + n].to_vec(),
            };
            assert_eq!(r.on_message(&body, t), Step::Store);
        }
        let end = s.end(t).unwrap();
        assert_eq!(r.on_message(&end, t), Step::Complete);
        let done = r.done(vec![7; 8]);
        assert_eq!(s.on_message(&done, t), Step::Done(vec![7; 8]));
        (r, s)
    }

    #[test]
    fn a_whole_transfer_runs() {
        let bytes: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let (r, s) = transfer(&bytes, 300);
        assert!(r.is_over() && s.is_over());
        assert_eq!(r.received(), 1000);
        // An empty file: no CHUNK, END at once.
        let (r, _) = transfer(&[], 300);
        assert_eq!(r.received(), 0);
    }

    #[test]
    fn a_request_is_answered_by_an_offer_or_a_refuse() {
        let t = Instant::now();
        let mut r = Receiving::requested(t);
        let offer = offer_of(b"0123456789abcdef");
        assert_eq!(
            r.on_message(&Body::Offer(offer.clone()), t),
            Step::Offered(offer)
        );
        assert!(r.accept(1 << 30, t).is_some());
        assert!(r.accept(10, t).is_none(), "accepted once");
        let mut r = Receiving::requested(t);
        assert_eq!(
            r.on_message(&Body::refuse(Code::FILES_OFF, "off"), t),
            Step::Ended {
                code: Code::FILES_OFF,
                reason: "off".into()
            }
        );
        assert!(r.is_over());
        assert_eq!(r.on_message(&Body::End { size: 0 }, t), Step::Continue);
    }

    #[test]
    fn chunks_out_of_order_past_the_size_or_over_chunk_max_are_protocol() {
        let t = Instant::now();
        let bytes = [1u8; 100];
        let start = || {
            let mut r = Receiving::offered(offer_of(&bytes), t);
            r.accept(40, t).unwrap();
            r
        };
        let chunk = |offset: u64, n: usize| Body::Chunk {
            offset,
            data: vec![1; n],
        };
        let abort = Step::Abort(Body::cancel(
            Code::PROTOCOL,
            "chunk out of order or too large",
        ));
        let mut r = start();
        assert_eq!(r.on_message(&chunk(10, 10), t), abort);
        let mut r = start();
        assert_eq!(r.on_message(&chunk(0, 41), t), abort, "over chunk_max");
        let mut r = start();
        for at in [0, 40] {
            assert_eq!(r.on_message(&chunk(at, 40), t), Step::Store);
        }
        assert_eq!(r.on_message(&chunk(80, 21), t), abort, "past size");
        // A CHUNK before ACCEPT.
        let mut r = Receiving::offered(offer_of(&bytes), t);
        assert!(matches!(r.on_message(&chunk(0, 1), t), Step::Abort(_)));
    }

    #[test]
    fn a_short_end_is_size_mismatch_and_a_flipped_byte_hash_mismatch() {
        let t = Instant::now();
        let bytes = [3u8; 64];
        let mut r = Receiving::offered(offer_of(&bytes), t);
        r.accept(64, t).unwrap();
        r.chunk(0, &bytes[..32], t).unwrap();
        assert_eq!(r.finish(32), Err(Code::SIZE_MISMATCH));
        let mut r = Receiving::offered(offer_of(&bytes), t);
        r.accept(64, t).unwrap();
        r.chunk(0, &bytes[..32], t).unwrap();
        assert_eq!(r.finish(64), Err(Code::SIZE_MISMATCH), "bytes short");
        let mut r = Receiving::offered(offer_of(&bytes), t);
        r.accept(64, t).unwrap();
        let mut flipped = bytes;
        flipped[10] ^= 1;
        r.chunk(0, &flipped, t).unwrap();
        assert!(matches!(
            r.on_message(&Body::End { size: 64 }, t),
            Step::Abort(Body::Cancel {
                code: Code::HASH_MISMATCH,
                ..
            })
        ));
    }

    #[test]
    fn the_timeouts_fire_from_on_tick() {
        let t = Instant::now();
        let s = Duration::from_secs;
        let timeout = |step: Step| {
            matches!(
                step,
                Step::Abort(Body::Cancel {
                    code: Code::TIMEOUT,
                    ..
                })
            )
        };
        // OFFER_TIMEOUT, both sides.
        let mut r = Receiving::requested(t);
        assert_eq!(r.on_tick(t + s(29)), Step::Continue);
        assert!(timeout(r.on_tick(t + s(30))));
        let offer = offer_of(&[0; 32]);
        let mut snd = Sending::new(&offer, t);
        assert!(timeout(snd.on_tick(t + s(30))));
        // IDLE_TIMEOUT, refreshed by each CHUNK.
        let mut r = Receiving::offered(offer.clone(), t);
        r.accept(8, t).unwrap();
        r.chunk(0, &[0; 8], t + s(50)).unwrap();
        assert_eq!(r.on_tick(t + s(109)), Step::Continue);
        assert!(timeout(r.on_tick(t + s(110))));
        // TRANSFER_MAX_TIME, however busy.
        let mut r = Receiving::offered(offer_of(&[0; 4096]), t);
        r.accept(1, t).unwrap();
        for (at, sec) in (0..600).step_by(30).enumerate() {
            r.chunk(at as u64, &[0], t + s(sec)).unwrap();
            assert_eq!(r.on_tick(t + s(sec)), Step::Continue);
        }
        assert!(timeout(r.on_tick(t + s(600))));
        // DONE_TIMEOUT.
        let mut snd = Sending::new(&offer, t);
        snd.on_message(&Body::Accept { chunk_max: 64 }, t);
        while snd.next_chunk(64).is_some() {}
        snd.end(t + s(1)).unwrap();
        assert_eq!(snd.on_tick(t + s(30)), Step::Continue);
        assert!(timeout(snd.on_tick(t + s(31))));
        // Over is quiet.
        assert_eq!(snd.on_tick(t + s(1000)), Step::Continue);
    }

    #[test]
    fn the_sender_keeps_to_chunk_max_and_refuses_out_of_order_messages() {
        let t = Instant::now();
        let offer = offer_of(&[0; 100]);
        let mut s = Sending::new(&offer, t);
        assert_eq!(s.next_chunk(10), None, "not before ACCEPT");
        assert!(s.end(t).is_none());
        s.on_message(&Body::Accept { chunk_max: 30 }, t);
        assert_eq!(s.next_chunk(1000), Some((0, 30)));
        assert_eq!(s.next_chunk(20), Some((30, 20)));
        assert!(s.end(t).is_none(), "not all sent");
        assert_eq!(s.next_chunk(1000), Some((50, 30)));
        assert_eq!(s.next_chunk(1000), Some((80, 20)));
        assert_eq!(s.next_chunk(1000), None);
        assert!(matches!(
            s.on_message(&Body::Done { body: vec![] }, t),
            Step::Abort(_)
        ));
        let mut s = Sending::new(&offer, t);
        assert_eq!(
            s.on_message(&Body::refuse(Code::BUSY, ""), t),
            Step::Ended {
                code: Code::BUSY,
                reason: String::new()
            }
        );
    }
}
