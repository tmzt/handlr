//! The pairing window's camera (docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md,
//! decisions 4-16): what the window shows of it, and what the app should
//! ask the camera to do, as pure state with no camera, clock or context of
//! its own, so it is tested without either.
//!
//! - The window opens on a placeholder: [`State::Idle`], and nothing is
//!   started or asked for. A tap in the area gives [`Action::Start`]
//!   (`start_qr` with [`qr_options`]); libmobile asks for CAMERA then, the
//!   first time.
//! - A scanned code ([`Cam::Qr`]) that is not a pairing link
//!   ([`crate::remotes::is_pairing_link`]) is ignored, with a note for 3 s;
//!   one `flankr_companion::scan::read` refuses keeps the scan going with
//!   its reason; one it accepts gives [`Action::Stop`] and then
//!   [`Action::Pair`], which the app hands to `App::open_url` as a pasted
//!   link.
//! - Two fingers on the running viewfinder zoom ([`Scanner::touch`]): the
//!   ratio of their distance now to their distance at the last call, when
//!   it differs from 1 by more than [`PINCH_STEP`], as
//!   [`Action::ZoomBy`]. A pinch's touches never count as a tap.
//! - A refusal goes back to the placeholder with its reason
//!   ([`State::Denied`]); *Open Settings* only when Android will not ask
//!   again (`settings_only`). Other errors are [`State::Failed`] with the
//!   error's text; `Unsupported` means no camera ([`Scanner::no_camera`]),
//!   and the window falls back to the paste screen.
//!
//! The id is generic so the tests can use plain numbers: libmorph's
//! [`ScanId`] cannot be made outside libmorph.

use std::time::{Duration, Instant};

use libmorph::TouchPhase;
use libmorph::camera::{
    CameraError, CameraEvent, Facing, PreviewOptions, QrMode, QrOptions, ScanId, ScreenExposure,
    ZoomSource,
};

use crate::remotes::is_pairing_link;
use crate::settings::copy;
use crate::theme::Role;

/// How long the note for a code that is not a pairing link shows.
pub const IGNORED_FOR: Duration = Duration::from_secs(3);
/// How long the zoom ratio shows after the last zoom from a pinch.
pub const ZOOM_SHOWN_FOR: Duration = Duration::from_secs(1);
/// The least change of a pinch's distance that zooms: 0.5 %.
pub const PINCH_STEP: f32 = 0.005;

/// The scan the window starts (decision 7): continuous, so a code that is
/// not a pairing link does not end it, the back camera, auto zoom, exposure
/// for screens on `Auto`, and preview frames for the viewfinder.
pub fn qr_options() -> QrOptions {
    let mut options = QrOptions::default();
    options.facing = Facing::Back;
    options.mode = QrMode::Continuous;
    options.auto_zoom = true;
    options.exposure = ScreenExposure::Auto;
    options.preview = Some(PreviewOptions::default());
    options
}

/// Where the camera is.
#[derive(Clone, Debug, PartialEq)]
pub enum State<Id> {
    /// The placeholder: nothing runs.
    Idle,
    /// `start_qr` was called; `Opened` has not come.
    Starting(Id),
    /// The viewfinder shows (and stays so while suspended).
    Running(Id),
    /// CAMERA was refused; `settings_only` when Android will not ask again.
    Denied { settings_only: bool },
    /// The camera could not start: the error's text.
    Failed(String),
}

/// What the app should do with the camera.
#[derive(Clone, Debug, PartialEq)]
pub enum Action<Id> {
    /// `start_qr(&qr_options())`, then [`Scanner::started`] with its id.
    Start,
    /// `stop(id)`.
    Stop(Id),
    /// `zoom_by(id, factor)`.
    ZoomBy(Id, f32),
    /// The accepted pairing link, for `App::open_url`.
    Pair(String),
}

/// A camera event, as the scanner reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Cam<Id> {
    Opened(Id),
    Closed(Id, Option<CameraError>),
    Suspended(Id),
    /// A zoom change; `app` when it came from `zoom_by`, not auto zoom.
    Zoom {
        scan: Id,
        ratio: f32,
        app: bool,
    },
    /// A code: its text when it is UTF-8, and its length in bytes.
    Qr {
        scan: Id,
        text: Option<String>,
        bytes: usize,
    },
}

/// libmorph's event in the scanner's terms; `None` for one it does not
/// read (a capture's).
pub fn cam(event: &CameraEvent) -> Option<Cam<ScanId>> {
    Some(match event {
        CameraEvent::Opened { scan, .. } => Cam::Opened(*scan),
        CameraEvent::Closed { scan, reason } => Cam::Closed(*scan, reason.clone()),
        CameraEvent::Suspended { scan } => Cam::Suspended(*scan),
        CameraEvent::Zoom {
            scan,
            ratio,
            source,
        } => Cam::Zoom {
            scan: *scan,
            ratio: *ratio,
            app: *source == ZoomSource::App,
        },
        CameraEvent::Qr { scan, code } => Cam::Qr {
            scan: *scan,
            text: code.text.clone(),
            bytes: code.bytes.len(),
        },
        _ => return None,
    })
}

/// A note under the area while the camera runs.
#[derive(Clone, Debug, PartialEq)]
enum Note {
    /// A code that is not a pairing link, until the instant.
    Ignored(Instant),
    /// A pairing link `scan::read` refused, with its reason.
    Unreadable(String),
}

/// Two touches of a pinch.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pinch {
    a: u64,
    b: u64,
    last_distance: f32,
}

/// What a touch on the window came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Touched<Id> {
    /// Not the scanner's: the window handles it as before (its end is a
    /// tap).
    Pass,
    /// Part of a pinch: nothing else happens.
    Consumed,
    /// Part of a pinch that moved enough: zoom.
    Zoom(Action<Id>),
}

/// The pairing window's camera.
#[derive(Clone, Debug)]
pub struct Scanner<Id = ScanId> {
    pub state: State<Id>,
    /// **Paste a link** was tapped: the field and Pair show instead.
    pub paste: bool,
    /// A pairing failed: the placeholder reads *Tap to scan again* and the
    /// line says why.
    pub retry: Option<String>,
    /// A code was handed over and the daemon has not answered: the area
    /// shows "pairing…" and no Tap to scan.
    pub handed_over: bool,
    /// The camera answered `Unsupported`: the window is the paste screen.
    pub no_camera: bool,
    note: Option<Note>,
    /// The touches down on the running viewfinder: `(id, x, y)`.
    touches: Vec<(u64, f32, f32)>,
    pinch: Option<Pinch>,
    /// Touches that were part of a pinch, until they end.
    spent: Vec<u64>,
    /// The ratio of the last zoom from a pinch, and until when it shows.
    zoom: Option<(f32, Instant)>,
}

impl<Id> Default for Scanner<Id> {
    fn default() -> Self {
        Self {
            state: State::Idle,
            paste: false,
            retry: None,
            handed_over: false,
            no_camera: false,
            note: None,
            touches: Vec::new(),
            pinch: None,
            spent: Vec::new(),
            zoom: None,
        }
    }
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl<Id: Copy + PartialEq> Scanner<Id> {
    /// The scan in progress (starting or running), if any.
    pub fn scan(&self) -> Option<Id> {
        match self.state {
            State::Starting(id) | State::Running(id) => Some(id),
            _ => None,
        }
    }

    /// The camera is on, or being turned on: the viewfinder and its marks.
    pub fn live(&self) -> bool {
        self.scan().is_some()
    }

    /// The viewfinder has frames to show.
    pub fn running(&self) -> Option<Id> {
        match self.state {
            State::Running(id) => Some(id),
            _ => None,
        }
    }

    /// A tap on the area: the camera starts, unless it runs already or a
    /// scanned code is being paired. The field hides again.
    pub fn tap(&mut self) -> Option<Action<Id>> {
        if self.live() || self.handed_over {
            return None;
        }
        self.paste = false;
        self.retry = None;
        self.note = None;
        Some(Action::Start)
    }

    /// `start_qr` answered with `scan`.
    pub fn started(&mut self, scan: Id) {
        self.state = State::Starting(scan);
    }

    /// Stops the camera if it runs, back to the placeholder: **Paste a
    /// link**, Cancel, Back, the window going, or another pairing input.
    pub fn stop(&mut self) -> Option<Action<Id>> {
        let scan = self.scan()?;
        self.state = State::Idle;
        self.forget_touches();
        self.note = None;
        self.zoom = None;
        Some(Action::Stop(scan))
    }

    /// **Paste a link**: the camera stops and the field shows.
    pub fn paste_link(&mut self) -> Option<Action<Id>> {
        self.paste = true;
        self.stop()
    }

    /// The window closed, or the pairing ended: the placeholder again, as
    /// on opening (the camera stopped first).
    pub fn reset(&mut self) -> Option<Action<Id>> {
        let stop = self.stop();
        let no_camera = self.no_camera;
        *self = Self {
            no_camera,
            ..Self::default()
        };
        stop
    }

    /// The pairing of a scanned (or pasted) link failed: the placeholder,
    /// with the reason and *Tap to scan again*.
    pub fn pair_failed(&mut self, why: &str) {
        self.handed_over = false;
        self.retry = Some(why.trim().trim_end_matches('.').to_string());
        if !self.live() {
            self.state = State::Idle;
        }
    }

    /// The app resumed: after a refusal only Settings could undo, the
    /// placeholder is plain again once CAMERA is granted there.
    pub fn resumed(&mut self, camera_granted: bool) {
        if camera_granted && matches!(self.state, State::Denied { .. }) {
            self.state = State::Idle;
        }
    }

    fn forget_touches(&mut self) {
        self.touches.clear();
        self.pinch = None;
        self.spent.clear();
    }

    /// A camera event at `now`; what to do about it.
    pub fn on_event(&mut self, event: Cam<Id>, now: Instant) -> Vec<Action<Id>> {
        let current = self.scan();
        match event {
            Cam::Opened(scan) if current == Some(scan) => self.state = State::Running(scan),
            Cam::Suspended(scan) if current == Some(scan) => self.forget_touches(),
            Cam::Closed(scan, reason) if current == Some(scan) => {
                self.forget_touches();
                self.zoom = None;
                self.note = None;
                self.state = match reason {
                    None => State::Idle,
                    Some(CameraError::PermissionDenied { settings_only }) => {
                        State::Denied { settings_only }
                    }
                    Some(CameraError::Unsupported(why)) => {
                        log::warn!("pairing: no camera ({why}); the paste screen instead");
                        self.no_camera = true;
                        State::Idle
                    }
                    Some(error) => State::Failed(error.to_string()),
                };
            }
            Cam::Zoom { scan, ratio, app } if current == Some(scan) && app => {
                self.zoom = Some((ratio, now + ZOOM_SHOWN_FOR));
            }
            Cam::Qr { scan, text, bytes } if current == Some(scan) => {
                return self.code(scan, text, bytes, now);
            }
            _ => {}
        }
        Vec::new()
    }

    /// A scanned code. Its text is never logged, only its length.
    fn code(
        &mut self,
        scan: Id,
        text: Option<String>,
        bytes: usize,
        now: Instant,
    ) -> Vec<Action<Id>> {
        let Some(text) = text.filter(|t| is_pairing_link(t)) else {
            log::info!("pairing: scanned a code that is not a pairing link ({bytes} bytes)");
            self.note = Some(Note::Ignored(now + IGNORED_FOR));
            return Vec::new();
        };
        match flankr_companion::scan::read(&text) {
            Err(why) => {
                log::warn!("pairing: scanned a pairing link that cannot be used: {why}");
                self.note = Some(Note::Unreadable(why));
                Vec::new()
            }
            Ok(_) => {
                log::info!("pairing: scanned a pairing link ({bytes} bytes)");
                self.stop();
                self.handed_over = true;
                self.retry = None;
                vec![Action::Stop(scan), Action::Pair(text.trim().to_string())]
            }
        }
    }

    /// A touch on the window while the camera runs: two fingers pinch.
    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) -> Touched<Id> {
        let Some(scan) = self.running() else {
            if self.spent.contains(&id)
                && matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
            {
                self.spent.retain(|&t| t != id);
                return Touched::Consumed;
            }
            return Touched::Pass;
        };
        match phase {
            TouchPhase::Started => {
                self.touches.retain(|&(t, ..)| t != id);
                self.touches.push((id, x, y));
                if self.pinch.is_none() && self.touches.len() == 2 {
                    let (a, b) = (self.touches[0], self.touches[1]);
                    self.pinch = Some(Pinch {
                        a: a.0,
                        b: b.0,
                        last_distance: distance((a.1, a.2), (b.1, b.2)),
                    });
                    self.spent.extend([a.0, b.0]);
                }
                if self.spent.contains(&id) {
                    Touched::Consumed
                } else {
                    Touched::Pass
                }
            }
            TouchPhase::Moved => {
                if let Some(t) = self.touches.iter_mut().find(|(t, ..)| *t == id) {
                    (t.1, t.2) = (x, y);
                }
                let Some(pinch) = self.pinch.filter(|p| p.a == id || p.b == id) else {
                    return if self.spent.contains(&id) {
                        Touched::Consumed
                    } else {
                        Touched::Pass
                    };
                };
                let at = |t: u64| {
                    self.touches
                        .iter()
                        .find(|(i, ..)| *i == t)
                        .map(|&(_, x, y)| (x, y))
                };
                let (Some(a), Some(b)) = (at(pinch.a), at(pinch.b)) else {
                    return Touched::Consumed;
                };
                let now = distance(a, b);
                if pinch.last_distance <= f32::EPSILON || now <= f32::EPSILON {
                    return Touched::Consumed;
                }
                let factor = now / pinch.last_distance;
                if (factor - 1.0).abs() <= PINCH_STEP {
                    return Touched::Consumed;
                }
                self.pinch = Some(Pinch {
                    last_distance: now,
                    ..pinch
                });
                Touched::Zoom(Action::ZoomBy(scan, factor))
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.touches.retain(|&(t, ..)| t != id);
                if self.pinch.is_some_and(|p| p.a == id || p.b == id) {
                    self.pinch = None;
                }
                if self.spent.contains(&id) {
                    self.spent.retain(|&t| t != id);
                    Touched::Consumed
                } else {
                    Touched::Pass
                }
            }
        }
    }

    /// The ratio at the top right of the area ("2.4×") for a second after
    /// the last zoom from a pinch; nothing for auto zoom.
    pub fn zoom_label(&self, now: Instant) -> Option<String> {
        let (ratio, until) = self.zoom?;
        (self.live() && now < until).then(|| format!("{ratio:.1}\u{d7}"))
    }

    /// The placeholder's label: *Tap to scan*, or *Tap to scan again*
    /// after a failed pairing; none while the camera runs or a scanned
    /// code is being paired.
    pub fn prompt(&self) -> Option<&'static str> {
        if self.live() || self.handed_over {
            None
        } else if self.retry.is_some() {
            Some(copy::PAIRING_SCAN_AGAIN.text)
        } else {
            Some(copy::PAIRING_TAP_TO_SCAN.text)
        }
    }

    /// **Open Settings** is in the bottom row.
    pub fn settings_button(&self) -> bool {
        self.state
            == State::Denied {
                settings_only: true,
            }
    }

    /// The line under the area at `now`, and its colour's role: what to do
    /// or what happened (decision 3). None while a scanned code is being
    /// paired (the app's "pairing…" note shows).
    pub fn line(&self, now: Instant) -> Option<(String, Role)> {
        if self.handed_over {
            return None;
        }
        // One paragraph: the copy's own line breaks are only its source's.
        // The window's face has no "→": the Settings path is drawn with ">".
        let flow = |text: &str| {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace('\u{2192}', ">")
        };
        let text = |c: &copy::CopyText| flow(c.text);
        let why = |c: &copy::CopyText, why: &str| flow(&c.fill(None, &[("why", why)]));
        Some(match &self.state {
            State::Starting(_) | State::Running(_) => match &self.note {
                Some(Note::Ignored(until)) if now < *until => {
                    (text(&copy::PAIRING_SCAN_IGNORED), Role::Warn)
                }
                Some(Note::Unreadable(reason)) => {
                    (why(&copy::PAIRING_SCAN_UNREADABLE, reason), Role::Warn)
                }
                _ => (text(&copy::PAIRING_SCAN_HINT), Role::Muted),
            },
            State::Denied {
                settings_only: false,
            } => (text(&copy::PAIRING_CAMERA_DENIED), Role::Warn),
            State::Denied {
                settings_only: true,
            } => (text(&copy::PAIRING_CAMERA_DENIED_SETTINGS), Role::Warn),
            State::Failed(error) => (why(&copy::PAIRING_CAMERA_FAILED, error), Role::Warn),
            State::Idle => match &self.retry {
                Some(reason) => (why(&copy::PAIRING_RETRY, reason), Role::Warn),
                None => (text(&copy::PAIRING_CAMERA_WHY), Role::Muted),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::link::{OFFER_LEN, PairingPayload};

    type S = Scanner<u32>;

    /// A version-3 pairing link on 127.0.0.1, as `flankr pair` shows it.
    fn link() -> String {
        PairingPayload {
            server_key: [7; 32],
            addresses: vec!["127.0.0.1:7347".into()],
            offer: Some([1; OFFER_LEN]),
        }
        .url()
        .unwrap()
    }

    fn qr(scan: u32, text: &str) -> Cam<u32> {
        Cam::Qr {
            scan,
            text: Some(text.into()),
            bytes: text.len(),
        }
    }

    /// Tapped and opened, as scan 1.
    fn running(now: Instant) -> S {
        let mut s = S::default();
        assert_eq!(s.tap(), Some(Action::Start));
        s.started(1);
        assert!(s.on_event(Cam::Opened(1), now).is_empty());
        assert_eq!(s.state, State::Running(1));
        s
    }

    #[test]
    fn the_window_opens_on_the_placeholder_and_a_tap_starts_the_scan() {
        let now = Instant::now();
        let s = S::default();
        assert_eq!(s.state, State::Idle);
        assert_eq!(s.prompt(), Some("Tap to scan"));
        assert!(!s.live() && !s.settings_button());
        let (line, _) = s.line(now).unwrap();
        assert!(
            line.starts_with("Scan the QR code flankr pair shows."),
            "{line}"
        );
        let mut s = running(now);
        assert_eq!(s.prompt(), None);
        assert!(s.line(now).unwrap().0.starts_with("Point at the QR code"));
        // A tap on the running viewfinder does nothing.
        assert_eq!(s.tap(), None);
    }

    #[test]
    fn the_scan_options_are_continuous_with_auto_exposure() {
        let o = qr_options();
        assert_eq!(o.mode, QrMode::Continuous);
        assert_eq!(o.facing, Facing::Back);
        assert!(o.auto_zoom);
        assert_eq!(o.exposure, ScreenExposure::Auto);
        assert_eq!(o.preview, Some(PreviewOptions::default()));
    }

    #[test]
    fn a_code_that_is_not_a_pairing_link_is_ignored_for_3_s() {
        let now = Instant::now();
        let mut s = running(now);
        assert!(s.on_event(qr(1, "https://example.com"), now).is_empty());
        assert_eq!(s.state, State::Running(1));
        assert_eq!(s.line(now).unwrap().0, "Not a flankr pairing code.");
        let later = now + IGNORED_FOR;
        assert!(s.line(later).unwrap().0.starts_with("Point at the QR code"));
        // Binary codes too.
        let binary = Cam::Qr {
            scan: 1,
            text: None,
            bytes: 9,
        };
        assert!(s.on_event(binary, later).is_empty());
        assert_eq!(s.line(later).unwrap().0, "Not a flankr pairing code.");
        // A code for another scan is not this one's.
        assert!(s.on_event(qr(2, &link()), later).is_empty());
        assert_eq!(s.state, State::Running(1));
    }

    #[test]
    fn a_pairing_link_that_cannot_be_read_keeps_scanning_with_its_reason() {
        let now = Instant::now();
        let mut s = running(now);
        let bad = "flankr://127.0.0.1:7347/pair#flankr1abc";
        assert!(s.on_event(qr(1, bad), now).is_empty());
        assert_eq!(s.state, State::Running(1));
        let (line, role) = s.line(now + IGNORED_FOR).unwrap();
        assert!(
            line.starts_with("That flankr pairing code can't be used: invalid pairing string"),
            "{line}"
        );
        assert_eq!(role, Role::Warn);
    }

    #[test]
    fn a_valid_link_stops_the_scan_then_pairs() {
        let now = Instant::now();
        let mut s = running(now);
        let url = link();
        assert_eq!(
            s.on_event(qr(1, &url), now),
            [Action::Stop(1), Action::Pair(url.clone())]
        );
        assert_eq!(s.state, State::Idle);
        assert!(s.handed_over);
        assert_eq!(s.prompt(), None, "no Tap to scan while it pairs");
        assert_eq!(s.line(now), None, "the app's pairing note shows");
        assert_eq!(s.tap(), None);
        // The Closed of the stop is for no scan of this window's now.
        assert!(s.on_event(Cam::Closed(1, None), now).is_empty());
        // The pairing failed: the placeholder, with why and Tap to scan
        // again, which starts the camera.
        s.pair_failed("the link has expired.");
        assert_eq!(s.prompt(), Some("Tap to scan again"));
        assert_eq!(
            s.line(now).unwrap().0,
            "the link has expired. Run flankr pair again, then scan its new code."
        );
        assert_eq!(s.tap(), Some(Action::Start));
        assert_eq!(s.prompt(), Some("Tap to scan"));
    }

    #[test]
    fn a_refusal_returns_to_the_placeholder_with_the_reason() {
        let now = Instant::now();
        let mut s = S::default();
        s.tap();
        s.started(1);
        let denied = |settings_only| Some(CameraError::PermissionDenied { settings_only });
        s.on_event(Cam::Closed(1, denied(false)), now);
        assert_eq!(
            s.state,
            State::Denied {
                settings_only: false
            }
        );
        assert!(!s.settings_button());
        assert_eq!(s.prompt(), Some("Tap to scan"));
        assert!(
            s.line(now)
                .unwrap()
                .0
                .starts_with("Camera permission was refused")
        );
        // A tap asks again.
        assert_eq!(s.tap(), Some(Action::Start));
        s.started(2);
        s.on_event(Cam::Closed(2, denied(true)), now);
        assert!(s.settings_button());
        let line = s.line(now).unwrap().0;
        assert!(line.contains("Android won't ask again"), "{line}");
        assert!(
            line.contains("Settings > Apps > flankr > Permissions > Camera"),
            "{line}"
        );
        assert_eq!(s.tap(), Some(Action::Start));
        s.started(3);
        s.on_event(Cam::Closed(3, denied(true)), now);
        // Granted in Settings meanwhile: back to plain Tap to scan.
        s.resumed(false);
        assert!(s.settings_button());
        s.resumed(true);
        assert_eq!(s.state, State::Idle);
        assert!(!s.settings_button());
    }

    #[test]
    fn other_errors_show_the_camera_failed_line_and_unsupported_means_no_camera() {
        let now = Instant::now();
        let mut s = S::default();
        s.tap();
        s.started(1);
        s.on_event(Cam::Closed(1, Some(CameraError::InUse)), now);
        assert_eq!(
            s.line(now).unwrap().0,
            "The camera couldn't start: the camera is in use."
        );
        assert_eq!(s.prompt(), Some("Tap to scan"));
        assert!(!s.no_camera);
        s.tap();
        s.started(2);
        s.on_event(
            Cam::Closed(2, Some(CameraError::Unsupported("no camera on this host"))),
            now,
        );
        assert!(s.no_camera);
        assert!(s.reset().is_none());
        assert!(s.no_camera, "kept across openings");
    }

    #[test]
    fn a_pinch_apart_by_10_percent_zooms_by_about_1_1_and_is_no_tap() {
        let now = Instant::now();
        let mut s = running(now);
        assert_eq!(
            s.touch(5, TouchPhase::Started, 400.0, 1000.0),
            Touched::Pass
        );
        assert_eq!(
            s.touch(6, TouchPhase::Started, 600.0, 1000.0),
            Touched::Consumed
        );
        // Less than 0.5 %: nothing yet.
        assert_eq!(
            s.touch(6, TouchPhase::Moved, 600.5, 1000.0),
            Touched::Consumed
        );
        match s.touch(6, TouchPhase::Moved, 620.0, 1000.0) {
            Touched::Zoom(Action::ZoomBy(1, factor)) => {
                assert!((factor - 1.1).abs() < 0.001, "{factor}");
            }
            other => panic!("{other:?}"),
        }
        // From the last distance on, not the first.
        match s.touch(5, TouchPhase::Moved, 380.0, 1000.0) {
            Touched::Zoom(Action::ZoomBy(1, factor)) => {
                assert!((factor - 240.0 / 220.0).abs() < 0.001, "{factor}");
            }
            other => panic!("{other:?}"),
        }
        // Its ends hit nothing.
        assert_eq!(
            s.touch(5, TouchPhase::Ended, 380.0, 1000.0),
            Touched::Consumed
        );
        assert_eq!(
            s.touch(6, TouchPhase::Ended, 620.0, 1000.0),
            Touched::Consumed
        );
        // The ratio shows for a second after an app zoom, not auto zoom.
        s.on_event(
            Cam::Zoom {
                scan: 1,
                ratio: 2.4,
                app: false,
            },
            now,
        );
        assert_eq!(s.zoom_label(now), None);
        s.on_event(
            Cam::Zoom {
                scan: 1,
                ratio: 2.4,
                app: true,
            },
            now,
        );
        assert_eq!(s.zoom_label(now).as_deref(), Some("2.4\u{d7}"));
        assert_eq!(s.zoom_label(now + ZOOM_SHOWN_FOR), None);
    }

    #[test]
    fn a_single_touch_gives_no_zoom() {
        let now = Instant::now();
        let mut s = running(now);
        assert_eq!(s.touch(1, TouchPhase::Started, 100.0, 100.0), Touched::Pass);
        assert_eq!(s.touch(1, TouchPhase::Moved, 300.0, 300.0), Touched::Pass);
        assert_eq!(s.touch(1, TouchPhase::Ended, 300.0, 300.0), Touched::Pass);
        // Without the camera, every touch is the window's.
        let mut idle = S::default();
        assert_eq!(idle.touch(1, TouchPhase::Started, 1.0, 1.0), Touched::Pass);
        assert_eq!(idle.touch(2, TouchPhase::Started, 9.0, 9.0), Touched::Pass);
    }

    #[test]
    fn suspended_then_opened_keeps_the_same_scan_running() {
        let now = Instant::now();
        let mut s = running(now);
        assert!(s.on_event(Cam::Suspended(1), now).is_empty());
        assert_eq!(s.state, State::Running(1));
        assert!(s.on_event(Cam::Opened(1), now).is_empty());
        assert_eq!(s.state, State::Running(1));
        // A close nobody asked for returns to the placeholder.
        s.on_event(Cam::Closed(1, None), now);
        assert_eq!(s.state, State::Idle);
        assert_eq!(s.prompt(), Some("Tap to scan"));
    }

    #[test]
    fn paste_a_link_stops_the_camera_and_a_tap_hides_the_field() {
        let now = Instant::now();
        let mut s = running(now);
        assert_eq!(s.paste_link(), Some(Action::Stop(1)));
        assert!(s.paste && s.state == State::Idle);
        assert_eq!(s.stop(), None, "stopped once");
        assert_eq!(s.tap(), Some(Action::Start));
        assert!(!s.paste);
        s.started(2);
        assert_eq!(s.reset(), Some(Action::Stop(2)));
        assert_eq!(s.state, State::Idle);
    }
}
