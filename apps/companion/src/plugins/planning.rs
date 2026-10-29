//! The planning plugin's device half, `dev.flankr.plugins.planning`
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.8, item 7), for
//! its daemon half's `0.1` releases.
//!
//! - **Plans in this project** (`plans`) opens the modal and calls
//!   `plans.list`: "Reading plans…", then one row per plan, newest at the
//!   top: its title on one line, cut with …, and under it the date and the
//!   front matter's status, with "(README: <status>)" when the README's
//!   table says otherwise. A row's tap calls `plans.show` and shows the
//!   plan in place of the list.
//! - **The plan**: a ‹ back to the list and **Pin** on the first line;
//!   its title, date and status; its `covers` as a short list; then
//!   **Amendments**, each its timestamp exactly as written and its
//!   subject.
//! - **The pinned card**, keyed by the plan's base name: its title, its
//!   status, and its last amendment's timestamp and subject; a tap opens
//!   the modal on that plan, calling `plans.show` again so it is current.
//!
//! The results are tpt's schema-1 shapes (decision 3.8, item 4); only
//! these fields are read, and nothing of a plan's body is shown.

use std::cell::Cell;
use std::collections::HashMap;

use libcontrols::{DrawContext, Rect};
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
/// A list row's height, a tap target's, and the space between blocks.
const ROW_H: f32 = 56.0;
const TAP_H: f32 = 44.0;
const GAP: f32 = 12.0;
/// The most `covers` items shown.
const COVERS_MAX: usize = 6;

/// A plan as `plans.list` gives it, as far as the list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanRow {
    pub base: String,
    pub title: String,
    pub date: String,
    pub status: String,
    /// The README's status, when its table says otherwise.
    pub readme_status: Option<String>,
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
        Some(PlanRow {
            base,
            title,
            date: text(v, "date"),
            status,
            readme_status,
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

/// `plans.list`'s plans, newest first (it gives them oldest first).
pub fn rows_of(result: &Value) -> Vec<PlanRow> {
    let mut rows: Vec<PlanRow> = result
        .get("plans")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(PlanRow::from_json).collect())
        .unwrap_or_default();
    rows.reverse();
    rows
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
    Plan(PlanView),
    Failed(String),
}

/// What a call was for.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    List,
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
    x: f32,
    y: f32,
    px: f32,
    ink: Ink,
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
                let row_h = ROW_H * s;
                for r in rows {
                    let top = y;
                    let mut ly = top + 6.0 * s;
                    push(
                        &mut laid,
                        cut(&r.title, w, TITLE_PT * s),
                        0.0,
                        &mut ly,
                        TITLE_PT * s,
                        Ink::Text,
                    );
                    push(
                        &mut laid,
                        cut(&r.subtitle(), w, SMALL_PT * s),
                        0.0,
                        &mut ly,
                        SMALL_PT * s,
                        Ink::Dim,
                    );
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

    fn message(&mut self, msg: PluginMessage, out: &mut PluginOut) {
        let PluginMessage::Result { id, result } = msg else {
            return;
        };
        let Some(asked) = self.asked.remove(&id) else {
            return;
        };
        out.redraw();
        match (asked, result) {
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
                        Page::Plan(p)
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
            ctx.draw_text(list, &l.text, [rect.x + l.x, rect.y + l.y], l.px, color);
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
                        self.cards.push((key.clone(), CardShows::Plan(p.clone())));
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
                self.asked.retain(|_, a| matches!(a, Asked::Card(_)));
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
        assert_eq!(shown.len(), 4, "{shown:?}");
        // One line each title, cut.
        assert!(shown[0].ends_with('\u{2026}') || shown[0].starts_with("Built-in"));
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
}
