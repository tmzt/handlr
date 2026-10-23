//! An inactive outlined button (plan 20261023_04, A3 and A6): an outline
//! and a label in one dim colour, no fill. It shows what is asked, such as
//! a question's Yes and No on an agent's card, without being a control: a
//! tap on it is a tap on whatever it sits on.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::{DrawList, SdfInstance, SdfKind};

use crate::agent::{DIM, line_h};

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineButton {
    pub label: &'static str,
    pub rect: Rect,
    pub color: [f32; 4],
}

impl OutlineButton {
    /// The outline's width and the label's size, in points.
    pub const STROKE: f32 = 1.5;
    pub const LABEL_PT: f32 = 15.0;

    /// `label` at `rect`, dim.
    pub fn new(label: &'static str, rect: Rect) -> Self {
        OutlineButton {
            label,
            rect,
            color: DIM,
        }
    }

    /// Draws it moved down by `dy`, at scale `s`.
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext, dy: f32, s: f32) {
        let r = Rect::new(self.rect.x, self.rect.y + dy, self.rect.w, self.rect.h);
        list.push(SdfInstance {
            kind: SdfKind::Outline {
                radius: 8.0 * s,
                thickness: Self::STROKE * s,
            },
            position: r.pos(),
            size: r.size(),
            color: self.color,
            anim: 0,
        });
        let px = Self::LABEL_PT * s;
        let w = ctx.text_advance(self.label, px);
        let pos = [r.x + (r.w - w) * 0.5, r.y + (r.h - line_h(px)) * 0.5];
        ctx.draw_text(list, self.label, pos, px, self.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_an_outline_with_no_fill() {
        let b = OutlineButton::new("Yes", Rect::new(10.0, 20.0, 144.0, 64.0));
        let mut list = DrawList::new();
        b.draw(&mut list, &DrawContext::default(), 100.0, 2.0);
        assert_eq!(list.instances.len(), 1, "the outline; no atlas, no text");
        let outline = &list.instances[0];
        assert!(matches!(
            outline.kind,
            SdfKind::Outline { thickness, .. } if thickness == 3.0
        ));
        assert_eq!(outline.position, [10.0, 120.0]);
        assert_eq!(outline.size, [144.0, 64.0]);
        assert_eq!(outline.color, DIM);
    }
}
