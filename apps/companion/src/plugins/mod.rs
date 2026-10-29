//! The plugins' device halves (docs/design/20261028_04_PLUGINS_PLAN.md,
//! decisions 3.1 item 4, 3.4 item 3, 3.7 and step 6).
//!
//! A plugin is built-in Rust in two halves under one reverse-DNS package
//! name, its namespace (`ns`): the daemon half in flankr, and the device
//! half here, a [`DevicePlugin`]. [`Plugins`] holds the device halves and
//! the daemon's [`PluginInfo`] list (from `hello` and
//! `server.capabilities`), and is the router of the message flow: each
//! `plugin.result` and `plugin.notify` goes to the device half whose `ns`
//! it names, and to no other. A device half cannot name another
//! namespace: what it asks of the app goes through [`PluginOut`], which
//! has no `ns`, and [`Plugins`] fills in its own.
//!
//! A plugin can be used when the daemon lists it, the app has its device
//! half, and the daemon's version is in the half's
//! [`DevicePlugin::supports`] range ([`availability`]); its menu items
//! show only while it is also enabled. Its views are one modal at a time
//! (over the screen, [`modal`]) and pinned cards at the end of the
//! overview's grid, at most [`MAX_CARDS`], kept for the app's run.

pub mod card;
pub mod modal;
pub mod planning;

use libcontrols::{DrawContext, Rect};
use libflankr_proto::ErrorObject;
use libflankr_proto::companion::{PluginInfo, PluginNotify, PluginResult};
use libmsdf::drawlist::DrawList;
use serde_json::Value;

/// The most plugin cards pinned at once; pinning another drops the oldest
/// (decision 3.7, item 2).
pub const MAX_CARDS: usize = 4;

/// A plugin view: the modal, or a pinned card with its plugin's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Modal,
    Card(String),
}

/// A message for a device half, of its own namespace only.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginMessage {
    /// The answer to the call with this `id`: its result, or its error
    /// (the daemon's, the plugin's, or the client's refusal).
    Result {
        id: u64,
        result: Result<Value, ErrorObject>,
    },
    /// A notification during a call.
    Notify { method: String, params: Value },
}

/// One call a device half asked for: the app sends it as `plugin.call`
/// with the half's `ns` and the agent view's pane as its `session`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub id: Option<u64>,
    pub method: String,
    pub params: Value,
}

/// What a device half asks of the app while it handles an action, a
/// message or a tap (decision 3.1, item 4).
#[derive(Debug, Default)]
pub struct PluginOut {
    next_id: u64,
    calls: Vec<Call>,
    opens: Vec<ViewKind>,
    close: bool,
    redraw: bool,
}

impl PluginOut {
    fn new(next_id: u64) -> Self {
        PluginOut {
            next_id,
            ..PluginOut::default()
        }
    }

    /// A call to this plugin's daemon half whose answer comes back as
    /// [`PluginMessage::Result`] with the id returned.
    pub fn call(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.calls.push(Call {
            id: Some(id),
            method: method.to_string(),
            params,
        });
        id
    }

    /// A call with no answer: a notification to the daemon half.
    pub fn notify(&mut self, method: &str, params: Value) {
        self.calls.push(Call {
            id: None,
            method: method.to_string(),
            params,
        });
    }

    /// Opens the modal, or pins a card with this key.
    pub fn open(&mut self, view: ViewKind) {
        self.opens.push(view);
    }

    /// Closes this plugin's modal.
    pub fn close(&mut self) {
        self.close = true;
    }

    /// Its views changed: they are drawn again.
    pub fn redraw(&mut self) {
        self.redraw = true;
    }
}

/// A plugin's device half (decision 3.1, item 4). Coordinates given to
/// [`DevicePlugin::tap`] are the view's own: from the top left of the
/// rect [`DevicePlugin::draw`] is given, scrolled content included.
pub trait DevicePlugin {
    /// The package name, e.g. `dev.flankr.plugins.planning`.
    fn ns(&self) -> &'static str;
    /// The daemon versions it can talk to, in Cargo's caret meaning
    /// (`"0.1"` accepts `0.1.x`), or exactly `"@g<hash>"` for one
    /// unreleased version (decision 3.4, item 3).
    fn supports(&self) -> &'static str;
    /// A menu item of its was chosen; it may send calls and open a view.
    fn action(&mut self, item: &str, out: &mut PluginOut);
    /// A result or notification for its namespace.
    fn message(&mut self, msg: PluginMessage, out: &mut PluginOut);
    /// The view's content height at width `w` and scale `s`: the modal
    /// scrolls it, a card is this tall.
    fn height(&self, view: &ViewKind, w: f32, s: f32) -> f32;
    /// The view's content, drawn in `rect` at scale `s`.
    fn draw(&self, view: &ViewKind, rect: Rect, s: f32, ctx: &DrawContext, list: &mut DrawList);
    /// A tap at `(x, y)` in the view's own coordinates.
    fn tap(&mut self, view: &ViewKind, x: f32, y: f32, out: &mut PluginOut);
    /// Back, the modal's close, a card unpinned, or the plugin turned off.
    fn close(&mut self, view: &ViewKind);
}

/// Whether the daemon's plugin `version` is in a device half's `range`
/// (decision 3.4, item 3): a release version against a caret
/// requirement, or a hash version (`g1a2b3c4`) only against exactly
/// `@g1a2b3c4`.
pub fn supports(range: &str, version: &str) -> bool {
    if let Some(hash) = range.strip_prefix('@') {
        return hash.starts_with('g') && hash == version;
    }
    match (
        semver::VersionReq::parse(range),
        semver::Version::parse(version),
    ) {
        (Ok(req), Ok(v)) => req.matches(&v),
        _ => false,
    }
}

/// Whether a listed plugin can be used here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Usable,
    /// The app has no device half for it, or its version is past the
    /// half's range (or an unreleased hash the half doesn't name).
    NeedsNewerApp,
    /// The daemon's version is below the half's range.
    NeedsNewerFlankr,
}

/// `info`'s availability with the device half `half`, if the app has one.
pub fn availability(info: &PluginInfo, half: Option<&dyn DevicePlugin>) -> Availability {
    let Some(half) = half else {
        return Availability::NeedsNewerApp;
    };
    let range = half.supports();
    if supports(range, &info.version) {
        return Availability::Usable;
    }
    // Below the range's lowest version: the daemon is the older one.
    let low = range
        .trim_start_matches(['^', '='])
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .chain(std::iter::repeat(0))
        .take(3)
        .collect::<Vec<_>>();
    match semver::Version::parse(&info.version) {
        Ok(v) if (v.major, v.minor, v.patch) < (low[0], low[1], low[2]) => {
            Availability::NeedsNewerFlankr
        }
        _ => Availability::NeedsNewerApp,
    }
}

/// The screens with a ⋮ (decision 3.6, item 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Agent,
    Settings,
}

impl Screen {
    /// Whether a menu item's `screens` (`"overview"`, `"agent"` or
    /// `"both"`) puts it on this screen. Settings has no plugin items.
    fn shows(self, screens: &str) -> bool {
        match self {
            Screen::Overview => matches!(screens, "overview" | "both"),
            Screen::Agent => matches!(screens, "agent" | "both"),
            Screen::Settings => false,
        }
    }
}

/// A plugin's menu item, for the ⋮ menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub ns: String,
    /// The plugin's name: the group's heading.
    pub name: String,
    pub item: String,
    pub label: String,
}

/// A call for the app to send, with its plugin's `ns`.
pub type Outgoing = (String, Call);

/// The device halves and the daemon's plugin list: the router by `ns`.
pub struct Plugins {
    halves: Vec<Box<dyn DevicePlugin>>,
    infos: Vec<PluginInfo>,
    next_id: u64,
    /// The plugin whose modal is open.
    modal: Option<String>,
    /// The pinned cards, oldest first: `(ns, key)`.
    cards: Vec<(String, String)>,
    dirty: bool,
}

impl Plugins {
    pub fn new(halves: Vec<Box<dyn DevicePlugin>>) -> Self {
        Plugins {
            halves,
            infos: Vec::new(),
            next_id: 1,
            modal: None,
            cards: Vec::new(),
            dirty: false,
        }
    }

    /// The app's built-in device halves.
    pub fn builtin() -> Self {
        Plugins::new(vec![Box::new(planning::Planning::new())])
    }

    fn half(&self, ns: &str) -> Option<&dyn DevicePlugin> {
        self.halves.iter().find(|h| h.ns() == ns).map(|h| &**h)
    }

    fn half_mut(&mut self, ns: &str) -> Option<&mut Box<dyn DevicePlugin>> {
        self.halves.iter_mut().find(|h| h.ns() == ns)
    }

    /// The daemon's plugins, as last told.
    pub fn infos(&self) -> &[PluginInfo] {
        &self.infos
    }

    /// `ns`'s availability; a plugin the daemon doesn't list needs a
    /// newer flankr.
    pub fn availability(&self, ns: &str) -> Availability {
        match self.infos.iter().find(|i| i.ns == ns) {
            Some(info) => availability(info, self.half(ns)),
            None => Availability::NeedsNewerFlankr,
        }
    }

    /// Enabled in the daemon and usable here.
    pub fn active(&self, ns: &str) -> bool {
        self.infos.iter().any(|i| i.ns == ns && i.enabled)
            && self.availability(ns) == Availability::Usable
    }

    /// The daemon's list, as `Event::Plugins` gives it: the views of a
    /// plugin no longer active close, and its cards go (decision 3.7,
    /// item 3).
    pub fn set_infos(&mut self, infos: Vec<PluginInfo>) {
        self.infos = infos;
        if let Some(ns) = self.modal.clone()
            && !self.active(&ns)
        {
            self.close_modal();
        }
        let gone: Vec<(String, String)> = self
            .cards
            .iter()
            .filter(|(ns, _)| !self.active(ns))
            .cloned()
            .collect();
        for (ns, key) in gone {
            self.drop_card(&ns, &key);
        }
        self.dirty = true;
    }

    /// The active plugins' items for `screen`, in the daemon's order.
    pub fn menu(&self, screen: Screen) -> Vec<MenuEntry> {
        self.infos
            .iter()
            .filter(|i| self.active(&i.ns))
            .flat_map(|i| {
                i.menu
                    .iter()
                    .filter(|m| screen.shows(&m.screens))
                    .map(|m| MenuEntry {
                        ns: i.ns.clone(),
                        name: i.name.clone(),
                        item: m.id.clone(),
                        label: m.label.clone(),
                    })
            })
            .collect()
    }

    /// Runs `f` on `ns`'s half with a fresh [`PluginOut`], then carries
    /// out what it asked. Nothing for an inactive plugin.
    fn with_half(
        &mut self,
        ns: &str,
        f: impl FnOnce(&mut dyn DevicePlugin, &mut PluginOut),
    ) -> Vec<Outgoing> {
        if !self.active(ns) {
            return Vec::new();
        }
        let mut out = PluginOut::new(self.next_id);
        let Some(half) = self.half_mut(ns) else {
            return Vec::new();
        };
        f(&mut **half, &mut out);
        self.apply(ns, out)
    }

    fn apply(&mut self, ns: &str, out: PluginOut) -> Vec<Outgoing> {
        self.next_id = out.next_id;
        if out.close && self.modal.as_deref() == Some(ns) {
            self.close_modal();
        }
        for view in out.opens {
            match view {
                ViewKind::Modal => {
                    if let Some(other) = self.modal.clone()
                        && other != ns
                    {
                        self.close_modal();
                    }
                    self.modal = Some(ns.to_string());
                }
                ViewKind::Card(key) => {
                    let card = (ns.to_string(), key);
                    if !self.cards.contains(&card) {
                        self.cards.push(card);
                    }
                    while self.cards.len() > MAX_CARDS {
                        let (ns, key) = self.cards[0].clone();
                        self.drop_card(&ns, &key);
                    }
                }
            }
        }
        self.dirty = true;
        out.calls.into_iter().map(|c| (ns.to_string(), c)).collect()
    }

    /// A menu item of `ns` was chosen.
    pub fn action(&mut self, ns: &str, item: &str) -> Vec<Outgoing> {
        self.with_half(ns, |h, out| h.action(item, out))
    }

    /// A `plugin.result`, for its namespace's half only.
    pub fn result(&mut self, r: PluginResult) -> Vec<Outgoing> {
        let result = match (r.result, r.error) {
            (_, Some(e)) => Err(e),
            (Some(v), None) => Ok(v),
            (None, None) => Ok(Value::Null),
        };
        let msg = PluginMessage::Result { id: r.id, result };
        self.with_half(&r.ns, |h, out| h.message(msg, out))
    }

    /// A `plugin.notify`, for its namespace's half only.
    pub fn notify(&mut self, n: PluginNotify) -> Vec<Outgoing> {
        let msg = PluginMessage::Notify {
            method: n.method,
            params: n.params,
        };
        self.with_half(&n.ns, |h, out| h.message(msg, out))
    }

    /// The plugin whose modal is open, and its name.
    pub fn modal(&self) -> Option<(&str, &str)> {
        let ns = self.modal.as_deref()?;
        let name = self
            .infos
            .iter()
            .find(|i| i.ns == ns)
            .map_or(ns, |i| i.name.as_str());
        Some((ns, name))
    }

    /// Closes the open modal, telling its half.
    pub fn close_modal(&mut self) {
        if let Some(ns) = self.modal.take()
            && let Some(h) = self.half_mut(&ns)
        {
            h.close(&ViewKind::Modal);
        }
        self.dirty = true;
    }

    /// The open modal's content height.
    pub fn modal_height(&self, w: f32, s: f32) -> f32 {
        self.modal
            .as_deref()
            .and_then(|ns| self.half(ns))
            .map_or(0.0, |h| h.height(&ViewKind::Modal, w, s))
    }

    pub fn draw_modal(&self, rect: Rect, s: f32, ctx: &DrawContext, list: &mut DrawList) {
        if let Some(h) = self.modal.as_deref().and_then(|ns| self.half(ns)) {
            h.draw(&ViewKind::Modal, rect, s, ctx, list);
        }
    }

    /// A tap in the open modal's content, in its coordinates.
    pub fn tap_modal(&mut self, x: f32, y: f32) -> Vec<Outgoing> {
        let Some(ns) = self.modal.clone() else {
            return Vec::new();
        };
        self.with_half(&ns, |h, out| h.tap(&ViewKind::Modal, x, y, out))
    }

    /// The pinned cards, oldest first: `(ns, key)`.
    pub fn cards(&self) -> &[(String, String)] {
        &self.cards
    }

    /// The name of card `i`'s plugin, its header.
    pub fn card_name(&self, i: usize) -> Option<&str> {
        let (ns, _) = self.cards.get(i)?;
        Some(
            self.infos
                .iter()
                .find(|p| &p.ns == ns)
                .map_or(ns.as_str(), |p| p.name.as_str()),
        )
    }

    pub fn card_height(&self, i: usize, w: f32, s: f32) -> f32 {
        let Some((ns, key)) = self.cards.get(i) else {
            return 0.0;
        };
        self.half(ns)
            .map_or(0.0, |h| h.height(&ViewKind::Card(key.clone()), w, s))
    }

    pub fn draw_card(&self, i: usize, rect: Rect, s: f32, ctx: &DrawContext, list: &mut DrawList) {
        if let Some((ns, key)) = self.cards.get(i)
            && let Some(h) = self.half(ns)
        {
            h.draw(&ViewKind::Card(key.clone()), rect, s, ctx, list);
        }
    }

    /// A tap on card `i`, in its coordinates.
    pub fn tap_card(&mut self, i: usize, x: f32, y: f32) -> Vec<Outgoing> {
        let Some((ns, key)) = self.cards.get(i).cloned() else {
            return Vec::new();
        };
        self.with_half(&ns, |h, out| h.tap(&ViewKind::Card(key), x, y, out))
    }

    /// Unpins card `i`.
    pub fn unpin(&mut self, i: usize) {
        if let Some((ns, key)) = self.cards.get(i).cloned() {
            self.drop_card(&ns, &key);
        }
    }

    fn drop_card(&mut self, ns: &str, key: &str) {
        self.cards.retain(|(n, k)| !(n == ns && k == key));
        if let Some(h) = self.half_mut(ns) {
            h.close(&ViewKind::Card(key.to_string()));
        }
        self.dirty = true;
    }

    /// The views changed since last asked: they are drawn again.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use libflankr_proto::companion::PluginMenuItem;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

    pub(crate) fn info(ns: &str, version: &str, enabled: bool, screens: &str) -> PluginInfo {
        PluginInfo {
            ns: ns.into(),
            name: "Test".into(),
            description: "A test plugin.".into(),
            version: version.into(),
            enabled,
            menu: vec![PluginMenuItem {
                id: "open".into(),
                label: "Open it".into(),
                opens: "modal".into(),
                screens: screens.into(),
            }],
            reason: None,
        }
    }

    /// A device half that records what it is given.
    struct Recorder {
        ns: &'static str,
        seen: Rc<RefCell<Vec<String>>>,
    }

    impl DevicePlugin for Recorder {
        fn ns(&self) -> &'static str {
            self.ns
        }
        fn supports(&self) -> &'static str {
            "0.1"
        }
        fn action(&mut self, item: &str, out: &mut PluginOut) {
            self.seen.borrow_mut().push(format!("action {item}"));
            out.open(ViewKind::Modal);
            out.call("things.list", json!({}));
        }
        fn message(&mut self, msg: PluginMessage, out: &mut PluginOut) {
            self.seen.borrow_mut().push(format!("{} {msg:?}", self.ns));
            if let PluginMessage::Result { id: 1, .. } = msg {
                out.open(ViewKind::Card(format!("k{}", self.seen.borrow().len())));
            }
        }
        fn height(&self, _: &ViewKind, _: f32, _: f32) -> f32 {
            10.0
        }
        fn draw(&self, _: &ViewKind, _: Rect, _: f32, _: &DrawContext, _: &mut DrawList) {}
        fn tap(&mut self, view: &ViewKind, x: f32, y: f32, out: &mut PluginOut) {
            self.seen.borrow_mut().push(format!("tap {view:?} {x} {y}"));
            out.close();
        }
        fn close(&mut self, view: &ViewKind) {
            self.seen.borrow_mut().push(format!("close {view:?}"));
        }
    }

    const A: &str = "dev.flankr.plugins.a";
    const B: &str = "dev.flankr.plugins.b";

    type Seen = Rc<RefCell<Vec<String>>>;

    fn two() -> (Plugins, Seen, Seen) {
        let (a, b) = (Rc::default(), Rc::default());
        let mut p = Plugins::new(vec![
            Box::new(Recorder {
                ns: A,
                seen: Rc::clone(&a),
            }),
            Box::new(Recorder {
                ns: B,
                seen: Rc::clone(&b),
            }),
        ]);
        p.set_infos(vec![
            info(A, "0.1.0", true, "both"),
            info(B, "0.1.2", true, "agent"),
        ]);
        (p, a, b)
    }

    #[test]
    fn supports_is_cargos_caret_and_a_hash_only_exactly() {
        assert!(supports("0.1", "0.1.0"));
        assert!(supports("0.1", "0.1.9"));
        assert!(!supports("0.1", "0.2.0"));
        assert!(!supports("0.1", "g1a2b3c4"));
        assert!(supports("@g1a2b3c4", "g1a2b3c4"));
        assert!(!supports("@g1a2b3c4", "g1a2b3c5"));
        assert!(!supports("@g1a2b3c4", "0.1.0"));
        assert!(supports("1", "1.4.2"));
        assert!(!supports("1", "2.0.0"));
    }

    #[test]
    fn availability_says_which_side_is_older() {
        let (p, ..) = two();
        assert_eq!(p.availability(A), Availability::Usable);
        let half = p.half(A);
        assert_eq!(
            availability(&info(A, "0.2.0", true, "both"), half),
            Availability::NeedsNewerApp
        );
        assert_eq!(
            availability(&info(A, "0.0.3", true, "both"), half),
            Availability::NeedsNewerFlankr
        );
        assert_eq!(
            availability(&info(A, "g1a2b3c4", true, "both"), half),
            Availability::NeedsNewerApp
        );
        // No device half: the app is the older one.
        assert_eq!(
            availability(&info("dev.flankr.plugins.c", "0.1.0", true, "both"), None),
            Availability::NeedsNewerApp
        );
    }

    #[test]
    fn a_message_reaches_only_its_namespace() {
        let (mut p, a, b) = two();
        p.result(PluginResult {
            ns: B.into(),
            id: 9,
            result: Some(json!({"x": 1})),
            error: None,
        });
        p.notify(PluginNotify {
            ns: B.into(),
            method: "progress".into(),
            params: json!(null),
        });
        assert!(a.borrow().is_empty(), "{:?}", a.borrow());
        assert_eq!(b.borrow().len(), 2);
        // One of no half here, or not listed: no one.
        p.notify(PluginNotify {
            ns: "dev.flankr.plugins.c".into(),
            method: "x".into(),
            params: json!(null),
        });
        assert!(a.borrow().is_empty());
        assert_eq!(b.borrow().len(), 2);
    }

    #[test]
    fn an_action_opens_the_modal_and_its_calls_carry_the_halfs_ns_and_fresh_ids() {
        let (mut p, a, _) = two();
        let calls = p.action(A, "open");
        assert_eq!(a.borrow()[0], "action open");
        assert_eq!(p.modal(), Some((A, "Test")));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, A);
        assert_eq!(calls[0].1.id, Some(1));
        let again = p.action(B, "open");
        assert_eq!(again[0].1.id, Some(2));
        // B's modal replaced A's, which was told.
        assert_eq!(p.modal().unwrap().0, B);
        assert!(a.borrow().contains(&"close Modal".to_string()));
        // The half's close closes it.
        p.tap_modal(3.0, 4.0);
        assert!(p.modal().is_none());
    }

    #[test]
    fn the_menu_holds_active_plugins_items_for_the_screen() {
        let (mut p, ..) = two();
        let ids = |p: &Plugins, s| -> Vec<String> { p.menu(s).into_iter().map(|m| m.ns).collect() };
        assert_eq!(ids(&p, Screen::Overview), [A]);
        assert_eq!(ids(&p, Screen::Agent), [A, B]);
        assert!(ids(&p, Screen::Settings).is_empty());
        // Disabled, or out of range: no items.
        p.set_infos(vec![
            info(A, "0.1.0", false, "both"),
            info(B, "0.2.0", true, "agent"),
        ]);
        assert!(ids(&p, Screen::Agent).is_empty());
        // And no calls.
        assert!(p.action(A, "open").is_empty());
    }

    #[test]
    fn turning_a_plugin_off_closes_its_modal_and_removes_its_cards() {
        let (mut p, a, _) = two();
        p.action(A, "open");
        // Result 1 pins a card.
        p.result(PluginResult {
            ns: A.into(),
            id: 1,
            result: Some(json!({})),
            error: None,
        });
        assert_eq!(p.cards().len(), 1);
        p.set_infos(vec![
            info(A, "0.1.0", false, "both"),
            info(B, "0.1.2", true, "agent"),
        ]);
        assert!(p.modal().is_none());
        assert!(p.cards().is_empty());
        let seen = a.borrow();
        assert!(seen.iter().any(|s| s == "close Modal"));
        assert!(seen.iter().any(|s| s.starts_with("close Card")));
    }

    #[test]
    fn at_most_four_cards_the_oldest_going() {
        let (mut p, ..) = two();
        for _ in 0..6 {
            p.result(PluginResult {
                ns: A.into(),
                id: 1,
                result: None,
                error: Some(ErrorObject::new(-32100, "x")),
            });
        }
        assert_eq!(p.cards().len(), MAX_CARDS);
        p.unpin(0);
        assert_eq!(p.cards().len(), MAX_CARDS - 1);
    }
}
