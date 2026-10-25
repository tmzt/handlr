---
title: Navy overview screen theme, saturated session palette and custom panel shader
date: 2026-10-25
status: Draft
covers:
  - "The default theme for the companion app becomes Dark (navy), placed first in `themes.toml` with the exact palette extracted from the overview screen mockup"
  - "The 16 session colors upgraded from pastels to saturated vibrant hues matching the design example, retained as session bars on the overview cards and agent view header"
  - "The overview screen adapted to the design mockup: top-left flankr title, centered metadata line, header divider, section header (OVERVIEW · 2-COLUMN FIXED TAB ORDER), and cards with status badges, choice pills, monospace command containers, and footer metadata lines"
  - "A custom WGSL shader (`PanelShader`) in `apps/companion` that renders the navy background panels with signed distance fields, anti-aliased borders and subtle gradients, layered beneath `libmsdf`'s load pass"
sources:
  - "Owner (2026-10-25): \"use this image for the default theme for the overview screen, adapt to the rendered UI, use a custom shader to recreate the navy background panels. /var/folders/69/550v__6j6ms9qwdtbw8nsncm0000gn/T/herdr-clipboard-images-501/client-236-clipboard-1792955812932620000-0.png\""
  - "Owner (2026-10-25): \"the overview color scheme is different than the session screens; either the color outlines should represent the session color, or the bars should be retained, but the more saturated version in that example instead of the pastel\" -> \"(Recommended) Retain session bars, updated to saturated vibrant colors matching the example; keep outlines for state alerts (asking, broke)\""
  - "The overview screen mockup (client-236-clipboard-1792955812932620000-0.png, 2546 × 1002 RGBA): dark navy background #0A0A1B / #0F0F25, card surface #1C1C3E, border #2C2C51, asking outline #94A7F9, broke outline #EC677C, working status #86EAA1, text #F4F2F9, muted text #A09FBD / #8584A5, code container #151530, choice pills #2E2E57"
related:
  - docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md (the overview home screen)
  - docs/design/20261024_05_THEMES_PLAN.md (themes.toml and theme system)
  - docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md (session bands and agent view)
---

# Navy overview screen theme, saturated session palette and custom panel shader

## 1. Goal

The companion app's home (overview) screen is styled after the high-fidelity navy design mockup:
deep navy surfaces, crisp typography, clean 2-column agent cards with state indicators (asking in periwinkle, working in vibrant emerald, finished in muted gray, broke in coral red), inactive choice pills for interactive questions, code containers for running commands, and footer metadata lines.

The session colors are upgraded from pastel tones to saturated, vibrant hues matching the design aesthetic, and are retained as colored bars on each card and under the agent view header. Card outlines remain reserved for urgent state alerts (asking, broke).

The navy background panels are rendered using a custom WGSL shader (`PanelShader`), taking advantage of GPU-accelerated signed distance fields and anti-aliased borders, composited with `libmsdf`'s glyph and UI overlay pass.

## 2. Decisions

1. **`dark_navy` is the default theme.**
   It is moved to the first entry in `apps/companion/themes.toml`. The first theme in `themes.toml` sets every role.
   - Surface roles:
     - `bg = "#0A0A1B"` (window background, header)
     - `header = "#0A0A1B"`
     - `card = "#1C1C3E"` (card surface)
     - `band = "#2C2C51"` (neutral card border, dividers)
     - `input = "#151530"` (code snippet pill container)
     - `overlay = [0.0, 0.0, 0.0, 0.65]`
   - Text roles:
     - `text = "#F4F2F9"` (primary white)
     - `fg = "#F4F2F9"`
     - `dim = "#A09FBD"` (subtitles, footers, choice text)
     - `muted = "#8584A5"` (section label, secondary markers)
     - `code = "#A09FBD"`
     - `link = "#94A7F9"`
     - `spoken = "#94A7F9"`
   - State & alert roles:
     - `accent = "#FFB340"` (amber approval badge/outline)
     - `question = "#94A7F9"` (periwinkle asking outline and status)
     - `green = "#86EAA1"` (working status badge)
     - `red = "#EC677C"` (broke outline and status)
     - `yellow = "#F5DC6A"`
     - `warn = "#FF8A50"`
     - `recording = "#FF5555"`
   - Button & pill roles:
     - `approve = "#1C4E30"`
     - `deny = "#5E1E28"`
     - `message = "#252E58"`
     - `back = "#2C2C51"`
     - `button = "#2E2E57"` (choice pills, secondary buttons)
     - `toggle = "#256B3E"`
   - Chrome:
     - `scrollbar = [0.55, 0.60, 0.75, 0.4]`
     - `placeholder = [0.95, 0.95, 0.98, 0.08]`
     - `shimmer = [0.95, 0.95, 0.98, 0.015]`
     - `shadow = [0.0, 0.0, 0.0, 0.6]`

2. **Saturated session palette.**
   The 16 session colors in `dark_navy` (and dark themes) are upgraded from pastels to saturated, rich hues that read clearly on `#1C1C3E`:
   - `session_1 = "#FF4D4D"` (saturated red)
   - `session_2 = "#D62839"` (deep crimson)
   - `session_3 = "#FF7A00"` (saturated orange)
   - `session_4 = "#D95700"` (deep amber)
   - `session_5 = "#FFD13B"` (vibrant yellow)
   - `session_6 = "#C49A00"` (gold)
   - `session_7 = "#2DD4BF"` (vibrant teal)
   - `session_8 = "#0F766E"` (deep teal)
   - `session_9 = "#38D9A9"` (emerald green)
   - `session_10 = "#1B9E77"` (forest green)
   - `session_11 = "#5C7CFA"` (periwinkle blue)
   - `session_12 = "#3B5BDB"` (deep cobalt)
   - `session_13 = "#9775FA"` (violet)
   - `session_14 = "#7048E8"` (deep violet)
   - `session_15 = "#F783AC"` (vibrant pink)
   - `session_16 = "#D6336C"` (magenta)

3. **Session bars retained; outlines for state alerts.**
   - Each overview card retains its top horizontal bar in the session's color (`band`, height 4 pt, radius 2 pt).
   - The agent view screen retains its `SESSION_BAND` under the header.
   - Card outlines are reserved for state alerts:
     - An agent needing approval gets an amber outline (`accent`, 2 pt).
     - An agent asking a question gets a periwinkle outline (`question` `#94A7F9`, 2 pt).
     - A broken agent gets a coral red outline (`red` `#EC677C`, 2 pt).
     - Working and idle/finished agents have neutral borders (`band` `#2C2C51`, 1 pt).

4. **Overview UI layout adaptations.**
   - **Header**:
     - Left-aligned title `flankr` in bold Roboto.
     - Centered subtitle `tailnet · <host> · <n> agents` in `dim` (`#A09FBD`).
     - Right gear button.
     - Full-width horizontal divider line at the foot of the header (`#2C2C51`).
   - **Section header**:
     - `OVERVIEW · 2-COLUMN FIXED TAB ORDER` in `muted` (`#8584A5`), spaced uppercase.
   - **Agent cards**:
     - Header line: `Agent in project, tab <id>` at left, status pill/text at right (`asking` in `#94A7F9`, `working · <duration>` in `#86EAA1`, `finished` in `#A09FBD`, `broke` in `#EC677C`).
     - Asking cards: Prompt question text + rounded inactive choice pills (`yes → "..."`, `no → "..."`) in `#2E2E57` pill background with `#A09FBD` text.
     - Working cards: Monospace command pill (`Running cargo test ...`) in `#151530` background with `#2C2C51` border; footer metadata line (`slot #... · ... · rev ...`).
     - Broke cards: Error message, footer metadata (`tap to inspect transcript`).
     - Finished cards: Summary line, footer metadata (`spoken summary cached`).

5. **Custom WGSL shader (`PanelShader`).**
   - File: `apps/companion/src/overview/panel_shader.rs` and `apps/companion/src/overview/panel.wgsl`.
   - Compiles a `wgpu::RenderPipeline` via `ctx.device()`.
   - Vertex shader emits a fullscreen triangle without vertex buffer allocation.
   - Uniform buffer holds viewport dimensions, scale, background color (`#0A0A1B`), and panel records (bounding rect, corner radius, surface color, border color, border thickness).
   - Fragment shader evaluates exact signed distance to rounded boxes (`sd_rounded_box`), applying anti-aliased edge falloff via `smoothstep` and subtle lighting.
   - In `MorphApp::render`:
     - When `shown == Shown::Overview`, `PanelShader::draw` renders the background and card panels to `frame.view` with `wgpu::LoadOp::Clear(theme().bg)`.
     - `self.sdf.render_full_scaled_load` renders `self.list` (text, choice pills, outlines, and buttons) on top with `wgpu::LoadOp::Load`.
     - Non-overview screens render with `self.sdf.render_full_scaled` as before.

## 3. Steps

1. **`themes.toml` default theme and saturated session palette.**
   Place `dark_navy` first in `apps/companion/themes.toml` with the complete extracted palette and saturated `session_1..16` colors. Keep `dark_gray` and subsequent themes valid.
   **Done when:** `cargo test --manifest-path apps/companion/Cargo.toml` passes `themes_check`, and `dark_navy` is the active default theme.

2. **Custom WGSL panel shader (`PanelShader`).**
   Add `apps/companion/src/overview/panel.wgsl` and `panel_shader.rs`. Wire into `App::render` in `apps/companion/src/lib.rs` using `render_full_scaled_load` over the shader's pass.
   **Done when:** the shader pipeline compiles on desktop and Android without wgpu validation errors, rendering anti-aliased navy panels.

3. **Overview UI layout adaptations.**
   Update `apps/companion/src/overview.rs`, `card_grid.rs`, and `agent_card.rs` to render the header divider, the `OVERVIEW · 2-COLUMN FIXED TAB ORDER` label, right-aligned status labels, choice pills, command pills, and footer metadata.
   **Done when:** the overview screen matches the design layout, card texts wrap properly within 2 columns, and existing touch hits and scroll interactions function cleanly.

4. **Verification and Android build.**
   Run formatting, linter, tests, and Android apk build.
   **Done when:** `cargo fmt -p companion -- --check`, `cargo clippy -p companion --all-targets`, and `cargo test` pass with 0 warnings, and `./apps/companion/scripts/android.sh --install` builds and runs cleanly on the device.
