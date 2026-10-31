//! The bottom row's buttons and the agent view's as drawn: the square
//! audio controls (volume, send, cancel, record) by glyph, and the rest
//! with their labels; and the glyphs drawn as strokes that the faces lack
//! (the keyboard, the terminal, the list, the speaker, the +).

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{
    AgentHit, CANCEL_GLYPH, CELL_EM, DOT, FlowButton, GEMINI_GLYPH, PLAY_GLYPH, Row, SEND_GLYPH,
    Terminal, button, draw_row, fill, term_h,
};
use crate::settings;
use crate::theme::theme;

/// `glyph` centred in `rect` at `px`, from the terminal face; `fallback`
/// in the context's font without it.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    glyph: &str,
    fallback: &str,
    rect: Rect,
    px: f32,
    color: [f32; 4],
) {
    let row = Row {
        text: if term.is_some() { glyph } else { fallback }.to_string(),
        x: 0.0,
        y: 0.0,
        px,
        color,
        mono: true,
    };
    let pos = [
        rect.x + (rect.w - CELL_EM * px * row.text.chars().count() as f32) * 0.5,
        rect.y + (rect.h - term_h(px)) * 0.5,
    ];
    draw_row(list, ctx, term, &row, pos);
}

/// The general agent button in active voice recognition mode:
/// fixed outer button size, inner pulsing glow ring, and glyph alternating
/// between the Gemini star and dingbats symbols.
fn draw_general_agent_listening(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    rect: Rect,
    bg: [f32; 4],
    s: f32,
) {
    // Outer button keeps the exact same size.
    fill(list, rect, bg, 10.0 * s);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let millis = now.as_millis();

    // 1. Pulsing / growing and shrinking inner ring (oscillates with ~1200ms period).
    let phase = ((millis % 1200) as f32 / 1200.0 * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let ring_pad = (2.0 + (1.0 - phase) * 6.0) * s;
    let ring_rect = Rect::new(
        rect.x + ring_pad,
        rect.y + ring_pad,
        rect.w - 2.0 * ring_pad,
        rect.h - 2.0 * ring_pad,
    );
    let glow_alpha = 0.20 + 0.35 * phase;
    fill(
        list,
        ring_rect,
        [1.0, 1.0, 1.0, glow_alpha],
        (8.0 - phase * 2.0) * s,
    );

    // 2. Glyph alternates with dingbats symbols across animation steps (~300ms each).
    const DINGBATS: &[(&str, &str)] = &[
        (GEMINI_GLYPH, "*"), // ✦ (Black four-pointed star)
        ("\u{2727}", "*"),   // ✧ (White four-pointed star)
        ("\u{2736}", "*"),   // ✶ (Six-pointed black star)
        ("\u{273B}", "*"),   // ✻ (Teardrop-spoked asterisk)
    ];
    let step = ((millis / 300) % (DINGBATS.len() as u128)) as usize;
    let (glyph, fallback) = DINGBATS[step];

    // Inner glyph gently grows and shrinks in sync with the pulse.
    let px = (21.0 + phase * 6.0) * s;
    draw_glyph(list, ctx, term, glyph, fallback, rect, px, theme().fg);
}

/// A keyboard drawn as strokes in `rect`, with no background: an outline
/// and three rows of keys, the last a space bar. The faces have no
/// keyboard character.
pub(crate) fn keyboard_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let w = rect.w * 0.62;
    let h = w * 0.62;
    let x = rect.x + (rect.w - w) * 0.5;
    let y = rect.y + (rect.h - h) * 0.5;
    let t = 1.8 * s;
    for (a, b) in [
        ([x, y], [x + w, y]),
        ([x + w, y], [x + w, y + h]),
        ([x + w, y + h], [x, y + h]),
        ([x, y + h], [x, y]),
    ] {
        list.push_line(a, b, t, color);
    }
    let key = w * 0.09;
    let cols = 6;
    let step = (w - 2.0 * key) / cols as f32;
    for row in 0..2 {
        let ky = y + h * (0.22 + 0.24 * row as f32);
        for c in 0..cols {
            let kx = x + key + step * c as f32 + (step - key) * 0.5;
            fill(list, Rect::new(kx, ky, key, key), color, key * 0.25);
        }
    }
    let bar = Rect::new(x + w * 0.28, y + h * 0.72, w * 0.44, key);
    fill(list, bar, color, key * 0.25);
}

/// The soft keyboard's show/hide toggle in `rect` (plan 20261028_06,
/// decision 8): while the keyboard is `up`, the keyboard glyph moved up
/// 3 pt with a down chevron 8 pt wide and 4 pt high 3 pt under it (hide);
/// while it is down, the keyboard glyph alone (show). Stroke 1.8 pt.
pub(crate) fn ime_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4], up: bool) {
    if !up {
        keyboard_glyph(list, rect, s, color);
        return;
    }
    let moved = Rect::new(rect.x, rect.y - 3.0 * s, rect.w, rect.h);
    keyboard_glyph(list, moved, s, color);
    let b = glyph_box(moved);
    let cx = rect.x + rect.w * 0.5;
    let top = b.y + b.h + 3.0 * s;
    let (half, h) = (4.0 * s, 4.0 * s);
    let t = 1.8 * s;
    list.push_line([cx - half, top], [cx, top + h], t, color);
    list.push_line([cx, top + h], [cx + half, top], t, color);
}

/// A microphone drawn as strokes in `rect`, with no background (the
/// 2026-10-29 15:09 EDT amendment): a capsule 8 × 13 pt, the holder's U
/// of radius 7 pt round its lower half, a stem and a base 10 pt wide.
/// Stroke 1.8 pt. The faces have no microphone character.
pub(crate) fn mic_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let [cx, cy] = rect.center();
    let t = 1.8 * s;
    crate::agent::fill(
        list,
        Rect::new(cx - 4.0 * s, cy - 12.0 * s, 8.0 * s, 13.0 * s),
        color,
        4.0 * s,
    );
    let (r, top, mid) = (7.0 * s, cy - 6.0 * s, cy - 2.0 * s);
    list.push_line([cx - r, top], [cx - r, mid], t, color);
    list.push_line([cx + r, top], [cx + r, mid], t, color);
    const STEPS: usize = 8;
    let at = |i: usize| {
        let a = std::f32::consts::PI * i as f32 / STEPS as f32;
        [cx + r * a.cos(), mid + r * a.sin()]
    };
    for i in 0..STEPS {
        list.push_line(at(i), at(i + 1), t, color);
    }
    let base = cy + 9.0 * s;
    list.push_line([cx, mid + r], [cx, base], t, color);
    list.push_line([cx - 5.0 * s, base], [cx + 5.0 * s, base], t, color);
}

/// A + centred in `rect` (plan 20261028_06, decision 8): two 2 pt strokes
/// 18 pt long.
pub(crate) fn plus_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let [cx, cy] = rect.center();
    let half = 9.0 * s;
    let t = 2.0 * s;
    list.push_line([cx - half, cy], [cx + half, cy], t, color);
    list.push_line([cx, cy - half], [cx, cy + half], t, color);
}

/// The volume button's state (plan 20261028_06, decision 4, and its
/// 2026-10-29 00:26 EDT amendment): muted, and either speak option on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    /// The tap's mute, above both options.
    pub muted: bool,
    /// *Speak coding responses* or *Speak agent (Gemini) responses* on.
    pub on: bool,
}

impl Volume {
    /// The glyph's colour: dim while muted, full strength otherwise.
    pub fn color(self) -> [f32; 4] {
        if self.muted { theme().dim } else { theme().fg }
    }
}

/// The volume glyph's sound waves (plan 20261028_06, decision 4, as its
/// 2026-10-29 00:26 EDT amendment replaces it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waves {
    /// Both options off: no arcs.
    Off,
    /// Muted: no arcs, and a slash through the speaker (the 2026-10-29
    /// 02:06 EDT amendment).
    Muted,
    /// Either option on: two arcs.
    Two,
    /// Something is being said: the arcs in turn, the inner, then both,
    /// then none, at this step (0, 1, 2).
    Speaking(u8),
}

impl Waves {
    /// How long each speaking step lasts, as the listening button's
    /// dingbats.
    pub const STEP_MS: u128 = 300;

    /// The waves for `volume`, while `speaking`, at the time now: none
    /// while muted or with both options off.
    pub fn now(volume: Volume, speaking: bool) -> Self {
        if volume.muted {
            return Waves::Muted;
        }
        if !volume.on {
            return Waves::Off;
        }
        if !speaking {
            return Waves::Two;
        }
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Waves::Speaking(((millis / Self::STEP_MS) % 3) as u8)
    }

    /// How many arcs are drawn: none, the inner one, or both.
    fn arcs(self) -> usize {
        match self {
            Waves::Off | Waves::Muted => 0,
            Waves::Two => 2,
            Waves::Speaking(0) => 1,
            Waves::Speaking(1) => 2,
            Waves::Speaking(_) => 0,
        }
    }
}

/// A speaker drawn as strokes in `rect`'s glyph box `b` (plan
/// 20261028_06, decision 4), stroke 1.8 pt: a body from 0.06 to 0.24 of
/// the box's width across its middle ± 0.22 of its height, a cone to 0.46
/// at ± 0.5 and its mouth; with `waves`, arcs centred on the mouth of
/// radii 0.22 and 0.40 of the width from -50° to +50°, six segments each;
/// [`Waves::Off`], the plain speaker (the 2026-10-29 00:26 EDT amendment
/// drops the ✕). No atlas glyph: libmsdf's Material subset has no
/// `volume_up`, and 🔊 is not in the emoji set.
pub(crate) fn volume_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4], waves: Waves) {
    let b = glyph_box(rect);
    let t = 1.8 * s;
    let cy = b.y + b.h * 0.5;
    let x = |f: f32| b.x + f * b.w;
    let y = |f: f32| cy + f * b.h;
    // The body.
    let (l, r, top, bottom) = (x(0.06), x(0.24), y(-0.22), y(0.22));
    for (a, c) in [
        ([l, top], [r, top]),
        ([r, top], [r, bottom]),
        ([r, bottom], [l, bottom]),
        ([l, bottom], [l, top]),
    ] {
        list.push_line(a, c, t, color);
    }
    // The cone and its mouth.
    let mouth = x(0.46);
    list.push_line([r, top], [mouth, y(-0.5)], t, color);
    list.push_line([r, bottom], [mouth, y(0.5)], t, color);
    list.push_line([mouth, y(-0.5)], [mouth, y(0.5)], t, color);
    for radius in [0.22, 0.40].into_iter().take(waves.arcs()) {
        let rad = radius * b.w;
        let point = |deg: f32| {
            let a = deg.to_radians();
            [mouth + rad * a.cos(), cy + rad * a.sin()]
        };
        const SEGMENTS: usize = 6;
        for i in 0..SEGMENTS {
            let a0 = -50.0 + 100.0 * i as f32 / SEGMENTS as f32;
            let a1 = -50.0 + 100.0 * (i + 1) as f32 / SEGMENTS as f32;
            list.push_line(point(a0), point(a1), t, color);
        }
    }
    // Muted: a slash from the box's top left to its bottom right.
    if waves == Waves::Muted {
        list.push_line([x(0.0), y(-0.5)], [x(0.62), y(0.5)], t, color);
    }
}

/// The glyphs' box in `rect`: the keyboard glyph's width, and its height.
fn glyph_box(rect: Rect) -> Rect {
    let w = rect.w * 0.62;
    let h = w * 0.62;
    Rect::new(
        rect.x + (rect.w - w) * 0.5,
        rect.y + (rect.h - h) * 0.5,
        w,
        h,
    )
}

/// A terminal drawn as strokes in `rect`, with no background (plan
/// 20261027_07, decision 7): a rounded frame with a prompt, `>_`. The
/// faces have no terminal character.
pub(crate) fn terminal_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let b = glyph_box(rect);
    let t = 1.8 * s;
    let r = 3.0 * s;
    // The frame: four sides, their corners cut by the radius, and the
    // corners as short diagonals.
    let (x0, y0, x1, y1) = (b.x, b.y, b.x + b.w, b.y + b.h);
    for (a, c) in [
        ([x0 + r, y0], [x1 - r, y0]),
        ([x1, y0 + r], [x1, y1 - r]),
        ([x1 - r, y1], [x0 + r, y1]),
        ([x0, y1 - r], [x0, y0 + r]),
        ([x1 - r, y0], [x1, y0 + r]),
        ([x1, y1 - r], [x1 - r, y1]),
        ([x0 + r, y1], [x0, y1 - r]),
        ([x0, y0 + r], [x0 + r, y0]),
    ] {
        list.push_line(a, c, t, color);
    }
    // The chevron and the cursor.
    let cx = x0 + b.w * 0.2;
    let cy = y0 + b.h * 0.5;
    let k = b.h * 0.2;
    list.push_line([cx, cy - k], [cx + k, cy], t, color);
    list.push_line([cx + k, cy], [cx, cy + k], t, color);
    let uy = cy + k;
    list.push_line([cx + 1.6 * k, uy], [cx + 3.2 * k, uy], t, color);
}

/// A list drawn as strokes in `rect`, with no background: three lines,
/// each after a dot, for the transcript.
pub(crate) fn list_glyph(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let b = glyph_box(rect);
    let t = 2.0 * s;
    let d = 3.4 * s;
    for i in 0..3 {
        let y = b.y + b.h * (0.18 + 0.32 * i as f32);
        fill(list, Rect::new(b.x, y - d * 0.5, d, d), color, d * 0.5);
        list.push_line([b.x + d + 4.0 * s, y], [b.x + b.w, y], t, color);
    }
}

/// Draws `b` at `rect`: the take-a-message button as a white record dot on
/// blue, the volume button as its glyph (dim while `volume` is muted,
/// with waves while an option is on, stepping while `speaking`), and the
/// rest as labelled buttons. A button the settings turn off (plan
/// 20261024_02, decision 4) is drawn at 35%, showing nothing live.
pub struct SquareButton;

impl SquareButton {
    /// Draws `b`, live or not as [`settings::allows`] says.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        volume: Volume,
        speaking: bool,
        recording: bool,
    ) {
        let enabled = settings::allows(b.hit);
        Self::draw_enabled(
            list, ctx, term, b, rect, s, volume, speaking, recording, enabled,
        );
    }

    /// Draws `b`, dim and showing nothing live unless `enabled`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_enabled(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        volume: Volume,
        speaking: bool,
        recording: bool,
        enabled: bool,
    ) {
        let start = list.instances.len();
        let (speaking, recording) = if enabled {
            (speaking, recording)
        } else {
            (false, false)
        };
        Self::draw_live(list, ctx, term, b, rect, s, volume, speaking, recording);
        if !enabled {
            settings::dim_since(list, start);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_live(
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        b: &FlowButton,
        rect: Rect,
        s: f32,
        volume: Volume,
        speaking: bool,
        recording: bool,
    ) {
        let px = 26.0 * s;
        match b.hit {
            // The terminal glyph while the transcript shows; the list glyph
            // (its label `LIST_LABEL`) while the terminal does.
            AgentHit::TerminalToggle if b.label == crate::agent::LIST_LABEL => {
                list_glyph(list, rect, s, theme().fg)
            }
            AgentHit::TerminalToggle => terminal_glyph(list, rect, s, theme().fg),
            AgentHit::Message | AgentHit::Reply => {
                fill(list, rect, b.bg, 10.0 * s);
                let d = DOT * s;
                let dot = Rect::new(
                    rect.x + (rect.w - d) * 0.5,
                    rect.y + (rect.h - d) * 0.5,
                    d,
                    d,
                );
                fill(
                    list,
                    dot,
                    if recording {
                        theme().recording
                    } else {
                        theme().fg
                    },
                    d * 0.5,
                );
            }
            AgentHit::Volume => {
                fill(list, rect, b.bg, 10.0 * s);
                // Muted: the plain speaker, dim. Unmuted: full strength,
                // with waves while either option is on, stepping while
                // something is said (the 2026-10-29 00:26 EDT amendment).
                let waves = Waves::now(volume, speaking);
                volume_glyph(list, rect, s, volume.color(), waves);
            }
            // A summary card's replay: the play glyph alone, dim.
            AgentHit::Replay(_) => {
                draw_glyph(
                    list,
                    ctx,
                    term,
                    PLAY_GLYPH,
                    ">",
                    rect,
                    18.0 * s,
                    theme().dim,
                );
            }
            AgentHit::Submit if b.label.is_empty() => {
                fill(list, rect, b.bg, 10.0 * s);
                draw_glyph(list, ctx, term, SEND_GLYPH, ">", rect, px, theme().fg);
            }
            AgentHit::Clear if b.label.is_empty() => {
                fill(list, rect, b.bg, 10.0 * s);
                draw_glyph(list, ctx, term, CANCEL_GLYPH, "x", rect, px, theme().fg);
            }
            AgentHit::GeneralAgent if b.label == "listening" => {
                draw_general_agent_listening(list, ctx, term, rect, b.bg, s);
            }
            // A turn runs (plan 20261024_03, the 2026-10-30 22:08 EDT
            // amendment, item 2): the drawer's Braille spinner, 80 ms a
            // frame, in the Gemini accent, in place of the ✦.
            AgentHit::GeneralAgent if b.label == "busy" => {
                fill(list, rect, b.bg, 10.0 * s);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                let glyph = crate::drawer::spinner::glyph_at(now).to_string();
                draw_glyph(list, ctx, term, &glyph, "*", rect, 24.0 * s, theme().accent);
            }
            AgentHit::GeneralAgent if b.label.is_empty() => {
                fill(list, rect, b.bg, 10.0 * s);
                let px = 24.0 * s;
                draw_glyph(list, ctx, term, GEMINI_GLYPH, "*", rect, px, theme().accent);
            }
            // A plan question's words are plain text in their colour (the
            // colour stands in `bg`), not buttons.
            // So are the daemon's answers in a text style (plan
            // 20261027_06, decision 3).
            AgentHit::Suggest(
                flankr_companion::Suggestion::Approve
                | flankr_companion::Suggestion::Discuss
                | flankr_companion::Suggestion::Implement,
            )
            | AgentHit::Answer { text: true, .. } => {
                let px = 19.0 * s;
                let y = rect.y + (rect.h - crate::agent::line_h(px)) * 0.5;
                ctx.draw_text(list, &b.label, [rect.x + 4.0 * s, y], px, b.bg);
            }
            _ => button(list, ctx, &b.label, rect, b.bg, s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(volume: Volume, speaking: bool) -> DrawList {
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 104.0, 104.0);
        volume_glyph(
            &mut list,
            rect,
            2.0,
            volume.color(),
            Waves::now(volume, speaking),
        );
        list
    }

    #[test]
    fn muted_the_speaker_is_plain_and_dim_and_unmuted_full_strength_with_waves() {
        let on = Volume {
            muted: false,
            on: true,
        };
        let off = Volume {
            muted: false,
            on: false,
        };
        let muted = Volume {
            muted: true,
            on: true,
        };
        // The plain speaker: its body's four lines and the cone's three;
        // muted, the same and one slash.
        let plain = glyph(off, false).instances.len();
        assert_eq!(glyph(muted, false).instances.len(), plain + 1);
        // No ✕ with both options off, nor waves while speaking.
        assert_eq!(Waves::now(off, true), Waves::Off);
        assert_eq!(Waves::now(muted, true), Waves::Muted);
        assert_eq!(Waves::now(muted, false), Waves::Muted);
        // Two arcs of six segments while an option is on.
        assert_eq!(glyph(on, false).instances.len(), plain + 12);
        assert_eq!(Waves::now(on, false), Waves::Two);
        assert!(matches!(Waves::now(on, true), Waves::Speaking(_)));
        // Dim while muted, full strength otherwise, whatever the options.
        assert_eq!(muted.color(), theme().dim);
        assert_eq!(on.color(), theme().fg);
        assert_eq!(off.color(), theme().fg);
        let colors = |l: &DrawList| l.instances.iter().map(|i| i.color).collect::<Vec<_>>();
        assert!(colors(&glyph(off, false)).iter().all(|c| *c == theme().fg));
    }
}
