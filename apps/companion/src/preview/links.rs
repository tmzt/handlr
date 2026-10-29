//! Paths and commit hashes in the transcript's cards, drawn as links that
//! open the preview (docs/design/20261028_05_BINARY_FILES_PLAN.md, decision
//! 10.1).
//!
//! A word of a card's shown text that the daemon's `path_words` would take
//! as a path (it holds a `/`, once quotes, brackets and trailing sentence
//! punctuation are off; never a URL), written relative to the session's
//! folder or absolute inside it, is a link only when it would be readable
//! by its name: made relative to the folder, it is not `always_refused` and
//! matches one of the daemon's `read` patterns, with libflankr_blob's own
//! matcher. A tool call's input is JSON, so a word is also cut at `"` and
//! `,`, and its `file_path` is found the same way. Whether the file exists
//! is the daemon's to say; the preview shows its refusal.
//!
//! A word of 7-40 lowercase hex digits with at least one digit and one of
//! `a-f`, bounded by anything not a letter or digit, is a commit: it opens
//! `git show <hash>`.
//!
//! The links ride the agent view's existing link spans
//! (`agent/link_spans.rs`) as [`Link`]s whose URL is in the
//! `flankr-preview:` scheme ([`Target`]), added to each entry's and card's
//! own links when the view's sections come.

use flankr_companion::{Shows, ViewSection};
use libflankr_blob::{NS_FILES, NS_GIT, ReadPattern, always_refused};
use libflankr_proto::common::Link;
use libflankr_proto::companion::BlobInfo;

/// The URL scheme of a preview link.
pub const SCHEME: &str = "flankr-preview:";

/// What a preview link opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A file through `files`, by its path relative to the session's
    /// folder.
    File(String),
    /// A `git` request (`show <hash>`, `diff`).
    Git(String),
}

impl Target {
    /// Its URL: `flankr-preview:files/<path>` or `flankr-preview:git/<request>`.
    pub fn url(&self) -> String {
        match self {
            Target::File(p) => format!("{SCHEME}{NS_FILES}/{p}"),
            Target::Git(r) => format!("{SCHEME}{NS_GIT}/{r}"),
        }
    }

    /// The target a URL names, if it is a preview link.
    pub fn parse(url: &str) -> Option<Target> {
        let rest = url.strip_prefix(SCHEME)?;
        if let Some(p) = rest.strip_prefix("files/") {
            return (!p.is_empty()).then(|| Target::File(p.to_string()));
        }
        let r = rest.strip_prefix("git/")?;
        (!r.is_empty()).then(|| Target::Git(r.to_string()))
    }

    /// The `git show` of a commit.
    pub fn show(hash: &str) -> Target {
        Target::Git(format!("show {hash}"))
    }
}

/// The words of `text` that look like paths, as the daemon's `path_words`
/// takes them, also cut at `"` and `,` (a tool call's JSON).
pub fn path_words(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == '"' || c == ',')
        .map(|w| {
            w.trim_matches(|c: char| "\"'`()[]{}<>".contains(c))
                .trim_end_matches(['.', ',', ';', ':', '?', '!'])
        })
        .filter(|w| (w.contains('/') || w.starts_with('~')) && !w.contains("://"))
        .collect()
}

/// Whether `word` is a commit hash of decision 10.1.
pub fn is_commit_hash(word: &str) -> bool {
    (7..=40).contains(&word.len())
        && word.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && word.bytes().any(|b| b.is_ascii_digit())
        && word.bytes().any(|b| b.is_ascii_lowercase())
}

/// The commit hashes in `text`: its runs of ASCII letters and digits that
/// are hashes.
pub fn hash_words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| is_commit_hash(w))
        .collect()
}

/// What makes links for one session: its folder and the daemon's `read`
/// patterns, and which of `files` and `git` it serves.
#[derive(Clone, Debug, Default)]
pub struct Linker {
    root: Option<String>,
    patterns: Vec<ReadPattern>,
    files: bool,
    git: bool,
}

impl Linker {
    /// For a session in `cwd`, on a daemon offering `blob`. Without
    /// `files` (or a folder), no path is a link; without `git`, no hash.
    pub fn new(blob: Option<&BlobInfo>, cwd: Option<&str>) -> Linker {
        let Some(blob) = blob else {
            return Linker::default();
        };
        let patterns = blob
            .read
            .iter()
            .filter_map(|p| ReadPattern::parse(p).ok())
            .collect::<Vec<_>>();
        let root = cwd
            .filter(|c| c.starts_with('/'))
            .map(|c| c.trim_end_matches('/').to_string());
        Linker {
            files: blob.serves(NS_FILES) && !patterns.is_empty() && root.is_some(),
            git: blob.serves(NS_GIT),
            root,
            patterns,
        }
    }

    /// Whether it makes any link.
    pub fn is_active(&self) -> bool {
        self.files || self.git
    }

    /// Whether `git` is served: Show uncommitted changes, and hashes.
    pub fn git(&self) -> bool {
        self.git
    }

    /// `word` relative to the session's folder, when it would be readable
    /// by its name.
    pub fn readable(&self, word: &str) -> Option<String> {
        if !self.files {
            return None;
        }
        let root = self.root.as_deref()?;
        let rel = if word.starts_with('/') {
            let rest = word.strip_prefix(root)?;
            if root.is_empty() {
                rest
            } else {
                rest.strip_prefix('/')?
            }
        } else if word.starts_with('~') {
            return None;
        } else {
            word.strip_prefix("./").unwrap_or(word)
        };
        if rel.is_empty() || always_refused(rel) {
            return None;
        }
        self.patterns
            .iter()
            .any(|p| p.matches(rel))
            .then(|| rel.to_string())
    }

    /// The preview links in `text`, as the agent view's links: the text as
    /// shown, and the [`Target`]'s URL. Each once.
    pub fn links(&self, text: &str) -> Vec<Link> {
        let mut out: Vec<Link> = Vec::new();
        let mut add = |text: &str, target: Target| {
            let link = Link {
                text: text.to_string(),
                url: target.url(),
            };
            if !out.contains(&link) {
                out.push(link);
            }
        };
        for w in path_words(text) {
            if let Some(rel) = self.readable(w) {
                add(w, Target::File(rel));
            }
        }
        if self.git {
            for h in hash_words(text) {
                add(h, Target::show(h));
            }
        }
        out
    }

    /// Adds the preview links of each entry's and card's text to its own
    /// links.
    pub fn augment(&self, sections: &mut [ViewSection]) {
        if !self.is_active() {
            return;
        }
        for section in sections {
            match &mut section.shows {
                Shows::Entries(entries) => {
                    for e in entries {
                        let found = self.links(e.text.as_deref().unwrap_or(""));
                        merge(&mut e.links, found);
                    }
                }
                Shows::Card(card) => {
                    let mut found = self.links(&card.title);
                    found.extend(self.links(&card.text));
                    merge(&mut card.links, found);
                }
                _ => {}
            }
        }
    }
}

/// `found` appended to `links`, but those whose text a link there already
/// has (the daemon's own repowiki link for a `docs/` path stays).
fn merge(links: &mut Vec<Link>, found: Vec<Link>) {
    for l in found {
        if !links.iter().any(|x| x.text == l.text) {
            links.push(l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(read: &[&str], namespaces: &[&str]) -> BlobInfo {
        BlobInfo {
            version: 1,
            namespaces: namespaces.iter().map(|s| s.to_string()).collect(),
            read: read.iter().map(|s| s.to_string()).collect(),
            image_max: 20 << 20,
            file_max: 8 << 20,
            git_max: 8 << 20,
            chunk_max: 983_040,
            images_per_send: 4,
        }
    }

    fn linker() -> Linker {
        Linker::new(
            Some(&blob(&["docs/**/*.md"], &["files", "git", "images"])),
            Some("/home/u/p"),
        )
    }

    fn texts(links: &[Link]) -> Vec<&str> {
        links.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn readable_paths_are_links_and_others_are_not() {
        let l = linker();
        let found = l.links(
            "See docs/design/x.md and /home/u/p/docs/design/x.md, not src/main.rs, \
             .env, ../x.md, /etc/docs/x.md, ~/docs/x.md or a bare x.md.",
        );
        assert_eq!(
            texts(&found),
            ["docs/design/x.md", "/home/u/p/docs/design/x.md"]
        );
        assert_eq!(
            Target::parse(&found[1].url),
            Some(Target::File("docs/design/x.md".into()))
        );
        assert_eq!(l.readable("docs/.private/a.md"), None);
        assert_eq!(l.readable("docs/.env.md"), None);
        assert_eq!(l.readable("./docs/a.md").as_deref(), Some("docs/a.md"));
        // Another folder that starts the same is not inside.
        assert_eq!(l.readable("/home/u/project/docs/a.md"), None);
    }

    #[test]
    fn a_tool_calls_file_path_is_a_link() {
        let l = linker();
        let found = l.links("Read {\"file_path\":\"/home/u/p/docs/a.md\",\"limit\":20}");
        assert_eq!(texts(&found), ["/home/u/p/docs/a.md"]);
    }

    #[test]
    fn hashes_are_links_only_with_a_digit_and_a_letter() {
        let l = linker();
        let found = l.links("Landed 1a2b3c4 and 2a6fb87e, not 1234567, deadbeefcafe or abc12.");
        assert_eq!(texts(&found), ["1a2b3c4", "2a6fb87e"]);
        assert_eq!(
            Target::parse(&found[0].url),
            Some(Target::Git("show 1a2b3c4".into()))
        );
        assert!(!is_commit_hash("1A2B3C4"));
        assert!(!is_commit_hash(&"a1".repeat(21)));
        assert!(is_commit_hash(&"a1".repeat(20)));
    }

    #[test]
    fn nothing_is_a_link_without_the_namespace() {
        let off = Linker::new(Some(&blob(&["docs/**/*.md"], &["images"])), Some("/p"));
        assert!(off.links("docs/a.md 1a2b3c4").is_empty());
        let no_read = Linker::new(Some(&blob(&[], &["files", "git"])), Some("/p"));
        assert_eq!(texts(&no_read.links("docs/a.md 1a2b3c4")), ["1a2b3c4"]);
        assert!(!Linker::new(None, Some("/p")).is_active());
        // No folder known: no path.
        let rootless = Linker::new(Some(&blob(&["docs/**/*.md"], &["files"])), None);
        assert!(rootless.links("docs/a.md").is_empty());
    }

    #[test]
    fn urls_round_trip() {
        for t in [
            Target::File("docs/a b.md".into()),
            Target::Git("diff".into()),
        ] {
            assert_eq!(Target::parse(&t.url()), Some(t));
        }
        assert_eq!(Target::parse("https://x"), None);
        assert_eq!(Target::parse("flankr-preview:files/"), None);
    }
}
