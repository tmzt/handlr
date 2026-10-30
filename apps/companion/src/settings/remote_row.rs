//! A remote's row in the Remotes section (plan 20261028_01, decision 26):
//! two lines, and an **X** at the right.
//!
//! - Line 1, at the label's size: the remote's shown name (its nickname,
//!   or the default: [`RemoteInfo::display_name`]; plan 20261024_02, the
//!   2026-10-28 17:56 and 17:57 EDT amendments), and after it, for the one
//!   in use, a dim *in use*, or *in use · offline* while its connection is
//!   down.
//! - Line 2, at the subtitle's size and dim: `<host:port> · <fingerprint>
//!   · paired <date>`, the date as `28 Oct 2026` in local time, or *paired
//!   before this list*; cut with an ellipsis in the host when too long.
//! - The X: a 44 pt target, drawn as two strokes in the dim colour (not
//!   red: the destructive word is the modal's red button). A tap on it is
//!   [`REMOVE_PART`]; a tap elsewhere on the row is the body, which opens
//!   the remote's details ([`super::remote_details`]).
//!
//! With no remote, one row of the section is the standard row
//! *No remotes paired* ([`RemoteRow::placeholder`]): its label, dim, and
//! no X; it takes no taps.

use flankr_companion::RemoteInfo;
use libcontrols::{
    ControlAction, ControlPart, DrawContext, DrawControl, EventContext, HostedControl,
    KeyboardEvent, MouseButton, MouseEvent, Rect,
};
use libmsdf::drawlist::DrawList;

use super::copy;
use super::screen::{LABEL_PT, SUBTITLE_PT};
use crate::agent::{PAD, line_h};
use crate::theme::theme;

/// The part a row reports for a tap on its X.
pub const REMOVE_PART: ControlPart = ControlPart::Value;
/// The X's target and its drawn size, in points.
pub const TARGET: f32 = 44.0;
const GLYPH: f32 = 12.0;

/// `secs` (Unix seconds) as `28 Oct 2026`, in the phone's time zone
/// (`localtime_r`; std has no time zones).
pub fn paired_date(secs: u64) -> String {
    paired_date_time(secs).0
}

/// `secs` (Unix seconds) as its date, `28 Oct 2026`, and its time of day,
/// `14:05`, in the phone's time zone; UTC when the C library has no
/// answer.
pub fn paired_date_time(secs: u64) -> (String, String) {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let t = secs as libc::time_t;
    // SAFETY: `localtime_r` writes only into `tm`, which outlives the call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    let (day, month, year, hour, minute) = if ok {
        (
            tm.tm_mday as u32,
            tm.tm_mon as usize,
            tm.tm_year as i64 + 1900,
            tm.tm_hour as u32,
            tm.tm_min as u32,
        )
    } else {
        let (day, month, year) = civil(secs);
        let of_day = secs % 86_400;
        (
            day,
            month,
            year,
            (of_day / 3600) as u32,
            (of_day % 3600 / 60) as u32,
        )
    };
    (
        format!("{day} {} {year}", MONTHS[month.min(11)]),
        format!("{hour:02}:{minute:02}"),
    )
}

/// The UTC day, month (0-based) and year of `secs`, for when the C library
/// has no answer (Howard Hinnant's `civil_from_days`).
fn civil(secs: u64) -> (u32, usize, i64) {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (day, (month - 1) as usize, year)
}

/// The second line for `info`, at most `cols` characters: the host is cut
/// with an ellipsis first, and the line kept whole otherwise.
pub fn detail(info: &RemoteInfo, cols: usize) -> String {
    let fill = |host: &str| match info.paired_at {
        Some(at) => copy::REMOTE_DETAIL.fill(
            None,
            &[
                ("host", host),
                ("fingerprint", &info.fingerprint),
                ("date", &paired_date(at)),
            ],
        ),
        None => copy::REMOTE_DETAIL_UNDATED
            .fill(None, &[("host", host), ("fingerprint", &info.fingerprint)]),
    };
    let whole = fill(&info.host).trim().to_string();
    let len = whole.chars().count();
    if len <= cols {
        return whole;
    }
    let host: Vec<char> = info.host.chars().collect();
    let keep = host.len().saturating_sub(len - cols + 1).max(1);
    let cut: String = host[..keep.min(host.len())].iter().collect();
    fill(&format!("{cut}\u{2026}")).trim().to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteRow {
    pub info: Option<RemoteInfo>,
    /// With no `info`, the label of a standard row that takes no taps
    /// (*No remotes paired*).
    pub placeholder: Option<&'static str>,
    pub rect: Rect,
    pub scale: f32,
}

impl RemoteRow {
    pub fn new(scale: f32) -> Self {
        RemoteRow {
            info: None,
            placeholder: None,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            scale,
        }
    }

    /// The first line's name: the shown name, or the placeholder's label.
    pub fn name(&self) -> &str {
        match &self.info {
            Some(i) => i.display_name.as_str(),
            None => self.placeholder.unwrap_or(""),
        }
    }

    /// The dim words after the name: *in use*, *in use · offline*, or
    /// none.
    pub fn status(&self) -> Option<&'static str> {
        let info = self.info.as_ref()?;
        info.active.then_some(if info.online {
            copy::REMOTE_IN_USE.text
        } else {
            copy::REMOTE_IN_USE_OFFLINE.text
        })
    }

    /// The second line, cut to the room left of the X (estimated at half
    /// an em a character, as layout runs without a font).
    pub fn detail(&self) -> String {
        let Some(info) = &self.info else {
            return String::new();
        };
        let s = self.scale;
        let room = (self.rect.w - 2.0 * PAD * s - TARGET * s).max(1.0);
        let cols = ((room / (0.5 * SUBTITLE_PT * s)) as usize).max(12);
        detail(info, cols)
    }

    /// The X's target, at the right, centred down the row.
    pub fn remove_rect(&self) -> Rect {
        let s = self.scale;
        let t = TARGET * s;
        Rect::new(
            self.rect.x + self.rect.w - PAD * s * 0.5 - t,
            self.rect.y + (self.rect.h - t) * 0.5,
            t,
            t,
        )
    }

    fn on_remove(&self, pos: [f32; 2]) -> bool {
        self.info.is_some() && self.remove_rect().contains(pos)
    }
}

impl DrawControl for RemoteRow {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        let px = LABEL_PT * s;
        if self.info.is_none() {
            if let Some(label) = self.placeholder {
                let x = self.rect.x + PAD * s;
                let y = self.rect.y + (self.rect.h - line_h(px)) * 0.5;
                ctx.draw_text(list, label, [x, y], px, theme().dim);
            }
            return;
        }
        let sub = SUBTITLE_PT * s;
        let x = self.rect.x + PAD * s;
        let y = self.rect.y + (self.rect.h - line_h(px) - line_h(sub)) * 0.5;
        ctx.draw_text(list, self.name(), [x, y], px, theme().text);
        if let Some(status) = self.status() {
            let nx = x + ctx.text_advance(self.name(), px) + 8.0 * s;
            // On the name's baseline, at the subtitle's size.
            let sy = y + line_h(px) - line_h(sub) - 1.0 * s;
            ctx.draw_text(list, status, [nx, sy], sub, theme().dim);
        }
        ctx.draw_text(list, &self.detail(), [x, y + line_h(px)], sub, theme().dim);
        // The X, as two strokes.
        let [cx, cy] = self.remove_rect().center();
        let h = GLYPH * s * 0.5;
        let t = 1.8 * s;
        list.push_line([cx - h, cy - h], [cx + h, cy + h], t, theme().dim);
        list.push_line([cx - h, cy + h], [cx + h, cy - h], t, theme().dim);
    }
}

impl HostedControl for RemoteRow {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        if self.info.is_none() || !self.rect.contains(pos) {
            return None;
        }
        let part = if self.on_remove(pos) {
            REMOVE_PART
        } else {
            ControlPart::Body
        };
        Some((part, self.rect.normalized_x(pos[0])))
    }

    /// On the X, removing it (after the modal); elsewhere, the row.
    fn handle_mouse(&self, event: &MouseEvent, _ctx: &EventContext) -> Option<ControlAction> {
        match event {
            MouseEvent::Click {
                pos,
                button: MouseButton::Left,
            } if self.info.is_some() => Some(ControlAction::Activated {
                part: if self.on_remove(*pos) {
                    REMOVE_PART
                } else {
                    ControlPart::Body
                },
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
    use flankr_companion::RemoteId;

    pub(crate) fn info(name: &str, active: bool, online: bool) -> RemoteInfo {
        RemoteInfo {
            id: RemoteId([1, 2, 3, 4, 5, 6, 7, 8]),
            name: name.into(),
            host: "m4.tail1a2b3c.ts.net:7347".into(),
            fingerprint: "3f9a 12c4".into(),
            paired_at: None,
            active,
            online,
            fingerprint_hex: "3f9a12c4".repeat(8),
            fingerprint_words: "able baker charlie delta echo fox".into(),
            nickname: None,
            display_name: name.into(),
            last_session: None,
        }
    }

    #[test]
    fn the_date_is_day_month_year() {
        // 2026-10-28 12:00 UTC: the same day in every zone within ±11 h.
        assert_eq!(paired_date(1_793_188_800), "28 Oct 2026");
        // And a time of day, as hh:mm, in the phone's zone.
        let (date, time) = paired_date_time(1_793_188_800 + 5 * 60);
        assert_eq!(date, "28 Oct 2026");
        let b = time.as_bytes();
        assert!(
            b.len() == 5
                && b[2] == b':'
                && time.replace(':', "").bytes().all(|c| c.is_ascii_digit()),
            "{time}"
        );
        assert_eq!(civil(1_793_188_800), (28, 9, 2026));
        assert_eq!(civil(0), (1, 0, 1970));
        assert_eq!(civil(951_782_400), (29, 1, 2000));
    }

    #[test]
    fn the_detail_line_is_cut_in_the_host() {
        let mut i = info("m4", false, false);
        assert_eq!(
            detail(&i, 200),
            "m4.tail1a2b3c.ts.net:7347 · 3f9a 12c4 · paired before this list"
        );
        i.paired_at = Some(1_793_188_800);
        let whole = detail(&i, 200);
        assert_eq!(
            whole,
            "m4.tail1a2b3c.ts.net:7347 · 3f9a 12c4 · paired 28 Oct 2026"
        );
        let cut = detail(&i, 40);
        assert_eq!(cut.chars().count(), 40, "{cut}");
        assert!(cut.starts_with("m4.tai\u{2026} · "), "{cut}");
        assert!(cut.ends_with("3f9a 12c4 · paired 28 Oct 2026"), "{cut}");
    }

    #[test]
    fn a_tap_on_the_x_is_told_from_one_on_the_row() {
        let mut row = RemoteRow::new(2.0);
        row.rect = Rect::new(0.0, 100.0, 1080.0, 104.0);
        let click = |row: &RemoteRow, pos| {
            row.handle_mouse(
                &MouseEvent::Click {
                    pos,
                    button: MouseButton::Left,
                },
                &EventContext::default(),
            )
        };
        // An empty row is not there, nor is the placeholder row.
        assert_eq!(click(&row, [40.0, 150.0]), None);
        assert!(row.hit_test([40.0, 150.0]).is_none());
        row.placeholder = Some("No remotes paired");
        assert_eq!(row.name(), "No remotes paired");
        assert_eq!(click(&row, [40.0, 150.0]), None);
        assert!(row.hit_test([40.0, 150.0]).is_none());
        row.info = Some(info("m4", true, false));
        let x = row.remove_rect();
        assert_eq!(x.w, TARGET * 2.0, "a 44 pt target");
        assert!(x.x > 900.0, "at the right");
        assert_eq!(
            click(&row, x.center()),
            Some(ControlAction::Activated { part: REMOVE_PART })
        );
        assert_eq!(
            click(&row, [40.0, 150.0]),
            Some(ControlAction::Activated {
                part: ControlPart::Body
            })
        );
        assert_eq!(row.status(), Some("in use · offline"));
        // The first line is the shown name, not the daemon's.
        let mut named = info("m4", false, false);
        named.display_name = "Office".into();
        row.info = Some(named);
        assert_eq!(row.name(), "Office");
        row.info = Some(info("m4", true, true));
        assert_eq!(row.status(), Some("in use"));
        row.info = Some(info("m4", false, false));
        assert_eq!(row.status(), None);
    }
}
