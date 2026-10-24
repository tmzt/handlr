//! Repowiki links and plan questions (docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md,
//! the 21:57 EDT amendment, living section L3).
//!
//! With `[repowiki] url` set, every `docs/…` path in an entry's or a pending
//! question's (redacted) text gets a [`Link`] to its page on repowiki:
//! `<url>/#/<site>/<path without .md>`, where the site is the agent's
//! directory's parent and own name joined with `--` (repowiki's own rule:
//! `/Users/tmeade/src/handlr` is `src--handlr`). Without the setting
//! nothing changes.
//!
//! A pending question is about a plan when it names a
//! `docs/design/…_PLAN.md` or says "plan", "amendment", "draft" or
//! "approve"; its plan's spoken name is given when exactly one plan path
//! names it, read as the reading rules read a path ("the overview screen
//! plan").

use std::path::Path;

use libhandlr_proto::common::{Entry, Link, PendingQuestion};

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
    let base = url.trim_end_matches('/');
    docs_paths(text)
        .into_iter()
        .map(|path| Link {
            text: path.to_string(),
            url: format!(
                "{base}/#/{site}/{}",
                path.strip_suffix(".md").unwrap_or(path)
            ),
        })
        .collect()
}

/// Where links point: the repowiki's url and the agent's site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linker {
    url: String,
    site: String,
}

impl Linker {
    /// The linker for an agent in `cwd`, when `[repowiki] url` is set and
    /// the directory has a site id.
    pub fn new(url: Option<&str>, cwd: Option<&str>) -> Option<Self> {
        let url = url.filter(|u| !u.trim().is_empty())?;
        let site = site_id(Path::new(cwd?))?;
        Some(Linker {
            url: url.trim().to_string(),
            site,
        })
    }

    /// Give `entry` the links of its summary.
    pub fn link_entry(&self, entry: &mut Entry) {
        entry.links = links(&self.url, &self.site, &entry.summary);
    }

    /// Give `question` the links of its text.
    pub fn link_question(&self, question: &mut PendingQuestion) {
        question.links = links(&self.url, &self.site, &question.text);
    }
}

/// A `docs/design/…_PLAN.md` path.
fn is_plan_path(path: &str) -> bool {
    path.starts_with("docs/design/") && path.ends_with("_PLAN.md")
}

/// The words that make a question about a plan.
const PLAN_WORDS: [&str; 4] = ["plan", "amendment", "draft", "approve"];

/// Whether `question` is about a plan, and the plan's spoken name when
/// exactly one plan path names it.
pub fn plan_question(question: &str) -> (bool, Option<String>) {
    let plans: Vec<&str> = docs_paths(question)
        .into_iter()
        .filter(|p| is_plan_path(p))
        .collect();
    let says = question
        .split(|c: char| !c.is_alphabetic())
        .any(|w| PLAN_WORDS.iter().any(|p| w.eq_ignore_ascii_case(p)));
    let about = says || !plans.is_empty();
    let name = match plans.as_slice() {
        [one] => {
            let spoken = handlr_speech::read_aloud(one, handlr_speech::LocalTime::default());
            (!spoken.is_empty()).then_some(spoken)
        }
        _ => None,
    };
    (about, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libhandlr_proto::common::{EntryKind, Role};

    const PLAN: &str = "docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md";

    #[test]
    fn the_site_is_the_parent_and_the_directory() {
        assert_eq!(
            site_id(Path::new("/Users/tmeade/src/handlr")).as_deref(),
            Some("src--handlr")
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
        let links = links("http://wiki:8088/", "src--handlr", &format!("Read {PLAN}."));
        assert_eq!(
            links,
            vec![Link {
                text: PLAN.to_string(),
                url: "http://wiki:8088/#/src--handlr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn nothing_is_linked_without_the_setting() {
        assert_eq!(Linker::new(None, Some("/src/handlr")), None);
        assert_eq!(Linker::new(Some(" "), Some("/src/handlr")), None);
        assert_eq!(Linker::new(Some("http://w"), None), None);
        let linker = Linker::new(Some("http://w"), Some("/Users/t/src/handlr")).unwrap();
        let mut entry = Entry {
            ts: "t".into(),
            role: Role::Assistant,
            kind: EntryKind::Text,
            tool: None,
            summary: format!("Wrote {PLAN}"),
            links: vec![],
        };
        linker.link_entry(&mut entry);
        assert_eq!(entry.links.len(), 1);
        assert_eq!(
            entry.links[0].url,
            "http://w/#/src--handlr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN"
        );
    }

    #[test]
    fn plan_questions_are_found_and_named() {
        assert_eq!(
            plan_question(&format!("Shall I implement `{PLAN}`?")),
            (true, Some("the overview screen plan".to_string()))
        );
        assert_eq!(plan_question("Shall I write the amendment?"), (true, None));
        assert_eq!(plan_question("Do you approve?"), (true, None));
        assert_eq!(plan_question("Is the DRAFT right?"), (true, None));
        assert_eq!(plan_question("Install it on the S23?"), (false, None));
        // "planet" and "planning" are not the word.
        assert_eq!(plan_question("Is the planet planning?"), (false, None));
        // Two plans: about plans, but no one name.
        assert_eq!(
            plan_question(&format!(
                "Merge {PLAN} into docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md?"
            )),
            (true, None)
        );
        // A docs path that is not a plan names none.
        assert_eq!(plan_question("Is docs/FONT_X.md right?"), (false, None));
    }
}
