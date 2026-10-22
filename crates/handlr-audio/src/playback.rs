//! The speaker's end, with a flush the session can ask for at any time.
//!
//! libvoicexn's TTS worker flushes the ring itself only when a `Stop`
//! reaches it while it is still making an utterance. Phonon runs ahead of
//! the speaker, so a reply is often fully made, with up to the ring's two
//! seconds still to play, when `StopSpeaking` comes. [`Playback`] drains the
//! ring on the audio thread whenever the session's flush count moves.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use libvoicexn::playback::Consumer;

/// Samples drained per pass; preallocated, so a flush does not allocate.
const SCRATCH: usize = 4096;

/// The output callback's end: [`fill`](Self::fill) it with the device's
/// interleaved buffer. It never blocks or allocates.
pub struct Playback {
    consumer: Consumer,
    playing: Arc<AtomicBool>,
    flush: Arc<AtomicU32>,
    seen: u32,
    scratch: Vec<f32>,
    /// Passes that empty a full ring, and one more.
    passes: usize,
}

impl Playback {
    /// Wraps `consumer`, whose ring holds `capacity` samples, and the
    /// session's flush count.
    pub(crate) fn new(consumer: Consumer, capacity: usize, flush: Arc<AtomicU32>) -> Self {
        let seen = flush.load(Ordering::Acquire);
        Self {
            playing: consumer.speaking(),
            consumer,
            flush,
            seen,
            scratch: vec![0.0; SCRATCH],
            passes: capacity / SCRATCH + 2,
        }
    }

    /// Fills `out` from the ring, zeros where it runs dry, after dropping what was queued if the
    /// session asked for a flush since the last call.
    pub fn fill(&mut self, out: &mut [f32]) {
        let want = self.flush.load(Ordering::Acquire);
        if want != self.seen {
            self.seen = want;
            for _ in 0..self.passes {
                self.consumer.fill(&mut self.scratch);
                if !self.playing.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
        // libvoicexn's `fill` writes only what the ring holds; the rest of a
        // device buffer is not always zeroed for us.
        out.fill(0.0);
        self.consumer.fill(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libvoicexn::playback::{ring, TTS_RATE};

    #[test]
    fn a_flush_drops_what_is_queued_and_stops_playing() {
        let (mut tx, rx) = ring(20_000, 1, TTS_RATE);
        let flush = Arc::new(AtomicU32::new(0));
        let mut playback = Playback::new(rx, 20_000, flush.clone());
        assert!(tx.push(&[0.5; 12_000], || false));
        let mut out = [0.0; 8];
        playback.fill(&mut out);
        assert_eq!(out, [0.5; 8]);
        assert!(playback.playing.load(Ordering::Relaxed));

        flush.fetch_add(1, Ordering::Release);
        let mut out = [9.0; 8];
        playback.fill(&mut out);
        // Nothing left: silence.
        assert_eq!(out, [0.0; 8]);
        assert!(!playback.playing.load(Ordering::Relaxed));

        // New speech after the flush plays (the worker resets the
        // resampler for each utterance).
        tx.reset();
        assert!(tx.push(&[0.25; 100], || false));
        let mut out = [0.0; 4];
        playback.fill(&mut out);
        assert_eq!(out, [0.25; 4]);
    }
}
