//! A pinned plugin card's frame (docs/design/20261028_04_PLUGINS_PLAN.md,
//! decision 3.7): full width at the end of the overview's grid, below the
//! agent cards, in the agent cards' colours and corner, with its plugin's
//! name as a small dim header over the device half's content. A tap goes
//! to the device half; held for 2 s it offers **Unpin**.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{fill, line_h};
use crate::overview::agent_card::AgentCard;
use crate::theme::theme;

/// The card's padding and its header's size, in points.
const PAD: f32 = 12.0;
const HEADER_PT: f32 = 12.0;
/// How long a press is held to offer Unpin, as the drawer's remote cards.
pub const HOLD: std::time::Duration = std::time::Duration::from_secs(2);

/// The card's height for content `content_h` tall, at scale `s`.
pub fn height(content_h: f32, s: f32) -> f32 {
    2.0 * PAD * s + line_h(HEADER_PT * s) + 4.0 * s + content_h
}

/// The content's rect in a card at `rect`.
pub fn content(rect: Rect, s: f32) -> Rect {
    let top = rect.y + PAD * s + line_h(HEADER_PT * s) + 4.0 * s;
    Rect::new(
        rect.x + PAD * s,
        top,
        (rect.w - 2.0 * PAD * s).max(0.0),
        (rect.y + rect.h - PAD * s - top).max(0.0),
    )
}

/// The card's background, outline and header.
pub fn draw_frame(list: &mut DrawList, ctx: &DrawContext, rect: Rect, name: &str, s: f32) {
    let r = AgentCard::RADIUS * s;
    let border = 1.0 * s;
    fill(list, rect, theme().band, r);
    fill(
        list,
        Rect::new(
            rect.x + border,
            rect.y + border,
            rect.w - 2.0 * border,
            rect.h - 2.0 * border,
        ),
        theme().card,
        (r - border).max(0.0),
    );
    ctx.draw_text(
        list,
        name,
        [rect.x + PAD * s, rect.y + PAD * s],
        HEADER_PT * s,
        theme().dim,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_content_fits_the_height_given_for_it() {
        let h = height(100.0, 2.0);
        let c = content(Rect::new(24.0, 500.0, 1032.0, h), 2.0);
        assert!((c.h - 100.0).abs() < 0.01);
        assert!(c.y > 500.0 && c.x > 24.0);
    }
}
