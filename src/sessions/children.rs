//! A session's children: the subagents it started
//! (docs/design/20261023_07_SUBAGENTS_PLAN.md, decision 1, and the 17:47
//! EDT amendment, items 3 and 5).
//!
//! A child is a session of its own, in its parent reader's format, so the
//! same reader reads it and everything above the readers applies to it
//! unchanged. Only Claude Code's reader finds children.

use super::Located;

/// One session started from another (decision 1): Claude Code's
/// background and foreground agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Child {
    /// Claude Code's agent id (`agent-<id>.jsonl`).
    pub id: String,
    /// Its transcript.
    pub located: Located,
    /// As the parent gave it; the id when its meta file is missing or
    /// unreadable.
    pub description: String,
    /// `general-purpose`, `Explore`, …; empty when not known.
    pub kind: String,
    /// Started in the background (`requestShape` `background`).
    pub background: bool,
    /// Marked finished, or not written within
    /// [`SUBAGENT_LIVE`](super::SUBAGENT_LIVE) (as
    /// [`Subagents::running`](super::Subagents::running) counts them).
    pub finished: bool,
    /// The parent's tool call that started it.
    pub started_by: Option<String>,
    /// A worktree's branch, never its path (decision 7).
    pub worktree: Option<String>,
    /// Its first entry's time, in seconds since 1970 (the 17:47 EDT
    /// amendment, item 5); the file's creation time when the first line
    /// has none.
    pub started: Option<u64>,
}

/// Words left out of a short name (the 17:47 EDT amendment, item 3):
/// articles, prepositions, conjunctions and other words that carry no
/// purpose, and the task verbs and plan words.
const STOP_WORDS: &[&str] = &[
    // Articles, pronouns and determiners.
    "a",
    "an",
    "the",
    "this",
    "that",
    "these",
    "those",
    "its",
    "it",
    "my",
    "our",
    "their",
    "all",
    "any",
    "each",
    "every",
    "some",
    "new",
    "one",
    "two", // Prepositions.
    "of",
    "in",
    "on",
    "at",
    "to",
    "for",
    "from",
    "with",
    "by",
    "into",
    "onto",
    "about",
    "over",
    "under",
    "after",
    "before",
    "through",
    "via",
    "per",
    "as",
    "without",
    "within",
    "across",
    "up",
    "out",
    "off",
    "against",
    "between", // Conjunctions.
    "and",
    "or",
    "but",
    "nor",
    "so",
    "yet",
    "then",
    "plus",
    "vs",
    "&", // Verbs of being.
    "is",
    "are",
    "be",
    "been", // Plan words.
    "plan",
    "plans",
    "step",
    "steps",
    "phase",
    "phases",
    "part",
    "parts",
    "task",
    "tasks",
    "amendment",
    "amendments", // Task verbs.
    "build",
    "builds",
    "building",
    "implement",
    "implements",
    "implementing",
    "fix",
    "fixes",
    "fixing",
    "add",
    "adds",
    "adding",
    "write",
    "writes",
    "writing",
    "update",
    "updates",
    "updating",
    "make",
    "makes",
    "making",
    "run",
    "runs",
    "running",
    "check",
    "checks",
    "checking",
    "create",
    "creates",
    "do",
    "does",
    "finish",
    "start",
    "continue",
    "apply",
    "review",
    "reviews",
    "explore",
    "research",
    "audit",
    "inspect",
    "debug",
    "investigate",
    "find",
    "look",
    "search",
    "read",
    "verify",
    "draft",
    "get",
    "set",
    "port",
    "test",
    "tests",
    "try",
    "handle",
    "redo",
    "rework",
];

/// A plan number such as `20261023_05`, a step number such as `3` or
/// `1-4`, or anything else with no letter in it.
fn is_number(word: &str) -> bool {
    !word.chars().any(char::is_alphabetic)
}

/// A word written in capitals (`README`, `S23`, `API`): two characters
/// or more, its letters all upper case.
fn in_capitals(word: &str) -> bool {
    let mut letters = word.chars().filter(|c| c.is_alphabetic()).peekable();
    word.chars().count() >= 2 && letters.peek().is_some() && letters.all(char::is_uppercase)
}

/// A subagent's one or two words of purpose (the 17:47 EDT amendment,
/// item 3; `ChildInfo::short`): the description's words with stop words,
/// task verbs and plan numbers removed, the first two left, in their
/// order, lower-cased except for words written in capitals. Nothing left:
/// `kind`. So "Build the overview screen plan" is "overview screen" and
/// "libtypedmd licences and README" is "libtypedmd licences".
pub fn short_name(description: &str, kind: &str) -> String {
    let words: Vec<String> = description
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '/' | '(' | ')'))
        .map(|w| w.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_')))
        .filter(|w| !w.is_empty() && !is_number(w))
        .filter(|w| !STOP_WORDS.contains(&w.to_lowercase().as_str()))
        .take(2)
        .map(|w| {
            if in_capitals(w) {
                w.to_string()
            } else {
                w.to_lowercase()
            }
        })
        .collect();
    if words.is_empty() {
        return kind.to_string();
    }
    words.join(" ")
}

/// Seconds since 1970 of an RFC 3339 time (`2026-10-24T15:56:00.123Z`,
/// `…+02:00`), its fraction dropped.
pub(crate) fn rfc3339_secs(ts: &str) -> Option<u64> {
    let ts = ts.trim();
    let b = ts.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let s = ts.get(r)?;
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let mut rest = &ts[19..];
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &r[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 && matches!(&r[..1], "+" | "-") && &r[3..4] == ":" => {
            let oh: i64 = r[1..3].parse().ok()?;
            let om: i64 = r[4..6].parse().ok()?;
            let o = oh * 3600 + om * 60;
            if r.starts_with('-') { -o } else { o }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + s - offset).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_of_recorded_descriptions() {
        // The amendment's own examples.
        assert_eq!(
            short_name("Build the overview screen plan", "general-purpose"),
            "overview screen"
        );
        assert_eq!(
            short_name("libtypedmd licences and README", "general-purpose"),
            "libtypedmd licences"
        );
        // Descriptions as this repository's sessions recorded them.
        for (description, want) in [
            ("Build subagents plan steps 1-4", "subagents"),
            (
                "Implement plan 20261023_05 spoken summaries",
                "spoken summaries",
            ),
            ("Fix the S23 install script", "S23 install"),
            ("Review the choice questions diff", "choice questions"),
            ("Write README and AGENTS.md", "README agents.md"),
            ("Explore session readers", "session readers"),
            ("Check: the link's keys", "link's keys"),
        ] {
            assert_eq!(short_name(description, "Explore"), want, "{description}");
        }
        // Nothing left: the agent type.
        assert_eq!(short_name("Build the plan", "Explore"), "Explore");
        assert_eq!(short_name("", "general-purpose"), "general-purpose");
        assert_eq!(short_name("20261023_05 step 3", ""), "");
    }

    #[test]
    fn rfc3339_times_in_seconds() {
        assert_eq!(rfc3339_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_secs("2026-01-05T09:00:00.000Z"),
            Some(1_767_603_600)
        );
        assert_eq!(
            rfc3339_secs("2026-01-05T11:00:00.5+02:00"),
            Some(1_767_603_600)
        );
        assert_eq!(rfc3339_secs("2026-01-05"), None);
        assert_eq!(rfc3339_secs("2026-13-05T09:00:00Z"), None);
        assert_eq!(rfc3339_secs("not a time at all"), None);
    }
}
