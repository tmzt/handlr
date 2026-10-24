---
title: A small Markdown renderer for the app, with links and no HTML
date: 2026-10-24
status: Draft
covers:
  - "A crate, `crates/handlr-markdown`, that parses basic CommonMark and
    lays it out as wrapped lines of styled runs, with links"
  - "The app drawing those runs, in Roboto regular, bold and italic and the
    terminal face for code"
  - "Its first use: the settings copy, from a build-time libtypedmd file,
    `apps/companion/SETTINGS_COPY.md`"
  - "Not covered: rendering the agent view's replies (a later L3
    amendment); tables, images, HTML and other GFM extensions"
sources:
  - "Owner (2026-10-24): \"move that copy to a buildtime libtypedmd file
    SETTINGS_COPY.md\"; \"also, use the md renderer with link support if
    we don't have one build it, basic formatting only no html\"; then
    \"yes plan and impl keep it simple\""
  - "handlr at `02299e1`: no Markdown rendering anywhere; links found in
    plain text by `apps/companion/src/agent/link_spans.rs`;
    `src/prompts.rs` reads `ADDITIONAL_PROMPTS.md` with libtypedmd
    (`a61c091`)"
  - "libtypedmd_pulldown uses `pulldown-cmark` 0.13 with default features
    off (GFM off); libmsdf `4de319c` has the features
    `font-apache-roboto-bold` and `font-apache-roboto-italic`"
related:
  - docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md (the settings screen, whose copy this renders)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (L3; replies may use this later)
---

# A small Markdown renderer for the app, with links and no HTML

## 1. Goal

The settings screen's explanations need a little formatting (a bold
word, a setting's name as code, a list, a link to Android's
documentation), and the copy should live in one Markdown file, not in
code. After this plan, the app renders basic Markdown with links, and the
settings copy comes from `SETTINGS_COPY.md`, checked at build time.

## 2. Decisions

1. **One small crate,** `crates/handlr-markdown`, with no drawing. It uses
   `pulldown-cmark` 0.13 with default features off (CommonMark, no GFM),
   as libtypedmd_pulldown does.
2. **What renders:**
   - paragraphs;
   - **bold**, *italic*, `code`;
   - links `[text](url)` and bare `http(s)://` URLs, by the same rules as
     the app's link spans;
   - bullet and numbered lists, one level of nesting;
   - headings, as bold text;
   - block quotes, dim;
   - fenced code blocks, in the terminal face.
3. **What does not:**
   - HTML, inline or block, is shown as its literal text and never
     interpreted;
   - an image is shown as its alt text, linked to its URL;
   - nothing else is supported.
4. **The interface:**

   ```rust
   pub enum Style { Regular, Bold, Italic, BoldItalic, Code }
   pub struct Run { pub text: String, pub style: Style, pub link: Option<usize>, pub dim: bool }
   pub struct Line { pub runs: Vec<Run>, pub indent: f32, pub gap_before: f32 }
   pub struct Laid { pub lines: Vec<Line>, pub links: Vec<String> }
   /// `measure(text, style)` is the text's width at the size given.
   pub fn lay_out(md: &str, width: f32, measure: &dyn Fn(&str, Style) -> f32) -> Laid;
   ```

   Words wrap at spaces. A word wider than the line breaks by character.
   A list item's lines are indented under its marker ("•" or "1.").
5. **The app draws it.** The app turns `Line`s into rows, using:
   - Roboto Regular, Bold and Italic for prose (libmsdf features
     `font-apache-roboto-bold` and `font-apache-roboto-italic`; BoldItalic
     falls back to Bold);
   - the terminal face for `Code`.

   Links are drawn and tapped like the agent view's link spans. They open
   through `ctx.open_link` once libmorph 0.4.0 is in, and are inert
   before.
6. **The copy file,** `apps/companion/SETTINGS_COPY.md`, holds one
   `# [Copy] <key>` object per piece of copy:
   - `title` (optional);
   - `text` (a literal block of Markdown, which may hold
     `{{ [abc] name }}` / `{{ [#] name }}` slots).

   The app's `build.rs` parses it with libtypedmd (`a61c091`, as a build
   dependency) and generates `settings_copy.rs`: one `pub const` per key,
   and a `fill(key, &[(name, value)])` for those with slots.

   The build fails when a key the code uses is missing, a slot is not
   declared, or a block does not parse.
7. **What moves into the file:**
   - all the settings screen's copy: row explanations, ⓘ texts, state
     lines, confirm texts, and "needs an app update";
   - the foreground service's notification templates (CHANGES.md, the
     entries of the same time).

## 3. Steps

1. **`crates/handlr-markdown`:** the parser and layout, added to the
   workspace in this commit.
   **Done when:** tests lay out each supported element, show HTML as
   literal text, wrap and break long words, indent lists, and collect
   links in order.
2. **The app:** the drawing of `Laid`, the bold and italic faces, the copy
   file and its `build.rs`, and the settings screen reading its copy from
   it.
   **Done when:**
   - app tests draw a paragraph with bold, italic, code and a link, and a
     tap on the link reports it;
   - the build fails for a copy file missing a used key (tested by a
     `build.rs` unit function);
   - the settings screen shows the same copy as before, now from the
     file.

Each step is one crate or subproject, one commit (Rule 3.1). 2 needs 1.

## 4. Open questions for the owner

None.

## Amendment — 2026-10-24 12:59 EDT: approved, to be built now

The owner approved the plan and its building on 2026-10-24 ("yes plan
and impl keep it simple"). The status becomes `In implementation` with
step 1's commit.
