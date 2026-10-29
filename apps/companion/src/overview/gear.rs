//! The gear at the right end of the overview's header (plan 20261024_02,
//! decision 1): a 44 pt touch target in the header's dim colour, drawn as
//! a toothed wheel with a hole. A tap opens the settings screen.

use libcontrols::Rect;
use libmsdf::drawlist::DrawList;

use crate::agent::fill;
use crate::theme::theme;

/// The gear's teeth.
const TEETH: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gear {
    /// The touch target.
    pub rect: Rect,
}

impl Gear {
    /// The touch target's side, in points.
    pub const SIZE: f32 = 44.0;

    /// The gear at the right of a header line from `y`, `line` high, `pad`
    /// in from the right edge `right`, at scale `s`.
    pub fn place(right: f32, pad: f32, y: f32, line: f32, s: f32) -> Self {
        let side = Self::SIZE * s;
        Gear {
            rect: Rect::new(right - pad - side, y + (line - side) * 0.5, side, side),
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rect.contains([x, y])
    }

    /// Draws it at about a third of its target: the teeth, the wheel, the
    /// hole. The target stays 44 pt.
    pub fn draw(&self, list: &mut DrawList) {
        self.draw_in(list, theme().muted, theme().bg);
    }

    /// [`Gear::draw`] in `color`, its hole in `under` (what it is drawn
    /// on): the title bar's gear is in the title's text colour.
    pub fn draw_in(&self, list: &mut DrawList, color: [f32; 4], under: [f32; 4]) {
        let [cx, cy] = self.rect.center();
        let r = self.rect.w * 0.16;
        let tooth = r * 0.5;
        for i in 0..TEETH {
            let a = i as f32 * std::f32::consts::TAU / TEETH as f32;
            let (tx, ty) = (cx + a.cos() * r * 1.05, cy + a.sin() * r * 1.05);
            fill(
                list,
                Rect::new(tx - tooth * 0.5, ty - tooth * 0.5, tooth, tooth),
                color,
                tooth * 0.3,
            );
        }
        let wheel = r * 0.95;
        fill(
            list,
            Rect::new(cx - wheel, cy - wheel, 2.0 * wheel, 2.0 * wheel),
            color,
            wheel,
        );
        let hole = r * 0.42;
        fill(
            list,
            Rect::new(cx - hole, cy - hole, 2.0 * hole, 2.0 * hole),
            under,
            hole,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_a_44_pt_target_at_the_right() {
        let g = Gear::place(1080.0, 24.0, 128.0, 86.4, 2.0);
        assert_eq!((g.rect.w, g.rect.h), (88.0, 88.0));
        assert_eq!(g.rect.x + g.rect.w, 1080.0 - 24.0);
        assert!((g.rect.center()[1] - (128.0 + 43.2)).abs() < 0.01);
        let mut list = DrawList::new();
        g.draw(&mut list);
        assert_eq!(list.instances.len(), TEETH + 2);
        for i in &list.instances {
            assert!(g.rect.contains(i.position), "drawn inside its target");
        }
    }
}
