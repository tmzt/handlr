//! The speaker's end, with a flush the session can ask for at any time.
//!
//! libvoicexn's TTS worker flushes the ring itself only when a `Stop`
//! reaches it while it is still making an utterance. Phonon runs ahead of
//! the speaker, so a reply is often fully made, with up to the ring's two
//! seconds still to play, when `StopSpeaking` comes. [`Playback`] drains the
//! ring on the audio thread whenever the session's flush count moves.
//!
//! The ding (plan 20261024_02) is a chime made here, at the speaker's rate,
//! once: whenever the session's chime count moves, it plays from its start,
//! mixed over what the ring gives.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use libvoicexn::playback::Consumer;

use crate::Format;

/// Samples drained per pass; preallocated, so a flush does not allocate.
const SCRATCH: usize = 4096;

/// The chime's length, in seconds.
const CHIME_SECONDS: f32 = 0.35;

/// The chime at `rate`, mono: two bell partials (C6 and G6) that start
/// over 5 ms and die away, at a quarter of full scale.
pub fn chime(rate: u32) -> Vec<f32> {
    let rate = rate.max(1) as f32;
    let len = (CHIME_SECONDS * rate) as usize;
    (0..len)
        .map(|i| {
            let t = i as f32 / rate;
            let attack = (t / 0.005).min(1.0);
            let decay = (-t / 0.09).exp();
            let tau = std::f32::consts::TAU;
            let tone = (tau * 1046.5 * t).sin() + 0.5 * (tau * 1568.0 * t).sin();
            0.25 * attack * decay * tone / 1.5
        })
        .collect()
}

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
    /// The chime, mono, and the speaker's channels it is spread over.
    chime: Vec<f32>,
    channels: usize,
    /// The session's chime count, as last seen, and where the chime is.
    chimes: Arc<AtomicU32>,
    chimes_seen: u32,
    chime_at: Option<usize>,
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
            chime: Vec::new(),
            channels: 1,
            chimes: Arc::new(AtomicU32::new(0)),
            chimes_seen: 0,
            chime_at: None,
        }
    }

    /// Plays the chime at `output`'s rate, over its channels, whenever
    /// `chimes` moves.
    pub(crate) fn with_chime(mut self, output: Format, chimes: Arc<AtomicU32>) -> Self {
        self.chime = chime(output.rate);
        self.channels = usize::from(output.channels.max(1));
        self.chimes_seen = chimes.load(Ordering::Acquire);
        self.chimes = chimes;
        self
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
        self.mix_chime(out);
    }

    /// Adds the chime over `out`, from its start if the count moved.
    fn mix_chime(&mut self, out: &mut [f32]) {
        let want = self.chimes.load(Ordering::Acquire);
        if want != self.chimes_seen {
            self.chimes_seen = want;
            self.chime_at = Some(0);
        }
        let Some(mut at) = self.chime_at else {
            return;
        };
        for frame in out.chunks_mut(self.channels) {
            let Some(&sample) = self.chime.get(at) else {
                break;
            };
            for s in frame {
                *s = (*s + sample).clamp(-1.0, 1.0);
            }
            at += 1;
        }
        self.chime_at = (at < self.chime.len()).then_some(at);
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

    #[test]
    fn the_chime_is_short_and_quiet() {
        let c = chime(48_000);
        assert_eq!(c.len(), (CHIME_SECONDS * 48_000.0) as usize);
        assert!(c.iter().all(|s| s.abs() <= 0.25));
        assert!(c.iter().any(|s| s.abs() > 0.1), "it is heard");
        assert_eq!(c[0], 0.0, "it starts from silence");
    }

    #[test]
    fn a_chime_plays_once_on_both_channels_over_the_ring() {
        let (mut tx, rx) = ring(20_000, 2, TTS_RATE);
        let chimes = Arc::new(AtomicU32::new(0));
        let output = Format {
            rate: 48_000,
            channels: 2,
        };
        let mut playback = Playback::new(rx, 20_000, Arc::new(AtomicU32::new(0)))
            .with_chime(output, chimes.clone());
        // Nothing asked: silence.
        let mut out = vec![0.0; 2000];
        playback.fill(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));

        chimes.fetch_add(1, Ordering::Release);
        let reference = chime(48_000);
        let mut played = Vec::new();
        loop {
            let mut out = vec![0.0; 2000];
            playback.fill(&mut out);
            for frame in out.chunks(2) {
                assert_eq!(frame[0], frame[1], "the same on both channels");
                played.push(frame[0]);
            }
            if played.len() > reference.len() + 1000 {
                break;
            }
        }
        assert_eq!(&played[..reference.len()], &reference[..]);
        assert!(played[reference.len()..].iter().all(|&s| s == 0.0), "once");

        // Over speech, it is added to it.
        assert!(tx.push(&[0.1; 4000], || false));
        chimes.fetch_add(1, Ordering::Release);
        let mut out = vec![0.0; 200];
        playback.fill(&mut out);
        let speech: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        assert!(speech
            .iter()
            .zip(&reference)
            .any(|(s, c)| *c != 0.0 && *s != *c));
    }
}
