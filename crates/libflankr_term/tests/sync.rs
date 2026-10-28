//! Plan 20261027_07, step 1's Done-when: 1000 random frame sequences
//! through `Mirror` and `Screen`, with frames and acks dropped, delayed and
//! reordered, end with the screen equal to the newest grid once the last
//! frame is delivered.

use libflankr_term::{Frame, Grid, Mirror, Screen, parse_ansi};

/// xorshift64*: small, deterministic, good enough for a test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

const WORDS: &[&str] = &[
    "flankr",
    "herdr",
    "\t",
    "  ",
    "é",
    "▶",
    "$ ls",
    "ok",
    "x",
    "─────",
];
const STYLES: &[&str] = &[
    "\x1b[0m",
    "\x1b[1m",
    "\x1b[2;3m",
    "\x1b[4m",
    "\x1b[7m",
    "\x1b[9m",
    "\x1b[22m",
    "\x1b[31m",
    "\x1b[42m",
    "\x1b[38;5;208m",
    "\x1b[48;2;10;20;30m",
    "\x1b[38:2::1:2:3m",
    "\x1b[39;49m",
    "\x1b]0;title\x07",
    "\x1b[2K",
];

fn random_line(rng: &mut Rng) -> String {
    let mut line = String::new();
    for _ in 0..rng.below(8) {
        if rng.chance(40) {
            line.push_str(STYLES[rng.below(STYLES.len())]);
        }
        line.push_str(WORDS[rng.below(WORDS.len())]);
    }
    line
}

/// The next screen: mostly the last one with a row or two changed,
/// sometimes scrolled, resized or the same.
fn next_lines(rng: &mut Rng, lines: &mut Vec<String>) {
    match rng.below(20) {
        0 => {
            let rows = 1 + rng.below(16);
            *lines = (0..rows).map(|_| random_line(rng)).collect();
        }
        1 | 2 => {}
        3 if !lines.is_empty() => {
            lines.remove(0);
            lines.push(random_line(rng));
        }
        _ => {
            if lines.is_empty() {
                lines.push(random_line(rng));
            }
            for _ in 0..1 + rng.below(2) {
                let i = rng.below(lines.len());
                lines[i] = random_line(rng);
            }
        }
    }
}

fn deliver(screen: &mut Screen, bytes: &[u8], acks: &mut Vec<u32>, rng: &mut Rng) {
    let frame = Frame::decode(bytes).expect("a mirror's frame decodes");
    if screen.apply(&frame) && !rng.chance(30) {
        acks.push(screen.seq());
    }
}

fn run_sequence(seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut mirror = Mirror::new(seed as u32);
    let mut screen = Screen::default();
    let mut lines: Vec<String> = (0..1 + rng.below(16))
        .map(|_| random_line(&mut rng))
        .collect();
    let mut newest = Grid::default();
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut acks: Vec<u32> = Vec::new();

    for _ in 0..10 + rng.below(60) {
        match rng.below(10) {
            0..=3 => {
                next_lines(&mut rng, &mut lines);
                newest = parse_ansi(&lines.join("\n"));
                if let Some(frame) = mirror.update(newest.clone()) {
                    assert_eq!(frame.terminal_id, seed as u32);
                    if !rng.chance(25) {
                        frames.push(frame.encode());
                    }
                }
            }
            4..=6 if !frames.is_empty() => {
                // Any frame in flight: reordered.
                let bytes = frames.swap_remove(rng.below(frames.len()));
                deliver(&mut screen, &bytes, &mut acks, &mut rng);
            }
            7 | 8 if !acks.is_empty() => {
                let seq = acks.swap_remove(rng.below(acks.len()));
                mirror.ack(seq);
            }
            9 => {
                if rng.chance(10) {
                    mirror.reset();
                } else if let Some(frame) = mirror.retransmit()
                    && !rng.chance(25)
                {
                    frames.push(frame.encode());
                }
            }
            _ => {}
        }
    }

    // The last frame: the newest grid, sent again if it was unchanged.
    let last = mirror
        .update(newest.clone())
        .or_else(|| mirror.retransmit());
    if let Some(last) = last {
        assert!(
            screen.apply(&Frame::decode(&last.encode()).unwrap()),
            "seed {seed}: the last frame applies"
        );
    }
    assert_eq!(screen.grid(), &newest, "seed {seed}");
    assert_eq!(screen.seq(), mirror.seq(), "seed {seed}");

    // Stale frames still in flight change nothing.
    for bytes in frames {
        let frame = Frame::decode(&bytes).unwrap();
        assert!(!screen.apply(&frame), "seed {seed}: a stale frame applied");
    }
    assert_eq!(screen.grid(), &newest, "seed {seed}");
}

#[test]
fn a_thousand_random_sequences_converge() {
    for seed in 1..=1000 {
        run_sequence(seed);
    }
}
