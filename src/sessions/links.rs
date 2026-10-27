//! Repowiki links and plan questions (docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md,
//! the 21:57 EDT amendment, living section L3).
//!
//! With `[urls] repowiki_url` set, every `docs/…` path in an entry's or a pending
//! question's (redacted) text gets a [`Link`] to its page on repowiki:
//! `<url>/#/<site>/<path without .md>`, where the site is the agent's
//! directory's parent and own name joined with `--` (repowiki's own rule:
//! `/Users/tmeade/src/flankr` is `src--flankr`). Without the setting
//! no `docs/…` path links.
//!
//! Every `http://` or `https://` URL in the text links to itself, with or
//! without the setting (the owner, 2026-10-23: "we should also detect other
//! http/https links and make them clickable"): only those two schemes,
//! never `file:`, `content:`, `intent:`, `javascript:` or others. A URL
//! ends at whitespace, without the `.,;:!?)]}>"'` that end a sentence or
//! wrap it (a `)` that closes a `(` inside the URL stays); a Markdown link
//! `[text](url)` links its text.
//!
//! A pending question is about a plan when it names a
//! `docs/design/…_PLAN.md` or says "plan", "amendment", "draft" or
//! "approve"; its plan's spoken name is given when exactly one plan path
//! names it, read as the reading rules read a path ("the overview screen
//! plan").

use std::path::Path;

use libflankr_proto::common::{Entry, Link, PendingQuestion};

/// The repowiki site id for an agent's directory: its parent's and its own
/// name joined with `--`. None for a directory without both.
pub fn site_id(cwd: &Path) -> Option<String> {
    let name = cwd.file_name()?.to_str()?;
    let parent = cwd.parent()?.file_name()?.to_str()?;
    Some(format!("{parent}--{name}"))
}

/// A character a path may hold.
fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')
}

/// The `docs/…` paths in `text`, as written and in order, each once. A path
/// starts at `docs/` not inside a longer word or path and runs while its
/// characters are path characters; a trailing `.` (a sentence's end) is not
/// part of it, and a path cut short with "…" is left out.
pub fn docs_paths(text: &str) -> Vec<&str> {
    let mut found: Vec<&str> = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("docs/").map(|i| from + i) {
        from = at + "docs/".len();
        if text[..at].chars().next_back().is_some_and(path_char) {
            continue;
        }
        let len = text[at..]
            .char_indices()
            .find(|&(_, c)| !path_char(c))
            .map_or(text.len() - at, |(i, _)| i);
        let path = text[at..at + len].trim_end_matches('.');
        from = at + len;
        if text[at + len..].starts_with('…') || path.len() <= "docs/".len() {
            continue;
        }
        if !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// The links for the `docs/…` paths in `text`, on the repowiki at `url`
/// for `site`.
pub fn links(url: &str, site: &str, text: &str) -> Vec<Link> {
    docs_links(url, site, text)
        .into_iter()
        .map(|(_, l)| l)
        .collect()
}

/// [`links`], each with where its text starts in `text`.
fn docs_links(url: &str, site: &str, text: &str) -> Vec<(usize, Link)> {
    let base = url.trim_end_matches('/');
    docs_paths(text)
        .into_iter()
        .map(|path| {
            let at = path.as_ptr() as usize - text.as_ptr() as usize;
            let link = Link {
                text: path.to_string(),
                url: format!(
                    "{base}/#/{site}/{}",
                    path.strip_suffix(".md").unwrap_or(path)
                ),
            };
            (at, link)
        })
        .collect()
}

/// The schemes a web link may have.
const SCHEMES: [&str; 2] = ["https://", "http://"];

/// What ends a sentence or wraps a URL, and is not part of it at its end.
const TRAILING: [char; 13] = [
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '"', '\'', '`',
];

/// `url` without the sentence or wrapping marks at its end; a `)` that
/// closes a `(` inside it stays.
fn trim_url(mut url: &str) -> &str {
    while let Some(last) = url.chars().next_back().filter(|c| TRAILING.contains(c)) {
        if last == ')' && url.matches('(').count() >= url.matches(')').count() {
            break;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
    url
}

/// A URL with one of [`SCHEMES`] and something after it.
fn is_web(url: &str) -> bool {
    SCHEMES
        .iter()
        .any(|s| url.len() > s.len() && url[..s.len()].eq_ignore_ascii_case(s))
}

/// The `http://` and `https://` links in `text`, each linking to itself,
/// or a Markdown link's text to its url, with where its text starts.
fn web_links(text: &str) -> Vec<(usize, Link)> {
    let mut found = Vec::new();
    // Markdown links first; their urls are not found again as bare ones.
    let mut covered: Vec<(usize, usize)> = Vec::new();
    let mut from = 0;
    while let Some(mid) = text[from..].find("](").map(|i| from + i) {
        from = mid + 2;
        let Some(open) = text[..mid].rfind('[') else {
            continue;
        };
        let label = &text[open + 1..mid];
        if label.contains(['[', '\n']) || label.trim().is_empty() {
            continue;
        }
        // The url runs to the `)` that balances its `(`.
        let rest = &text[mid + 2..];
        let mut depth = 0usize;
        let Some(close) = rest.char_indices().find_map(|(i, c)| match c {
            '(' => {
                depth += 1;
                None
            }
            ')' if depth == 0 => Some(i),
            ')' => {
                depth -= 1;
                None
            }
            _ => None,
        }) else {
            continue;
        };
        let url = rest[..close].trim();
        if !is_web(url) || url.contains(char::is_whitespace) {
            continue;
        }
        found.push((
            open + 1,
            Link {
                text: label.to_string(),
                url: url.to_string(),
            },
        ));
        covered.push((open, mid + 2 + close + 1));
        from = mid + 2 + close + 1;
    }
    // Bare URLs.
    let lower = text.to_ascii_lowercase();
    for scheme in SCHEMES {
        let mut from = 0;
        while let Some(at) = lower[from..].find(scheme).map(|i| from + i) {
            from = at + scheme.len();
            if covered.iter().any(|&(a, b)| at >= a && at < b)
                || text[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            let len = text[at..]
                .char_indices()
                .find(|&(_, c)| c.is_whitespace())
                .map_or(text.len() - at, |(i, _)| i);
            let url = trim_url(&text[at..at + len]);
            from = at + len;
            if url.ends_with('…') || !is_web(url) {
                continue;
            }
            found.push((
                at,
                Link {
                    text: url.to_string(),
                    url: url.to_string(),
                },
            ));
        }
    }
    found
}

/// Where links point: the repowiki's url and the agent's site, when
/// `[urls] repowiki_url` is set and the directory has a site; web links need
/// neither.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Linker {
    repowiki: Option<(String, String)>,
}

impl Linker {
    /// The linker for an agent in `cwd`.
    pub fn new(url: Option<&str>, cwd: Option<&str>) -> Self {
        let url = url.map(str::trim).filter(|u| !u.is_empty());
        let site = cwd.and_then(|c| site_id(Path::new(c)));
        Linker {
            repowiki: url.zip(site).map(|(u, s)| (u.to_string(), s)),
        }
    }

    /// Every link in `text`, in order, each text once: its web links, and
    /// its `docs/…` paths when there is a repowiki.
    pub fn links(&self, text: &str) -> Vec<Link> {
        let mut all = web_links(text);
        if let Some((url, site)) = &self.repowiki {
            all.extend(docs_links(url, site, text));
        }
        all.sort_by_key(|(at, _)| *at);
        let mut out: Vec<Link> = Vec::new();
        for (_, link) in all {
            if !out.iter().any(|l| l.text == link.text) {
                out.push(link);
            }
        }
        out
    }

    /// Give `entry` the links of its summary.
    pub fn link_entry(&self, entry: &mut Entry) {
        entry.links = self.links(entry.text.as_deref().unwrap_or(&entry.summary));
    }

    /// Give `question` the links of its text.
    pub fn link_question(&self, question: &mut PendingQuestion) {
        question.links = self.links(&question.text);
    }
}

/// Whether `question` asks to approve a plan, and the plan's id as the text
/// gives it (plan 20261022_03, L3, the 2026-10-27 13:21, 13:24 and 14:04 EDT
/// amendments): a heuristic on the words alone, tied to no planning format.
/// It is about a plan when it has "approve" and "plan", or "approve" and
/// "draft", as whole words. The id is the word after "plan" or "draft" when
/// that holds a digit, given as "plan <id>"; without one in the question,
/// the last such id in `context` (the rest of the reply), so "Approve the
/// draft?" after "I wrote plan 06 as a draft." names plan 06; with none
/// there either, a question about a draft names "the draft". Two differing
/// ids in the question name none.
pub fn plan_question(question: &str, context: &str) -> (bool, Option<String>) {
    let words = words_of(question);
    let has = |word: &str| words.iter().any(|w| bare(w) == word);
    if !(has("approve") && (has("plan") || has("draft"))) {
        return (false, None);
    }
    (true, plan_named(question, context))
}

/// The plan `question` names, as [`plan_question`] names it, whatever its
/// other words: "plan <id>" from the question or else the last id in
/// `context`, "the draft" for a question that says "draft" and not "plan",
/// and none otherwise (or for two differing ids in the question).
fn plan_named(question: &str, context: &str) -> Option<String> {
    let words = words_of(question);
    let has = |word: &str| words.iter().any(|w| bare(w) == word);
    match plan_ids(&words).as_slice() {
        [one] => Some(format!("plan {one}")),
        [] => match plan_ids(&words_of(context)).last() {
            Some(id) => Some(format!("plan {id}")),
            None if has("draft") && !has("plan") => Some("the draft".to_string()),
            None => None,
        },
        _ => None,
    }
}

/// The `detected_plan_id` replacement of SUGGESTIONS.md (plan 20261027_06,
/// decision 4): the plan [`plan_question`] would name, else "the draft"
/// when the question says "draft" and not "plan", else "the plan".
pub fn detected_plan_id(question: &str, context: &str) -> String {
    plan_named(question, context).unwrap_or_else(|| "the plan".to_string())
}

/// The words of `text`, split at spaces, brackets, quotes and commas.
pub(crate) fn words_of(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '`' | '"' | '\'' | ','))
        .filter(|w| !w.is_empty())
        .collect()
}

/// A word without its surrounding punctuation, lower case.
pub(crate) fn bare(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
        .to_ascii_lowercase()
}

/// The ids after "plan" or "draft" (a word holding a digit, `#` dropped), in
/// order, each once.
fn plan_ids(words: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for pair in words.windows(2) {
        if !matches!(bare(pair[0]).as_str(), "plan" | "draft") {
            continue;
        }
        let id = pair[1]
            .trim_start_matches(['#', '№'])
            .trim_end_matches(|c: char| !c.is_alphanumeric());
        let ok = !id.is_empty()
            && id.chars().any(|c| c.is_ascii_digit())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if ok && !ids.iter().any(|known| known == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::common::{EntryKind, Role};

    const PLAN: &str = "docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md";

    #[test]
    fn the_site_is_the_parent_and_the_directory() {
        assert_eq!(
            site_id(Path::new("/Users/tmeade/src/flankr")).as_deref(),
            Some("src--flankr")
        );
        assert_eq!(site_id(Path::new("/")), None);
    }

    #[test]
    fn docs_paths_are_found_as_written() {
        let text = format!(
            "See `{PLAN}`. Also [the spec](docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md), \
             docs/FONT_X.md and {PLAN} again; not src/docs/x.md, mydocs/y.md or docs/."
        );
        assert_eq!(
            docs_paths(&text),
            vec![
                PLAN,
                "docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md",
                "docs/FONT_X.md"
            ]
        );
        assert!(docs_paths("cut at docs/design/2026…").is_empty());
        assert!(docs_paths("no paths here").is_empty());
    }

    #[test]
    fn links_point_at_the_page_without_md() {
        let links = links("http://wiki:8088/", "src--flankr", &format!("Read {PLAN}."));
        assert_eq!(
            links,
            vec![Link {
                text: PLAN.to_string(),
                url: "http://wiki:8088/#/src--flankr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn docs_paths_are_not_linked_without_the_setting() {
        let text = format!("Wrote {PLAN}");
        assert!(
            Linker::new(None, Some("/src/flankr"))
                .links(&text)
                .is_empty()
        );
        assert!(
            Linker::new(Some(" "), Some("/src/flankr"))
                .links(&text)
                .is_empty()
        );
        assert!(Linker::new(Some("http://w"), None).links(&text).is_empty());
        let linker = Linker::new(Some("http://w"), Some("/Users/t/src/flankr"));
        let mut entry = Entry {
            ts: "t".into(),
            role: Role::Assistant,
            kind: EntryKind::Text,
            tool: None,
            summary: format!("Wrote {PLAN}"),
            links: vec![],
            origin: None,
            text: None,
        };
        linker.link_entry(&mut entry);
        assert_eq!(entry.links.len(), 1);
        assert_eq!(
            entry.links[0].url,
            "http://w/#/src--flankr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN"
        );
    }

    fn web(text: &str) -> Vec<(String, String)> {
        Linker::default()
            .links(text)
            .into_iter()
            .map(|l| (l.text, l.url))
            .collect()
    }

    fn same(url: &str) -> (String, String) {
        (url.to_string(), url.to_string())
    }

    #[test]
    fn web_links_link_to_themselves_without_the_setting() {
        assert_eq!(
            web("See https://example.com/a?b=1 and http://host:8088/x."),
            vec![
                same("https://example.com/a?b=1"),
                same("http://host:8088/x")
            ]
        );
        assert_eq!(
            web("HTTPS://Example.com/"),
            vec![same("HTTPS://Example.com/")]
        );
    }

    #[test]
    fn web_links_lose_the_marks_that_end_a_sentence_or_wrap_them() {
        for (text, url) in [
            ("Read https://a.b/c.", "https://a.b/c"),
            ("Read https://a.b/c, then", "https://a.b/c"),
            ("Is it https://a.b/c?", "https://a.b/c"),
            ("(see https://a.b/c)", "https://a.b/c"),
            ("[https://a.b/c]", "https://a.b/c"),
            ("{https://a.b/c}", "https://a.b/c"),
            ("<https://a.b/c>", "https://a.b/c"),
            ("\"https://a.b/c\"", "https://a.b/c"),
            ("'https://a.b/c';", "https://a.b/c"),
            ("`https://a.b/c`", "https://a.b/c"),
            ("https://a.b/c!:", "https://a.b/c"),
            // A `)` that closes a `(` inside the URL stays.
            (
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            ),
            (
                "(https://en.wikipedia.org/wiki/Rust_(programming_language)).",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            ),
        ] {
            assert_eq!(web(text), vec![same(url)], "{text}");
        }
    }

    #[test]
    fn only_http_and_https_link() {
        for text in [
            "file:///etc/passwd",
            "content://media/x",
            "intent://scan/#Intent;end",
            "javascript:alert(1)",
            "ftp://host/x",
            "https://",
            "xhttps://a.b",
            "cut at https://a.b/c…",
        ] {
            assert!(web(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_markdown_link_links_its_text() {
        assert_eq!(
            web("See [the docs](https://a.b/Rust_(lang)) and https://c.d."),
            vec![
                (
                    "the docs".to_string(),
                    "https://a.b/Rust_(lang)".to_string()
                ),
                same("https://c.d")
            ]
        );
        // Not a web link: nothing.
        assert!(web("[x](javascript:alert(1))").is_empty());
    }

    #[test]
    fn web_and_docs_links_come_in_text_order() {
        let linker = Linker::new(Some("http://w"), Some("/Users/t/src/flankr"));
        let texts: Vec<String> = linker
            .links(&format!("{PLAN} is at https://a.b/c; see {PLAN}."))
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(texts, vec![PLAN.to_string(), "https://a.b/c".to_string()]);
    }

    #[test]
    fn plan_questions_are_found_and_named() {
        let yes = |id: Option<&str>| (true, id.map(str::to_string));
        assert_eq!(plan_question("Approve plan 03?", ""), yes(Some("plan 03")));
        assert_eq!(
            plan_question("Do you approve the plan (plan 20261027_04) as written?", ""),
            yes(Some("plan 20261027_04"))
        );
        assert_eq!(
            plan_question("Approve plan #5, then build it?", ""),
            yes(Some("plan 5"))
        );
        assert_eq!(plan_question("Shall I approve the plan?", ""), yes(None));
        // Two different ids: about a plan, but no one id.
        assert_eq!(plan_question("Approve plan 03 or plan 04?", ""), yes(None));
        // Both words are needed.
        assert_eq!(
            plan_question("Restart the daemon on the plan 05 build?", ""),
            (false, None)
        );
        assert_eq!(plan_question("Do you approve?", ""), (false, None));
        assert_eq!(
            plan_question("Shall I write the amendment?", ""),
            (false, None)
        );
        assert_eq!(plan_question("Is the DRAFT right?", ""), (false, None));
        // "approve" with "draft" is about a plan too, named as the draft.
        assert_eq!(
            plan_question("Approve the DRAFT?", ""),
            yes(Some("the draft"))
        );
        assert_eq!(
            plan_question("Approve the draft of plan 06?", ""),
            yes(Some("plan 06"))
        );
        // The id after "draft", or from the rest of the reply.
        assert_eq!(plan_question("Approve draft 07?", ""), yes(Some("plan 07")));
        let reply = "I wrote plan 20261027_06 as a draft, after plan 05.";
        assert_eq!(
            plan_question("Approve the draft?", reply),
            yes(Some("plan 05"))
        );
        assert_eq!(
            plan_question("Approve the draft?", "I wrote it as plan 06, a draft."),
            yes(Some("plan 06"))
        );
        assert_eq!(
            plan_question("Approve the draft?", "Nothing numbered."),
            yes(Some("the draft"))
        );
        assert_eq!(plan_question("Install it on the S23?", ""), (false, None));
        // A plan's path alone is no longer enough.
        assert_eq!(
            plan_question(&format!("Shall I implement `{PLAN}`?"), ""),
            (false, None)
        );
        // "planet", "planning" and "approved" are not the words.
        assert_eq!(
            plan_question("Is the planet planning approved?", ""),
            (false, None)
        );
    }
}
