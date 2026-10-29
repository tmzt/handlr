//! The critical rules' planning format, read as tpt's query plan reads it
//! (~/src/tpt/docs/design/20261027_02_TPT_QUERY_PLAN.md, decisions 5, 6
//! and 12; docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.8, item 5).
//!
//! - the file name `^(\d{8})_(\d{2})_([A-Z0-9]+(?:_[A-Z0-9]+)*)\.md$`;
//!   a name ending `_PLAN` is a plan, any other a spec;
//! - the front matter between a first line `---` and the next `---`, in a
//!   fixed YAML subset;
//! - level-2 headings and `## Amendment — <timestamp>: <subject>` headings
//!   outside fenced code blocks, timestamps kept as written;
//! - `README.md`'s `## Architectural decisions` table.
//!
//! Only titles, headings and subjects are returned, never the bodies.

use serde_json::{Map, Value, json};

/// A plan or a spec, by its file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plan,
    Spec,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Plan => "plan",
            Kind::Spec => "spec",
        }
    }
}

/// What a file name says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileName {
    /// The name without `.md`: `20261028_04_PLUGINS_PLAN`.
    pub base: String,
    /// `2026-10-28`.
    pub date: String,
    pub nn: u32,
    /// `PLUGINS_PLAN`.
    pub name: String,
    pub kind: Kind,
}

/// `file` (a name, not a path) as a plan's file name, or `None`.
pub fn file_name(file: &str) -> Option<FileName> {
    let base = file.strip_suffix(".md")?;
    if !crate::plugins::command::is_plan_name(base) {
        return None;
    }
    let date = format!("{}-{}-{}", &base[0..4], &base[4..6], &base[6..8]);
    let nn = base[9..11].parse().ok()?;
    let name = base[12..].to_string();
    let kind = if name.ends_with("_PLAN") {
        Kind::Plan
    } else {
        Kind::Spec
    };
    Some(FileName {
        base: base.to_string(),
        date,
        nn,
        name,
        kind,
    })
}

/// The front matter's known keys, and the others in `extra`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrontMatter {
    pub title: Option<String>,
    pub date: Option<String>,
    pub status: Option<String>,
    pub covers: Vec<String>,
    pub sources: Vec<String>,
    pub related: Vec<String>,
    pub living_sections: Vec<String>,
    pub extra: Map<String, Value>,
}

/// One front-matter value.
#[derive(Debug, Clone, PartialEq)]
enum Yaml {
    Scalar(String),
    List(Vec<String>),
}

/// The front matter of `lines` (the file's lines): `None` when the first
/// line is not `---`; else the parsed keys and the line after the closing
/// `---` (0-based), or an error naming its line (1-based).
pub fn front_matter(lines: &[&str]) -> Option<Result<(FrontMatter, usize), String>> {
    if lines.first().copied() != Some("---") {
        return None;
    }
    let Some(end) = lines
        .iter()
        .skip(1)
        .position(|l| *l == "---")
        .map(|i| i + 1)
    else {
        return Some(Err("the front matter has no closing ---".to_string()));
    };
    Some(parse_yaml(&lines[1..end], 2).map(|pairs| (known(pairs), end + 1)))
}

/// `key: value` pairs in order; `first` is the line number of `lines[0]`.
fn parse_yaml(lines: &[&str], first: usize) -> Result<Vec<(String, Yaml)>, String> {
    let mut pairs: Vec<(String, Yaml)> = Vec::new();
    let mut i = 0;
    let at = |i: usize| first + i;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            i += 1;
            continue;
        }
        let Some((key, rest)) = key_line(line) else {
            return Err(format!(
                "front matter line {}: not `key: value` or a list item",
                at(i)
            ));
        };
        let rest = rest.trim();
        i += 1;
        if rest == "[]" {
            pairs.push((key, Yaml::List(Vec::new())));
        } else if rest.is_empty() {
            // A list, or nothing.
            let mut items: Vec<String> = Vec::new();
            while i < lines.len() {
                let line = lines[i];
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    i += 1;
                    continue;
                }
                let indent = line.len() - line.trim_start().len();
                let item = line.trim_start();
                if indent == 0 || !(item.starts_with("- ") || item == "-") {
                    break;
                }
                let start = i;
                let mut text = item[1..].trim().to_string();
                i += 1;
                while i < lines.len() {
                    let next = lines[i];
                    let next_indent = next.len() - next.trim_start().len();
                    if next.trim().is_empty() || next_indent <= indent {
                        break;
                    }
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(next.trim());
                    i += 1;
                }
                items.push(
                    unquote(&text).map_err(|e| format!("front matter line {}: {e}", at(start)))?,
                );
            }
            pairs.push((key, Yaml::List(items)));
        } else if rest.starts_with('"') {
            let start = i - 1;
            let mut text = rest.to_string();
            while !closed(&text) {
                let Some(next) = lines.get(i) else {
                    return Err(format!(
                        "front matter line {}: the quoted value is not closed",
                        at(start)
                    ));
                };
                text.push(' ');
                text.push_str(next.trim());
                i += 1;
            }
            pairs.push((
                key,
                Yaml::Scalar(
                    unquote(&text).map_err(|e| format!("front matter line {}: {e}", at(start)))?,
                ),
            ));
        } else {
            // A plain scalar, continued on lines indented more than 0.
            let mut text = rest.to_string();
            while let Some(next) = lines.get(i) {
                let indent = next.len() - next.trim_start().len();
                if next.trim().is_empty() || indent == 0 || next.trim_start().starts_with("- ") {
                    break;
                }
                text.push(' ');
                text.push_str(next.trim());
                i += 1;
            }
            pairs.push((key, Yaml::Scalar(text)));
        }
    }
    Ok(pairs)
}

/// `key` and the rest of a `key: value` line at column 0, `key` matching
/// `[a-z_]+`.
fn key_line(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return None;
    }
    let rest = &line[colon + 1..];
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    Some((key.to_string(), rest))
}

/// Whether a value starting with `"` has its closing quote.
fn closed(text: &str) -> bool {
    let mut chars = text.chars().skip(1);
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' => return true,
            _ => {}
        }
    }
    false
}

/// A double-quoted scalar's text, with `\"` and `\\` read; anything else
/// as written.
fn unquote(text: &str) -> Result<String, String> {
    let Some(inner) = text.strip_prefix('"') else {
        return Ok(text.to_string());
    };
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => return Err("the quoted value is not closed".to_string()),
            },
            '"' => {
                let after = chars.as_str().trim();
                if !after.is_empty() {
                    return Err(format!("text after the closing quote: {after:?}"));
                }
                return Ok(out);
            }
            c => out.push(c),
        }
    }
    Err("the quoted value is not closed".to_string())
}

fn known(pairs: Vec<(String, Yaml)>) -> FrontMatter {
    let mut fm = FrontMatter::default();
    let list = |v: Yaml| match v {
        Yaml::Scalar(s) => vec![s],
        Yaml::List(l) => l,
    };
    let scalar = |v: Yaml| match v {
        Yaml::Scalar(s) => s,
        Yaml::List(l) => l.join(" "),
    };
    for (key, value) in pairs {
        match key.as_str() {
            "title" => fm.title = Some(scalar(value)),
            "date" => fm.date = Some(scalar(value)),
            "status" => fm.status = Some(scalar(value)),
            "covers" => fm.covers = list(value),
            "sources" => fm.sources = list(value),
            "related" => fm.related = list(value),
            "living_sections" => fm.living_sections = list(value),
            _ => {
                let value = match value {
                    Yaml::Scalar(s) => Value::String(s),
                    Yaml::List(l) => json!(l),
                };
                fm.extra.insert(key, value);
            }
        }
    }
    fm
}

/// A level-2 heading of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub line: usize,
    pub text: String,
}

/// One `## Amendment — <timestamp>: <subject>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amendment {
    /// 1 for the first.
    pub index: usize,
    pub line: usize,
    /// As written: `2026-10-28 19:47 EDT`.
    pub timestamp: String,
    pub subject: String,
    /// `<X>` of a subject ending `(living section <X>)`.
    pub living_section: Option<String>,
}

/// `line` as an amendment heading: its timestamp and subject.
pub fn amendment_heading(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("## Amendment \u{2014} ")?;
    let b = rest.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    let shape = digits(0..4)
        && b.get(4) == Some(&b'-')
        && digits(5..7)
        && b.get(7) == Some(&b'-')
        && digits(8..10)
        && b.get(10) == Some(&b' ')
        && digits(11..13)
        && b.get(13) == Some(&b':')
        && digits(14..16)
        && b.get(16) == Some(&b' ');
    if !shape {
        return None;
    }
    let after = &rest[17..];
    let colon = after.find(": ")?;
    let zone = &after[..colon];
    let subject = &after[colon + 2..];
    if !zone_ok(zone) || subject.is_empty() {
        return None;
    }
    Some((rest[..17 + colon].to_string(), subject.to_string()))
}

/// `[A-Z][A-Za-z0-9+-]{1,5}`, or an offset `[+-]\d{2}(\d{2})?`.
fn zone_ok(zone: &str) -> bool {
    let b = zone.as_bytes();
    let name = matches!(b.first(), Some(b'A'..=b'Z'))
        && (2..=6).contains(&b.len())
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-'));
    let offset = matches!(b.first(), Some(b'+' | b'-'))
        && matches!(b.len(), 3 | 5)
        && b[1..].iter().all(u8::is_ascii_digit);
    name || offset
}

/// `(living section <X>)` at the end of `subject`.
fn living_section(subject: &str) -> Option<String> {
    let inner = subject.trim_end().strip_suffix(')')?;
    let at = inner.rfind("(living section ")?;
    let name = inner[at + "(living section ".len()..].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Whether `line` opens or closes a fenced code block.
fn fence(line: &str) -> bool {
    line.starts_with("```") || line.starts_with("~~~")
}

/// One file read.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Root-relative: `docs/design/20261028_04_PLUGINS_PLAN.md`.
    pub file: String,
    pub name: FileName,
    /// `None` with no front matter, or one that does not parse.
    pub front: Option<FrontMatter>,
    pub headings: Vec<Heading>,
    pub amendments: Vec<Amendment>,
    /// Its row in the README's table, if it has one.
    pub readme: Option<ReadmeRow>,
    pub error: Option<String>,
}

/// What a plan file reads as. `cut` says the text stops at the read cap.
pub fn read_plan(file: &str, name: FileName, text: &str, cut: bool) -> Plan {
    // A cut file's last line may be part of one; it is left out.
    let text = if cut {
        text.rfind('\n').map_or("", |at| &text[..at])
    } else {
        text
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut plan = Plan {
        file: file.to_string(),
        name,
        front: None,
        headings: Vec::new(),
        amendments: Vec::new(),
        readme: None,
        error: None,
    };
    let body_start = match front_matter(&lines) {
        None => {
            plan.error = Some("no front matter".to_string());
            0
        }
        Some(Err(e)) => {
            plan.error = Some(e);
            // The headings are still read after the closing `---`, if any.
            lines
                .iter()
                .skip(1)
                .position(|l| *l == "---")
                .map_or(lines.len(), |i| i + 2)
        }
        Some(Ok((front, after))) => {
            plan.front = Some(front);
            after
        }
    };
    let mut in_fence = false;
    for (i, line) in lines.iter().enumerate().skip(body_start) {
        if fence(line) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || !line.starts_with("## ") {
            continue;
        }
        match amendment_heading(line) {
            Some((timestamp, subject)) => plan.amendments.push(Amendment {
                index: plan.amendments.len() + 1,
                line: i + 1,
                living_section: living_section(&subject),
                timestamp,
                subject,
            }),
            None if plan.amendments.is_empty() => plan.headings.push(Heading {
                line: i + 1,
                text: line[3..].trim().to_string(),
            }),
            None => {}
        }
    }
    if cut && plan.error.is_none() {
        // Amendments may follow the cut unseen.
        plan.error = Some("cut at 64 KiB".to_string());
    }
    plan
}

/// One row of the README's `## Architectural decisions` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadmeRow {
    /// The row's line, 1-based.
    pub row: usize,
    /// The Document cell's link target, as written.
    pub target: String,
    pub status: String,
    pub decision: String,
}

/// The rows of `## Architectural decisions` in `readme`: after the heading
/// (outside fences), a header row with exactly `Document`, `Date`,
/// `Status`, `Decision`, a delimiter row, then rows up to the first line
/// that is not one. No table: no rows.
pub fn readme_rows(readme: &str) -> Vec<ReadmeRow> {
    let lines: Vec<&str> = readme.lines().collect();
    let mut in_fence = false;
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        if fence(line) {
            in_fence = !in_fence;
        } else if !in_fence && line.trim_end() == "## Architectural decisions" {
            start = Some(i + 1);
            break;
        }
    }
    let Some(mut i) = start else {
        return Vec::new();
    };
    while lines.get(i).is_some_and(|l| l.trim().is_empty()) {
        i += 1;
    }
    let header = lines.get(i).map(|l| cells(l)).unwrap_or_default();
    if header != ["Document", "Date", "Status", "Decision"] {
        return Vec::new();
    }
    let delimiter = lines.get(i + 1).map(|l| cells(l)).unwrap_or_default();
    if delimiter.len() != 4
        || !delimiter
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')))
    {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for (n, line) in lines.iter().enumerate().skip(i + 2) {
        if !line.trim_start().starts_with('|') {
            break;
        }
        let cells = cells(line);
        if cells.len() < 4 {
            continue;
        }
        let Some(target) = link_target(&cells[0]) else {
            continue;
        };
        rows.push(ReadmeRow {
            row: n + 1,
            target,
            status: cells[2].clone(),
            decision: cells[3].clone(),
        });
    }
    rows
}

/// A table row's cells: split on `|` not preceded by `\`, trimmed, with
/// the outer pipes left out and `\|` read as `|`.
fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let Some(line) = line.strip_prefix('|') else {
        return Vec::new();
    };
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cell).trim().to_string()),
            c => cell.push(c),
        }
    }
    if !cell.trim().is_empty() {
        cells.push(cell.trim().to_string());
    }
    cells
}

/// The target of the first Markdown link `[text](target)` in `cell`.
fn link_target(cell: &str) -> Option<String> {
    let open = cell.find("](")?;
    let rest = &cell[open + 2..];
    let close = rest.find(')')?;
    Some(rest[..close].trim().to_string())
}

/// A link target or reference as a root-relative path, `.` and `..`
/// segments folded (a path leaving the root keeps its leading `..`);
/// `None` for a URL or an absolute path.
pub fn root_relative(target: &str) -> Option<String> {
    if target.contains("://") || target.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// A status's base and its `(by <ref>)`, as tpt's decision 6 splits it.
pub fn split_status(status: &str) -> (String, Option<String>) {
    match status.find(" (by ") {
        Some(at) if status.ends_with(')') => (
            status[..at].trim().to_string(),
            Some(status[at + 5..status.len() - 1].trim().to_string()),
        ),
        _ => (status.trim().to_string(), None),
    }
}

/// The file a `(by <ref>)` names, root-relative (decision 6): a link's
/// target, a base or file name in `docs/design/`, or a path from the root.
pub fn resolve_ref(reference: &str) -> Option<String> {
    let reference = link_target(reference).unwrap_or_else(|| reference.trim().to_string());
    if reference.contains('/') {
        return root_relative(&reference);
    }
    let file = if reference.ends_with(".md") {
        reference
    } else {
        format!("{reference}.md")
    };
    Some(format!("docs/design/{file}"))
}

/// Whether two statuses agree as `tpt check`'s `README_STATUS` compares
/// them: the same base, and a `(by …)` naming the same file on each side.
pub fn same_status(a: &str, b: &str) -> bool {
    let (base_a, by_a) = split_status(a);
    let (base_b, by_b) = split_status(b);
    base_a == base_b
        && match (by_a, by_b) {
            (None, None) => true,
            (Some(x), Some(y)) => resolve_ref(&x) == resolve_ref(&y),
            _ => false,
        }
}

/// The plan as tpt's schema 1 has it (decision 12): the list item's
/// fields, and with `full` those of `plans show` too.
pub fn plan_json(plan: &Plan, full: bool) -> Value {
    let fm = plan.front.as_ref();
    let status = fm.and_then(|f| f.status.clone());
    let (status_base, status_by) = match &status {
        Some(s) => {
            let (base, by) = split_status(s);
            (Some(base), by.map(|r| json!({ "ref": r })))
        }
        None => (None, None),
    };
    let mut value = json!({
        "file": plan.file,
        "base": plan.name.base,
        "date_from_name": plan.name.date,
        "nn": plan.name.nn,
        "name": plan.name.name,
        "kind": plan.name.kind.as_str(),
        "title": fm.and_then(|f| f.title.clone()),
        "date": fm.and_then(|f| f.date.clone()),
        "status": status,
        "status_base": status_base,
        "status_by": status_by,
        "last_amendment": plan.amendments.last().map(|a| a.timestamp.clone()),
        "readme": plan.readme.as_ref().map(|r| json!({
            "row": r.row,
            "status": r.status,
            "decision": r.decision,
        })),
        "error": plan.error,
    });
    if full {
        let empty = FrontMatter::default();
        let fm = fm.unwrap_or(&empty);
        let mut living = fm.living_sections.clone();
        for a in &plan.amendments {
            if let Some(x) = &a.living_section
                && !living.contains(x)
            {
                living.push(x.clone());
            }
        }
        let extra = Value::Object(fm.extra.clone());
        let more = json!({
            "covers": fm.covers,
            "sources": fm.sources,
            "related": fm.related,
            "living_sections": living,
            "extra": extra,
            "headings": plan.headings.iter().map(|h| json!({"line": h.line, "text": h.text})).collect::<Vec<_>>(),
            "amendments": plan.amendments.iter().map(|a| json!({
                "index": a.index,
                "line": a.line,
                "timestamp": a.timestamp,
                "subject": a.subject,
                "living_section": a.living_section,
            })).collect::<Vec<_>>(),
        });
        if let (Value::Object(map), Value::Object(more)) = (&mut value, more) {
            map.extend(more);
        }
    }
    value
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
/// zone counts as UTC; a date alone is its midnight). The same reading as
/// the app's plan table (the plan's 23:14 EDT amendment, item 2).
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

/// A status as the plan table abbreviates it (the plan's 23:14 EDT
/// amendment, "the plan table's statuses abbreviated"): `Draft`, `Hold`,
/// `InImpl`, `Impl`, `Succd` and `Rtrd`; any other its first five letters.
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

/// `planning_plans_list`'s answer (plan 20261028_04, the 02:10 EDT
/// amendment of 2026-10-29, item 6), from `plans.list`'s: each plan's
/// base name, NN, date, title and abbreviated status, and when it was last
/// updated (its newest amendment, else its date, else its file name's),
/// newest first; a plan whose time cannot be read goes last, and ties go
/// by base name, the later first.
pub fn agent_list(listed: &Value) -> Value {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let mut rows: Vec<(Option<i64>, String, Value)> = listed
        .get("plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let base = text(p, "base")?;
            let date = text(p, "date").or_else(|| text(p, "date_from_name"));
            let updated = [
                text(p, "last_amendment"),
                date.clone(),
                text(p, "date_from_name"),
            ]
            .into_iter()
            .flatten()
            .find(|t| update_key(t).is_some());
            let key = updated.as_deref().and_then(update_key);
            let title = text(p, "title").filter(|t| !t.trim().is_empty());
            let mut row = json!({
                "name": base,
                "nn": p.get("nn").cloned().unwrap_or(Value::Null),
                "date": date,
                "title": title.unwrap_or_else(|| base.clone()),
                "status": text(p, "status").map(|s| status_abbrev(&s)),
                "updated": updated,
            });
            if let Some(error) = text(p, "error") {
                row["error"] = json!(error);
            }
            Some((key, base, row))
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    json!({
        "root": listed.get("root").cloned().unwrap_or(Value::Null),
        "plans": rows.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(text: &str) -> Plan {
        read_plan(
            "docs/design/20261028_04_X_PLAN.md",
            file_name("20261028_04_X_PLAN.md").unwrap(),
            text,
            false,
        )
    }

    #[test]
    fn file_names() {
        let f = file_name("20261028_04_PLUGINS_PLAN.md").unwrap();
        assert_eq!(f.base, "20261028_04_PLUGINS_PLAN");
        assert_eq!(f.date, "2026-10-28");
        assert_eq!(f.nn, 4);
        assert_eq!(f.name, "PLUGINS_PLAN");
        assert_eq!(f.kind, Kind::Plan);
        assert_eq!(
            file_name("20261028_02_LIBCONTROLS_SPEC.md").unwrap().kind,
            Kind::Spec
        );
        assert_eq!(
            file_name("20261028_02_PLANNER.md").unwrap().kind,
            Kind::Spec
        );
        for bad in [
            "README.md",
            "20261028_04_PLUGINS_PLAN",
            "20261028_04_plugins.md",
            "x_20261028_04_A.md",
        ] {
            assert_eq!(file_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_yaml_subset() {
        let text = r#"---
title: Built-in plugins: a title with a colon
date: 2026-10-28
status: Superseded (by [the next plan](docs/design/20261029_01_NEXT_PLAN.md))
# a comment
covers:
  - "A plugin model for flankr: plugins are built-in Rust, each in two
    halves under one \"package name\""
  - plain item
    continued here
sources: []
related: docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md
living_sections:
  - 3.9 Settings rows
owner: "tmeade"
tags:
  - a
---

# Title
"#;
        let p = plan(text);
        assert_eq!(p.error, None);
        let fm = p.front.unwrap();
        assert_eq!(
            fm.title.as_deref(),
            Some("Built-in plugins: a title with a colon")
        );
        assert_eq!(fm.date.as_deref(), Some("2026-10-28"));
        assert_eq!(
            fm.covers,
            [
                "A plugin model for flankr: plugins are built-in Rust, each in two halves under one \"package name\"",
                "plain item continued here"
            ]
        );
        assert!(fm.sources.is_empty());
        assert_eq!(
            fm.related,
            ["docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md"]
        );
        assert_eq!(fm.living_sections, ["3.9 Settings rows"]);
        assert_eq!(fm.extra["owner"], "tmeade");
        assert_eq!(fm.extra["tags"], json!(["a"]));
        let (base, by) = split_status(fm.status.as_deref().unwrap());
        assert_eq!(base, "Superseded");
        assert_eq!(
            by.as_deref(),
            Some("[the next plan](docs/design/20261029_01_NEXT_PLAN.md)")
        );
        // A multi-line quoted title.
        let p = plan("---\ntitle: \"one\n  two\"\ndate: 2026-10-28\nstatus: Draft\n---\n");
        assert_eq!(p.front.unwrap().title.as_deref(), Some("one two"));
    }

    #[test]
    fn front_matter_errors() {
        let p = plan("# No front matter\n\n## 1. Goal\n");
        assert_eq!(p.error.as_deref(), Some("no front matter"));
        assert_eq!(p.headings[0].text, "1. Goal");
        let p = plan("---\ntitle: x\nNot A Key\n---\n## 1. Goal\n");
        assert!(
            p.error.as_deref().unwrap().contains("line 3"),
            "{:?}",
            p.error
        );
        assert_eq!(
            p.headings.len(),
            1,
            "headings read after the bad front matter"
        );
        let p = plan("---\ntitle: \"never closed\n---\n");
        assert!(p.error.is_some());
        let p = plan("---\ntitle: x\n");
        assert!(p.error.as_deref().unwrap().contains("closing"));
    }

    #[test]
    fn amendments_headings_fences_and_living_sections() {
        let text = "---\ntitle: T\ndate: 2026-10-28\nstatus: In implementation\n---\n\n# T\n\n## 1. Goal\n\n```text\n## Amendment \u{2014} 2026-10-28 09:00 EDT: inside a fence\n## Not a heading\n```\n\n~~~\n## Amendment \u{2014} 2026-10-28 09:01 EDT: in a tilde fence\n~~~\n\n## 2. Decisions\n\n## Amendment \u{2014} 2026-10-28 18:48 EDT: the first\n\nbody\n\n## Amendment - 2026-10-28 18:49 EDT: a hyphen, not one\n\n## Amendment \u{2014} 2026-10-28 19:47 +0530: a row (living section 3.9 Settings)\n\n## Amendment \u{2014} 2026-10-28 19:48 edt: a lowercase zone\n";
        let p = plan(text);
        assert_eq!(
            p.headings,
            [
                Heading {
                    line: 9,
                    text: "1. Goal".into()
                },
                Heading {
                    line: 20,
                    text: "2. Decisions".into()
                },
            ]
        );
        assert_eq!(p.amendments.len(), 2, "{:?}", p.amendments);
        assert_eq!(p.amendments[0].timestamp, "2026-10-28 18:48 EDT");
        assert_eq!(p.amendments[0].subject, "the first");
        assert_eq!((p.amendments[0].index, p.amendments[0].line), (1, 22));
        assert_eq!(p.amendments[1].timestamp, "2026-10-28 19:47 +0530");
        assert_eq!(
            p.amendments[1].living_section.as_deref(),
            Some("3.9 Settings")
        );
        let v = plan_json(&p, true);
        assert_eq!(v["last_amendment"], "2026-10-28 19:47 +0530");
        assert_eq!(v["living_sections"], json!(["3.9 Settings"]));
    }

    #[test]
    fn amendment_heading_shapes() {
        let ok = amendment_heading("## Amendment \u{2014} 2026-10-28 19:47 EDT: approved; implementation on main-plugins: a colon").unwrap();
        assert_eq!(ok.0, "2026-10-28 19:47 EDT");
        assert_eq!(ok.1, "approved; implementation on main-plugins: a colon");
        for good in ["UTC", "GMT", "EDT", "UTC+1", "-04", "+0530"] {
            assert!(zone_ok(good), "{good}");
        }
        for bad in ["", "E", "edt", "EASTERN", "+4", "+053", "-04:00"] {
            assert!(!zone_ok(bad), "{bad}");
        }
        for bad in [
            "## Amendment \u{2014} 2026-10-28 19:47 EDT:",
            "## Amendment \u{2014} 2026-10-28 19:47 EDT: ",
            "## Amendment \u{2014} 2026-10-28 7:47 EDT: x",
            "## Amendment \u{2014} 2026-10-28T19:47 EDT: x",
            "## Amendment \u{2014} 2026-10-28 19:47: x",
            "### Amendment \u{2014} 2026-10-28 19:47 EDT: x",
        ] {
            assert_eq!(amendment_heading(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_cut_file() {
        let mut text =
            "---\ntitle: T\ndate: 2026-10-28\nstatus: Draft\n---\n## 1. Goal\n".to_string();
        text.push_str("## Amendment \u{2014} 2026-10-28 18:48 E");
        let p = read_plan(
            "docs/design/20261028_04_X_PLAN.md",
            file_name("20261028_04_X_PLAN.md").unwrap(),
            &text,
            true,
        );
        assert_eq!(p.error.as_deref(), Some("cut at 64 KiB"));
        assert!(p.amendments.is_empty(), "the partial last line is left out");
        assert_eq!(p.front.unwrap().status.as_deref(), Some("Draft"));
    }

    #[test]
    fn the_readme_table() {
        let readme = "# P\n\nIntro.\n\n```\n## Architectural decisions\n```\n\n## Architectural decisions\n\n| Document | Date | Status | Decision |\n| --- | --- | --- | --- |\n| [A](docs/design/20261028_01_A_PLAN.md) | 2026-10-28 | Draft | Does a \\| b. |\n| [B](./docs/design/20261028_02_B_SPEC.md) | 2026-10-28 | Retired (by [A](docs/design/20261028_01_A_PLAN.md)) | Gone. |\n| no link | x | y | z |\n\n";
        let rows = readme_rows(readme);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row, 13);
        assert_eq!(rows[0].target, "docs/design/20261028_01_A_PLAN.md");
        assert_eq!(rows[0].decision, "Does a | b.");
        assert_eq!(
            root_relative(&rows[1].target).as_deref(),
            Some("docs/design/20261028_02_B_SPEC.md")
        );
        assert_eq!(
            rows[1].status,
            "Retired (by [A](docs/design/20261028_01_A_PLAN.md))"
        );
        assert!(readme_rows("# no table\n").is_empty());
        assert!(
            readme_rows("## Architectural decisions\n\n| A | B |\n| - | - |\n| x | y |\n")
                .is_empty()
        );
    }

    #[test]
    fn statuses_compare_by_the_file_their_reference_names() {
        assert!(same_status("Draft", "Draft"));
        assert!(!same_status("Draft", "In implementation"));
        assert!(same_status(
            "Retired (by 20261022_01_ORCHESTRATION_DAEMON_PLAN)",
            "Retired (by [handlr as a daemon](docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md))"
        ));
        assert!(!same_status(
            "Retired (by 20261022_01_ORCHESTRATION_DAEMON_PLAN)",
            "Retired (by [other](docs/design/20261022_02_OTHER_PLAN.md))"
        ));
        // An external reference, as ~/src/rules' README has one.
        assert!(same_status(
            "Superseded (by ../tpt/docs/design/20261027_01_TPT_PLAN.md)",
            "Superseded (by [tpt](../tpt/./docs/design/20261027_01_TPT_PLAN.md))"
        ));
        assert!(!same_status(
            "Superseded (by ../tpt/docs/design/20261027_01_TPT_PLAN.md)",
            "Superseded (by ../rules/docs/design/20261027_01_TPT_PLAN.md)"
        ));
        assert_eq!(root_relative("a/../../b").as_deref(), Some("../b"));
        assert_eq!(root_relative("https://x/y"), None);
    }

    #[test]
    fn the_list_shape_matches_tpts_schema_1() {
        let p =
            plan("---\ntitle: T\ndate: 2026-10-28\nstatus: Draft\ncovers: one\n---\n## 1. Goal\n");
        let v = plan_json(&p, false);
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        let mut want = vec![
            "file",
            "base",
            "date_from_name",
            "nn",
            "name",
            "kind",
            "title",
            "date",
            "status",
            "status_base",
            "status_by",
            "last_amendment",
            "readme",
            "error",
        ];
        want.sort();
        let mut keys = keys;
        keys.sort();
        assert_eq!(keys, want);
        assert_eq!(v["nn"], 4);
        assert_eq!(v["kind"], "plan");
        assert_eq!(v["status_by"], Value::Null);
        assert_eq!(v["readme"], Value::Null);
        assert_eq!(v["last_amendment"], Value::Null);
        let v = plan_json(&p, true);
        assert_eq!(v["covers"], json!(["one"]));
        assert_eq!(v["sources"], json!([]), "never null for lists");
        assert_eq!(v["headings"], json!([{"line": 7, "text": "1. Goal"}]));
        assert_eq!(v["amendments"], json!([]));
        assert_eq!(v["extra"], json!({}));
    }

    #[test]
    fn statuses_abbreviated_as_the_plan_table() {
        for (status, shown) in [
            ("Draft", "Draft"),
            ("Hold", "Hold"),
            ("In implementation", "InImpl"),
            ("Implemented", "Impl"),
            ("Superseded (by 20261028_04_PLUGINS_PLAN)", "Succd"),
            ("Retired (by x)", "Rtrd"),
            ("Withdrawn", "Withd"),
        ] {
            assert_eq!(status_abbrev(status), shown, "{status}");
        }
    }

    #[test]
    fn update_times_read_zones_and_bare_dates() {
        let edt = update_key("2026-10-28 23:14 EDT").unwrap();
        assert_eq!(update_key("2026-10-29 03:14 UTC"), Some(edt));
        assert_eq!(update_key("2026-10-29 08:44 +05:30"), Some(edt));
        assert!(update_key("2026-10-28").unwrap() < edt);
        assert_eq!(update_key("soon"), None);
        assert_eq!(update_key("2026-13-01"), None);
    }

    #[test]
    fn the_agent_list_sorts_by_last_update_and_falls_back() {
        let listed = json!({"root": "r", "plans": [
            {"base": "20261001_01_A_PLAN", "nn": 1, "date_from_name": "2026-10-01",
             "title": "A", "status": "Draft", "date": "2026-10-01", "last_amendment": null},
            {"base": "20261002_01_B_PLAN", "nn": 1, "date_from_name": "2026-10-02",
             "title": null, "status": null, "date": null, "last_amendment": null,
             "error": "unreadable"},
            {"base": "20260901_02_C_PLAN", "nn": 2, "date_from_name": "2026-09-01",
             "title": "C", "status": "Hold", "date": "2026-09-01",
             "last_amendment": "2026-10-03 09:00 EDT"},
        ]});
        let v = agent_list(&listed);
        let names: Vec<&str> = v["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "20260901_02_C_PLAN",
                "20261002_01_B_PLAN",
                "20261001_01_A_PLAN"
            ]
        );
        let b = &v["plans"][1];
        assert_eq!(b["title"], "20261002_01_B_PLAN", "no title: the name");
        assert_eq!(b["date"], "2026-10-02", "no date: the file name's");
        assert_eq!(b["status"], Value::Null);
        assert_eq!(b["error"], "unreadable");
        assert_eq!(v["plans"][0]["status"], "Hold");
    }
}
