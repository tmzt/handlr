//! `flankr-speech`: an agent's reply as it is read aloud
//! (docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md, the 16:28 EDT
//! amendment, A1, A2 and A5).
//!
//! [`read_aloud`] applies the reading rules (links as their text, paths and
//! inline code as their English words, no code blocks or Markdown marks)
//! and reads clock times relative to `now`. [`complexity`] measures what is
//! left after the rules, to decide whether a reply needs Gemini's summary.
//! The daemon and the companion client both use them, so there is one copy
//! of the rules. Nothing here reads the clock: "now" is a parameter.

mod times;

pub use times::LocalTime;

/// A1's thresholds: complex when code-like words are at least
/// `CODE_WORDS_PERCENT` of the words or at least `CODE_WORDS_MIN` of them,
/// or when there are `LIST_ITEMS_MIN` list items or a table row.
pub const CODE_WORDS_PERCENT: usize = 10;
pub const CODE_WORDS_MIN: usize = 4;
pub const LIST_ITEMS_MIN: usize = 3;

/// A reply as it is read aloud: without code blocks, Markdown emphasis,
/// headings or list bullets, on one line. A link is read as its text, and
/// a path or inline code as its English words only: a path as "the" and
/// the words of its last part ("docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md"
/// is "the overview screen plan"), an identifier as its words
/// (`AgentInfo.asking` is "agent info asking"), and a commit hash not at
/// all. A clock time is read relative to `now` ("at 16:13:48" is "three
/// minutes ago"; A2).
pub fn read_aloud(text: &str, now: LocalTime) -> String {
    times::relative_times(&rules(text), now)
}

/// A reply's measures (A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Complexity {
    /// Words left after the reading rules, split on whitespace.
    pub words: usize,
    /// Of those, the words that still look like code.
    pub code_words: usize,
    /// Lines of the reply as written starting `- `, `* ` or `<n>. `.
    pub list_items: usize,
    /// Lines of the reply as written starting `|`.
    pub table_rows: usize,
}

impl Complexity {
    /// Whether the reply needs a summary rather than being read through
    /// the rules.
    pub fn is_complex(&self) -> bool {
        (self.words > 0 && self.code_words * 100 >= self.words * CODE_WORDS_PERCENT)
            || self.code_words >= CODE_WORDS_MIN
            || self.list_items >= LIST_ITEMS_MIN
            || self.table_rows > 0
    }
}

/// Measure `text` (A1): the code-like words of what the reading rules
/// leave, and the list items and table rows of the reply as written.
/// Neither its length nor a backquote alone makes it complex.
pub fn complexity(text: &str) -> Complexity {
    let mut c = Complexity::default();
    // Clock times become words whatever "now" is; midnight UTC will do.
    let spoken = read_aloud(text, LocalTime::default());
    for word in spoken.split_whitespace() {
        c.words += 1;
        if code_like(word) {
            c.code_words += 1;
        }
    }
    for line in text.lines().map(str::trim_start) {
        if line.starts_with('|') {
            c.table_rows += 1;
        } else if is_list_item(line) {
            c.list_items += 1;
        }
    }
    c
}

/// Whether a word looks like code: it has `/`, `::`, `_`, `=`, `{`, `}`,
/// `<`, `>`, `#` or `$`, a `.` between two letters, a capital after a
/// lower-case letter, or both letters and digits.
fn code_like(word: &str) -> bool {
    if word.contains("::") || word.contains(['/', '_', '=', '{', '}', '<', '>', '#', '$']) {
        return true;
    }
    let chars: Vec<char> = word.chars().collect();
    let dotted = chars
        .windows(3)
        .any(|w| w[1] == '.' && w[0].is_alphabetic() && w[2].is_alphabetic());
    let camel = chars
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase());
    let mixed = chars.iter().any(|c| c.is_alphabetic()) && chars.iter().any(|c| c.is_ascii_digit());
    dotted || camel || mixed
}

fn is_list_item(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") {
        return true;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
}

/// The reading rules, before clock times.
fn rules(text: &str) -> String {
    let mut lines = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.is_empty() {
            continue;
        }
        let t = t.trim_start_matches('#').trim_start();
        let t = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .unwrap_or(t);
        lines.push(spoken_line(&unlink(t)));
    }
    lines
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `[text](target)` as `text`.
fn unlink(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|c| open + c) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|e| close + e) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A line's words, with inline code and paths read as their English words.
/// A word's punctuation stays; brackets left empty go.
fn spoken_line(line: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for (i, part) in line.split('`').enumerate() {
        let code = i % 2 == 1;
        for word in part.split_whitespace() {
            let punct = |c: char| matches!(c, ',' | ';' | ':' | '.' | '(' | ')' | '"' | '!' | '?');
            let start = word.len() - word.trim_start_matches(punct).len();
            let end = word.trim_end_matches(punct).len().max(start);
            let core = &word[start..end];
            if code || looks_like_path(core) {
                let spoken = spoken_code(core);
                out.push(format!("{}{spoken}{}", &word[..start], &word[end..]));
            } else {
                out.push(word.replace('*', ""));
            }
        }
    }
    out.retain(|w| !w.is_empty());
    out.join(" ")
        .replace("( )", "")
        .replace("()", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" .", ".")
        .replace(" ,", ",")
}

/// Whether a bare word is a path or file name: it has a `/`, or ends in a
/// known extension.
fn looks_like_path(word: &str) -> bool {
    const EXT: [&str; 9] = [
        ".md", ".rs", ".toml", ".json", ".jsonl", ".sh", ".txt", ".log", ".apk",
    ];
    word.len() > 1 && (word.contains('/') || EXT.iter().any(|e| word.ends_with(e)))
}

/// Inline code or a path as its English words.
fn spoken_code(code: &str) -> String {
    const EXT: [&str; 9] = [
        "md", "rs", "toml", "json", "jsonl", "sh", "txt", "log", "apk",
    ];
    let is_path = code.contains('/') || looks_like_path(code);
    // A number or a version reads as it is.
    if code
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
    {
        return code.to_string();
    }
    // A commit hash: no words to read.
    if code.len() >= 7 && code.chars().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    let last = if is_path {
        code.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(code)
    } else {
        code
    };
    let mut words = Vec::new();
    for piece in last.split(|c: char| !c.is_ascii_alphanumeric()) {
        // camelCase and PascalCase into words.
        let mut word = String::new();
        let mut prev_lower = false;
        for c in piece.chars() {
            if c.is_ascii_uppercase() && prev_lower {
                words.push(std::mem::take(&mut word));
            }
            prev_lower = c.is_ascii_lowercase();
            word.push(c.to_ascii_lowercase());
        }
        words.push(word);
    }
    words.retain(|w| w.len() > 1 && w.chars().all(|c| c.is_ascii_alphabetic()));
    // A file's extension is not read.
    if is_path {
        if words.last().is_some_and(|w| EXT.contains(&w.as_str())) {
            words.pop();
        }
        if words.is_empty() {
            return String::new();
        }
        return format!("the {}", words.join(" "));
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16:16:48 EDT, three minutes after the replies' 16:13:48.
    const NOW: LocalTime = LocalTime {
        hour: 16,
        minute: 16,
        second: 48,
        utc_offset_minutes: -240,
    };

    fn said(text: &str) -> String {
        read_aloud(text, NOW)
    }

    #[test]
    fn markdown_is_read_as_its_words() {
        assert_eq!(
            said("## Result\n\nThe **tests** pass:\n```\nok\n```\n- `fmt` is clean"),
            "Result The tests pass: fmt is clean"
        );
    }

    #[test]
    fn paths_and_code_are_read_as_their_english_words() {
        assert_eq!(
            said("Wrote docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md and pushed."),
            "Wrote the overview screen plan and pushed."
        );
        assert_eq!(
            said(
                "Up as a Draft: [The overview (home) screen](docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md) (`355f5f0`)."
            ),
            "Up as a Draft: The overview (home) screen."
        );
        assert_eq!(
            said("A new field, `AgentInfo.asking`, in libflankr_proto 0.2.0."),
            "A new field, agent info asking, in libflankr_proto 0.2.0."
        );
        assert_eq!(
            said("They go in `src/overview/` and `apps/companion/src/agent/`."),
            "They go in the overview and the agent."
        );
        assert_eq!(said("Code -32014 is new."), "Code -32014 is new.");
    }

    /// Replies as agents gave them (step 6).
    const PLAIN: &str = "I ran the tests and they all pass. Shall I go on with the next step?";
    const BACKQUOTES: &str = "Done: `Sessions::reply_text` now joins the parts.";
    const LIST: &str = "I changed three files:\n\
        - src/server/speech.rs\n\
        - src/sessions/mod.rs\n\
        - crates/libflankr_proto/src/common.rs\n\
        All tests pass.";
    const PATHS: &str = "I wrote the plan in docs/design/20261023_05_SPOKEN_SUMMARIES_PLAN.md, \
        moved the rules from crates/flankr-companion/src/client.rs into \
        crates/flankr-speech/src/lib.rs, and changed `src/server/speech.rs` and \
        `src/server/conn.rs` to use them. The tests are in tests/companion.rs.";

    #[test]
    fn a_plain_reply_is_not_complex() {
        let c = complexity(PLAIN);
        assert_eq!(c.code_words, 0);
        assert!(!c.is_complex(), "{c:?}");
        assert!(!complexity("Yes, that works.").is_complex());
    }

    #[test]
    fn a_long_plain_reply_is_not_complex() {
        let long = format!(
            "{}Shall I go on?",
            "The rest of the change keeps every reply in order. ".repeat(14)
        );
        assert!(long.chars().count() > 600);
        assert!(!complexity(&long).is_complex());
        assert_eq!(said(&long), long);
    }

    #[test]
    fn a_reply_full_of_paths_is_read_through_the_rules_and_not_complex() {
        let c = complexity(PATHS);
        assert_eq!(c.code_words, 0, "{}", said(PATHS));
        assert!(!c.is_complex(), "{c:?}");
        assert_eq!(
            said(PATHS),
            "I wrote the plan in the spoken summaries plan, moved the rules from \
             the client into the lib, and changed the speech and the conn to use \
             them. The tests are in the companion."
        );
    }

    #[test]
    fn backquotes_alone_are_not_complex() {
        assert!(!complexity(BACKQUOTES).is_complex());
        assert_eq!(
            said(BACKQUOTES),
            "Done: sessions reply text now joins the parts."
        );
    }

    #[test]
    fn a_list_of_changes_is_complex() {
        let c = complexity(LIST);
        assert_eq!(c.list_items, 3);
        assert!(c.is_complex(), "{c:?}");
    }

    #[test]
    fn the_measures_each_count() {
        // Code-like words: each kind.
        for word in [
            "a/b",
            "a::b",
            "snake_case",
            "x=1",
            "{",
            "}",
            "<T>",
            "#3",
            "$HOME",
            "foo.rs",
            "AgentInfo",
            "v0",
            "355f5f0",
        ] {
            assert!(code_like(word), "{word}");
        }
        for word in ["plain", "Hello,", "end.", "3"] {
            assert!(!code_like(word), "{word}");
        }
        // Measured after the rules: 1 code word in 10 is 10%.
        let one_in_ten = "one two three four five six seven eight nine snake_case";
        assert!(complexity(one_in_ten).is_complex());
        let one_in_eleven = "one two three four five six seven eight nine ten snake_case";
        assert!(!complexity(one_in_eleven).is_complex());
        // Four code-like words are complex however long the reply.
        let four = format!(
            "{} AgentInfo FocusInfo SessionPeek PromptSet",
            "word ".repeat(60)
        );
        assert!(complexity(&four).is_complex());
        // Two list items are not enough; a table row is.
        assert!(!complexity("Two things:\n- one\n- two").is_complex());
        assert!(complexity("1. one\n2. two\n10. three").is_complex());
        assert!(complexity("| a | b |").is_complex());
        // Clock times are words once read.
        assert!(!complexity("Started at 20:02:49Z and done.").is_complex());
    }
}
