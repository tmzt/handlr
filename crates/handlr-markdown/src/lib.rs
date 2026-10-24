//! `handlr-markdown`: basic CommonMark laid out as wrapped lines of styled
//! runs, with links and no HTML
//! (docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decisions 1–4).
//!
//! [`lay_out`] parses with `pulldown-cmark` (CommonMark, no GFM) and wraps
//! the text to a width, measured by the caller. It renders paragraphs,
//! **bold**, *italic* and `code`, links and bare `http(s)://` URLs, bullet
//! and numbered lists, headings (as bold), block quotes (dim) and code
//! blocks (in [`Style::Code`]). HTML is shown as its literal text, and an
//! image as its alt text linked to its URL. Nothing here draws.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// A run's face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Code,
}

/// Text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// Its index in [`Laid::links`], when it is a link's text.
    pub link: Option<usize>,
    /// In a block quote.
    pub dim: bool,
}

/// One wrapped line.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Line {
    pub runs: Vec<Run>,
    /// From the left, in the units `measure` gives.
    pub indent: f32,
    /// The space above it, in lines: [`BLOCK_GAP`] before a block,
    /// [`ITEM_GAP`] before a list item, 0 within a block.
    pub gap_before: f32,
}

/// The laid-out text, and the links its runs index, in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Laid {
    pub lines: Vec<Line>,
    pub links: Vec<String>,
}

/// The space before a paragraph, heading, quote or list, in lines.
pub const BLOCK_GAP: f32 = 0.5;
/// The space before a list item after the list's first, in lines.
pub const ITEM_GAP: f32 = 0.15;

/// The URL schemes found as bare links, as the daemon finds them
/// (`src/sessions/links.rs`).
const SCHEMES: [&str; 2] = ["http://", "https://"];
/// What ends a sentence or wraps a URL, and is not part of it at its end.
const TRAILING: [char; 13] = [
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '"', '\'', '`',
];
/// A block quote's indent, measured as this text.
const QUOTE_INDENT: &str = "    ";

/// `md` laid out in lines at most `width` wide. `measure(text, style)` is
/// the text's width at the size it will be drawn. Words wrap at spaces; a
/// word wider than the line breaks by character. A list item's lines are
/// indented under its marker ("•" or "1.").
pub fn lay_out(md: &str, width: f32, measure: &dyn Fn(&str, Style) -> f32) -> Laid {
    let mut b = Builder::new(measure);
    for event in Parser::new(md) {
        b.event(event);
    }
    b.flush();
    let mut lines = Vec::new();
    for block in &b.blocks {
        wrap(block, width, measure, &mut lines);
    }
    Laid {
        lines,
        links: b.links,
    }
}

/// A piece of a block: text, or a hard line break.
#[derive(Clone, Debug)]
enum Piece {
    Text(Run),
    Break,
}

/// A block before wrapping.
#[derive(Clone, Debug)]
struct Block {
    pieces: Vec<Piece>,
    indent: f32,
    /// A list item's marker with its space ("• ", "1. ").
    marker: Option<String>,
    gap: f32,
    /// A code block: spaces are kept and only long lines break.
    pre: bool,
    dim: bool,
}

struct Builder<'a> {
    measure: &'a dyn Fn(&str, Style) -> f32,
    blocks: Vec<Block>,
    current: Option<Block>,
    links: Vec<String>,
    strong: u32,
    emphasis: u32,
    quote: u32,
    link: Option<usize>,
    code_block: bool,
    /// The open lists' next numbers (`None` for bullets).
    lists: Vec<Option<u64>>,
    /// The open items' content indents.
    items: Vec<f32>,
    /// The next item is its list's first.
    first_item: bool,
    /// The last HTML block line ended with a newline: the next starts a line.
    html_break: bool,
}

impl<'a> Builder<'a> {
    fn new(measure: &'a dyn Fn(&str, Style) -> f32) -> Self {
        Builder {
            measure,
            blocks: Vec::new(),
            current: None,
            links: Vec::new(),
            strong: 0,
            emphasis: 0,
            quote: 0,
            link: None,
            code_block: false,
            lists: Vec::new(),
            items: Vec::new(),
            first_item: false,
            html_break: false,
        }
    }

    fn style(&self) -> Style {
        if self.code_block {
            return Style::Code;
        }
        match (self.strong > 0, self.emphasis > 0) {
            (true, true) => Style::BoldItalic,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (false, false) => Style::Regular,
        }
    }

    /// Where a new block's text starts.
    fn content_indent(&self) -> f32 {
        self.items.last().copied().unwrap_or(0.0)
            + self.quote as f32 * (self.measure)(QUOTE_INDENT, Style::Regular)
    }

    /// The gap a block gets: none for the first.
    fn gap(&self, gap: f32) -> f32 {
        if self.blocks.is_empty() && self.current.is_none() {
            0.0
        } else {
            gap
        }
    }

    /// A block to add text to: the open one while it is still empty (an
    /// item's first paragraph), otherwise a new one.
    fn open(&mut self, pre: bool) {
        if self.current.as_ref().is_some_and(|b| b.pieces.is_empty()) {
            if let Some(b) = &mut self.current {
                b.pre = pre;
            }
            return;
        }
        self.flush();
        self.current = Some(Block {
            pieces: Vec::new(),
            indent: self.content_indent(),
            marker: None,
            gap: self.gap(BLOCK_GAP),
            pre,
            dim: self.quote > 0,
        });
    }

    /// Ends the open block.
    fn flush(&mut self) {
        if let Some(block) = self.current.take()
            && !block.pieces.is_empty()
        {
            self.blocks.push(block);
        }
    }

    /// The block text goes to, opened if none is.
    fn block(&mut self) -> &mut Block {
        if self.current.is_none() {
            self.open(self.code_block);
        }
        self.current.as_mut().expect("opened")
    }

    fn push_run(&mut self, text: &str, style: Style, link: Option<usize>) {
        if text.is_empty() {
            return;
        }
        let dim = self.quote > 0;
        self.block().pieces.push(Piece::Text(Run {
            text: text.to_string(),
            style,
            link,
            dim,
        }));
    }

    fn push_break(&mut self) {
        self.block().pieces.push(Piece::Break);
    }

    /// Lines of literal text, broken where it has newlines.
    fn push_lines(&mut self, text: &str, style: Style) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.push_break();
            }
            self.push_run(line, style, self.link);
        }
    }

    /// Text, with its bare URLs as links unless it is already a link's.
    fn push_text(&mut self, text: &str) {
        let style = self.style();
        if self.link.is_some() {
            return self.push_run(text, style, self.link);
        }
        let mut from = 0;
        for (start, end) in bare_urls(text) {
            self.push_run(&text[from..start], style, None);
            let link = self.link_index(&text[start..end]);
            self.push_run(&text[start..end], style, Some(link));
            from = end;
        }
        self.push_run(&text[from..], style, None);
    }

    fn link_index(&mut self, url: &str) -> usize {
        match self.links.iter().position(|l| l == url) {
            Some(i) => i,
            None => {
                self.links.push(url.to_string());
                self.links.len() - 1
            }
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.code_block => self.push_lines(&text, Style::Code),
            Event::Text(text) => self.push_text(&text),
            Event::Code(text) => self.push_run(&text, Style::Code, self.link),
            // Shown as written, never interpreted (decision 3).
            Event::Html(text) => {
                if self.html_break {
                    self.push_break();
                }
                self.push_lines(&text, self.style());
                self.html_break = text.ends_with('\n');
            }
            Event::InlineHtml(text) => self.push_lines(&text, self.style()),
            Event::SoftBreak => {
                let style = self.style();
                self.push_run(" ", style, self.link);
            }
            Event::HardBreak => self.push_break(),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => self.open(false),
            Tag::Heading { .. } => {
                self.open(false);
                self.strong += 1;
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.open(true);
                self.code_block = true;
            }
            Tag::List(first) => {
                self.flush();
                self.lists.push(first);
                self.first_item = true;
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "\u{2022} ".to_string(),
                };
                let indent = self.content_indent();
                let gap = self.gap(if self.first_item { BLOCK_GAP } else { ITEM_GAP });
                self.first_item = false;
                let hang = (self.measure)(&marker, Style::Regular);
                self.current = Some(Block {
                    pieces: Vec::new(),
                    indent,
                    marker: Some(marker),
                    gap,
                    pre: false,
                    dim: self.quote > 0,
                });
                self.items
                    .push(self.items.last().copied().unwrap_or(0.0) + hang);
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.link = Some(self.link_index(&dest_url));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::HtmlBlock => {
                self.html_break = false;
                self.flush();
            }
            TagEnd::Heading(_) => {
                self.strong = self.strong.saturating_sub(1);
                self.flush();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                self.code_block = false;
                self.flush();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                self.first_item = false;
            }
            TagEnd::Item => {
                self.flush();
                self.items.pop();
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => self.link = None,
            _ => {}
        }
    }
}

/// `url` without the sentence or wrapping marks at its end; a `)` that
/// closes a `(` inside it stays.
fn trim_url(mut url: &str) -> &str {
    while let Some(last) = url.chars().next_back().filter(|c| TRAILING.contains(c)) {
        if last == ')' && url.matches('(').count() >= url.matches(')').count() {
            break;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
    url
}

/// The bare `http(s)://` URLs in `text`, as byte ranges, by the daemon's
/// rules: not after a letter or digit, up to whitespace, without trailing
/// punctuation, and not cut short with "…".
fn bare_urls(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut found = Vec::new();
    for scheme in SCHEMES {
        let mut from = 0;
        while let Some(at) = lower[from..].find(scheme).map(|i| from + i) {
            from = at + scheme.len();
            if text[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            let len = text[at..]
                .char_indices()
                .find(|&(_, c)| c.is_whitespace())
                .map_or(text.len() - at, |(i, _)| i);
            let url = trim_url(&text[at..at + len]);
            from = at + len;
            if url.len() <= scheme.len() || url.ends_with('\u{2026}') {
                continue;
            }
            found.push((at, at + url.len()));
        }
    }
    found.sort_unstable();
    found
}

/// A word, a space between words, or a forced break.
enum Token {
    Word(Vec<Run>),
    Space(Run),
    Break,
}

/// `block`'s pieces as words and spaces; in a code block, spaces are part
/// of the words.
fn tokens(block: &Block) -> Vec<Token> {
    let mut out = Vec::new();
    let mut word: Vec<Run> = Vec::new();
    let end_word = |word: &mut Vec<Run>, out: &mut Vec<Token>| {
        if !word.is_empty() {
            out.push(Token::Word(std::mem::take(word)));
        }
    };
    for piece in &block.pieces {
        let run = match piece {
            Piece::Break => {
                end_word(&mut word, &mut out);
                out.push(Token::Break);
                continue;
            }
            Piece::Text(run) => run,
        };
        for c in run.text.chars() {
            if c.is_whitespace() && !block.pre {
                end_word(&mut word, &mut out);
                if !matches!(out.last(), Some(Token::Space(_))) {
                    out.push(Token::Space(Run {
                        text: " ".to_string(),
                        ..run.clone()
                    }));
                }
                continue;
            }
            match word.last_mut() {
                Some(last) if same(last, run) => last.text.push(c),
                _ => word.push(Run {
                    text: c.to_string(),
                    ..run.clone()
                }),
            }
        }
    }
    end_word(&mut word, &mut out);
    out
}

/// Two runs that draw alike.
fn same(a: &Run, b: &Run) -> bool {
    a.style == b.style && a.link == b.link && a.dim == b.dim
}

/// `run` added to `line`, merged into the last run when they draw alike.
fn append(line: &mut Vec<Run>, run: Run) {
    match line.last_mut() {
        Some(last) if same(last, &run) => last.text.push_str(&run.text),
        _ => line.push(run),
    }
}

/// `block`'s lines, wrapped to `width`, added to `out`.
fn wrap(block: &Block, width: f32, measure: &dyn Fn(&str, Style) -> f32, out: &mut Vec<Line>) {
    let hang = block
        .marker
        .as_deref()
        .map_or(0.0, |m| measure(m, Style::Regular));
    let avail = (width - block.indent - hang).max(1.0);
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut cur: Vec<Run> = Vec::new();
    let mut cur_w = 0.0;
    let mut space: Option<Run> = None;
    let width_of = |runs: &[Run]| -> f32 { runs.iter().map(|r| measure(&r.text, r.style)).sum() };
    for token in tokens(block) {
        match token {
            Token::Space(run) => {
                if !cur.is_empty() {
                    space = Some(run);
                }
            }
            Token::Break => {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
                space = None;
            }
            Token::Word(word) => {
                let w = width_of(&word);
                let sw = space.as_ref().map_or(0.0, |s| measure(&s.text, s.style));
                if !cur.is_empty() && cur_w + sw + w > avail {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0.0;
                    space = None;
                }
                if let Some(s) = space.take()
                    && !cur.is_empty()
                {
                    append(&mut cur, s);
                    cur_w += sw;
                }
                if cur.is_empty() && w > avail {
                    // Too wide for any line: broken by character.
                    for run in word {
                        for c in run.text.chars() {
                            let piece = Run {
                                text: c.to_string(),
                                ..run.clone()
                            };
                            let cw = measure(&piece.text, piece.style);
                            if !cur.is_empty() && cur_w + cw > avail {
                                lines.push(std::mem::take(&mut cur));
                                cur_w = 0.0;
                            }
                            append(&mut cur, piece);
                            cur_w += cw;
                        }
                    }
                } else {
                    for run in word {
                        append(&mut cur, run);
                    }
                    cur_w += w;
                }
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    for (i, runs) in lines.into_iter().enumerate() {
        let mut line = Line {
            runs: Vec::new(),
            indent: block.indent + hang,
            gap_before: 0.0,
        };
        if i == 0 {
            line.gap_before = block.gap;
            if let Some(marker) = &block.marker {
                line.indent = block.indent;
                line.runs.push(Run {
                    text: marker.clone(),
                    style: Style::Regular,
                    link: None,
                    dim: block.dim,
                });
            }
        }
        // The marker stays a run of its own.
        let marker_runs = line.runs.len();
        for run in runs {
            if line.runs.len() == marker_runs {
                line.runs.push(run);
            } else {
                append(&mut line.runs, run);
            }
        }
        out.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One unit a character, in every style.
    fn chars(text: &str, _: Style) -> f32 {
        text.chars().count() as f32
    }

    fn lay(md: &str, width: f32) -> Laid {
        lay_out(md, width, &chars)
    }

    /// Each line's text.
    fn texts(laid: &Laid) -> Vec<String> {
        laid.lines
            .iter()
            .map(|l| l.runs.iter().map(|r| r.text.as_str()).collect())
            .collect()
    }

    fn run(text: &str, style: Style) -> Run {
        Run {
            text: text.to_string(),
            style,
            link: None,
            dim: false,
        }
    }

    #[test]
    fn a_paragraph_with_bold_italic_code_and_a_link() {
        let laid = lay(
            "Some **bold**, *italic*, `code` and [a link](https://example.com).",
            200.0,
        );
        assert_eq!(laid.lines.len(), 1);
        let line = &laid.lines[0];
        assert_eq!(line.gap_before, 0.0);
        assert_eq!(
            line.runs,
            [
                run("Some ", Style::Regular),
                run("bold", Style::Bold),
                run(", ", Style::Regular),
                run("italic", Style::Italic),
                run(", ", Style::Regular),
                run("code", Style::Code),
                run(" and ", Style::Regular),
                Run {
                    link: Some(0),
                    ..run("a link", Style::Regular)
                },
                run(".", Style::Regular),
            ]
        );
        assert_eq!(laid.links, ["https://example.com"]);
    }

    #[test]
    fn bold_with_italic_inside_is_bold_italic() {
        let laid = lay("**bold *both***", 80.0);
        assert_eq!(
            laid.lines[0].runs,
            [run("bold ", Style::Bold), run("both", Style::BoldItalic)]
        );
    }

    #[test]
    fn headings_are_bold_quotes_dim_and_code_blocks_keep_their_lines() {
        let laid = lay(
            "# Title\n\nText.\n\n> quoted\n\n```\nfn main() {\n    go();\n}\n```\n",
            80.0,
        );
        assert_eq!(
            texts(&laid),
            ["Title", "Text.", "quoted", "fn main() {", "    go();", "}"]
        );
        assert_eq!(laid.lines[0].runs, [run("Title", Style::Bold)]);
        assert_eq!(laid.lines[1].gap_before, BLOCK_GAP);
        let quote = &laid.lines[2];
        assert!(quote.runs.iter().all(|r| r.dim));
        assert_eq!(quote.indent, 4.0);
        for line in &laid.lines[3..] {
            assert!(line.runs.iter().all(|r| r.style == Style::Code));
            assert_eq!(line.indent, 0.0);
        }
        assert_eq!(laid.lines[3].gap_before, BLOCK_GAP);
        assert_eq!(laid.lines[4].gap_before, 0.0);
    }

    #[test]
    fn html_is_shown_as_its_literal_text() {
        let laid = lay("Hi <b>there</b> & <script>x</script>.", 80.0);
        assert_eq!(texts(&laid), ["Hi <b>there</b> & <script>x</script>."]);
        let laid = lay("<div class=\"a\">\nblock\n</div>\n\nAfter.", 80.0);
        assert_eq!(
            texts(&laid),
            ["<div class=\"a\">", "block", "</div>", "After."]
        );
    }

    #[test]
    fn an_image_is_its_alt_text_linked_to_its_url() {
        let laid = lay("![a chart](https://example.com/c.png)", 80.0);
        assert_eq!(texts(&laid), ["a chart"]);
        assert_eq!(laid.lines[0].runs[0].link, Some(0));
        assert_eq!(laid.links, ["https://example.com/c.png"]);
    }

    #[test]
    fn words_wrap_at_spaces_and_a_long_word_breaks_by_character() {
        let laid = lay("one two three four", 9.0);
        assert_eq!(texts(&laid), ["one two", "three", "four"]);
        let laid = lay("a abcdefghijkl b", 5.0);
        assert_eq!(texts(&laid), ["a", "abcde", "fghij", "kl b"]);
        // A long code line breaks too.
        let laid = lay("```\nabcdefgh\n```", 5.0);
        assert_eq!(texts(&laid), ["abcde", "fgh"]);
        // Styles stay with their words across the wrap.
        let laid = lay("plain **bold words**", 8.0);
        assert_eq!(laid.lines[1].runs, [run("bold", Style::Bold)]);
        assert_eq!(laid.lines[2].runs, [run("words", Style::Bold)]);
    }

    #[test]
    fn list_items_are_indented_under_their_markers() {
        let laid = lay("- first item\n- second\n  - nested\n\n1. one\n2. two", 10.0);
        assert_eq!(
            texts(&laid),
            [
                "\u{2022} first",
                "item",
                "\u{2022} second",
                "\u{2022} nested",
                "1. one",
                "2. two"
            ]
        );
        let indents: Vec<f32> = laid.lines.iter().map(|l| l.indent).collect();
        assert_eq!(indents, [0.0, 2.0, 0.0, 2.0, 0.0, 0.0]);
        let gaps: Vec<f32> = laid.lines.iter().map(|l| l.gap_before).collect();
        assert_eq!(gaps, [0.0, 0.0, ITEM_GAP, BLOCK_GAP, BLOCK_GAP, ITEM_GAP]);
        assert_eq!(laid.lines[0].runs[0], run("\u{2022} ", Style::Regular));
    }

    #[test]
    fn links_are_collected_in_order_with_bare_urls() {
        let laid = lay(
            "See https://b.example/x. Then [docs](https://a.example), \
             <https://c.example> and https://b.example/x again; not xhttps://d.example.",
            400.0,
        );
        assert_eq!(
            laid.links,
            [
                "https://b.example/x",
                "https://a.example",
                "https://c.example"
            ]
        );
        let linked: Vec<(String, usize)> = laid.lines[0]
            .runs
            .iter()
            .filter_map(|r| r.link.map(|l| (r.text.clone(), l)))
            .collect();
        assert_eq!(
            linked,
            [
                ("https://b.example/x".to_string(), 0),
                ("docs".to_string(), 1),
                ("https://c.example".to_string(), 2),
                ("https://b.example/x".to_string(), 0),
            ]
        );
    }

    #[test]
    fn empty_text_lays_out_nothing() {
        assert_eq!(lay("", 10.0), Laid::default());
    }
}
