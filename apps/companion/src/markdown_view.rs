//! Markdown in the app (docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md,
//! decision 5): a [`MarkdownView`] lays its text out with
//! `handlr_markdown::lay_out` and draws the runs, in Roboto Regular, Bold
//! and Italic (BoldItalic falls back to Bold) and the terminal face for
//! `Code`. Links are drawn as the agent view's link spans are, in the link
//! colour and underlined, and a tap on one is
//! `ControlAction::Activated { part: ControlPart::Custom(<its index>) }`.
//! They open through `ctx.open_link` once libmorph 0.4.0 is in; before
//! that, the screen that hosts the view leaves them inert.
//!
//! libcontrols' `DrawContext` carries one face, so the others come in a
//! [`Faces`], which the app makes once. Without one (the tests), every
//! style is drawn and measured in the context's font.

use std::sync::Arc;

use handlr_markdown::{Laid, Style, lay_out};
use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::{AtlasId, AtlasRef, DrawList};
use libmsdf::font::{FontAtlas, GlyphStyle, TerminalShaper, TextShaper};

use crate::agent::link_spans::LINK;
use crate::agent::{CELL_EM, DIM, fill, line_h};

/// One prose face: its atlas in the renderer's set (metrics only, see
/// [`Faces::metrics`]), and its shaper.
pub struct Face {
    pub id: AtlasId,
    pub atlas: FontAtlas,
    pub shaper: TextShaper,
}

/// The terminal face, for `Code`.
pub struct CodeFace {
    pub shaper: TerminalShaper,
    pub text: (AtlasId, FontAtlas),
    pub symbols: (AtlasId, FontAtlas),
}

/// The faces a [`MarkdownView`] draws with.
pub struct Faces {
    pub regular: Face,
    pub bold: Face,
    pub italic: Face,
    pub code: CodeFace,
    pub px_range: f32,
}

impl Faces {
    /// `atlas` without its texels: drawing reads only the glyph metrics, and
    /// the texels are on the GPU through the renderer's atlas set.
    pub fn metrics(mut atlas: FontAtlas) -> FontAtlas {
        atlas.pixel_data = Vec::new();
        atlas
    }

    fn prose(&self, style: Style) -> &Face {
        match style {
            Style::Bold | Style::BoldItalic => &self.bold,
            Style::Italic => &self.italic,
            Style::Regular | Style::Code => &self.regular,
        }
    }
}

/// `text`'s width in `style` at `px`: by `faces` when given, else as the
/// context's font measures without a shaper.
pub fn measure(faces: Option<&Faces>, text: &str, style: Style, px: f32) -> f32 {
    let chars = text.chars().count() as f32;
    match (faces, style) {
        (Some(f), Style::Code) => chars * px * f.code.shaper.cell_em(),
        (Some(f), style) => f.prose(style).shaper.shape(text).advance_px(px),
        (None, Style::Code) => chars * px * CELL_EM,
        // libcontrols' `DrawContext::text_advance` without a shaper.
        (None, _) => chars * px * 0.55,
    }
}

/// A run where it is drawn.
#[derive(Clone, Debug, PartialEq)]
struct Placed {
    text: String,
    style: Style,
    link: Option<usize>,
    dim: bool,
    x: f32,
    y: f32,
    w: f32,
}

/// Markdown laid out in a column, as a libcontrols control.
pub struct MarkdownView {
    source: String,
    laid: Laid,
    placed: Vec<Placed>,
    rect: Rect,
    px: f32,
    color: [f32; 4],
    faces: Option<Arc<Faces>>,
}

impl MarkdownView {
    /// `md` at `(x, y)`, `width` wide, in `px` type of `color`.
    pub fn new(
        md: &str,
        x: f32,
        y: f32,
        width: f32,
        px: f32,
        color: [f32; 4],
        faces: Option<Arc<Faces>>,
    ) -> Self {
        let f = faces.as_deref();
        let laid = lay_out(md, width, &|text, style| measure(f, text, style, px));
        let lh = line_h(px);
        let mut placed = Vec::new();
        let mut cy = y;
        for line in &laid.lines {
            cy += line.gap_before * lh;
            let mut cx = x + line.indent;
            for run in &line.runs {
                let w = measure(f, &run.text, run.style, px);
                placed.push(Placed {
                    text: run.text.clone(),
                    style: run.style,
                    link: run.link,
                    dim: run.dim,
                    x: cx,
                    y: cy,
                    w,
                });
                cx += w;
            }
            cy += lh;
        }
        MarkdownView {
            source: md.to_string(),
            laid,
            placed,
            rect: Rect::new(x, y, width, cy - y),
            px,
            color,
            faces,
        }
    }

    pub fn height(&self) -> f32 {
        self.rect.h
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The Markdown it shows.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Its lines' text, one a line.
    pub fn text(&self) -> String {
        self.laid
            .lines
            .iter()
            .map(|l| l.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The links its runs index.
    pub fn links(&self) -> &[String] {
        &self.laid.links
    }

    /// Moves it, keeping its size.
    pub fn move_to(&mut self, x: f32, y: f32) {
        let (dx, dy) = (x - self.rect.x, y - self.rect.y);
        for p in &mut self.placed {
            p.x += dx;
            p.y += dy;
        }
        self.rect.x = x;
        self.rect.y = y;
    }

    /// A link run's tap area: a little taller than its line.
    fn tap_rect(&self, p: &Placed) -> Rect {
        let h = line_h(self.px);
        Rect::new(p.x, p.y - h * 0.15, p.w, h * 1.3)
    }

    /// The link at `pos`, if any.
    pub fn link_at(&self, pos: [f32; 2]) -> Option<usize> {
        self.placed
            .iter()
            .filter(|p| p.link.is_some())
            .find(|p| self.tap_rect(p).contains(pos))
            .and_then(|p| p.link)
    }

    /// Where link `index`'s first run is.
    pub fn link_rect(&self, index: usize) -> Option<Rect> {
        self.placed
            .iter()
            .find(|p| p.link == Some(index))
            .map(|p| Rect::new(p.x, p.y, p.w, line_h(self.px)))
    }

    fn draw_run(&self, list: &mut DrawList, ctx: &DrawContext, p: &Placed, color: [f32; 4]) {
        let pos = [p.x, p.y];
        let Some(f) = &self.faces else {
            ctx.draw_text(list, &p.text, pos, self.px, color);
            return;
        };
        if p.style == Style::Code {
            let c = &f.code;
            let line = c.shaper.line(&p.text, GlyphStyle::Regular);
            let text = AtlasRef {
                id: c.text.0,
                atlas: &c.text.1,
            };
            let symbols = AtlasRef {
                id: c.symbols.0,
                atlas: &c.symbols.1,
            };
            list.push_terminal_line(&line, text, symbols, pos, self.px, f.px_range, color);
            return;
        }
        let face = f.prose(p.style);
        let run = face.shaper.shape(&p.text);
        let atlas = AtlasRef {
            id: face.id,
            atlas: &face.atlas,
        };
        list.push_shaped_text_with(&run, atlas, pos, self.px, f.px_range, color);
    }
}

impl DrawControl for MarkdownView {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        for p in &self.placed {
            let color = if p.link.is_some() {
                LINK
            } else if p.dim {
                DIM
            } else {
                self.color
            };
            self.draw_run(list, ctx, p, color);
            if p.link.is_some() && !p.text.trim().is_empty() {
                let h = line_h(self.px);
                let thick = (self.px * 0.07).max(1.0);
                fill(
                    list,
                    Rect::new(p.x, p.y + h * 0.92 - thick, p.w, thick),
                    LINK,
                    0.0,
                );
            }
        }
    }
}

impl HostedControl for MarkdownView {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        self.rect
            .contains(pos)
            .then(|| (ControlPart::Body, self.rect.normalized_x(pos[0])))
    }

    /// A tap on a link is its index; elsewhere, nothing.
    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } => self.link_at(*pos).map(|i| ControlAction::Activated {
                part: ControlPart::Custom(i as u32),
            }),
            _ => None,
        }
    }

    fn handle_keyboard(
        &self,
        _event: &KeyboardEvent,
        _ctx: &EventContext,
    ) -> Option<ControlAction> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FG: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    fn click(view: &MarkdownView, pos: [f32; 2]) -> Option<ControlAction> {
        view.handle_mouse(
            &MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            },
            &EventContext::default(),
        )
    }

    #[test]
    fn a_paragraph_with_bold_italic_code_and_a_link_draws_and_its_link_taps() {
        let md = "Some **bold**, *italic*, `code` and [a link](https://example.com).";
        let view = MarkdownView::new(md, 10.0, 20.0, 2000.0, 20.0, FG, None);
        assert_eq!(view.text(), "Some bold, italic, code and a link.");
        assert_eq!(view.links(), ["https://example.com"]);
        assert_eq!(view.height(), line_h(20.0));
        let mut list = DrawList::new();
        view.draw(&mut list, &DrawContext::default());
        // The underline, in the link colour, under the link's run.
        let r = view.link_rect(0).expect("the link is placed");
        assert!(
            list.instances
                .iter()
                .any(|i| i.color == LINK && (i.position[0] - r.x).abs() < 1e-3)
        );
        let at = [r.x + r.w * 0.5, r.y + r.h * 0.5];
        assert_eq!(
            click(&view, at),
            Some(ControlAction::Activated {
                part: ControlPart::Custom(0)
            })
        );
        assert_eq!(click(&view, [12.0, at[1]]), None, "not on the link");
    }

    #[test]
    fn it_wraps_to_its_width_and_moves() {
        let mut view = MarkdownView::new(
            "one two three four five six",
            0.0,
            0.0,
            100.0,
            20.0,
            FG,
            None,
        );
        assert!(view.text().lines().count() > 1);
        assert_eq!(
            view.height(),
            view.text().lines().count() as f32 * line_h(20.0)
        );
        let before = view.rect();
        view.move_to(5.0, 50.0);
        assert_eq!(view.rect(), Rect::new(5.0, 50.0, before.w, before.h));
    }
}
