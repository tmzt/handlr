//! The drawer's Braille spinner (plan 20261028_03, 3.5 item 18): ten
//! frames, 80 ms each, from one clock, so every spinning row shows the
//! same frame and the list does not shimmer. The glyphs are drawn from the
//! terminal face's symbol page, which has them (2.2 item 5).

use std::time::Duration;

/// The frames, in order: U+280B, 2819, 2839, 2838, 283C, 2834, 2826,
/// 2827, 2807, 280F.
pub const FRAMES: [char; 10] = [
    '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280f}',
];

/// How long each frame shows.
pub const FRAME: Duration = Duration::from_millis(80);

/// The frame's index at `t` on the app's clock.
pub fn frame_at(t: Duration) -> usize {
    ((t.as_millis() / FRAME.as_millis()) % FRAMES.len() as u128) as usize
}

/// The glyph at `t`.
pub fn glyph_at(t: Duration) -> char {
    FRAMES[frame_at(t)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_frames_of_80_ms_round_again() {
        let ms = Duration::from_millis;
        assert_eq!(glyph_at(ms(0)), '\u{280b}');
        assert_eq!(glyph_at(ms(79)), '\u{280b}');
        assert_eq!(glyph_at(ms(80)), '\u{2819}');
        assert_eq!(glyph_at(ms(80 * 9)), '\u{280f}');
        assert_eq!(glyph_at(ms(799)), '\u{280f}');
        assert_eq!(glyph_at(ms(800)), '\u{280b}', "round again");
        assert_eq!(frame_at(ms(80 * 23 + 5)), 3);
        // Every one a Braille pattern, none repeated.
        for (i, c) in FRAMES.iter().enumerate() {
            assert!(('\u{2800}'..='\u{28ff}').contains(c));
            assert!(!FRAMES[..i].contains(c));
        }
    }
}
