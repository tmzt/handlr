//! The planning plugin's device half, `dev.flankr.plugins.planning`
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.8, item 7), for
//! its daemon half's `0.1` releases.
//!
//! - **The plan table** (the plan's 23:14 EDT amendments, "the plan menu
//!   is a table, by last update" and "the plan table's statuses
//!   abbreviated", with the 23:15 and 23:16 EDT ones on its sizes): each
//!   plan is one row of four columns, its **NN** and **mm/dd** from the
//!   file name, right-aligned, its **title** cut with …, and its
//!   **status** abbreviated ([`status_abbrev`]) in the dim colour; NN,
//!   mm/dd and the status at 0.8 of the row's text size, vertically
//!   centred, the title at full size. Rows are sorted by last update,
//!   newest first ([`rows_of`]).
//! - **Plans in this project** (`plans`) opens the modal and calls
//!   `plans.list`: "Reading plans…", then the plan table. A row's tap
//!   calls `plans.show` and shows the plan in place of the list.
//! - **The plan**: a ‹ back to the list and **Pin** on the first line;
//!   its title, date and status, with "(README: <status>)" when the
//!   README's table says otherwise; its `covers` as a short list; then
//!   **Amendments**, each its timestamp exactly as written and its
//!   subject.
//! - **The ☑ title icon** (`plans` with `opens: "menu"`, the daemon
//!   half's 0.1.1; the plan's 22:03 EDT amendment, item 5): a popup menu,
//!   "Reading plans…" as a heading until `plans.list` answers, then the
//!   plan table's first ten rows as `PopupRow`s of `PopupCell`s, and
//!   **All plans…**, which opens the modal; choosing a plan opens the
//!   modal on it (`plans.show`).
//! - **The pinned card**, keyed by the plan's base name: its title, its
//!   status, and its last amendment's timestamp and subject; a tap opens
//!   the modal on that plan, calling `plans.show` again so it is current.
//!
//! The results are tpt's schema-1 shapes (decision 3.8, item 4); only
//! these fields are read, and nothing of a plan's body is shown.

use std::cell::Cell;
use std::collections::HashMap;

use libcontrols::{DrawContext, PopupCell, PopupItem, PopupRow, Rect, TextAlign};
use libmsdf::drawlist::DrawList;
use serde_json::{Value, json};

use super::{DevicePlugin, PluginMessage, PluginOut, ViewKind};
use crate::agent::{columns, fill, line_h};
use crate::settings::copy;
use crate::theme::theme;
use crate::ui::wrap;

pub const NS: &str = "dev.flankr.plugins.planning";

/// The type sizes, in points.
const TITLE_PT: f32 = 16.0;
const SMALL_PT: f32 = 13.0;
const HEAD_PT: f32 = 18.0;
/// A tap target's height, a table row's, and the space between blocks.
const TAP_H: f32 = 44.0;
const GAP: f32 = 12.0;
/// The most `covers` items shown.
const COVERS_MAX: usize = 6;
/// The plans the popup menu lists, newest first.
pub const MENU_PLANS: usize = 10;
/// The popup's characters a line: its 320 pt widest, less its padding,
/// at its 16 pt, as [`columns`] counts.
const MENU_COLUMNS: usize = 36;
/// The NN, mm/dd and status columns' size, times the row's.
pub const CELL_SCALE: f32 = 0.8;
/// The modal table's space between columns, in points.
const COLUMN_GAP: f32 = 10.0;
/// About how wide a digit or a status letter is, in em, for the modal's
/// fixed columns: a little over Roboto's digits, so nothing overruns.
const CELL_EM: f32 = 0.6;
/// The popup's item ids: a plan's is this and its base name.
pub const MENU_PLAN: &str = "plan/";
pub const MENU_ALL: &str = "all";

/// A plan as `plans.list` gives it, as far as the list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanRow {
    pub base: String,
    pub title: String,
    pub date: String,
    pub status: String,
    /// The README's status, when its table says otherwise.
    pub readme_status: Option<String>,
    /// The sequence within its day, from the file name (`nn` when the
    /// name has none), without leading zeros.
    pub nn: String,
    /// `mm/dd`, from the file name (`date_from_name` when it has none).
    pub mmdd: String,
    /// Its last update, in minutes, UTC ([`update_key`]): its newest
    /// amendment's timestamp, else its front-matter date, else the file
    /// name's; `None` when none can be read.
    pub updated: Option<i64>,
}

/// A plan as `plans.show` gives it, as far as the modal and the card show
/// it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanView {
    pub row: PlanRow,
    pub covers: Vec<String>,
    /// `(timestamp, subject)`, in file order.
    pub amendments: Vec<(String, String)>,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

impl PlanRow {
    fn from_json(v: &Value) -> Option<PlanRow> {
        let base = text(v, "base");
        if base.is_empty() {
            return None;
        }
        let status = text(v, "status");
        let readme_status = v
            .get("readme")
            .and_then(|r| r.get("status"))
            .and_then(Value::as_str)
            .filter(|r| r.trim() != status.trim())
            .map(str::to_string);
        let mut title = text(v, "title");
        if title.is_empty() {
            title = base.clone();
        }
        let date = text(v, "date");
        let named = name_date(&base).or_else(|| {
            let d = text(v, "date_from_name");
            (d.len() == 10).then(|| d.replace('-', ""))
        });
        let mmdd = named
            .as_deref()
            .map(|d| format!("{}/{}", &d[4..6], &d[6..8]))
            .unwrap_or_default();
        let nn = name_nn(&base)
            .or_else(|| v.get("nn").and_then(Value::as_u64).map(|n| n.to_string()))
            .unwrap_or_default();
        let named_dash = named
            .as_deref()
            .map(|d| format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8]));
        let updated = [
            v.get("last_amendment")
                .and_then(Value::as_str)
                .map(str::to_string),
            Some(date.clone()),
            named_dash,
        ]
        .into_iter()
        .flatten()
        .find_map(|t| update_key(&t));
        Some(PlanRow {
            base,
            title,
            date,
            status,
            readme_status,
            nn,
            mmdd,
            updated,
        })
    }

    /// The line under the title: date · status, and the README's.
    fn subtitle(&self) -> String {
        let mut line = [self.date.as_str(), self.status.as_str()]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        if let Some(r) = &self.readme_status {
            line.push(' ');
            line.push_str(&copy::PLANNING_README.fill(None, &[("status", r)]));
        }
        line
    }
}

/// The file name's date, `YYYYMMDD`, when it starts `YYYYMMDD_`.
fn name_date(base: &str) -> Option<String> {
    let d = base.get(0..8)?;
    (d.bytes().all(|b| b.is_ascii_digit()) && base.as_bytes().get(8) == Some(&b'_'))
        .then(|| d.to_string())
}

/// The file name's NN, `YYYYMMDD_NN_`, without leading zeros.
fn name_nn(base: &str) -> Option<String> {
    name_date(base)?;
    let nn = base.get(9..)?.split('_').next()?;
    let n: u32 = nn.parse().ok()?;
    Some(n.to_string())
}

/// Days from 1970-01-01 to a civil date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A time zone's offset from UTC, in minutes: the North American
/// abbreviations the plans are stamped with, UTC, or `+HH:MM`/`-HHMM`.
fn zone_offset(tz: &str) -> Option<i64> {
    let hours = match tz {
        "UTC" | "GMT" | "Z" => 0,
        "EDT" => -4,
        "EST" | "CDT" => -5,
        "CST" | "MDT" => -6,
        "MST" | "PDT" => -7,
        "PST" => -8,
        _ => {
            let sign = match tz.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = tz[1..].chars().filter(|c| *c != ':').collect();
            if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let h: i64 = digits[0..2].parse().ok()?;
            let m: i64 = digits[2..4].parse().ok()?;
            return Some(sign * (h * 60 + m));
        }
    };
    Some(hours * 60)
}

/// A plan's update time as minutes since 1970, UTC, for sorting:
/// `YYYY-MM-DD`, optionally ` HH:MM` and a zone (an unknown or missing
/// zone counts as UTC; a date alone is its midnight).
pub fn update_key(t: &str) -> Option<i64> {
    let mut parts = t.split_whitespace();
    let date = parts.next()?;
    let mut ymd = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    if ymd.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut minutes = days_from_civil(y, m, d) * 1440;
    if let Some(hm) = parts.next() {
        let (h, mi) = hm.split_once(':')?;
        let (h, mi): (i64, i64) = (h.parse().ok()?, mi.parse().ok()?);
        minutes += h * 60 + mi;
        if let Some(off) = parts.next().and_then(zone_offset) {
            minutes -= off;
        }
    }
    Some(minutes)
}

/// The status column's text (the plan's 23:14 EDT amendment, "the plan
/// table's statuses abbreviated"): `Draft`, `Hold`, `InImpl`, `Impl`,
/// `Succd` and `Rtrd`; any other status its first five letters.
pub fn status_abbrev(status: &str) -> String {
    let s = status.trim();
    let shown = match s {
        "Draft" => "Draft",
        "Hold" => "Hold",
        "In implementation" => "InImpl",
        "Implemented" => "Impl",
        _ if s.starts_with("Superseded") => "Succd",
        _ if s.starts_with("Retired") => "Rtrd",
        _ => return s.chars().take(5).collect(),
    };
    shown.to_string()
}

/// `plans.list`'s plans by last update, newest first; a plan whose time
/// cannot be read goes last, and ties go by file name, the later first.
pub fn rows_of(result: &Value) -> Vec<PlanRow> {
    let mut rows: Vec<PlanRow> = result
        .get("plans")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(PlanRow::from_json).collect())
        .unwrap_or_default();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.base.cmp(&a.base)));
    rows
}

/// A plan's popup row: the item, and its four cells (NN and mm/dd
/// right-aligned, the title taking the room left, the status dim), NN,
/// mm/dd and the status at [`CELL_SCALE`]. Each column is as wide as its
/// widest cell, so the rows line up.
pub fn menu_row(row: &PlanRow) -> PopupRow {
    PopupItem::action(format!("{MENU_PLAN}{}", row.base), row.title.clone()).with_cells(vec![
        PopupCell::new(row.nn.clone())
            .align(TextAlign::Right)
            .scale(CELL_SCALE),
        PopupCell::new(row.mmdd.clone())
            .align(TextAlign::Right)
            .scale(CELL_SCALE),
        PopupCell::new(row.title.clone()).flex(),
        PopupCell::new(status_abbrev(&row.status))
            .dim(true)
            .scale(CELL_SCALE),
    ])
}

/// The modal table's columns at width `w`, the title's size `px`: each
/// `[x, width]`, NN, mm/dd, title, status. NN and mm/dd fit their widest
/// at [`CELL_SCALE`], the status its widest abbreviation, the title the
/// rest.
fn table_columns(rows: &[PlanRow], w: f32, px: f32, gap: f32) -> [[f32; 2]; 4] {
    let small = px * CELL_SCALE;
    let cw = |n: usize| n as f32 * small * CELL_EM;
    let widest = |f: &dyn Fn(&PlanRow) -> usize| rows.iter().map(f).max().unwrap_or(0);
    let nn_w = cw(widest(&|r| r.nn.chars().count()).max(1));
    let md_w = cw(widest(&|r| r.mmdd.chars().count()));
    let st_w = cw(widest(&|r| status_abbrev(&r.status).chars().count()));
    let title_x = nn_w + md_w + 2.0 * gap;
    let title_w = (w - title_x - st_w - gap).max(0.0);
    [
        [0.0, nn_w],
        [nn_w + gap, md_w],
        [title_x, title_w],
        [title_x + title_w + gap, st_w],
    ]
}

/// `plans.show`'s plan.
pub fn plan_of(result: &Value) -> Option<PlanView> {
    let plan = result.get("plan")?;
    let row = PlanRow::from_json(plan)?;
    let strings = |key| -> Vec<String> {
        plan.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let amendments = plan
        .get("amendments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|a| (text(a, "timestamp"), text(a, "subject")))
                .collect()
        })
        .unwrap_or_default();
    Some(PlanView {
        row,
        covers: strings("covers"),
        amendments,
    })
}

/// What the modal shows.
#[derive(Clone, Debug, PartialEq)]
enum Page {
    Reading,
    List(Vec<PlanRow>),
    Plan(Box<PlanView>),
    Failed(String),
}

/// What a call was for.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    List,
    /// `plans.list` for the popup menu.
    Menu,
    /// `plans.show` for the modal.
    Show,
    /// `plans.show` for the card with this key.
    Card(String),
}

/// What a card shows: the plan, or why it could not be read again.
#[derive(Clone, Debug, PartialEq)]
enum CardShows {
    Plan(PlanView),
    Failed(String, String),
}

/// What a tap in a view hit.
#[derive(Clone, Debug, PartialEq)]
enum Tap {
    Row(String),
    Back,
    Pin,
    Card,
}

/// Which colour a line is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ink {
    Text,
    Dim,
    Accent,
}

/// One line of a view, in its own coordinates.
#[derive(Clone, Debug, PartialEq)]
struct Line {
    text: String,
    /// The left edge, or the right edge when `right`.
    x: f32,
    y: f32,
    px: f32,
    ink: Ink,
    right: bool,
}

/// A view laid out at one width: its lines, its taps, a separator under
/// each row, and its height.
#[derive(Debug, Default)]
struct Laid {
    lines: Vec<Line>,
    taps: Vec<(Tap, Rect)>,
    rules: Vec<f32>,
    height: f32,
}

/// `text` on one line of `w` at `px`, cut with … when longer.
fn cut(text: &str, w: f32, px: f32) -> String {
    let max = columns(w, px);
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

pub struct Planning {
    page: Page,
    /// The list last read, for the plan's ‹.
    list: Option<Vec<PlanRow>>,
    asked: HashMap<u64, Asked>,
    /// The pinned cards, by base name.
    cards: Vec<(String, CardShows)>,
    /// The width and scale the views were last laid out at, for taps.
    laid: Cell<(f32, f32)>,
    /// The popup menu's plans: `None` while reading, else the list or
    /// why it could not be read.
    menu: Option<Result<Vec<PlanRow>, String>>,
}

impl Default for Planning {
    fn default() -> Self {
        Self::new()
    }
}

impl Planning {
    pub fn new() -> Self {
        Planning {
            page: Page::Reading,
            list: None,
            asked: HashMap::new(),
            cards: Vec::new(),
            laid: Cell::new((0.0, 1.0)),
            menu: None,
        }
    }

    fn list(&mut self, out: &mut PluginOut) {
        self.page = Page::Reading;
        let id = out.call("plans.list", json!({}));
        self.asked.insert(id, Asked::List);
    }

    fn show(&mut self, base: &str, out: &mut PluginOut) {
        self.page = Page::Reading;
        let id = out.call("plans.show", json!({ "name": base }));
        self.asked.insert(id, Asked::Show);
    }

    fn lay_out(&self, view: &ViewKind, w: f32, s: f32) -> Laid {
        match view {
            ViewKind::Modal => self.lay_out_modal(w, s),
            ViewKind::Card(key) => self.lay_out_card(key, w, s),
        }
    }

    fn lay_out_modal(&self, w: f32, s: f32) -> Laid {
        let mut laid = Laid::default();
        let mut y = 0.0;
        let push = |laid: &mut Laid, text: String, x: f32, y: &mut f32, px: f32, ink| {
            laid.lines.push(Line {
                text,
                x,
                y: *y,
                px,
                ink,
                right: false,
            });
            *y += line_h(px);
        };
        let wrapped = |laid: &mut Laid, text: &str, y: &mut f32, px: f32, ink| {
            for l in wrap(text, columns(w, px)) {
                push(laid, l, 0.0, y, px, ink);
            }
        };
        match &self.page {
            Page::Reading => wrapped(
                &mut laid,
                copy::PLANNING_READING.text,
                &mut y,
                TITLE_PT * s,
                Ink::Dim,
            ),
            Page::Failed(why) => wrapped(&mut laid, why, &mut y, TITLE_PT * s, Ink::Dim),
            Page::List(rows) if rows.is_empty() => wrapped(
                &mut laid,
                copy::PLANNING_NONE.text,
                &mut y,
                TITLE_PT * s,
                Ink::Dim,
            ),
            Page::List(rows) => {
                let row_h = TAP_H * s;
                let px = TITLE_PT * s;
                let small = px * CELL_SCALE;
                let cols = table_columns(rows, w, px, COLUMN_GAP * s);
                for r in rows {
                    let top = y;
                    let cells = [
                        (r.nn.clone(), small, Ink::Text, true),
                        (r.mmdd.clone(), small, Ink::Text, true),
                        (cut(&r.title, cols[2][1], px), px, Ink::Text, false),
                        (status_abbrev(&r.status), small, Ink::Dim, false),
                    ];
                    for ((text, size, ink, right), [x, cw]) in cells.into_iter().zip(cols) {
                        if text.is_empty() {
                            continue;
                        }
                        laid.lines.push(Line {
                            text,
                            x: if right { x + cw } else { x },
                            y: top + (row_h - line_h(size)) * 0.5,
                            px: size,
                            ink,
                            right,
                        });
                    }
                    laid.taps
                        .push((Tap::Row(r.base.clone()), Rect::new(0.0, top, w, row_h)));
                    y = top + row_h;
                    laid.rules.push(y);
                }
            }
            Page::Plan(p) => {
                let tap_h = TAP_H * s;
                let px = TITLE_PT * s;
                let ty = (tap_h - line_h(px)) * 0.5;
                let mut by = ty;
                push(
                    &mut laid,
                    copy::PLANNING_BACK.text.to_string(),
                    0.0,
                    &mut by,
                    px,
                    Ink::Accent,
                );
                laid.taps
                    .push((Tap::Back, Rect::new(0.0, 0.0, w * 0.5, tap_h)));
                let pin = copy::PLANNING_PIN.text;
                let pin_w = columns_w(pin, px);
                let pinned = self.cards.iter().any(|(k, _)| *k == p.row.base);
                if !pinned {
                    let mut py = ty;
                    push(
                        &mut laid,
                        pin.to_string(),
                        w - pin_w,
                        &mut py,
                        px,
                        Ink::Accent,
                    );
                    laid.taps
                        .push((Tap::Pin, Rect::new(w * 0.5, 0.0, w * 0.5, tap_h)));
                }
                y = tap_h + GAP * s * 0.5;
                wrapped(&mut laid, &p.row.title, &mut y, HEAD_PT * s, Ink::Text);
                wrapped(&mut laid, &p.row.subtitle(), &mut y, SMALL_PT * s, Ink::Dim);
                if !p.covers.is_empty() {
                    y += GAP * s * 0.5;
                    for c in p.covers.iter().take(COVERS_MAX) {
                        let item = format!(
                            "\u{2022} {}",
                            c.split_whitespace().collect::<Vec<_>>().join(" ")
                        );
                        wrapped(&mut laid, &item, &mut y, SMALL_PT * s, Ink::Dim);
                    }
                }
                y += GAP * s;
                push(
                    &mut laid,
                    copy::PLANNING_AMENDMENTS.text.to_string(),
                    0.0,
                    &mut y,
                    TITLE_PT * s,
                    Ink::Text,
                );
                if p.amendments.is_empty() {
                    wrapped(
                        &mut laid,
                        copy::PLANNING_NO_AMENDMENTS.text,
                        &mut y,
                        SMALL_PT * s,
                        Ink::Dim,
                    );
                }
                for (at, subject) in &p.amendments {
                    y += 4.0 * s;
                    push(&mut laid, at.clone(), 0.0, &mut y, SMALL_PT * s, Ink::Dim);
                    wrapped(&mut laid, subject, &mut y, SMALL_PT * s, Ink::Text);
                }
            }
        }
        laid.height = y + 4.0 * s;
        laid
    }

    fn lay_out_card(&self, key: &str, w: f32, s: f32) -> Laid {
        let mut laid = Laid::default();
        let mut y = 0.0;
        let push = |laid: &mut Laid, text: &str, y: &mut f32, px: f32, ink| {
            laid.lines.push(Line {
                text: cut(text, w, px),
                x: 0.0,
                y: *y,
                px,
                ink,
                right: false,
            });
            *y += line_h(px);
        };
        match self.cards.iter().find(|(k, _)| k == key).map(|(_, c)| c) {
            Some(CardShows::Plan(p)) => {
                push(&mut laid, &p.row.title, &mut y, TITLE_PT * s, Ink::Text);
                push(&mut laid, &p.row.status, &mut y, SMALL_PT * s, Ink::Dim);
                if let Some((at, subject)) = p.amendments.last() {
                    push(&mut laid, at, &mut y, SMALL_PT * s, Ink::Dim);
                    push(&mut laid, subject, &mut y, SMALL_PT * s, Ink::Text);
                }
            }
            Some(CardShows::Failed(name, why)) => {
                push(&mut laid, name, &mut y, TITLE_PT * s, Ink::Text);
                push(&mut laid, why, &mut y, SMALL_PT * s, Ink::Dim);
            }
            None => {}
        }
        laid.taps.push((Tap::Card, Rect::new(0.0, 0.0, w, y)));
        laid.height = y;
        laid
    }

    /// The page the modal shows, for the tests.
    #[cfg(test)]
    fn page(&self) -> &Page {
        &self.page
    }
}

/// `text` in at most `max` characters, cut with ….
fn cut_to(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// About how wide `text` is at `px`: half an em a character, as
/// [`columns`] counts.
fn columns_w(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * px * 0.5
}

impl DevicePlugin for Planning {
    fn ns(&self) -> &'static str {
        NS
    }

    fn supports(&self) -> &'static str {
        "0.1"
    }

    fn action(&mut self, item: &str, out: &mut PluginOut) {
        if item == "plans" {
            out.open(ViewKind::Modal);
            self.list(out);
        }
    }

    fn menu_opened(&mut self, item: &str, out: &mut PluginOut) {
        if item == "plans" {
            self.menu = None;
            self.asked.retain(|_, a| *a != Asked::Menu);
            let id = out.call("plans.list", json!({}));
            self.asked.insert(id, Asked::Menu);
        }
    }

    fn menu(&self, item: &str) -> Vec<PopupRow> {
        if item != "plans" {
            return Vec::new();
        }
        let all = PopupItem::action(MENU_ALL, copy::PLANNING_ALL_PLANS.text).into();
        match &self.menu {
            None => vec![PopupItem::heading(copy::PLANNING_READING.text).into()],
            Some(Ok(rows)) if rows.is_empty() => vec![
                PopupItem::heading(copy::PLANNING_MENU_NONE.text).into(),
                PopupItem::separator().into(),
                all,
            ],
            Some(Ok(rows)) => {
                let mut items: Vec<PopupRow> = rows.iter().take(MENU_PLANS).map(menu_row).collect();
                items.push(PopupItem::separator().into());
                items.push(all);
                items
            }
            Some(Err(why)) => vec![
                PopupItem::heading(cut_to(why, MENU_COLUMNS)).into(),
                PopupItem::separator().into(),
                all,
            ],
        }
    }

    fn menu_chosen(&mut self, item: &str, id: &str, out: &mut PluginOut) {
        if item != "plans" {
            return;
        }
        if id == MENU_ALL {
            self.action(item, out);
        } else if let Some(base) = id.strip_prefix(MENU_PLAN) {
            out.open(ViewKind::Modal);
            self.show(base, out);
        }
    }

    fn message(&mut self, msg: PluginMessage, out: &mut PluginOut) {
        let PluginMessage::Result { id, result } = msg else {
            return;
        };
        let Some(asked) = self.asked.remove(&id) else {
            return;
        };
        out.redraw();
        match (asked, result) {
            (Asked::Menu, Ok(v)) => {
                let rows = rows_of(&v);
                // The modal's ‹ lists them without reading again.
                self.list = Some(rows.clone());
                self.menu = Some(Ok(rows));
            }
            (Asked::Menu, Err(e)) => self.menu = Some(Err(e.message)),
            (Asked::List, Ok(v)) => {
                let rows = rows_of(&v);
                self.list = Some(rows.clone());
                if self.page == Page::Reading {
                    self.page = Page::List(rows);
                }
            }
            (Asked::Show, Ok(v)) => {
                self.page = match plan_of(&v) {
                    Some(p) => {
                        // A pinned card of it shows it as it is now.
                        if let Some((_, c)) = self.cards.iter_mut().find(|(k, _)| *k == p.row.base)
                        {
                            *c = CardShows::Plan(p.clone());
                        }
                        Page::Plan(Box::new(p))
                    }
                    None => Page::Failed(copy::PLANNING_UNREADABLE.text.to_string()),
                };
            }
            (Asked::Card(key), Ok(v)) => {
                if let Some((_, c)) = self.cards.iter_mut().find(|(k, _)| *k == key) {
                    *c = match plan_of(&v) {
                        Some(p) => CardShows::Plan(p),
                        None => {
                            CardShows::Failed(key.clone(), copy::PLANNING_UNREADABLE.text.into())
                        }
                    };
                }
            }
            (Asked::List | Asked::Show, Err(e)) => self.page = Page::Failed(e.message),
            (Asked::Card(key), Err(e)) => {
                if let Some((_, c)) = self.cards.iter_mut().find(|(k, _)| *k == key) {
                    *c = CardShows::Failed(key.clone(), e.message);
                }
            }
        }
    }

    fn height(&self, view: &ViewKind, w: f32, s: f32) -> f32 {
        self.lay_out(view, w, s).height
    }

    fn draw(&self, view: &ViewKind, rect: Rect, s: f32, ctx: &DrawContext, list: &mut DrawList) {
        self.laid.set((rect.w, s));
        let laid = self.lay_out(view, rect.w, s);
        for &y in &laid.rules {
            fill(
                list,
                Rect::new(rect.x, rect.y + y - 0.5 * s, rect.w, 1.0 * s),
                theme().band,
                0.0,
            );
        }
        for l in &laid.lines {
            let color = match l.ink {
                Ink::Text => theme().text,
                Ink::Dim => theme().dim,
                Ink::Accent => theme().link,
            };
            let x = if l.right {
                l.x - ctx.text_advance(&l.text, l.px)
            } else {
                l.x
            };
            ctx.draw_text(list, &l.text, [rect.x + x, rect.y + l.y], l.px, color);
        }
    }

    fn tap(&mut self, view: &ViewKind, x: f32, y: f32, out: &mut PluginOut) {
        let (w, s) = self.laid.get();
        let laid = self.lay_out(view, w, s);
        let Some((tap, _)) = laid.taps.into_iter().find(|(_, r)| r.contains([x, y])) else {
            return;
        };
        out.redraw();
        match tap {
            Tap::Row(base) => self.show(&base, out),
            Tap::Back => match self.list.clone() {
                Some(rows) => self.page = Page::List(rows),
                None => self.list(out),
            },
            Tap::Pin => {
                if let Page::Plan(p) = &self.page {
                    let key = p.row.base.clone();
                    if !self.cards.iter().any(|(k, _)| *k == key) {
                        self.cards
                            .push((key.clone(), CardShows::Plan((**p).clone())));
                    }
                    out.open(ViewKind::Card(key));
                }
            }
            Tap::Card => {
                if let ViewKind::Card(key) = view {
                    out.open(ViewKind::Modal);
                    self.show(key, out);
                    // The card is read again with it.
                    let id = out.call("plans.show", json!({ "name": key }));
                    self.asked.insert(id, Asked::Card(key.clone()));
                }
            }
        }
    }

    fn close(&mut self, view: &ViewKind) {
        match view {
            ViewKind::Modal => {
                self.page = Page::Reading;
                // Answers still on their way find nothing to fill.
                self.asked
                    .retain(|_, a| matches!(a, Asked::Card(_) | Asked::Menu));
            }
            ViewKind::Card(key) => {
                self.cards.retain(|(k, _)| k != key);
                self.asked.retain(|_, a| *a != Asked::Card(key.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::ErrorObject;

    /// `plans.list` as the daemon half gives it, oldest first.
    fn list_fixture() -> Value {
        json!({"schema": 1, "root": "flankr", "plans": [
            {"file": "docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md",
             "base": "20261024_02_SETTINGS_SCREEN_PLAN", "date_from_name": "2026-10-24",
             "nn": 2, "name": "SETTINGS_SCREEN", "kind": "plan",
             "title": "The settings screen", "date": "2026-10-24",
             "status": "In implementation", "status_base": "In implementation",
             "status_by": null, "last_amendment": "2026-10-28 17:57 EDT",
             "readme": {"row": 30, "status": "In implementation", "decision": "…"},
             "error": null},
            {"file": "docs/design/20261028_04_PLUGINS_PLAN.md",
             "base": "20261028_04_PLUGINS_PLAN", "date_from_name": "2026-10-28",
             "nn": 4, "name": "PLUGINS", "kind": "plan",
             "title": "Built-in plugins, with a namespaced message flow, a title-bar menu and the planning plugin",
             "date": "2026-10-28", "status": "In implementation",
             "status_base": "In implementation", "status_by": null,
             "last_amendment": "2026-10-28 20:11 EDT",
             "readme": {"row": 40, "status": "Draft", "decision": "…"}, "error": null}
        ]})
    }

    fn show_fixture() -> Value {
        json!({"schema": 1, "plan": {
        "file": "docs/design/20261028_04_PLUGINS_PLAN.md",
        "base": "20261028_04_PLUGINS_PLAN", "title": "Built-in plugins",
        "date": "2026-10-28", "status": "In implementation",
        "readme": null,
        "covers": ["A plugin model", "The planning plugin"],
        "amendments": [
            {"index": 1, "line": 939, "timestamp": "2026-10-28 19:47 EDT",
             "subject": "approved; implementation on main-plugins", "living_section": null},
            {"index": 2, "line": 954, "timestamp": "2026-10-28 20:11 EDT",
             "subject": "implementation started; the document is now immutable",
             "living_section": null}
        ]}})
    }

    fn texts(p: &Planning, view: &ViewKind) -> Vec<String> {
        p.lay_out(view, 1000.0, 2.0)
            .lines
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    /// Answers the call `id` with `result`.
    fn answer(p: &mut Planning, id: u64, result: Value) -> PluginOut {
        let mut out = PluginOut::new(100);
        p.message(
            PluginMessage::Result {
                id,
                result: Ok(result),
            },
            &mut out,
        );
        out
    }

    #[test]
    fn the_rows_are_newest_first_with_the_readmes_status_when_it_differs() {
        let rows = rows_of(&list_fixture());
        assert_eq!(rows[0].base, "20261028_04_PLUGINS_PLAN");
        assert_eq!(rows[0].readme_status.as_deref(), Some("Draft"));
        assert_eq!(rows[1].readme_status, None);
        assert!(
            rows[0]
                .subtitle()
                .ends_with("In implementation (README: Draft)")
        );
        assert!(
            rows[1]
                .subtitle()
                .starts_with("2026-10-24 \u{b7} In implementation")
        );
    }

    #[test]
    fn the_action_reads_then_lists_and_a_row_shows_its_plan() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.action("plans", &mut out);
        assert_eq!(out.opens, [ViewKind::Modal]);
        assert_eq!(out.calls[0].method, "plans.list");
        assert_eq!(texts(&p, &ViewKind::Modal), [copy::PLANNING_READING.text]);
        answer(&mut p, 1, list_fixture());
        let shown = texts(&p, &ViewKind::Modal);
        // Four cells a row: NN, mm/dd, the title (cut), the status.
        assert_eq!(shown.len(), 8, "{shown:?}");
        assert_eq!(shown[..2], ["4", "10/28"]);
        assert!(shown[2].starts_with("Built-in"));
        assert_eq!(shown[3], "InImpl");
        // Tap the first row.
        p.laid.set((1000.0, 2.0));
        let mut out = PluginOut::new(2);
        p.tap(&ViewKind::Modal, 10.0, 10.0, &mut out);
        assert_eq!(out.calls[0].method, "plans.show");
        assert_eq!(
            out.calls[0].params,
            json!({"name": "20261028_04_PLUGINS_PLAN"})
        );
        answer(&mut p, 2, show_fixture());
        let shown = texts(&p, &ViewKind::Modal);
        assert!(
            shown.contains(&"2026-10-28 19:47 EDT".to_string()),
            "{shown:?}"
        );
        assert!(shown.contains(&"2026-10-28 20:11 EDT".to_string()));
        assert!(shown.contains(&copy::PLANNING_AMENDMENTS.text.to_string()));
        assert!(shown.iter().any(|t| t.contains("A plugin model")));
        // ‹ returns to the list without a call.
        let mut out = PluginOut::new(3);
        p.tap(&ViewKind::Modal, 10.0, 10.0, &mut out);
        assert!(out.calls.is_empty());
        assert!(matches!(p.page(), Page::List(_)));
    }

    #[test]
    fn pin_makes_a_card_that_shows_the_last_amendment_and_reads_the_plan_again() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.action("plans", &mut out);
        let mut out = PluginOut::new(2);
        p.show("20261028_04_PLUGINS_PLAN", &mut out);
        answer(&mut p, 2, show_fixture());
        p.laid.set((1000.0, 2.0));
        // Pin, at the first line's right.
        let mut out = PluginOut::new(3);
        p.tap(&ViewKind::Modal, 990.0, 10.0, &mut out);
        let key = "20261028_04_PLUGINS_PLAN".to_string();
        assert_eq!(out.opens, [ViewKind::Card(key.clone())]);
        let card = ViewKind::Card(key.clone());
        assert_eq!(
            texts(&p, &card),
            [
                "Built-in plugins",
                "In implementation",
                "2026-10-28 20:11 EDT",
                "implementation started; the document is now immutable"
            ]
        );
        // Pinned: no Pin offered.
        assert!(!texts(&p, &ViewKind::Modal).contains(&copy::PLANNING_PIN.text.to_string()));
        // A tap on the card opens the modal on it, read again.
        let mut out = PluginOut::new(4);
        p.tap(&card, 10.0, 10.0, &mut out);
        assert_eq!(out.opens, [ViewKind::Modal]);
        assert_eq!(out.calls.len(), 2);
        assert!(out.calls.iter().all(|c| c.method == "plans.show"));
        // Unpinned: gone.
        p.close(&card);
        assert!(texts(&p, &card).is_empty());
    }

    #[test]
    fn an_error_is_shown_and_a_late_answer_after_close_is_ignored() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.action("plans", &mut out);
        let mut out = PluginOut::new(5);
        p.message(
            PluginMessage::Result {
                id: 1,
                result: Err(ErrorObject::new(-32103, "Reading files is off")),
            },
            &mut out,
        );
        assert_eq!(texts(&p, &ViewKind::Modal), ["Reading files is off"]);
        let mut out = PluginOut::new(2);
        p.action("plans", &mut out);
        p.close(&ViewKind::Modal);
        answer(&mut p, 2, list_fixture());
        assert_eq!(texts(&p, &ViewKind::Modal), [copy::PLANNING_READING.text]);
    }

    /// Each row as `id = cells` (or its label), headings `# `.
    fn labels(rows: &[PopupRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r.item.kind {
                libcontrols::PopupItemKind::Heading => format!("# {}", r.item.label),
                libcontrols::PopupItemKind::Separator => "---".into(),
                _ if r.cells.is_empty() => format!("{} = {}", r.item.id.as_str(), r.item.label),
                _ => format!(
                    "{} = {}",
                    r.item.id.as_str(),
                    r.cells
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" | ")
                ),
            })
            .collect()
    }

    #[test]
    fn the_popup_reads_then_lists_the_newest_with_all_plans_last() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.menu_opened("plans", &mut out);
        // No modal: only the list asked for.
        assert!(out.opens.is_empty());
        assert_eq!(out.calls.len(), 1);
        assert_eq!(out.calls[0].method, "plans.list");
        assert_eq!(
            labels(&p.menu("plans")),
            [format!("# {}", copy::PLANNING_READING.text)]
        );
        assert!(p.menu("other").is_empty());
        let out = answer(&mut p, 1, list_fixture());
        assert!(out.redraw);
        assert_eq!(
            labels(&p.menu("plans")),
            [
                "plan/20261028_04_PLUGINS_PLAN = 4 | 10/28 | Built-in plugins, with a namespaced \
                 message flow, a title-bar menu and the planning plugin | InImpl"
                    .to_string(),
                "plan/20261024_02_SETTINGS_SCREEN_PLAN = 2 | 10/24 | The settings screen | InImpl"
                    .to_string(),
                "---".to_string(),
                format!("all = {}", copy::PLANNING_ALL_PLANS.text),
            ]
        );
        // The modal is not opened by it.
        assert_eq!(texts(&p, &ViewKind::Modal), [copy::PLANNING_READING.text]);
    }

    #[test]
    fn the_popup_holds_ten_and_says_why_when_it_cannot_read() {
        let plans: Vec<Value> = (1..=12)
            .map(|n| {
                json!({"base": format!("20261028_{n:02}_P{n}_PLAN"), "title": format!("Plan {n}"),
                       "status": "Draft"})
            })
            .collect();
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.menu_opened("plans", &mut out);
        answer(&mut p, 1, json!({"schema": 1, "plans": plans}));
        let items = p.menu("plans");
        assert_eq!(items.len(), MENU_PLANS + 2);
        assert_eq!(items[0].item.id.as_str(), "plan/20261028_12_P12_PLAN");
        assert_eq!(
            labels(&items)[0],
            "plan/20261028_12_P12_PLAN = 12 | 10/28 | Plan 12 | Draft"
        );
        assert_eq!(items[9].item.id.as_str(), "plan/20261028_03_P3_PLAN");
        // An empty list, then an error.
        let mut out = PluginOut::new(2);
        p.menu_opened("plans", &mut out);
        assert_eq!(p.menu("plans").len(), 1, "reading again");
        answer(&mut p, 2, json!({"schema": 1, "plans": []}));
        assert_eq!(
            labels(&p.menu("plans"))[0],
            format!("# {}", copy::PLANNING_MENU_NONE.text)
        );
        let mut out = PluginOut::new(3);
        p.menu_opened("plans", &mut out);
        let mut out = PluginOut::new(4);
        p.message(
            PluginMessage::Result {
                id: 3,
                result: Err(ErrorObject::new(-32110, "No plans in docs/design")),
            },
            &mut out,
        );
        let l = labels(&p.menu("plans"));
        assert_eq!(l[0], "# No plans in docs/design");
        assert_eq!(
            l.last().unwrap(),
            &format!("all = {}", copy::PLANNING_ALL_PLANS.text)
        );
    }

    #[test]
    fn all_plans_opens_the_modal_and_a_plan_opens_it_on_that_plan() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.menu_opened("plans", &mut out);
        answer(&mut p, 1, list_fixture());
        // All plans…: today's modal, read again.
        let mut out = PluginOut::new(2);
        p.menu_chosen("plans", MENU_ALL, &mut out);
        assert_eq!(out.opens, [ViewKind::Modal]);
        assert_eq!(out.calls[0].method, "plans.list");
        p.close(&ViewKind::Modal);
        // A plan: the modal on it.
        let mut out = PluginOut::new(3);
        p.menu_chosen("plans", "plan/20261028_04_PLUGINS_PLAN", &mut out);
        assert_eq!(out.opens, [ViewKind::Modal]);
        assert_eq!(out.calls[0].method, "plans.show");
        assert_eq!(
            out.calls[0].params,
            json!({"name": "20261028_04_PLUGINS_PLAN"})
        );
        answer(&mut p, 3, show_fixture());
        assert!(matches!(p.page(), Page::Plan(_)));
        // ‹ lists what the popup read, without a call.
        p.laid.set((1000.0, 2.0));
        let mut out = PluginOut::new(4);
        p.tap(&ViewKind::Modal, 10.0, 10.0, &mut out);
        assert!(out.calls.is_empty());
        assert!(matches!(p.page(), Page::List(rows) if rows.len() == 2));
    }

    fn plan(base: &str, date: Option<&str>, last: Option<&str>, status: &str) -> Value {
        json!({"base": base, "title": base, "status": status,
               "date": date, "last_amendment": last})
    }

    #[test]
    fn rows_are_sorted_by_last_amendment_else_date_else_the_file_names_date() {
        let result = json!({"schema": 1, "plans": [
            // Oldest by name, but amended last (23:30 EST is 04:30 UTC).
            plan("20261001_01_A_PLAN", Some("2026-10-01"), Some("2026-11-02 23:30 EST"), "Draft"),
            plan("20261102_01_B_PLAN", Some("2026-11-02"), Some("2026-11-03 00:10 EDT"), "Draft"),
            // No amendment: its front-matter date, a day later than its name.
            plan("20261020_01_C_PLAN", Some("2026-10-25"), None, "Draft"),
            // Neither: its file name's date.
            plan("20261024_02_D_PLAN", None, None, "Draft"),
            plan("20261024_01_E_PLAN", None, None, "Draft"),
            // Nothing readable: last.
            plan("NOTES", None, None, "Draft"),
        ]});
        let order: Vec<String> = rows_of(&result).into_iter().map(|r| r.base).collect();
        assert_eq!(
            order,
            [
                "20261001_01_A_PLAN",
                "20261102_01_B_PLAN",
                "20261020_01_C_PLAN",
                "20261024_02_D_PLAN",
                "20261024_01_E_PLAN",
                "NOTES"
            ]
        );
        assert_eq!(
            update_key("2026-10-28 23:14 EDT"),
            update_key("2026-10-29 03:14 UTC")
        );
        assert_eq!(
            update_key("2026-10-28 23:14 -04:00"),
            update_key("2026-10-29 03:14")
        );
        assert_eq!(update_key("1970-01-02"), Some(1440));
        assert_eq!(update_key("not a date"), None);
        assert_eq!(update_key("2026-13-01"), None);
    }

    #[test]
    fn nn_and_mm_dd_come_from_the_file_name_else_the_fields() {
        let r = PlanRow::from_json(&json!({"base": "20261028_04_PLUGINS_PLAN"})).unwrap();
        assert_eq!((r.nn.as_str(), r.mmdd.as_str()), ("4", "10/28"));
        let r = PlanRow::from_json(
            &json!({"base": "PLUGINS", "nn": 7, "date_from_name": "2026-03-09"}),
        )
        .unwrap();
        assert_eq!((r.nn.as_str(), r.mmdd.as_str()), ("7", "03/09"));
        assert_eq!(r.updated, update_key("2026-03-09"));
    }

    #[test]
    fn statuses_are_abbreviated() {
        for (status, shown) in [
            ("Draft", "Draft"),
            ("Hold", "Hold"),
            ("In implementation", "InImpl"),
            ("Implemented", "Impl"),
            (
                "Superseded (by [the next](docs/design/20261029_01_X_PLAN.md))",
                "Succd",
            ),
            ("Retired (by 20261029_02_Y_PLAN)", "Rtrd"),
            ("Approved", "Appro"),
            ("Open", "Open"),
            ("  Draft ", "Draft"),
        ] {
            assert_eq!(status_abbrev(status), shown, "{status}");
        }
    }

    #[test]
    fn a_popup_row_is_four_cells_with_the_small_ones_at_0_8() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.menu_opened("plans", &mut out);
        answer(&mut p, 1, list_fixture());
        let rows = p.menu("plans");
        let cells = &rows[0].cells;
        assert_eq!(cells.len(), 4);
        let [nn, mmdd, title, status] = [&cells[0], &cells[1], &cells[2], &cells[3]];
        assert_eq!((nn.text.as_str(), nn.align), ("4", TextAlign::Right));
        assert_eq!(
            (mmdd.text.as_str(), mmdd.align),
            ("10/28", TextAlign::Right)
        );
        assert_eq!(title.align, TextAlign::Left);
        assert_eq!(title.width, libcontrols::PopupCellWidth::Flex);
        assert_eq!(status.text, "InImpl");
        assert_eq!(status.align, TextAlign::Left);
        // Sizes: all but the title at 0.8; only the status dim.
        assert_eq!(
            [nn.scale, mmdd.scale, title.scale, status.scale],
            [0.8, 0.8, 1.0, 0.8]
        );
        assert_eq!(
            [nn.dim, mmdd.dim, title.dim, status.dim],
            [false, false, false, true]
        );
        // The other columns size to their widest cell, so they line up.
        for c in [nn, mmdd, status] {
            assert_eq!(c.width, libcontrols::PopupCellWidth::Auto);
        }
        // Headings and All plans… stay plain labels.
        assert!(rows.last().unwrap().cells.is_empty());
    }

    #[test]
    fn the_popup_lays_its_cells_out_in_shared_columns_cut_and_centred() {
        let menu = libcontrols::PopupMenu::new(Rect::new(0.0, 0.0, 1000.0, 1000.0))
            .with_options(libcontrols::PopupMenuOptions::new().max_width(320.0));
        let state = std::sync::Arc::new(libcontrols::PopupMenuState::new());
        let menu = menu.with_state(std::sync::Arc::clone(&state));
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.menu_opened("plans", &mut out);
        answer(&mut p, 1, list_fixture());
        state.open_rows(Rect::new(900.0, 0.0, 40.0, 40.0), p.menu("plans"));
        let ctx = DrawContext::default();
        let mut list = DrawList::new();
        libcontrols::DrawControl::draw(&menu, &mut list, &ctx);
        let layout = state.layout().expect("drawn");
        assert_eq!(layout.columns.len(), 4);
        // In order, left to right, inside the frame.
        for k in 1..4 {
            assert!(layout.columns[k][0] > layout.columns[k - 1][0]);
        }
        let [x, w] = layout.columns[3];
        assert!(x + w <= layout.frame.x + layout.frame.w);
        // The long title takes the room left: the menu is at its widest.
        assert!((layout.frame.w - 320.0).abs() < 0.01, "{}", layout.frame.w);
    }

    #[test]
    fn the_modal_table_has_four_columns_with_the_small_ones_at_0_8_centred() {
        let mut p = Planning::new();
        let mut out = PluginOut::new(1);
        p.action("plans", &mut out);
        answer(&mut p, 1, list_fixture());
        let (w, s) = (1000.0, 2.0);
        let laid = p.lay_out(&ViewKind::Modal, w, s);
        assert_eq!(laid.lines.len(), 8);
        let row_h = TAP_H * s;
        let px = TITLE_PT * s;
        for (i, row) in laid.lines.chunks(4).enumerate() {
            let top = i as f32 * row_h;
            let sizes: Vec<f32> = row.iter().map(|l| l.px).collect();
            assert_eq!(sizes, [px * 0.8, px * 0.8, px, px * 0.8]);
            assert_eq!(
                row.iter().map(|l| l.ink).collect::<Vec<_>>(),
                [Ink::Text, Ink::Text, Ink::Text, Ink::Dim]
            );
            assert_eq!(
                row.iter().map(|l| l.right).collect::<Vec<_>>(),
                [true, true, false, false]
            );
            // Each vertically centred on its row.
            for l in row {
                let mid = l.y + line_h(l.px) * 0.5;
                assert!((mid - (top + row_h * 0.5)).abs() < 0.01, "{l:?}");
            }
            // Left to right, inside the width.
            assert!(row[0].x < row[1].x && row[1].x < row[2].x && row[2].x < row[3].x);
            assert!(row[3].x + columns_w(&row[3].text, row[3].px) * 1.2 <= w);
        }
        // The rows line up: the same edges in each.
        let xs =
            |i: usize| -> Vec<f32> { laid.lines[i * 4..i * 4 + 4].iter().map(|l| l.x).collect() };
        assert_eq!(xs(0), xs(1));
        // One tap and one rule a row.
        assert_eq!(laid.taps.len(), 2);
        assert_eq!(laid.rules, [row_h, 2.0 * row_h]);
        // A narrow modal cuts the title with … and keeps the rest.
        let narrow = p.lay_out(&ViewKind::Modal, 300.0, 1.0);
        assert!(
            narrow.lines[2].text.ends_with('\u{2026}'),
            "{:?}",
            narrow.lines[2]
        );
        assert_eq!(narrow.lines[3].text, "InImpl");
        // The second row taps to its plan.
        p.laid.set((w, s));
        let mut out = PluginOut::new(2);
        p.tap(&ViewKind::Modal, 10.0, row_h + 10.0, &mut out);
        assert_eq!(
            out.calls[0].params,
            json!({"name": "20261024_02_SETTINGS_SCREEN_PLAN"})
        );
    }
}
