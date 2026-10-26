---
title: Markdown detection in agent output and automatic switch to rich rendering
date: 2026-10-26
status: Implemented
covers:
  - "Heuristic detection of Markdown formatting in agent output text and conversation turns"
  - "A scoring metric computing Markdown feature density and character coverage against a configurable percentage threshold"
  - "Automatic switching between plain terminal line wrapping and `flankr-markdown` rich rendering in the companion app"
  - "Interactive Markdown link handling and styled code block rendering within agent views and conversation overlays"
sources:
  - "Owner (2026-10-26): \"Add a plan to detect markdown usage above a certain percent in output text and switch to markdown rendering\""
  - "docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md (the basic CommonMark parser and layout into Laid)"
  - "apps/companion/src/agent.rs (Row, lay_out_block, lay_out_card) and apps/companion/src/overview/turn_log.rs"
related:
  - docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md
  - docs/design/20261024_03_GENERAL_AGENT_CHAT_PLAN.md
---

# Markdown detection in agent output and automatic switch to rich rendering

## 1. Goal

Coding agents (Claude, agy) and conversational assistants (Gemini) emit text
ranging from raw terminal commands and stdout traces to structured Markdown
summaries, bulleted plans, and fenced code blocks. Currently, agent views lay out
all output as plain text terminal rows with hard wrapping and ad-hoc link span
detection.

This plan adds a lightweight, fast heuristic Markdown detector to
`flankr-markdown` that measures the percentage of Markdown usage in output text.
When Markdown content exceeds a specified threshold (default 15%), the companion
app automatically switches rendering from plain terminal rows to rich CommonMark
layout (`flankr-markdown::lay_out`): proportional prose in Roboto Regular, Bold,
and Italic, bullet and numbered lists with indentation, blockquotes, styled
fenced code blocks in the terminal font, and clickable links. Below the
threshold, text remains in the fast terminal row layout.

## 2. Decisions

1. **Lightweight detector in `crates/flankr-markdown`:**
   - The detector runs before layout as a linear scan over the text without
     requiring a full AST parse.
   - It identifies structural and inline Markdown features:
     - Fenced code blocks (` ``` ` or `~~~`);
     - ATX headings (`# `, `## `, `### `, etc.);
     - Bullet list markers at start of line (`- `, `* `, `+ `) and ordered list
       markers (`1. `, `2. `);
     - Blockquotes (`> `);
     - Inline code spans (`` `...` ``);
     - Bold and italic emphasis spans (`**...**`, `*...*`, `__...__`, `_..._`);
     - Markdown links (`[text](url)`).

2. **The Markdown Scoring Metric (`MarkdownScore`):**
   ```rust
   pub struct MarkdownScore {
       /// Non-whitespace characters belonging to recognized Markdown syntax
       /// or formatted spans.
       pub formatted_chars: usize,
       /// Total non-whitespace characters in the text.
       pub total_chars: usize,
       /// Number of lines with structural Markdown indicators (headings, lists,
       /// quotes, fences).
       pub structural_lines: usize,
       /// Total non-empty lines in the text.
       pub total_lines: usize,
       /// Whether the text contains at least one fenced code block.
       pub has_fenced_code: bool,
   }

   impl MarkdownScore {
       /// Ratio of formatted characters to total characters (0.0 to 1.0).
       pub fn char_ratio(&self) -> f32;

       /// Ratio of structural lines to total lines (0.0 to 1.0).
       pub fn line_ratio(&self) -> f32;

       /// Whether Markdown usage exceeds `threshold_pct` (e.g. 15.0%).
       pub fn is_markdown(&self, threshold_pct: f32) -> bool;
   }
   ```
   - **Threshold evaluation:** Text is classified as Markdown when:
     1. `has_fenced_code` is true (fenced code blocks are a definitive signal
        of Markdown intent); OR
     2. `char_ratio() * 100.0 >= threshold_pct` (default: 15.0%), provided
        `total_chars >= 20`; OR
     3. `line_ratio() >= 0.25` (at least a quarter of the lines are list items,
        headings, or quotes) and `total_lines >= 3`.

3. **Avoiding false positives in terminal output:**
   - Glob patterns (e.g. `ls -l *.rs`), math expressions (`2 * 3 = 6`), CLI
     flags (`--flag`, `-v`), horizontal rules, or shell redirects (`cat < in > out`)
     must not falsely trigger Markdown rendering.
   - Asterisks and underscores only count as emphasis if they follow CommonMark
     flanking rules (not preceded/followed by whitespace or punctuation in
     disallowed ways).
   - Indented code blocks (4 spaces) without fences do not count towards the
     score to avoid misclassifying indented terminal output.

4. **Public detection API:**
   ```rust
   pub const DEFAULT_MARKDOWN_THRESHOLD_PCT: f32 = 15.0;

   /// Computes the Markdown usage metrics for the given text.
   pub fn score_markdown(text: &str) -> MarkdownScore;

   /// Returns whether the text exceeds the given Markdown threshold percentage.
   pub fn is_markdown(text: &str, threshold_pct: f32) -> bool;
   ```

5. **Rendering switch in the companion app (`AgentScreen`):**
   - In `apps/companion/src/agent.rs`, each block or card (`AgentCard`,
     `Section::Reply`, `Section::Body`) checks `is_markdown`:
     - **When true:** The block is laid out with `flankr_markdown::lay_out(text, width, &measure)`.
       - Prose runs use Roboto Regular, Bold, and Italic (`libmsdf` fonts).
       - Code runs and fenced code blocks use the terminal monospace font.
       - Fenced code blocks are drawn with a distinct slightly darker shaded
         background panel and padding.
       - Markdown links (`Laid::links`) are collected into `flow.targets` so
         existing tap handling and `ctx.open_link` work unchanged.
     - **When false:** The block uses the existing monospace `wrap_hard` row
       layout and `link_spans`.

6. **Rendering switch in General Agent Turn Log (`TurnOverlay`):**
   - In `apps/companion/src/overview/turn_log.rs`, Gemini turns exceeding the
     threshold render via `flankr_markdown::lay_out`, giving conversational
     replies formatted paragraphs, bulleted lists, and readable code blocks.

7. **Memoization & Performance:**
   - The Markdown detection result (`is_markdown`) is computed once when a
     section or turn is received/updated and stored alongside the cached block
     layout, avoiding re-calculation on scrolling or animation ticks.

## 3. Steps

1. **`crates/flankr-markdown` detector and scoring:**
   Add `score_markdown` and `is_markdown` to `flankr-markdown` with full
   CommonMark flanking checks and feature extraction.
   **Done when:** Comprehensive unit tests verify:
   - Fenced code blocks trigger Markdown;
   - Lists, headings, quotes, bold, italic, and links accumulate formatted
     characters and calculate accurate percentages;
     - Common terminal outputs (`ls -la *`, git status, diff traces, compiler
       errors) score below the threshold and return false;
     - Text exceeding 15% formatted characters returns true.

2. **App drawing of Markdown blocks in `apps/companion`:**
   Add support in `apps/companion/src/agent.rs` for rendering `flankr_markdown::Laid`
   within `lay_out_block` and `lay_out_card`.
   **Done when:**
   - An agent card with Markdown usage above 15% renders with styled runs
     (bold, italic, code blocks) and indented list items.
   - Links inside the Markdown layout are tapped and opened identically to
     terminal link spans.
   - Plain text cards below 15% continue to render as terminal rows.

3. **General Agent chat overlay Markdown support:**
   Integrate `is_markdown` and `lay_out` into `TurnLog` and `TurnOverlay`.
   **Done when:**
   - Gemini turns containing Markdown lists or code blocks render formatted
     in the overlay sheet.
   - Plain text turns render in standard conversational typography.

## 4. Open questions for the owner

None.

## Amendment — 2026-10-26 12:58 EDT: approved, implementation has started

The owner approved the plan on 2026-10-26 ("approve and implement"). Implementation has started with step 1 in `crates/flankr-markdown`; this document is now immutable except on explicit instruction.

## Amendment — 2026-10-26 13:25 EDT: implemented

All steps of the plan are implemented:
- Step 1: `crates/flankr-markdown` detection heuristics, scoring metric, and unit tests (`690eeaf`).
- Step 2: `apps/companion` agent card automatic Markdown rendering with styled runs, fenced code panel backgrounds, and interactive link spans (`e15cbd3`).
- Step 3: `apps/companion` General Agent conversation turn log (`TurnLog` / `TurnOverlay`) automatic Markdown rendering (`e15cbd3`).

