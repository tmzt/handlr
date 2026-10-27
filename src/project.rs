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
pub fn always_refused(path: &str) -> bool {
    path.split('/').any(|name| {
        matches!(name, ".private" | ".git" | ".env" | "." | ".." | "") || name.starts_with(".env.")
    })
}

impl ProjectFiles {
    /// The project at `cwd`, which must be a folder.
    pub fn open(cwd: &str, read: Vec<ReadPattern>) -> Result<ProjectFiles, String> {
        let root = std::fs::canonicalize(cwd)
            .map_err(|e| format!("The project's folder {cwd} cannot be read: {e}"))?;
        if !root.is_dir() {
            return Err(format!("The project's folder {cwd} is not a folder."));
        }
        Ok(ProjectFiles { root, read })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the root-relative `path` may be read by its name.
    pub fn allowed(&self, path: &str) -> bool {
        !always_refused(path) && self.read.iter().any(|p| p.matches(path))
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

    /// The file at the root-relative `path`, at most [`MAX_READ_BYTES`].
    pub fn read(&self, path: &str) -> Result<FileText, String> {
        // A leading `/` is the project's root, never the filesystem's
        // (CHANGES.md 2026-10-27, plan 20261027_04).
        let given = from_root(path);
        let refused = || format!("'{path}' may not be read. {PATH_FORM}");
        if given.is_empty() || given.contains('\\') || !self.allowed(given) {
            return Err(refused());
        }
        // The real file, which a symbolic link may move: inside the root,
        // and allowed by its own name too.
        let canonical = std::fs::canonicalize(self.root.join(given))
            .map_err(|_| format!("'{path}' does not exist. {PATH_FORM}"))?;
        let real = relative(&self.root, &canonical).ok_or_else(refused)?;
        if !self.allowed(&real) {
            return Err(refused());
        }
        let meta = std::fs::metadata(&canonical).map_err(|_| refused())?;
        if !meta.is_file() {
            return Err(format!("'{path}' is not a file."));
        }
        let (text, cut) = read_utf8(&canonical, MAX_READ_BYTES)
            .map_err(|e| format!("'{path}' cannot be read: {e}"))?;
        Ok(FileText {
            text,
            cut,
            len: meta.len(),
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
