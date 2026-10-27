//! `recent_commits`: the current project's git history, read-only, with
//! the `git` binary (docs/design/20261027_05_PLANNING_FOLLOWUPS_PLAN.md,
//! decision 1, as amended at 12:37 EDT).
//!
//! `git` runs with a fixed argument list and no shell, `GIT_CONFIG_NOSYSTEM`,
//! `GIT_TERMINAL_PROMPT=0` and `LC_ALL=C`, stdin closed and a [`TIMEOUT`].
//! It returns each commit's short hash, date, author and subject, and with
//! `files` the paths it changed: never bodies or diffs.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;

/// Commits returned when no count is given.
pub const DEFAULT_COUNT: usize = 20;
/// At most this many commits.
pub const MAX_COUNT: usize = 50;
/// At most this many paths per commit with `files`.
pub const MAX_FILES: usize = 50;
/// How long one `git` run may take.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// One commit, as returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub hash: String,
    pub date: String,
    pub author: String,
    pub subject: String,
    /// With `files`: the paths it changed, at most [`MAX_FILES`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    /// With `files`: how many more paths it changed than are listed.
    #[serde(skip_serializing_if = "is_zero")]
    pub more_files: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What `recent_commits` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum History {
    /// Newest first.
    Commits(Vec<Commit>),
    /// The root is not inside a git work tree.
    NotARepository,
}

/// The first executable `git` in `path_var` (a `PATH` value).
pub fn find_on_path(path_var: Option<&OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path_var?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("git"))
        .find(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// `git` on the daemon's `PATH`, found once at first use; its absence is
/// logged once, and `recent_commits` is then not declared.
pub fn binary() -> Option<&'static Path> {
    static GIT: OnceLock<Option<PathBuf>> = OnceLock::new();
    GIT.get_or_init(|| {
        let found = find_on_path(std::env::var_os("PATH").as_deref());
        if found.is_none() {
            log::warn!("git is not on PATH: the general agent has no recent_commits");
        }
        found
    })
    .as_deref()
}

/// `count` as asked: [`DEFAULT_COUNT`] when absent, else 1 to [`MAX_COUNT`].
pub fn clamp_count(count: Option<i64>) -> usize {
    match count {
        None => DEFAULT_COUNT,
        Some(n) => n.clamp(1, MAX_COUNT as i64) as usize,
    }
}

/// The path filter, checked as a message's paths are
/// ([`crate::commands::check_paths`], against `root`): a leading `/` is the
/// project's root. Returns the pathspec passed after `--`, relative to the
/// root and starting with `./`, so it is never read as pathspec magic.
pub fn check_path(path: &str, root: &str) -> Result<String, String> {
    let path = path.trim();
    if path.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!(
            "The path '{path}' can't be used: spaces and control characters are refused."
        ));
    }
    let relative = path.trim_start_matches('/');
    if relative.starts_with('~') {
        return Err(format!(
            "The path '{path}' is outside the project; paths are relative to the project's root, \
             e.g. docs/design."
        ));
    }
    let spec = if relative.is_empty() {
        ".".to_string()
    } else {
        format!("./{relative}")
    };
    // Checked with a trailing `/`, so a final `..` is not taken for
    // sentence punctuation.
    let checked = format!("{spec}/");
    crate::commands::check_paths(&checked, Some(root)).map_err(|e| {
        e.replace("Message rejected: ", "")
            .replace(&format!("'{checked}'"), &format!("'{path}'"))
    })?;
    Ok(spec)
}

/// A `git` command for `root` with the fixed environment.
fn command(git: &Path, root: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(git);
    cmd.arg("-C")
        .arg(root)
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// Runs `cmd` within [`TIMEOUT`]: its stdout, or its stderr's first line.
async fn run(mut cmd: tokio::process::Command) -> Result<String, String> {
    let output = tokio::time::timeout(TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("git took longer than {} s", TIMEOUT.as_secs()))?
        .map_err(|e| format!("git can't run: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr
            .lines()
            .next()
            .unwrap_or("git failed")
            .trim()
            .to_string())
    }
}

/// The last `count` commits under `root`, newest first, those touching
/// `pathspec` (from [`check_path`]) when given.
pub async fn recent_commits(
    git: &Path,
    root: &Path,
    count: usize,
    pathspec: Option<&str>,
    files: bool,
) -> Result<History, String> {
    let mut check = command(git, root);
    check.args(["rev-parse", "--is-inside-work-tree"]);
    match run(check).await {
        Ok(out) if out.trim() == "true" => {}
        _ => return Ok(History::NotARepository),
    }
    let mut log = command(git, root);
    log.args([
        "log",
        "-n",
        &count.min(MAX_COUNT).to_string(),
        // Each record starts with a separator, so `--name-only`'s paths
        // fall inside it.
        "--format=%x1e%h%x1f%ad%x1f%an%x1f%s",
        "--date=short",
    ]);
    if files {
        log.arg("--name-only");
    }
    if let Some(spec) = pathspec {
        log.arg("--").arg(spec);
    }
    let out = match run(log).await {
        Ok(out) => out,
        // A repository without commits yet.
        Err(e) if e.contains("does not have any commits") => String::new(),
        Err(e) => return Err(e),
    };
    Ok(History::Commits(parse(&out, files)))
}

fn parse(out: &str, files: bool) -> Vec<Commit> {
    out.split('\u{1e}')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut lines = record.lines();
            let mut fields = lines.next()?.splitn(4, '\u{1f}');
            let mut commit = Commit {
                hash: fields.next()?.to_string(),
                date: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                subject: fields.next()?.to_string(),
                files: None,
                more_files: 0,
            };
            if files {
                let paths: Vec<String> = lines
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect();
                commit.more_files = paths.len().saturating_sub(MAX_FILES);
                commit.files = Some(paths.into_iter().take(MAX_FILES).collect());
            }
            Some(commit)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> &'static Path {
        binary().expect("git on PATH for these tests")
    }

    /// Runs git in `dir` for a test, with a fixed identity.
    fn sh(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new(git())
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .env("GIT_AUTHOR_DATE", "2026-10-27T12:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-10-27T12:00:00Z")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository with `n` commits, "commit 1" … "commit n", the odd
    /// ones touching `docs/`, the even ones `src/`.
    fn repo(n: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        sh(dir.path(), &["init", "-q", "-b", "main"]);
        for i in 1..=n {
            let file = if i % 2 == 1 {
                "docs/plan.md"
            } else {
                "src/main.rs"
            };
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("{i}")).unwrap();
            sh(dir.path(), &["add", "-A"]);
            sh(dir.path(), &["commit", "-q", "-m", &format!("commit {i}")]);
        }
        dir
    }

    fn commits(h: History) -> Vec<Commit> {
        match h {
            History::Commits(c) => c,
            History::NotARepository => panic!("not a repository"),
        }
    }

    #[tokio::test]
    async fn newest_first_with_the_count_default_and_cap() {
        let dir = repo(55);
        let all = commits(
            recent_commits(git(), dir.path(), clamp_count(None), None, false)
                .await
                .unwrap(),
        );
        assert_eq!(all.len(), DEFAULT_COUNT);
        assert_eq!(all[0].subject, "commit 55");
        assert_eq!(all[1].subject, "commit 54");
        assert_eq!(all[0].author, "Ada");
        assert_eq!(all[0].date, "2026-10-27");
        assert_eq!(all[0].hash.len(), 7, "{:?}", all[0]);
        assert!(all[0].files.is_none());
        let capped = commits(
            recent_commits(git(), dir.path(), clamp_count(Some(500)), None, false)
                .await
                .unwrap(),
        );
        assert_eq!(capped.len(), MAX_COUNT);
        assert_eq!(clamp_count(Some(0)), 1);
        assert_eq!(clamp_count(Some(-3)), 1);
        assert_eq!(clamp_count(Some(3)), 3);
    }

    #[tokio::test]
    async fn a_path_filter_and_the_files_flag() {
        let dir = repo(5);
        let root = dir.path().to_str().unwrap();
        let spec = check_path("docs", root).unwrap();
        assert_eq!(spec, "./docs");
        let docs = commits(
            recent_commits(git(), dir.path(), 20, Some(&spec), true)
                .await
                .unwrap(),
        );
        let subjects: Vec<&str> = docs.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, ["commit 5", "commit 3", "commit 1"]);
        assert_eq!(
            docs[0].files.as_deref(),
            Some(&["docs/plan.md".to_string()][..])
        );
        // A leading `/` is the project's root; a file works as well.
        let spec = check_path("/src/main.rs", root).unwrap();
        let src = commits(
            recent_commits(git(), dir.path(), 20, Some(&spec), false)
                .await
                .unwrap(),
        );
        assert_eq!(src.len(), 2);
        // Pathspec magic is not read as such.
        let spec = check_path(":(top)docs", root).unwrap();
        let none = commits(
            recent_commits(git(), dir.path(), 20, Some(&spec), false)
                .await
                .unwrap(),
        );
        assert!(none.is_empty());
    }

    #[test]
    fn a_path_outside_the_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        for path in [
            "..",
            "../other",
            "docs/../../x",
            "docs/../..",
            "/..",
            "~/src",
            "a b",
            "a\nb",
        ] {
            assert!(check_path(path, root).is_err(), "{path:?}");
        }
        let e = check_path("../other", root).unwrap_err();
        assert!(e.contains("'../other' is outside the project"), "{e}");
        assert!(!e.contains("Message rejected"), "{e}");
    }

    #[test]
    fn many_files_are_cut_at_the_limit() {
        let mut out = String::from("\u{1e}abc1234\u{1f}2026-10-27\u{1f}Ada\u{1f}many\n\n");
        for i in 0..60 {
            out.push_str(&format!("f{i}\n"));
        }
        out.push_str("\u{1e}abc1233\u{1f}2026-10-26\u{1f}Ada\u{1f}a \u{1f} subject\n");
        let c = parse(&out, true);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].files.as_ref().unwrap().len(), MAX_FILES);
        assert_eq!(c[0].more_files, 10);
        assert_eq!(c[1].subject, "a \u{1f} subject");
        assert_eq!(c[1].files.as_deref(), Some(&[][..]));
    }

    #[tokio::test]
    async fn a_folder_without_git_and_a_repository_without_commits() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            recent_commits(git(), dir.path(), 20, None, false)
                .await
                .unwrap(),
            History::NotARepository
        );
        sh(dir.path(), &["init", "-q"]);
        assert_eq!(
            recent_commits(git(), dir.path(), 20, None, false)
                .await
                .unwrap(),
            History::Commits(Vec::new())
        );
    }

    #[test]
    fn git_is_found_on_path_or_not() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(find_on_path(Some(empty.path().as_os_str())), None);
        assert_eq!(find_on_path(None), None);
        let bin = tempfile::tempdir().unwrap();
        let fake = bin.path().join("git");
        std::fs::write(&fake, "").unwrap();
        assert_eq!(
            find_on_path(Some(bin.path().as_os_str())),
            None,
            "not executable"
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([empty.path(), bin.path()]).unwrap();
        assert_eq!(find_on_path(Some(&path)), Some(fake));
    }
}
