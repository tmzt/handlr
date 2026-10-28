---
title: Scanning flankr pair's QR code in the pairing window
date: 2026-10-28
status: Draft
covers:
  - "The companion app's pairing window redesigned around a camera
    viewfinder: a placeholder with **Tap to scan** when it opens, the
    camera started (and CAMERA asked for, the first time) only on that tap,
    libmorph's wgpu `Viewfinder` drawn full-bleed while scanning, and
    libmorph's QR scan (`start_qr`, auto zoom, exposure for screens)
    reading `flankr pair`'s code"
  - "What a scanned code does: only a `flankr://<host>:<port>/pair#…` link
    that `scan::read` accepts becomes `Input::Pair`; anything else is
    ignored with a short note while scanning goes on"
  - "The hand-over to the six pairing words (the camera stops), pinch zoom
    as the app's own gesture calling `zoom_by`, the refusal states
    (the reason; **Open Settings** only when `settings_only`), the paste
    and device-code fallbacks, Back, Cancel, suspend and resume"
  - "Desktop and web builds: no camera, today's paste screen"
  - "Moving the app (and flankr-audio) to libmorph 0.5.0 with
    `mobile-camera-qr`, and the scripted-camera tests"
  - "Not covered: anything on the daemon or in `flankr pair` (unchanged);
    the pairing protocol (plan 20261028_01's amendments of 13:59-14:47
    EDT); a desktop webcam scan (libmorph's camera view plan, on Hold)"
sources:
  - "Owner (2026-10-28, through the lead, verbatim): \"plan for the
    pairing window to add the QR capture\""
  - "Owner (2026-10-28, through the lead, verbatim): \"We could have the
    viewfinder as a placeholder with a like a tap to scan. Then that would
    activate the permissions\""
  - "Owner, recorded in plan 20261028_01: \"likely path is immediate qr
    scan then confirm on Mac\" and \"app is too noisy\" (14:42 EDT
    amendment); \"were redesigning the pairing screen around the camera, it
    should probably be a details or similar button\" (14:03 EDT amendment)"
  - "Owner's camera decisions, recorded in libmorph's and libmobile's camera
    plans (14:46 EDT amendments): CAMERA asked only when the camera is
    used, never at launch; a refusal fails with the reason; Settings only
    through `open_app_settings` when Android will not ask again; pinch zoom
    is the app's gesture calling `set_zoom`/`zoom_by`; no Jetpack; a wgpu
    viewfinder"
  - "flankr at `d9873bb` (branch `main-terminal`):
    `apps/companion/src/{lib, ui, remotes}.rs`,
    `apps/companion/{Cargo.toml, build.rs, SETTINGS_COPY.md}`,
    `crates/flankr-companion/src/{client, scan}.rs`,
    `crates/flankr-audio/{Cargo.toml, src/lib.rs}`,
    `crates/qrcodeurl/src/lib.rs`"
  - "libmorph branch `wt-camera-api` at `ba4ab6c` (worktree
    `~/src/libmorph/.claude/worktrees/camera-api`, read only):
    `src/camera.rs`, `src/camera/{types, viewfinder, convert,
    scripted}.rs`, `src/context.rs`, `src/app.rs`, `src/script.rs`,
    `Cargo.toml`"
  - "libmorph `docs/design/20261028_01_CAMERA_CAPTURE_AND_QR_PLAN.md`
    (In implementation) and libmobile
    `docs/design/20261028_01_CAMERA_CAPTURE_AND_QR_PLAN.md`
    (In implementation; 0.3.1 at `9bd2342`)"
related:
  - "docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md (In implementation):
    the pairing flow this window drives, and the quiet phone of its 14:42
    EDT amendment (\"the camera while scanning; then the six pairing
    words…\"), whose camera part this plan builds. That plan is not
    amended (decision 16)"
  - "docs/design/20261022_06_QRCODEURL_PLAN.md: the QR code `flankr pair`
    prints, in the terminal and as an inline image"
  - "docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md and
    docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md: `SETTINGS_COPY.md`,
    where the pairing copy lives"
  - "libmorph docs/design/20261014_01_CAMERA_VIEW_AND_CAPTURE_PLAN.md
    (Hold): desktop and web camera backends, if ever built, would give the
    desktop window a scan too"
---

# Scanning flankr pair's QR code in the pairing window

## 1. Goal

The owner runs `flankr pair` on the Mac, opens the pairing window on the
phone, taps **Tap to scan**, points the phone at the Mac's screen, and the
phone goes straight to the six pairing words; the owner types **yes** on
the Mac. Pasting a link stays possible but is no longer the first thing
shown. The window stays quiet: a viewfinder, one line, and the buttons the
moment needs.

## 2. Findings

### 2.1 The pairing window today (flankr)

1. **One screen, two modes.** `apps/companion/src/ui.rs` is a libcontrols
   `Router` (`ui.rs:1-17`): title and two status lines at the top; at the
   bottom the field ("paste a pairing URL", `ui.rs:366-367`) and **Pair**
   (`ui.rs:514-570`); above them the pairing area of up to 20 lines
   (`CODE_LINES`, `ui.rs:28-29`; placed at `ui.rs:572-600`). While a
   pairing waits, **Details** and **Cancel** replace the field and Pair
   (`ui.rs:546-570`). `View` (`ui.rs:300-321`) carries `waiting`,
   `details` and `adding`; `Hit` (`ui.rs:323-336`) is `Field`, `Pair`,
   `Cancel`, `Details`, `Back`.
2. **When it shows.** `App::pairing()` (`lib.rs:1862-1866`): the add mode,
   a waiting pairing, or neither paired nor online. `render` picks
   `Shown::Pairing` for the add mode first and for the unpaired, offline app
   (`lib.rs:2767-2773`). So it is the **first-run** screen and the screen
   **+** in Settings → Remotes opens (`SettingsHit::AddRemote`,
   `lib.rs:1208`; `open_add`, `lib.rs:1286-1296`).
3. **Its state.** `remotes::Pairing` (`remotes.rs:80-143`): `adding`,
   `waiting: Option<Waiting>`, `details`; `open_add`, `show` (from
   `Event::PairingWords`), `toggle_details`, `cancel` (gives
   `Input::CancelPairing`), `paired`, `failed`. `Pairing::lines`
   (`remotes.rs:145-182`) draws the quiet screen: the six words large and
   one line (`pairing_confirm`), or Details (fingerprint words,
   `host:port`, time left, the device-code fallback; `remotes.rs:220-288`).
4. **Every pairing input goes through `App::open_url`** (`lib.rs:559-584`):
   the field's **Pair** (`App::pair`, `lib.rs:1856-1860`), desktop
   `--pair`, `pair.txt` pushed into the data directory (`read_pair_file`,
   `lib.rs:1344-1360`, read on start, on opening the add mode and on each
   resume in it, `lib.rs:2229-2232`), and an opened `flankr://` link
   (`on_open_url`, `lib.rs:2307-2321`, which accepts only
   `flankr://<host:port>/pair` before the fragment). `open_url` opens the
   add mode when a remote is already paired, sends `Input::Pair(text)` and
   notes "pairing…".
5. **The client's side.** `Input::Pair(String)` and
   `Input::CancelPairing` (`crates/flankr-companion/src/client.rs:30-48`);
   `Event::PairingWords`, `Paired`, `PairFailed`
   (`client.rs:502-538`). `flankr_companion::scan::read`
   (`scan.rs:30-56`, public: `lib.rs:29`) parses the URL, the bare
   `flankr1…` string or a QR data URL and refuses a payload with no
   addresses or an unknown version.
6. **Touches and keys.** On the pairing screen only a touch's `Ended`
   counts (`lib.rs:1938-1945` for the add mode, `lib.rs:1984-1987`
   otherwise), routed to `pairing_tap` (`lib.rs:1990-2003`). The system
   Back in the add mode cancels the pairing (`on_key`, `lib.rs:2637-2644`);
   elsewhere on the pairing screen Back and Escape clear the field
   (`lib.rs:2708-2711`).
7. **Lifecycle.** `on_suspend`/`on_resume` (`lib.rs:2204-2235`) send
   `Input::Background` and re-read `pair.txt` in the add mode. No camera
   anywhere.
8. **Drawing.** The pairing screen is drawn with libmsdf's
   `render_draw_list_in` (`lib.rs:2846-2856`), which renders through
   `render_full_scaled` and **clears the target to black**
   (libmsdf `4de319c`, `src/gpu/mod.rs:612-640`, `:895`). The overview
   already draws something of its own first (the panel shader) and then
   the SDF list with `render_full_scaled_load`, which keeps what is there
   (`lib.rs:2815-2845`; libmsdf `src/gpu/mod.rs:902-940`). The viewfinder
   needs the same two-pass shape.
9. **The copy.** The pairing keys are in `apps/companion/SETTINGS_COPY.md`
   (`remote_add` at `:436`, `pairing_confirm` `:444`, `pairing_fallback`
   `:456`, `pairing_fallback_no_words` `:464`, `pairing_paired` `:471`,
   `pairing_details`/`_hide` `:481-488`, `pairing_fingerprint` `:489`,
   `pairing_address` `:493`, `pairing_expires`/`_expired` `:500-510`,
   `pairing_no_words` `:511`, `pairing_paste` `:517`, `pairing_replace`
   `:529`), compiled into `copy::*` by `build.rs` (`settings_copy`, which
   also checks the keys `src/` uses).
10. **Dependencies.** The app pins libmorph at `0b288e8` (0.4.2 plus text
    input) with `secure-store` on every host and, on Android, `audio`,
    `mobile`, `mobile-foreground-service`, `mobile-system-events`,
    `mobile-battery` (`apps/companion/Cargo.toml`, the `[dependencies]`
    and Android sections); `libmorph-build` at the same rev
    (`[build-dependencies]`); the desktop dev-dependency adds `script`.
    `crates/flankr-audio/Cargo.toml:22` pins the same rev, and its public
    `AudioSession::spawn_morph` takes and returns libmorph types
    (`crates/flankr-audio/src/lib.rs:203-207`). `build.rs` asks for
    `RECORD_AUDIO` and `INTERNET` and registers the `flankr` URL scheme.
11. **The code on the Mac.** `flankr pair` prints the URL as a terminal QR
    code in Unicode half blocks with a 4-module quiet zone
    (`crates/qrcodeurl/src/lib.rs:1-21`, `Dense1x2`), or as an inline kitty
    or iTerm2 image of the PNG.

### 2.2 What libmorph's camera gives (`wt-camera-api`, `ba4ab6c`)

1. **The API** (`src/camera.rs:1-42`): `ctx.camera()`
   (`src/context.rs:295-297`) returns `Camera` with `start_qr(&QrOptions)
   -> ScanId` (`camera.rs:220-247`), `start_preview`, `stop(scan)`
   (`:279-290`), `preview(scan) -> Option<PreviewFrame>` (`:296`),
   `zoom_range`, `zoom`, `set_zoom`, `zoom_by(scan, factor)`
   (`:311-360`). Calls never block; answers come to
   `MorphApp::on_camera(ctx, &CameraEvent)`, which has a default body
   (`src/app.rs:153-161`).
2. **Events** (`src/camera/types.rs:113-151`): `Opened { scan, facing,
   size, zoom_range }`, `Closed { scan, reason: Option<CameraError> }`
   (`None` for `stop`, a replacing session or a `Once` scan that read its
   code), `Suspended { scan }` (closed on pause, reopened on resume with
   no event until `Opened`), `Zoom { scan, ratio, source: App | Auto }`,
   `Qr { scan, code: QrCode }`. `CameraEvent` and `CameraError` are
   `#[non_exhaustive]`.
3. **Errors** (`types.rs:53-79`): `PermissionDenied { settings_only }`
   (`settings_only` when asking again shows no dialog), `NoCamera`,
   `InUse`, `Disabled`, `Unsupported(&'static str)`, `Failed(String)`
   and others, each with a `Display` text (`types.rs:81-100`).
   `ctx.open_app_settings()` (`src/context.rs:299-312`) opens the app's
   page in Android's Settings; elsewhere `OpenLinkError::Unsupported`.
4. **QR options** (`types.rs:247-298`): `facing` (Back), `mode` (`Once`, or
   `Continuous`, which reports the same payload again only after a pause
   without it), `auto_zoom` (true), `exposure` (`Auto`: lower it when the
   code looks like it is on a bright screen; `Off`; `Screen`: start low
   and meter the centre), `preview` (`Some(PreviewOptions::default())`:
   640 px on the long side, upright). `QrOptions` is `#[non_exhaustive]`:
   built from `Default` and its public fields set.
   `QrCode` (`types.rs:300-315`) has `bytes`, `text: Option<String>`,
   `corners`, `version`; its payload may be a secret and is not logged.
5. **Auto zoom** (libmobile plan, section 6): zooms in on a code too small
   to read, with fine steps on API 30+; after an app zoom it pauses
   1.5 s (`QR_USER_HOLD_MS`) and returns to the app's zoom, not 1.0, when
   the code is lost.
6. **The viewfinder** (`src/camera/viewfinder.rs`): `Viewfinder::new(ctx)`
   once; `update(ctx, scan) -> bool` uploads only a new frame (`:170`);
   `draw(ctx, &mut pass, [x, y, w, h], &ViewfinderOptions)` inside the
   app's own render pass, setting its viewport (`:286-300`), one draw per
   queue submission; `clear()` (`:282`). `ViewfinderOptions { fit, mirror
   }` with `Fit::Cover` (default, crop to fill), `Contain`, `Stretch`
   (`src/camera/convert.rs:128-141`).
7. **Permission.** libmobile asks for CAMERA on the first camera call and
   keeps its own answer; it never reaches `on_permissions`. A build without
   a camera feature has no CAMERA in its manifest.
8. **Features** (`Cargo.toml:28-40`): `mobile-camera` (the base),
   `mobile-camera-capture`, `mobile-camera-qr` (on the base), and
   `mobile-camera-intent` (the old hand-off, renamed: **Breaking**,
   libmorph 0.5.0, `Cargo.toml:3`).
9. **Off Android** every call answers `Closed { reason:
   Some(Unsupported(..)) }` on the next frame (`camera.rs:236-246`).
10. **Scripted runs** (`src/script.rs`, desktop with `script`): `tap <x>
    <y>`, `touch <id> <start|move|end|cancel> <x> <y>`, `suspend`,
    `resume`, `screenshot`, and `camera-qr <text>` (the rest of the line,
    `#` included, as the running scan's code; `script.rs:196-214`),
    `camera-deny`, `camera-deny-forever` (the next camera call answers
    `PermissionDenied { settings_only: false | true }`). A scripted
    `start_qr` answers `Opened` at once with a zoom range of 1.0-8.0, zoom
    calls are reported as `Zoom` events, and `preview` returns `None`
    (`camera.rs:296-299`), so a scripted viewfinder draws nothing.
11. **Where it stands.** The branch's plan is approved and In
    implementation (its 15:37 EDT amendment); it builds against libmobile
    0.3.1 through a local, gitignored `[patch]`, and merges into libmorph
    `main` once libmobile 0.3.1 is published, as libmorph **0.5.0** (main
    is 0.4.2), with a libmorph-build release for the libmobile bump.

## 3. Decisions

### Layout

1. **The viewfinder fills the window.** While the window is open, a
   *viewfinder area* runs from the top safe area to the top of the bottom
   button row, full width, with `Fit::Cover`. The title line ("flankr", or
   "Pair another remote" with its "<" in the add mode) is drawn over the
   top of the area on a band of `theme().bg` at 70 % opacity, and the
   status lines are not shown while scanning (they say "not paired", which
   the window already says). The bottom row and the line above it sit on
   the window's background, not over the camera image.
2. **A quiet reticle.** While the camera runs, four corner marks (L
   shapes, 3 pt thick, 28 pt long, `theme().text` at 80 %) frame a centred
   square of 62 % of the area's shorter side. No dimming outside it, no
   animation, no outline of a detected code. It only shows where to aim;
   the decoder reads codes anywhere in the frame.
3. **One line under the area.** It says what to do or what happened:
   `pairing_scan_hint` while scanning, `pairing_scan_ignored` for 3 s after
   a code that is not a pairing link, the reason after a refusal or a
   failure. Nothing else is shown with the camera.

### The placeholder and when the camera starts

4. **The window opens on a placeholder; the camera starts only on a tap**
   (the owner's addition). Opening the window, from **+** or on first run,
   never starts the camera and never asks for anything. The viewfinder
   area shows a placeholder: `theme().band`, the reticle's corner marks
   dimmed (40 %), and in its middle a camera glyph (drawn from the
   reticle's primitives, no new font) above **Tap to scan**
   (`pairing_tap_to_scan`), and under the area the explanation
   `pairing_camera_why`. A tap anywhere in the area calls `start_qr`;
   libmobile then asks for CAMERA if it was never granted. This keeps the
   owner's rule (CAMERA only when the camera is used, never at launch) on
   first run too, where the pairing window is the launch screen.
5. **A camera already granted still starts on a tap (recommended).** Each
   time the window opens, it opens on the placeholder; the camera turns on
   only when tapped. One tap costs little, and the camera (and Android's
   green camera-in-use dot) is never on without the owner's gesture. The
   alternative, starting at once on later openings when
   `permission_granted("android.permission.CAMERA")` is true, is open
   question 1.
6. **The permission's explanation is the window's own line, shown before
   the tap** (`pairing_camera_why`): what the camera is for, that the
   frames are decoded in the app and kept nowhere, and that it is closed
   on leaving the window. It is the only rationale; no extra dialog before
   Android's.

### Scanning

7. **The scan:** `QrOptions` from `Default`, with `mode = Continuous`,
   `facing = Back`, `auto_zoom = true`, `exposure = ScreenExposure::Auto`,
   `preview = Some(PreviewOptions::default())` (640 px, upright).
   `Continuous`, so a code that is not a pairing link does not end the
   scan; the app stops it itself when it accepts one. `Auto` exposure
   rather than `Screen`: `Auto` lowers the exposure as soon as the code
   looks like a bright screen (the Mac's) and keeps a normal image while
   the owner aims; `Screen` starts dark. Auto zoom and exposure are on by
   default, with no setting (open question 2 asks about `Screen`).
8. **What a code does.** On `CameraEvent::Qr` for the running scan:
   - `code.text` is `None`, or it is not a pairing link by the same test
     `on_open_url` uses (moved into `remotes::is_pairing_link(&str) ->
     bool` and used by both): **ignored**, with `pairing_scan_ignored`
     under the area for 3 s. The scan goes on. The code is never logged;
     the log says `pairing: scanned a code that is not a pairing link
     (<n> bytes)`.
   - A pairing link that `flankr_companion::scan::read` refuses (an
     unknown version from a newer or older `flankr pair`, no addresses):
     its reason under the area (`pairing_scan_unreadable`), the scan goes
     on.
   - A pairing link that `scan::read` accepts: **accepted**. The app calls
     `stop(scan)`, clears the viewfinder, and hands the text to
     `App::open_url`, which sends `Input::Pair(text)` exactly as a pasted
     link does. No change in flankr-companion.
   The bare `flankr1…` string and data URLs are accepted from the field
   and `pair.txt` only, not from the camera.
9. **Then the six words.** With the camera stopped, the area shows the
   placeholder's background with "pairing…" until
   `Event::PairingWords`, and then the window becomes today's quiet screen
   (`Pairing::lines`): the six words large, `pairing_confirm`, Details and
   Cancel. Details keeps the fingerprint words, `host:port`, the time left
   and the device-code fallback (plan 20261028_01, 14:42 EDT amendment).
   The camera is not shown again during the pairing.
10. **After a failure** (`Event::PairFailed`: refused, expired,
    unreachable), the window returns to the placeholder with the reason
    under the area and **Tap to scan** reading `pairing_scan_again`; as
    each `flankr pair` run has one attempt, the copy says to run it again.
    The camera does not restart by itself.
11. **Paired.** As today: `pairing_paired`, and the add mode returns to
    Settings.

### Pinch zoom

12. **Two fingers on the viewfinder zoom.** While the camera runs, the
    pairing window takes every touch phase (not only `Ended`). A new
    `apps/companion/src/scanner.rs` keeps the two touches of a pinch
    (`Pinch { a: (id, x, y), b: (id, x, y), last_distance }`); on each
    `Moved` of either it computes `d_now / last_distance` and, when it
    differs from 1 by more than 0.5 %, calls `ctx.camera().zoom_by(scan,
    factor)` and keeps `d_now`. A pinch never counts as a tap (its touches'
    `Ended` hit nothing). While the zoom changes from the app's gesture,
    the ratio ("2.4×") shows at the top right of the area for 1 s after
    the last `Zoom { source: App }`; auto zoom shows nothing. No
    double-tap, no zoom slider.

### Refusals and errors

13. **A refusal shows the reason, in the placeholder.** On `Closed {
    reason: Some(PermissionDenied { settings_only }) }` the area goes back
    to the placeholder and the line under it reads:
    - `settings_only: false`: `pairing_camera_denied`; **Tap to scan**
      stays, and a tap asks again (Android shows its dialog once more).
    - `settings_only: true`: `pairing_camera_denied_settings`, and the
      bottom row gains **Open Settings** (`pairing_open_settings`), which
      calls `ctx.open_app_settings()`. A tap on the area still calls
      `start_qr` (Android will answer at once with the same refusal, so
      nothing changes until the permission is granted in Settings); on
      `on_resume` the window returns to plain **Tap to scan** if
      `permission_granted` now says CAMERA is granted.
    Other errors (`NoCamera`, `InUse`, `Disabled`, `Failed`) show
    `pairing_camera_failed` with the error's own `Display` text; **Tap to
    scan** stays. `Unsupported` cannot happen on Android with the feature
    and is treated as decision 17's no-camera window.
14. **The paste fallback** stays one tap away: while the placeholder or
    the camera shows, the bottom row is **Paste a link** (left) and, in the
    add mode, **Cancel** (right); on first run **Paste a link** has the row
    to itself. **Paste a link** stops the camera if it runs and shows
    today's field and **Pair** in that row, with the area back on the
    placeholder (tapping it returns to scanning and hides the field).
    `pair.txt`, opened `flankr://` links and desktop `--pair` work at any
    time as today; one arriving while the camera runs stops it first. The
    device-code fallback stays in Details once a pairing waits (unchanged).

### Lifecycle

15. **The camera runs only while the window shows it.** It is stopped by:
    an accepted code, **Paste a link**, Cancel, the add mode's "<", the
    system Back (which, while scanning on first run, returns to the
    placeholder instead of clearing the field), any `open_url`, and the
    window being replaced (paired, or Settings opened over it). On
    `on_suspend` the app does nothing itself: libmobile closes the camera
    and reports `Suspended`; the app keeps `scanning` and on `Opened`
    after the resume the viewfinder continues. If the window was left
    while suspended (a link opened the app elsewhere), the app calls
    `stop`. `Closed { reason: None }` that the app did not ask for (never
    expected with `Continuous`) returns to the placeholder.
16. **State.** `scanner.rs` holds `Scanner { state: Idle | Starting(ScanId)
    | Running(ScanId) | Denied { settings_only } | Failed(String), pinch:
    Option<Pinch>, zoom_shown_until: Option<Instant>, note: Option<(String,
    Instant)> }`, owned by `App` beside `pairing`, with pure methods
    (`tap`, `on_event(&CameraEvent) -> Action`, `touch`) returning what the
    app should do (`Action::Start`, `Stop(ScanId)`, `ZoomBy(ScanId, f32)`,
    `Pair(String)`, `None`), so it is unit-tested without a camera.
    `App::on_camera` routes events; `App::update` calls
    `viewfinder.update(ctx, scan)` while `Running`.

### Drawing

17. **The viewfinder pass.** `App` keeps one `Viewfinder`, made in `init`
    for the surface format. When the pairing window shows the camera,
    `render` begins one render pass that clears to `theme().bg`, calls
    `viewfinder.draw` into the area's rectangle (in surface pixels), ends
    the pass, and then draws the window's draw list with
    `render_full_scaled_load` (the overview's pattern, `lib.rs:2829-2845`),
    so the SDF text and buttons land over the image. The window's draw
    list has no full-screen background rectangle (it has none today, as
    the clear provides it). The window is redrawn every frame while the
    camera runs (15 fps, `FPS`), as the viewfinder changes even when the
    controls do not.

### Hosts without a camera

18. **Desktop and web builds show today's paste screen.** The app knows at
    build time: the viewfinder area, **Tap to scan** and the camera copy
    exist only under `cfg(target_os = "android")`; elsewhere the window is
    exactly today's field and **Pair** (and the six words, Details and
    Cancel while a pairing waits). Scripted desktop tests opt in to the
    Android layout through a test-only constructor flag
    (`App` built with `camera: true` under `cfg(test)` and scripted runs
    started with the argument `--camera`), so the scanner is tested on the
    host with libmorph's scripted camera.

### Build

19. **Features.** `apps/companion/Cargo.toml`, Android section: libmorph
    gains `mobile-camera-qr` (which brings `mobile-camera`, CAMERA and
    `uses-feature android.hardware.camera required=false` into the
    manifest through libmorph-build's Gradle hand-off). Not
    `mobile-camera-capture` (no still frames) and not `mobile-camera-intent`.
    The desktop dev-dependency keeps `script`. `build.rs` needs no
    `.permission("android.permission.CAMERA")`: the feature's metadata adds
    it; step 2 checks the generated manifest.
20. **The libmorph move.** The app's three libmorph entries,
    libmorph-build, and `crates/flankr-audio/Cargo.toml:22` move together
    from `0b288e8` to the commit tagged `v0.5.0` (and libmorph-build's
    matching tag). The app uses no `mobile-camera` today, so the rename to
    `mobile-camera-intent` touches nothing.

### Copy

21. **New keys in `SETTINGS_COPY.md`**, after `pairing_replace`, with a
    lead paragraph saying they are the pairing window's camera
    (this plan). Proposed text:
    - `pairing_tap_to_scan`: "Tap to scan"
    - `pairing_scan_again`: "Tap to scan again"
    - `pairing_camera_why`: "Scan the QR code flankr pair shows. The camera
      opens only when you tap, and Android asks for the Camera permission
      the first time. Frames are decoded on the phone, never saved or sent,
      and the camera closes when you leave this screen."
    - `pairing_scan_hint`: "Point at the QR code
      flankr pair shows. Pinch to zoom."
    - `pairing_scan_ignored`: "Not a flankr pairing code."
    - `pairing_scan_unreadable` (`[abc] why`): "That flankr pairing code
      can't be used: {{ [abc] why }}."
    - `pairing_camera_denied`: "Camera permission was refused, so the code
      can't be scanned. Tap to be asked again, or paste the link instead."
    - `pairing_camera_denied_settings`: "Camera permission is refused and
      Android won't ask again. Allow it under Settings → Apps → flankr →
      Permissions → Camera, or paste the link instead."
    - `pairing_open_settings`: "Open Settings"
    - `pairing_camera_failed` (`[abc] why`): "The camera couldn't start:
      {{ [abc] why }}."
    - `pairing_paste_link`: "Paste a link"
    - `pairing_retry` (`[abc] why`), the failure line of decision 10:
      "{{ [abc] why }}. Run flankr pair again, then scan its new code."
    - `remote_add`'s text changes to: "Run flankr pair on the machine whose
      daemon you're adding, then scan its QR code. flankr stays connected to
      the remote in use meanwhile." (Copy only; the key stays.)
    The desktop keeps `remote_add`'s paste wording through a
    `remote_add_paste` key with today's text, used where there is no
    camera.

### Relation to the multiple-remotes plan

22. **This plan carries the camera; plan 20261028_01 is not amended.** Its
    14:42 EDT amendment already puts "the camera while scanning" first on
    the quiet phone, and its 14:03 EDT amendment says the window is to be
    redesigned around the camera with Details carried over. Nothing here
    changes its flow, its events or its Details; the placeholder and the
    tap before the camera (the owner's addition here) come before the
    "camera while scanning" that plan describes. The README rows stay
    separate.

## 4. Dependencies and order

1. **libmobile 0.3.1** published (pushed, tagged) — `9bd2342`, tagged
   locally.
2. **libmorph `wt-camera-api` merged into `main`** with `Cargo.lock`
   re-resolved against the published libmobile and the local `[patch]`
   removed; released as **libmorph 0.5.0** (tag `v0.5.0`) with
   **libmorph-build**'s next release (libmobile 0.3.x for the camera's
   manifest metadata).
3. **flankr moves to them** (step 2 below).

**Before that**, steps 1 and 3-5 can be built and tested on the desktop
against the branch: a local, gitignored `.cargo/config.toml` in flankr
with `[patch."https://github.com/tmzt/libmorph"] libmorph = { path =
"/Users/tmeade/src/libmorph/.claude/worktrees/camera-api" }` (and
libmorph-build), plus the branch's own libmobile patch, lets the scripted
camera (`camera-qr`, `camera-deny`, `camera-deny-forever`, `tap`,
`touch`) drive the app. Those commits are not pushed until step 2 pins a
published libmorph, as nothing on the shared branch may depend on a local
patch.

## 5. Testing

### Host (desktop, scripted camera; `cargo test -p companion`)

Unit tests in `scanner.rs` (pure):
- placeholder first: a fresh `Scanner` is `Idle`; opening the window gives
  no action; a tap in the area gives `Start`;
- a `Qr` that is not a pairing link keeps `Running` and sets the ignored
  note; one with an unknown version keeps `Running` with its reason; a
  valid one gives `Stop` then `Pair(text)`;
- `PermissionDenied { false }` returns to the placeholder with the denial
  line and no Settings button; `{ true }` adds **Open Settings**; a tap
  after either gives `Start` again;
- two touches moving apart by 10 % give one `ZoomBy` of about 1.1; a pinch
  ends with no tap; a single touch gives no zoom;
- `Suspended` then `Opened` keeps `Running` with the same `ScanId`.

Scripted runs of the whole `App` (`apps/companion/tests/pairing_scan.rs`,
libmorph `script`, the window at S24 size, `--camera`):
1. **Placeholder, then tap:** start; screenshot shows **Tap to scan** and
   no scan was started (the script log has no `Opened`); `tap` in the area;
   `Opened`. Then `camera-qr https://example.com`: still scanning, the
   ignored note on screen.
2. **A real pairing URL:** a test daemon in pair mode as
   `crates/flankr-companion/tests/link.rs` builds one (`PairMode`,
   `pair_run`, `link.rs:62-160`) on 127.0.0.1 gives a URL; `tap`;
   `camera-qr <url>`; the scan is stopped, and within a few frames the
   window shows six words that equal the daemon's.
3. **Deny:** `camera-deny`; `tap`; the denial line, no **Open Settings**;
   `tap` again; `Opened`.
4. **Deny forever:** `camera-deny-forever`; `tap`; the Settings line and
   **Open Settings** (whose `open_app_settings` answers `Unsupported` on
   the host and is logged).
5. **Pinch:** `tap`; `touch 1 start`, `touch 2 start`, moves apart; the
   script's `Zoom { source: App }` events show ratios above 1.
6. **Lifecycle:** `tap`; `suspend`; `resume`; still scanning; Cancel in
   the add mode stops the scan (`Closed { reason: None }`) and returns to
   Settings.
7. **No camera:** without `--camera`, the window is today's field and
   **Pair**, unchanged (the existing `ui.rs` tests keep passing).

### On the S23 (the owner is asked first)

1. Build and install with `apps/companion/scripts/android.sh`; check with
   `aapt dump permissions` that CAMERA is in the APK.
2. First run (data cleared): the window opens with no permission dialog;
   **Tap to scan** asks for CAMERA; allow.
3. `flankr pair` on the Mac (dark terminal theme, then light; and the
   inline image in iTerm2 or kitty): scan from about 40 cm and from 1 m
   (auto zoom); the six words match `flankr pair`'s; type **yes**; "Paired
   with <name>".
4. **+** in Settings → Remotes: same flow beside the open connection.
5. Pinch zoom on a code across the room.
6. Refuse twice: the Settings line and **Open Settings** open flankr's
   page; grant there; back in the app, **Tap to scan** works.
7. Home during a scan and back: the scan resumes; Back leaves and the green
   camera dot goes off.
Evidence (screenshots, the log without payloads) under
`out/pairing/<YYYYMMDD_NN>_qr_scan/`, logged in `out/pairing/LOG.md`.

## 6. Versions (Rule 5)

- **The app** (`apps/companion`, `publish = false`, no public API): its
  manifest and the lock file only; no `VERSIONS.md` entry.
- **flankr-companion**: unchanged (`scan::read` is already public;
  `Input::Pair` takes the scanned text).
- **flankr-audio**: moving libmorph 0.4.2 → 0.5.0, whose types are in its
  public `spawn_morph` (`crates/flankr-audio/src/lib.rs:203-207`), is not
  semver-compatible, so under Rule 5.5 flankr-audio 0.1.0 → **0.2.0**,
  **Breaking**, with its first `VERSIONS.md` entry and tag
  `flankr-audio-v0.2.0` (open question 4, as Rule 5.4 applies "from 0.3.0
  on").

## 7. Steps

Each step is one commit to one crate (Rule 3.1), passing `cargo fmt -p
<crate> -- --check`, `cargo clippy -p <crate> --all-targets` (and for the
app also `--target aarch64-linux-android`) and `cargo test -p <crate>`.

1. **The scanner's state, pure** (`apps/companion/src/scanner.rs`,
   `remotes::is_pairing_link`, `on_open_url` using it). Built against the
   local libmorph patch for the camera types.
   **Done when:** the unit tests of section 5 pass.
2. **libmorph 0.5.0** (after section 4's items 1-2): flankr-audio's pin
   and its 0.2.0 entry (its own commit), then the app's pins, libmorph-build
   and `mobile-camera-qr` on Android (its own commit), the lock file.
   **Done when:** both crates build and test on the host and for
   `aarch64-linux-android`, and the generated manifest has CAMERA.
3. **The window around the viewfinder** (`ui.rs`, `lib.rs`): the
   placeholder, **Tap to scan**, the area, the reticle, the title band, the
   line under the area, **Paste a link**, **Open Settings**; `on_camera`;
   the viewfinder pass in `render`; all touch phases while scanning; Back,
   Cancel, `open_url` and suspend stopping or keeping the scan; desktop
   unchanged.
   **Done when:** scripted runs 1-4, 6 and 7 pass.
4. **Pinch zoom** (`scanner.rs`, `lib.rs`): the gesture, `zoom_by`, the
   ratio for 1 s.
   **Done when:** scripted run 5 passes.
5. **The copy** (`SETTINGS_COPY.md`): decision 21's keys and
   `remote_add`'s new text, with `remote_add_paste` for the desktop. May be
   folded into step 3's commit (same crate).
   **Done when:** `build.rs`'s copy check passes with every key used.
6. **On the S23** (the owner asked first): section 5's phone checks.
   **Done when:** all seven pass and the evidence is logged.
7. **This plan's status**: an amendment setting `Implemented` with the
   commits per step, and its README row updated, in one commit.

## 8. Risks

1. **The terminal QR in a dark theme.** `flankr pair`'s half-block code is
   light-on-dark in some terminal themes. rqrr may not read an inverted
   code. The S23 check covers both themes; if dark fails, the fix belongs
   in `qrcodeurl`'s terminal rendering (explicit colours) or in libmobile
   (trying the inverted image), each its own plan.
2. **Moiré and glare** from a Retina screen at close range. Auto exposure
   for screens and auto zoom are designed for it; the S23 check is the
   test. Open question 2 offers `ScreenExposure::Screen`.
3. **The libmobile 0.3 move** brings other changes under the features the
   app already uses (foreground service, system events, battery, secure
   store, audio). Step 2 re-runs the background and notification checks
   of the settings plan on the S23 in step 6.
4. **Frame rate.** The app runs at 15 fps; the viewfinder at 15 fps looks
   steppy but is enough to aim. Raising it only while scanning is a
   possible later change.
5. **The scripted viewfinder draws nothing** (`preview` is `None` in
   scripts), so the host tests check state and controls, not the image;
   the image is checked on the phone.
6. **Depending on an unmerged branch.** Steps 1, 3 and 4 written against
   `wt-camera-api` may need changes if the API moves before 0.5.0.

## 9. Open questions for the owner

1. **Later openings:** with CAMERA already granted, keep **Tap to scan**
   every time (recommended: the camera is never on without your tap), or
   start the camera at once when the window opens?
2. **Exposure:** `ScreenExposure::Auto` (recommended: normal image while
   aiming, lowered once a bright screen is seen), or `Screen` (starts dark,
   always metering the centre), since the code is always on a screen?
3. **The reticle:** the four corner marks of decision 2, or no reticle at
   all?
4. **flankr-audio's version:** bump it 0.1.0 → 0.2.0 (**Breaking**, Rule
   5.5) for the libmorph move, or leave it unversioned as before 0.3.0?
5. **The copy** of decision 21, in particular `pairing_camera_why`'s
   length and the Settings path wording on One UI ("Settings → Apps →
   flankr → Permissions → Camera").

## Amendment — 2026-10-28 16:39 EDT: the corner-mark reticle stays

The owner (2026-10-28), on open question 3, "keep them": the four quiet
corner marks over the viewfinder stay, as the body has them. The other open
questions are still open.
