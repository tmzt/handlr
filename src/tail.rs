//! `flankr session tail`: follow one pane's session over the daemon socket
//! and print each delivery as it comes (plan 20261023_03, section 8, and
//! A8.8). A read-only client: it says `hello` as `flankr.companion` with
//! the client name `tail`, subscribes, and sends nothing to herdr or to
//! any pane.
//!
//! ```text
//! flankr session tail [--pane <id or name>] [--follow] [--entries N] [--history N]
//!                     [--outline] [--from N] [--to N]
//! ```
//!
//! Without `--pane` it starts on herdr's focus. `--follow` moves with
//! `focus.changed` as the companion's Follow does, by unsubscribing and
//! subscribing again. `--entries N` is the backfill's size in sections
//! (1 to 200, 50 by default). `--history N` then pages back N sections
//! with `session.history`, printing their serials. Each entry is one line:
//! its section's serial, role, kind and the first line of its summary;
//! cards, resets, and status and question changes have lines of their own.
//!
//! `--outline` subscribes in outline mode (plan 20261024_01): each slot is
//! printed as `#<index> <serial> <kind> <lines>x<chars> rev <rev>`, and the
//! contents that ride along as entries are, each line after its index.
//! `--from N` and `--to N` (either or both; they imply `--outline`) then
//! fetch that inclusive index range with `session.content` and print its
//! sections, paging on while a range with both bounds has more.

use std::io::Write;
use std::path::Path;

use futures_util::StreamExt;
use libflankr_proto::common::{
    self, AgentsListResult, PendingQuestion, SECTIONS_MAX, Section, SectionKind,
    SessionContentParams, SessionContentResult, SessionDelivery, SessionHistoryParams,
    SessionHistoryResult, SessionSubscribeParams, SessionSubscribeResult, SessionUnsubscribeParams,
    Slot, methods, protocols,
};
use libflankr_proto::{Id, Incoming, Request, companion};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::server::conn::MAX_LINE;

/// What `flankr session tail` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailOptions {
    /// A pane id or a name; herdr's focus when absent.
    pub pane: Option<String>,
    /// Move with herdr's focus.
    pub follow: bool,
    /// The backfill's size, in sections.
    pub sections: usize,
    /// Sections to page back after the backfill.
    pub history: usize,
    /// Subscribe in outline mode.
    pub outline: bool,
    /// An index range to fetch with `session.content` once subscribed
    /// (in outline mode); `None` for no fetch.
    pub content: Option<(Option<u64>, Option<u64>)>,
}

impl Default for TailOptions {
    fn default() -> Self {
        TailOptions {
            pane: None,
            follow: false,
            sections: common::SUBSCRIBE_SECTIONS_DEFAULT,
            history: 0,
            outline: false,
            content: None,
        }
    }
}

/// The first line of `text`, trimmed, or `""`.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}

/// A section's lines: one per entry, or the card's.
pub fn section_lines(section: &Section) -> Vec<String> {
    let serial = section.serial;
    if let Some(card) = &section.card {
        let state = if card.live { "live" } else { "resolved" };
        return vec![format!(
            "{serial} card {} {state}: {}: {}",
            word(&card.card),
            card.title,
            first_line(&card.text)
        )];
    }
    let mut lines: Vec<String> = section
        .entries
        .iter()
        .map(|e| {
            let mut role = word(&e.role);
            if let Some(origin) = &e.origin {
                role = format!("{role}({origin})");
            }
            let tool = e
                .tool
                .as_deref()
                .map(|t| format!(" {t}"))
                .unwrap_or_default();
            format!(
                "{serial} {role} {}{tool}: {}",
                word(&e.kind),
                first_line(&e.summary)
            )
        })
        .collect();
    if lines.is_empty() && section.kind != SectionKind::Card {
        lines.push(format!("{serial} {} (empty)", word(&section.kind)));
    }
    lines
}

/// A slot's line: `#<index> <serial> <kind> <lines>x<chars> rev <rev>`.
pub fn slot_line(slot: &Slot) -> String {
    format!(
        "#{} {} {} {}x{} rev {}",
        slot.index,
        slot.serial,
        word(&slot.kind),
        slot.lines,
        slot.chars,
        slot.rev
    )
}

/// A section's lines, each after its index (outline mode).
pub fn indexed_lines(section: &Section) -> Vec<String> {
    section_lines(section)
        .into_iter()
        .map(|l| format!("#{} {l}", section.index))
        .collect()
}

/// What a tail has printed so far, to tell what changed.
#[derive(Debug, Default)]
pub struct Printed {
    session: Option<Option<String>>,
    deliveries: usize,
}

/// A delivery's lines: a reset line when it is one, the session's reason
/// when there is none, status and question changes, then its sections.
pub fn delivery_lines(d: &SessionDelivery, printed: &mut Printed) -> Vec<String> {
    let mut out = Vec::new();
    let session = d.session.as_ref().map(|s| s.session_id.clone());
    if d.reset {
        let why = if let Some(reason) = &d.reason {
            reason.clone()
        } else if printed.deliveries == 0 {
            "backfill".to_string()
        } else if session.is_some() && printed.session.as_ref() != Some(&session.clone().flatten())
        {
            match session.clone().flatten() {
                Some(id) => format!("new session {id}"),
                None => "new session".to_string(),
            }
        } else {
            "caught up".to_string()
        };
        out.push(format!("-- reset ({why}) --"));
    } else if let Some(reason) = &d.reason {
        out.push(format!("-- {reason} --"));
    }
    if let Some(s) = &session {
        printed.session = Some(s.clone());
    }
    printed.deliveries += 1;
    if let Some(status) = &d.status {
        out.push(format!("-- status: {status} --"));
    }
    match &d.pending_question {
        Some(Some(PendingQuestion { text, .. })) => {
            out.push(format!("-- question: {} --", first_line(text)));
        }
        Some(None) if printed.deliveries > 1 => out.push("-- question cleared --".to_string()),
        _ => {}
    }
    for s in &d.removed {
        out.push(format!("{s} removed"));
    }
    if let Some(last) = d.last {
        out.push(format!(
            "-- outline: {} slot(s), last #{last} --",
            d.slots.len()
        ));
    }
    for i in &d.removed_index {
        out.push(format!("#{i} removed"));
    }
    out.extend(d.slots.iter().map(slot_line));
    for s in &d.sections {
        if d.last.is_some() {
            out.extend(indexed_lines(s));
        } else {
            out.extend(section_lines(s));
        }
    }
    out
}

/// One connection to the daemon, as the tail uses it.
struct Wire {
    write: OwnedWriteHalf,
    lines: FramedRead<OwnedReadHalf, LinesCodec>,
    next_id: i64,
    /// Notifications that came while waiting for a response.
    notes: std::collections::VecDeque<libflankr_proto::Notification>,
}

impl Wire {
    async fn send(&mut self, method: &str, params: impl serde::Serialize) -> Result<Id, String> {
        self.next_id += 1;
        let id = Id::Number(self.next_id);
        let mut line = serde_json::to_string(&Request::new(id.clone(), method, params))
            .map_err(|e| e.to_string())?;
        line.push('\n');
        self.write
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
        Ok(id)
    }

    async fn next(&mut self) -> Result<Option<Incoming>, String> {
        loop {
            let Some(line) = self.lines.next().await else {
                return Ok(None);
            };
            let line = line.map_err(|e| format!("read: {e}"))?;
            if let Ok(incoming) = serde_json::from_str::<Incoming>(&line) {
                return Ok(Some(incoming));
            }
        }
    }

    /// Call `method` and give its result, keeping notifications.
    async fn call(&mut self, method: &str, params: impl serde::Serialize) -> Result<Value, String> {
        let id = self.send(method, params).await?;
        loop {
            match self.next().await? {
                None => return Err(format!("{method}: the daemon closed the connection")),
                Some(Incoming::Response(r)) if r.id.as_ref() == Some(&id) => {
                    if let Some(e) = r.error {
                        return Err(format!("{method}: {} ({})", e.message, e.code));
                    }
                    return Ok(r.result.unwrap_or(Value::Null));
                }
                Some(Incoming::Notification(n)) => self.notes.push_back(n),
                Some(_) => {}
            }
        }
    }

    async fn note(&mut self) -> Result<Option<libflankr_proto::Notification>, String> {
        if let Some(n) = self.notes.pop_front() {
            return Ok(Some(n));
        }
        loop {
            match self.next().await? {
                None => return Ok(None),
                Some(Incoming::Notification(n)) => return Ok(Some(n)),
                Some(_) => {}
            }
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: Value, what: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("a bad {what}: {e}"))
}

fn print(out: &mut impl Write, lines: &[String]) -> Result<(), String> {
    for line in lines {
        writeln!(out, "{line}").map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

/// Follow a session over the daemon socket at `socket`, printing to
/// `out`, until the connection or the pane closes. Returns the process exit code.
pub async fn run(
    socket: &Path,
    options: &TailOptions,
    out: &mut impl Write,
) -> Result<i32, String> {
    if !(1..=SECTIONS_MAX).contains(&options.sections) {
        return Err(format!("--entries must be 1 to {SECTIONS_MAX}"));
    }
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        format!(
            "connect {}: {e} (is `flankr serve` running?)",
            socket.display()
        )
    })?;
    let (read, write) = stream.into_split();
    let mut wire = Wire {
        write,
        lines: FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE)),
        next_id: 0,
        notes: Default::default(),
    };
    wire.call(
        methods::HELLO,
        companion::HelloParams {
            protocol: protocols::COMPANION.to_string(),
            version: companion::VERSION,
            client: "tail".to_string(),
            session_id: None,
        },
    )
    .await?;
    let mut pane = match &options.pane {
        Some(p) => p.clone(),
        None => {
            let list: AgentsListResult = parse(
                wire.call(methods::AGENTS_LIST, common::Empty {}).await?,
                "list",
            )?;
            let focus = list.focus.ok_or("herdr has no focused pane; give --pane")?;
            focus.pane_id
        }
    };
    let outline = options.outline || options.content.is_some();
    let mut printed = Printed::default();
    let (mut sub, oldest) = subscribe(
        &mut wire,
        &pane,
        options.sections,
        outline,
        &mut printed,
        out,
    )
    .await?;
    if options.history > 0
        && let Some(oldest) = oldest
    {
        page_back(&mut wire, sub, oldest, options.history, out).await?;
    }
    if let Some((from, to)) = options.content {
        fetch_content(&mut wire, sub, from, to, out).await?;
    }
    while let Some(note) = wire.note().await? {
        match note.method.as_str() {
            methods::SESSION_DELIVERY => {
                let d: SessionDelivery = parse(note.params, "delivery")?;
                if d.subscription_id != sub {
                    continue; // one still in flight from before a move
                }
                print(out, &delivery_lines(&d, &mut printed))?;
                if d.reason.as_deref() == Some(common::delivery_reasons::PANE_CLOSED) {
                    return Ok(0);
                }
            }
            companion::methods::FOCUS_CHANGED if options.follow => {
                let changed: companion::FocusChanged = parse(note.params, "focus.changed")?;
                let Some(focus) = changed.focus.filter(|f| f.agent.is_some()) else {
                    continue;
                };
                if focus.pane_id == pane {
                    continue;
                }
                wire.call(
                    methods::SESSION_UNSUBSCRIBE,
                    SessionUnsubscribeParams {
                        subscription_id: sub,
                    },
                )
                .await
                .ok();
                print(out, &[format!("-- following {} --", focus.name)])?;
                pane = focus.pane_id;
                printed = Printed::default();
                sub = subscribe(
                    &mut wire,
                    &pane,
                    options.sections,
                    outline,
                    &mut printed,
                    out,
                )
                .await?
                .0;
            }
            _ => {}
        }
    }
    Ok(0)
}

/// Subscribe to `pane` and print the first delivery; the subscription and
/// the oldest serial it gave.
async fn subscribe(
    wire: &mut Wire,
    pane: &str,
    sections: usize,
    outline: bool,
    printed: &mut Printed,
    out: &mut impl Write,
) -> Result<(u64, Option<common::Serial>), String> {
    let result: SessionSubscribeResult = parse(
        wire.call(
            methods::SESSION_SUBSCRIBE,
            SessionSubscribeParams {
                pane_id: pane.to_string(),
                sections,
                after: None,
                outline,
                from: None,
                epoch: None,
            },
        )
        .await?,
        "subscribe result",
    )?;
    print(out, &[format!("-- subscribed to {pane} --")])?;
    print(out, &delivery_lines(&result.delivery, printed))?;
    let oldest = result.delivery.sections.first().map(|s| s.serial);
    Ok((result.subscription_id, oldest))
}

/// Fetch the index range `from` to `to` with `session.content` and print
/// its sections; a range with both bounds is paged through while it has
/// more.
async fn fetch_content(
    wire: &mut Wire,
    sub: u64,
    mut from: Option<u64>,
    to: Option<u64>,
    out: &mut impl Write,
) -> Result<(), String> {
    loop {
        let page: SessionContentResult = parse(
            wire.call(
                methods::SESSION_CONTENT,
                SessionContentParams {
                    subscription_id: sub,
                    from,
                    to,
                    limit: SECTIONS_MAX,
                    slots_only: false,
                },
            )
            .await?,
            "content",
        )?;
        let bound = |b: Option<u64>| b.map_or("…".to_string(), |b| format!("#{b}"));
        print(
            out,
            &[format!(
                "-- content {} to {}: {} section(s){}{} --",
                bound(from),
                bound(to),
                page.sections.len(),
                if page.removed_index.is_empty() {
                    String::new()
                } else {
                    format!(", {} removed", page.removed_index.len())
                },
                if page.more { ", more" } else { "" }
            )],
        )?;
        for i in &page.removed_index {
            print(out, &[format!("#{i} removed")])?;
        }
        let lines: Vec<String> = page.sections.iter().flat_map(indexed_lines).collect();
        print(out, &lines)?;
        match (page.more, from, to, page.sections.last()) {
            (true, Some(_), Some(_), Some(last)) => from = Some(last.index + 1),
            _ => return Ok(()),
        }
    }
}

/// Page back `n` sections below `before`, printing each page, newest page
/// first, each oldest first.
async fn page_back(
    wire: &mut Wire,
    sub: u64,
    mut before: common::Serial,
    mut n: usize,
    out: &mut impl Write,
) -> Result<(), String> {
    while n > 0 {
        let want = n.min(SECTIONS_MAX);
        let page: SessionHistoryResult = parse(
            wire.call(
                methods::SESSION_HISTORY,
                SessionHistoryParams {
                    subscription_id: sub,
                    before,
                    sections: want,
                },
            )
            .await?,
            "history page",
        )?;
        print(
            out,
            &[format!(
                "-- history: {} section(s) before {before}{} --",
                page.sections.len(),
                if page.start { ", from the start" } else { "" }
            )],
        )?;
        let lines: Vec<String> = page.sections.iter().flat_map(section_lines).collect();
        print(out, &lines)?;
        n = n.saturating_sub(page.sections.len());
        match page.sections.first() {
            Some(first) if !page.start => before = first.serial,
            _ => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::common::{
        CardBody, CardKind, Entry, EntryKind, Role, Serial, SessionMatched, SessionRef,
    };

    fn entry(role: Role, kind: EntryKind, summary: &str) -> Entry {
        Entry {
            ts: "t".into(),
            role,
            kind,
            tool: (kind != EntryKind::Text).then(|| "Bash".to_string()),
            summary: summary.into(),
            links: vec![],
            origin: None,
            text: None,
        }
    }

    fn delivery(reset: bool, sections: Vec<Section>) -> SessionDelivery {
        SessionDelivery {
            subscription_id: 1,
            pane_id: "w2:pZ".into(),
            agent: "claude".into(),
            reset,
            session: Some(SessionRef {
                session_id: Some("s1".into()),
                matched: SessionMatched::Id,
            }),
            reason: None,
            sections,
            status: None,
            pending_question: None,
            removed: vec![],
            slots: vec![],
            removed_index: vec![],
            first: 0,
            last: None,
            epoch: None,
        }
    }

    #[test]
    fn entries_cards_and_changes_print_a_line_each() {
        let tool = Section {
            serial: Serial::new(0x3fa9_c21e, 9, 0),
            kind: SectionKind::Tool,
            entries: vec![
                entry(
                    Role::Assistant,
                    EntryKind::ToolUse,
                    "Bash {\"command\":\"ls\"}",
                ),
                entry(Role::Tool, EntryKind::ToolResult, "a\nb"),
            ],
            card: None,
            index: 0,
        };
        let card = Section {
            serial: Serial::new(0x3fa9_c21e, 9, 130),
            kind: SectionKind::Card,
            entries: vec![],
            card: Some(CardBody {
                card: CardKind::Broke,
                live: true,
                title: "Something broke".into(),
                text: "Bash: exit 1".into(),
                confirm_id: None,
                about_plan: false,
                links: vec![],
                points: vec![],
                spoken: None,
            }),
            index: 0,
        };
        let mut printed = Printed::default();
        let mut first = delivery(true, vec![tool.clone(), card]);
        first.status = Some("working".into());
        first.pending_question = Some(None);
        assert_eq!(
            delivery_lines(&first, &mut printed),
            [
                "-- reset (backfill) --",
                "-- status: working --",
                "3fa9c21e:9.0 assistant tool_use Bash: Bash {\"command\":\"ls\"}",
                "3fa9c21e:9.0 tool tool_result Bash: a",
                "3fa9c21e:9.130 card broke live: Something broke: Bash: exit 1",
            ]
        );
        // A question, then a new session's reset, then a removed card.
        let mut asked = delivery(false, vec![]);
        asked.pending_question = Some(Some(PendingQuestion {
            text: "Shall I go on?".into(),
            ts: "t".into(),
            about_plan: false,
            plan: None,
            links: vec![],
        }));
        assert_eq!(
            delivery_lines(&asked, &mut printed),
            ["-- question: Shall I go on? --"]
        );
        let mut moved = delivery(true, vec![]);
        moved.session = Some(SessionRef {
            session_id: Some("s2".into()),
            matched: SessionMatched::Id,
        });
        assert_eq!(
            delivery_lines(&moved, &mut printed),
            ["-- reset (new session s2) --"]
        );
        let mut gone = delivery(false, vec![]);
        gone.removed = vec![Serial::new(0x3fa9_c21e, 9, 194)];
        gone.pending_question = Some(None);
        assert_eq!(
            delivery_lines(&gone, &mut printed),
            ["-- question cleared --", "3fa9c21e:9.194 removed"]
        );
        let mut closed = delivery(true, vec![]);
        closed.session = None;
        closed.reason = Some("pane_closed".into());
        assert_eq!(
            delivery_lines(&closed, &mut printed),
            ["-- reset (pane_closed) --"]
        );
    }

    #[test]
    fn outline_deliveries_print_their_slots_and_indexed_contents() {
        let serial = Serial::new(0x3fa9_c21e, 12, 0);
        let mut d = delivery(true, vec![]);
        d.slots = vec![Slot {
            index: 4,
            serial,
            kind: SectionKind::Text,
            lines: 3,
            chars: 90,
            rev: 1,
            bookend: false,
        }];
        d.removed_index = vec![2];
        d.last = Some(4);
        d.sections = vec![Section {
            serial,
            kind: SectionKind::Text,
            entries: vec![entry(Role::Assistant, EntryKind::Text, "Done.")],
            card: None,
            index: 4,
        }];
        assert_eq!(
            delivery_lines(&d, &mut Printed::default()),
            [
                "-- reset (backfill) --",
                "-- outline: 1 slot(s), last #4 --",
                "#2 removed",
                "#4 3fa9c21e:12.0 text 3x90 rev 1",
                "#4 3fa9c21e:12.0 assistant text: Done.",
            ]
        );
    }
}
