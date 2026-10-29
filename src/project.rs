//! The current project's files, read-only, for the general agent's
//! `list_files` and `read_file` and its `agents_prompt`
//! (docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md, decisions 4
//! and 5, as amended at 11:20 and 11:22 EDT).
//!
//! A path may be read when it matches one of `read`'s patterns and none
//! of the refusals that hold whatever the patterns say: `.private`,
//! `.git`, `.env` and `.env.*`, anything whose canonical path is outside
//! the root, and anything that is not a regular UTF-8 file. Contents are
//! never logged.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::llm::ReadPattern;

/// At most this many paths in one `list_files` answer.
pub const MAX_LISTED: usize = 500;
/// At most this much of one file in a `read_file` answer.
pub const MAX_READ_BYTES: usize = 64 * 1024;
/// Folders `list_files` does not walk into.
const SKIPPED_DIRS: [&str; 3] = [".git", "target", "node_modules"];
/// The agent instruction files, in order of preference (decision 5).
pub const INSTRUCTION_FILES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

/// One project: its root, canonical, and what may be read in it.
#[derive(Debug, Clone)]
pub struct ProjectFiles {
    root: PathBuf,
    read: Vec<ReadPattern>,
    /// A second set a path must also match, when set: a plugin's own
    /// `read_patterns()` (docs/design/20261028_04_PLUGINS_PLAN.md, decision
    /// 3.3, item 3). An empty set allows nothing.
    also: Option<Vec<ReadPattern>>,
}

/// What a file or git operation says when there is no current agent.
pub const NO_PROJECT: &str = "No project is open: open a session, or focus an agent first";

/// An agent's pane and its folder, as herdr's snapshot gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFolder {
    /// Canonical, as the snapshot has it (`w1:p6`).
    pub pane_id: String,
    /// Lowercased; `shell` when the pane runs no known agent.
    pub agent: String,
    /// The pane's `cwd`, as herdr reports it.
    pub folder: String,
}

/// Refused when herdr is reached over SSH: the project's files are on
/// another machine.
pub fn check_local(client: &crate::transport::HerdrClient) -> Result<(), String> {
    if matches!(
        client.transport(),
        crate::transport::HerdrTransport::RemoteSsh { .. }
    ) {
        return Err(
            "The project's files are on another machine; flankr reads only its own.".to_string(),
        );
    }
    Ok(())
}

/// The folder of the agent in `pane_id`, on this machine: refused over
/// SSH, else the pane's `cwd` from herdr's snapshot (plan 20261028_04,
/// decision 3.3, item 2; moved here from the general agent's executor so
/// both use one function).
pub async fn agent_folder(
    client: &crate::transport::HerdrClient,
    pane_id: &str,
) -> Result<String, String> {
    agent_folder_named(client, pane_id).await.map(|a| a.folder)
}

/// As [`agent_folder`], with the pane's agent.
pub async fn agent_folder_named(
    client: &crate::transport::HerdrClient,
    pane_id: &str,
) -> Result<AgentFolder, String> {
    check_local(client)?;
    let snapshot = client
        .list_panes()
        .await
        .map_err(|e| format!("herdr is not answering: {e}"))?;
    folder_in(&snapshot, pane_id)
}

/// The agent in `pane_id` and its folder in `snapshot`.
pub fn folder_in(snapshot: &serde_json::Value, pane_id: &str) -> Result<AgentFolder, String> {
    crate::core::agents::agents(snapshot)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, pane_id))
        .and_then(|a| {
            Some(AgentFolder {
                folder: a.cwd?,
                pane_id: a.pane_id,
                agent: a.agent,
            })
        })
        .ok_or_else(|| format!("The folder of the current agent (pane {pane_id}) is not known."))
}

/// A `list_files` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Root-relative, `/`-separated, sorted.
    pub paths: Vec<String>,
    /// How many more matched than are listed.
    pub more: usize,
}

/// A `read_file` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    pub text: String,
    /// The file was longer than [`MAX_READ_BYTES`] and is cut.
    pub cut: bool,
    /// The file's length in bytes.
    pub len: u64,
}

/// Whether a root-relative path is refused whatever the patterns say:
/// any segment `.private`, `.git`, `.env` or `.env.*`, or `.` and `..`.
/// Moved to libflankr_blob, so the app matches paths with the same code
/// (docs/design/20261028_05_BINARY_FILES_PLAN.md, decision 4.9).
pub use libflankr_blob::always_refused;

/// A `files` transfer's file (plan 20261028_05, decision 6.2): its bytes,
/// whole, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBytes {
    pub bytes: Vec<u8>,
    /// `text/markdown`, `text/x-diff`, `text/plain` or an image type.
    pub mime: &'static str,
    /// The root-relative path, as resolved.
    pub name: String,
    /// The file's mtime in Unix ms, 0 when unknown.
    pub modified_ms: u64,
}

/// Why [`ProjectFiles::read_bytes`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRefusal {
    /// The path's rules refuse it (`not_permitted`).
    NotPermitted,
    /// No such file (`not_found`).
    NotFound,
    /// Over the size asked (`too_large`).
    TooLarge,
    /// Not a previewable type (`bad_type`).
    BadType,
    /// It could not be read (`io`).
    Io,
}

impl ProjectFiles {
    /// The project at `cwd`, which must be a folder.
    pub fn open(cwd: &str, read: Vec<ReadPattern>) -> Result<ProjectFiles, String> {
        let root = std::fs::canonicalize(cwd)
            .map_err(|e| format!("The project's folder {cwd} cannot be read: {e}"))?;
        if !root.is_dir() {
            return Err(format!("The project's folder {cwd} is not a folder."));
        }
        Ok(ProjectFiles {
            root,
            read,
            also: None,
        })
    }

    /// The same project, where a path must also match one of `also`
    /// (a plugin's own patterns); an empty `also` allows nothing.
    pub fn narrowed(mut self, also: Vec<ReadPattern>) -> ProjectFiles {
        self.also = Some(also);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the root-relative `path` may be read by its name.
    pub fn allowed(&self, path: &str) -> bool {
        !always_refused(path)
            && self.read.iter().any(|p| p.matches(path))
            && self
                .also
                .as_ref()
                .is_none_or(|also| also.iter().any(|p| p.matches(path)))
    }

    /// The readable regular files, narrowed by `glob`, sorted, at most
    /// [`MAX_LISTED`]. Symbolic links are neither followed nor listed.
    pub fn list(&self, glob: Option<&str>) -> Result<Listing, String> {
        let glob = glob
            .map(from_root)
            .filter(|g| !g.is_empty())
            .map(ReadPattern::parse)
            .transpose()?;
        let walk = walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !(e.depth() > 0
                    && e.file_type().is_dir()
                    && e.file_name()
                        .to_str()
                        .is_some_and(|n| SKIPPED_DIRS.contains(&n)))
            });
        let mut paths = Vec::new();
        for entry in walk.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(rel) = relative(&self.root, entry.path()) else {
                continue;
            };
            if self.allowed(&rel) && glob.as_ref().is_none_or(|g| g.matches(&rel)) {
                paths.push(rel);
            }
        }
        paths.sort();
        let more = paths.len().saturating_sub(MAX_LISTED);
        paths.truncate(MAX_LISTED);
        Ok(Listing { paths, more })
    }

    /// The path rules `read` and `read_bytes` share (plan 20261028_05,
    /// decision 6.2): the root-relative `path` allowed by its name, its
    /// canonical path inside the root and allowed by its own name too, and
    /// a regular file. Gives the canonical path, the resolved relative
    /// path and the metadata.
    fn resolve(&self, path: &str) -> Result<(PathBuf, String, std::fs::Metadata), Resolve> {
        // A leading `/` is the project's root, never the filesystem's
        // (CHANGES.md 2026-10-27, plan 20261027_04).
        let given = from_root(path);
        if given.is_empty() || given.contains('\\') || !self.allowed(given) {
            return Err(Resolve::Refused);
        }
        // The real file, which a symbolic link may move: inside the root,
        // and allowed by its own name too.
        let canonical =
            std::fs::canonicalize(self.root.join(given)).map_err(|_| Resolve::Missing)?;
        let real = relative(&self.root, &canonical).ok_or(Resolve::Refused)?;
        if !self.allowed(&real) {
            return Err(Resolve::Refused);
        }
        let meta = std::fs::metadata(&canonical).map_err(|_| Resolve::Refused)?;
        if !meta.is_file() {
            return Err(Resolve::NotAFile);
        }
        Ok((canonical, real, meta))
    }

    /// The file at the root-relative `path`, at most [`MAX_READ_BYTES`].
    pub fn read(&self, path: &str) -> Result<FileText, String> {
        let refused = || format!("'{path}' may not be read. {PATH_FORM}");
        let (canonical, _, meta) = self.resolve(path).map_err(|e| match e {
            Resolve::Refused => refused(),
            Resolve::Missing => format!("'{path}' does not exist. {PATH_FORM}"),
            Resolve::NotAFile => format!("'{path}' is not a file."),
        })?;
        let (text, cut) = read_utf8(&canonical, MAX_READ_BYTES)
            .map_err(|e| format!("'{path}' cannot be read: {e}"))?;
        Ok(FileText {
            text,
            cut,
            len: meta.len(),
        })
    }

    /// The file at the root-relative `path`, whole, under exactly
    /// [`read`](Self::read)'s path rules, for a `files` transfer (plan
    /// 20261028_05, decision 6.2): served only as UTF-8 text or a PNG,
    /// JPEG, WebP or GIF image, and not cut: over `max` bytes is
    /// [`ReadRefusal::TooLarge`].
    pub fn read_bytes(&self, path: &str, max: u64) -> Result<FileBytes, ReadRefusal> {
        let (canonical, name, meta) = self.resolve(path).map_err(|e| match e {
            Resolve::Missing => ReadRefusal::NotFound,
            Resolve::Refused | Resolve::NotAFile => ReadRefusal::NotPermitted,
        })?;
        if meta.len() > max {
            return Err(ReadRefusal::TooLarge);
        }
        let file = std::fs::File::open(&canonical).map_err(|_| ReadRefusal::Io)?;
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ReadRefusal::Io)?;
        if bytes.len() as u64 > max {
            return Err(ReadRefusal::TooLarge);
        }
        let mime = match libflankr_blob::ImageType::sniff(&bytes) {
            Some(image) => image.mime(),
            None if std::str::from_utf8(&bytes).is_ok() => text_mime(&name),
            None => return Err(ReadRefusal::BadType),
        };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        Ok(FileBytes {
            bytes,
            mime,
            name,
            modified_ms,
        })
    }

    /// The first of [`INSTRUCTION_FILES`] in the root, cut at `max_bytes`
    /// (decision 5): its name, its text, and whether it was cut. `@`
    /// imports in it are left as written. Read whatever `read` says.
    pub fn instructions(&self, max_bytes: usize) -> Option<(String, String, bool)> {
        for name in INSTRUCTION_FILES {
            let Ok(canonical) = std::fs::canonicalize(self.root.join(name)) else {
                continue;
            };
            // A link to AGENTS.md (CLAUDE.md often is one) is fine; one out
            // of the project is not.
            if relative(&self.root, &canonical).is_none_or(|real| always_refused(&real)) {
                continue;
            }
            if !std::fs::metadata(&canonical).is_ok_and(|m| m.is_file()) {
                continue;
            }
            match read_utf8(&canonical, max_bytes) {
                Ok((text, cut)) => return Some((name.to_string(), text, cut)),
                Err(e) => log::warn!("{} in {}: {e}", name, self.root.display()),
            }
        }
        None
    }
}

/// Why [`ProjectFiles::resolve`] refused a path.
enum Resolve {
    Refused,
    Missing,
    NotAFile,
}

/// The MIME type of a UTF-8 file by its name (plan 20261028_05, decision
/// 6.2).
fn text_mime(name: &str) -> &'static str {
    let ext = name
        .rsplit('/')
        .next()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("md" | "markdown") => "text/markdown",
        Some("diff" | "patch") => "text/x-diff",
        _ => "text/plain",
    }
}

/// `path` relative to `root`, `/`-separated; `None` outside it or not
/// UTF-8.
/// How a refused or missing path should have been written, for Gemini to
/// try again.
const PATH_FORM: &str = "Paths are relative to the project's root, e.g. docs/design/plan.md.";

/// `path` relative to the project's root: trimmed, with a leading `/` or
/// `./` taken as the root.
fn from_root(path: &str) -> &str {
    let path = path.trim();
    let path = path.trim_start_matches('/');
    path.strip_prefix("./").unwrap_or(path)
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let names: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    let rel = names?.join("/");
    (!rel.is_empty()).then_some(rel)
}

/// At most `max` bytes of `path` as UTF-8, and whether it was cut; a
/// character split by the cut is left out.
fn read_utf8(path: &Path, max: usize) -> Result<(String, bool), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let cut = bytes.len() > max;
    bytes.truncate(max);
    match String::from_utf8(bytes) {
        Ok(text) => Ok((text, cut)),
        Err(e) => {
            let error = e.utf8_error();
            if cut && error.error_len().is_none() {
                let mut bytes = e.into_bytes();
                bytes.truncate(error.valid_up_to());
                Ok((String::from_utf8(bytes).expect("valid up to here"), cut))
            } else {
                Err("it is not UTF-8 text".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(list: &[&str]) -> Vec<ReadPattern> {
        list.iter()
            .map(|p| ReadPattern::parse(p).unwrap())
            .collect()
    }

    fn write(root: &Path, rel: &str, text: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A project with the plan's examples, and a file outside it.
    fn sample() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        for rel in [
            "README.md",
            "docs/design/x.md",
            "docs/x.txt",
            "src/README.md",
            "src/main.rs",
            ".env",
            ".env.local",
            ".private/key.md",
            ".git/config.md",
            "target/debug/x.md",
            "docs/node_modules/y.md",
        ] {
            write(&root, rel, b"text");
        }
        write(dir.path(), "x.md", b"outside");
        std::os::unix::fs::symlink(dir.path().join("x.md"), root.join("docs/out.md")).unwrap();
        std::os::unix::fs::symlink(root.join(".env"), root.join("docs/env.md")).unwrap();
        (dir, root)
    }

    #[test]
    fn the_patterns_decide_and_the_refusals_hold_whatever_they_say() {
        let (_dir, root) = sample();
        let files = ProjectFiles::open(root.to_str().unwrap(), patterns(&["*.md", "docs/**/*.md"]))
            .unwrap();
        assert_eq!(files.read("README.md").unwrap().text, "text");
        assert_eq!(files.read("./docs/design/x.md").unwrap().text, "text");
        // A leading `/` is the project's root; the filesystem's absolute
        // path to the same file is not accepted, and the error says how.
        assert_eq!(files.read("/docs/design/x.md").unwrap().text, "text");
        let absolute = format!("{}/docs/design/x.md", root.display());
        let err = files.read(&absolute).unwrap_err();
        assert!(err.contains("relative to the project's root"), "{err}");
        assert_eq!(
            files.list(Some("/docs/**/*.md")).unwrap().paths,
            ["docs/design/x.md"]
        );
        for path in [
            "docs/x.txt",
            "src/README.md",
            "src/main.rs",
            ".env",
            ".private/key.md",
            "../x.md",
            "docs/../../x.md",
            "/etc/passwd",
            "docs/out.md",
            "docs/env.md",
        ] {
            let err = files.read(path).unwrap_err();
            assert!(err.contains("may not be read"), "{path}: {err}");
        }
        // Every file, by the patterns: still not the refused ones.
        let everything =
            ProjectFiles::open(root.to_str().unwrap(), patterns(&["**", "**/*"])).unwrap();
        for path in [".env", ".env.local", ".private/key.md", ".git/config.md"] {
            assert!(everything.read(path).is_err(), "{path}");
        }
        let listed = everything.list(None).unwrap().paths;
        assert_eq!(
            listed,
            [
                "README.md",
                "docs/design/x.md",
                "docs/x.txt",
                "src/README.md",
                "src/main.rs"
            ],
            "no links, no skipped folders, nothing refused"
        );

        let listing = files.list(None).unwrap();
        assert_eq!(listing.paths, ["README.md", "docs/design/x.md"]);
        assert_eq!(listing.more, 0);
        assert_eq!(
            files.list(Some("docs/**")).unwrap().paths,
            ["docs/design/x.md"]
        );
        assert!(files.list(Some("docs/[a]")).is_err());
    }

    // Plan 20261028_05, step 4: `files` keeps every path rule of
    // `read_file`, and serves bytes.
    #[test]
    fn read_bytes_keeps_the_path_rules_and_serves_text_and_images() {
        let (_dir, root) = sample();
        write(
            &root,
            "docs/a.png",
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0],
        );
        write(&root, "docs/bin.md", &[0xff, 0xfe, 0x00, 1, 2]);
        write(&root, "docs/c.diff", b"+a\n");
        let files = ProjectFiles::open(
            root.to_str().unwrap(),
            patterns(&["*.md", "docs/**/*.md", "docs/*.png", "docs/*.diff"]),
        )
        .unwrap();
        let readme = files.read_bytes("README.md", 1024).unwrap();
        assert_eq!(readme.bytes, b"text");
        assert_eq!(readme.mime, "text/markdown");
        assert_eq!(readme.name, "README.md");
        assert!(readme.modified_ms > 0);
        let doc = files.read_bytes("/docs/design/x.md", 1024).unwrap();
        assert_eq!(doc.name, "docs/design/x.md");
        assert_eq!(
            files.read_bytes("docs/a.png", 1024).unwrap().mime,
            "image/png"
        );
        assert_eq!(
            files.read_bytes("docs/c.diff", 1024).unwrap().mime,
            "text/x-diff"
        );
        assert_eq!(
            files.read_bytes("docs/bin.md", 1024),
            Err(ReadRefusal::BadType)
        );
        assert_eq!(files.read_bytes("README.md", 3), Err(ReadRefusal::TooLarge));
        assert_eq!(
            files.read_bytes("docs/none.md", 1024),
            Err(ReadRefusal::NotFound)
        );
        for path in [
            "src/main.rs",
            ".env",
            ".private/key.md",
            "../x.md",
            "docs/out.md",
            "docs/env.md",
            "docs",
        ] {
            assert_eq!(
                files.read_bytes(path, 1024),
                Err(ReadRefusal::NotPermitted),
                "{path}"
            );
        }
    }

    #[test]
    fn the_limits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..(MAX_LISTED + 3) {
            write(root, &format!("docs/{i:04}.md"), b"x");
        }
        // Long, with a multi-byte character across the cut.
        let mut long = vec![b'a'; MAX_READ_BYTES - 1];
        long.extend("é and more".as_bytes());
        write(root, "long.md", &long);
        write(root, "binary.md", &[0xff, 0xfe, 0x00]);
        let files = ProjectFiles::open(root.to_str().unwrap(), patterns(&["**/*.md"])).unwrap();
        let listing = files.list(None).unwrap();
        assert_eq!(listing.paths.len(), MAX_LISTED);
        assert_eq!(listing.more, 5, "503 notes, long.md and binary.md");
        assert!(listing.paths.windows(2).all(|w| w[0] < w[1]), "sorted");
        let read = files.read("long.md").unwrap();
        assert!(read.cut);
        assert_eq!(read.text.len(), MAX_READ_BYTES - 1);
        assert_eq!(read.len, long.len() as u64);
        let err = files.read("binary.md").unwrap_err();
        assert!(err.contains("not UTF-8"), "{err}");
        assert!(files.read("docs").is_err());
    }

    #[test]
    fn a_narrowed_project_needs_both_sets() {
        let (_dir, root) = sample();
        let files = ProjectFiles::open(root.to_str().unwrap(), patterns(&["*.md", "docs/**/*.md"]))
            .unwrap()
            .narrowed(patterns(&["README.md", "docs/x.txt"]));
        assert!(files.read("README.md").is_ok());
        // The configured patterns match, the narrowing ones don't.
        assert!(files.read("docs/design/x.md").is_err());
        // The narrowing ones match, the configured ones don't.
        assert!(files.read("docs/x.txt").is_err());
        assert_eq!(files.list(None).unwrap().paths, ["README.md"]);
        let none = ProjectFiles::open(root.to_str().unwrap(), patterns(&["**/*"]))
            .unwrap()
            .narrowed(vec![]);
        assert!(none.read("README.md").is_err());
        assert!(none.list(None).unwrap().paths.is_empty());
    }

    #[test]
    fn an_agents_folder_comes_from_the_snapshot() {
        let snap = serde_json::json!({
            "panes": [
                {"pane_id": "w1:p1", "workspace_id": "w1", "agent": "claude", "cwd": "/src/a"},
                {"pane_id": "w1:p2", "workspace_id": "w1"}
            ]
        });
        assert_eq!(
            folder_in(&snap, "w1:p1").unwrap(),
            AgentFolder {
                pane_id: "w1:p1".into(),
                agent: "claude".into(),
                folder: "/src/a".into(),
            }
        );
        let err = folder_in(&snap, "w1:p2").unwrap_err();
        assert!(err.contains("is not known"), "{err}");
        assert!(folder_in(&snap, "w1:p9").is_err());
    }

    #[test]
    fn agents_md_is_preferred_and_cut() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let files = ProjectFiles::open(root.to_str().unwrap(), vec![]).unwrap();
        assert_eq!(files.instructions(1024), None);
        write(root, "GEMINI.md", b"gemini");
        write(root, "CLAUDE.md", b"claude @AGENTS.md");
        assert_eq!(
            files.instructions(1024),
            Some(("CLAUDE.md".into(), "claude @AGENTS.md".into(), false))
        );
        write(root, "AGENTS.md", b"agents rules");
        assert_eq!(
            files.instructions(6),
            Some(("AGENTS.md".into(), "agents".into(), true))
        );
    }
}
