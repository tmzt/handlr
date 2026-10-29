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

/// What a capped run gave (plan 20261028_05, decisions 6.7 and 6.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capped {
    /// stdout, at most `max` bytes; when cut, ending in the marker.
    pub stdout: Vec<u8>,
    /// stdout reached `max` less the marker: the process was killed and
    /// the marker appended.
    pub cut: bool,
    /// The exit code (`None` killed by a signal, or by the cut).
    pub code: Option<i32>,
    /// stderr's first line, at most 200 characters.
    pub stderr_line: String,
}

/// Why a capped run failed to give an output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CappedError {
    /// Longer than [`TIMEOUT`]; the process was killed.
    Timeout,
    /// It could not start or be read.
    Io(String),
}

/// The variables a capped run's child gets, its environment cleared
/// first (decision 6.7): `PATH` and `HOME` (and `TZ`) as the daemon has
/// them, and git's fixed ones.
pub fn capped_env() -> Vec<(String, String)> {
    let mut env = Vec::new();
    for key in ["PATH", "HOME", "TZ"] {
        if let Ok(v) = std::env::var(key) {
            env.push((key.to_string(), v));
        }
    }
    for (k, v) in [
        ("LC_ALL", "C"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_LITERAL_PATHSPECS", "1"),
        ("GIT_OPTIONAL_LOCKS", "0"),
    ] {
        env.push((k.to_string(), v.to_string()));
    }
    env
}

/// The argv after `git -C <root>` for a `git` namespace request (decision
/// 6.5 and 6.7): the common prefix, then `show` or `diff` with
/// `--no-color --no-ext-diff --no-textconv`, `--end-of-options` before the
/// revs, and `-- <pathspec>` (from [`check_path`]) when given.
pub fn show_diff_args(request: &libflankr_blob::GitRequest, pathspec: Option<&str>) -> Vec<String> {
    use libflankr_blob::GitRequest;
    let mut args: Vec<String> = [
        "--no-pager",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "log.showSignature=false",
        "-c",
        "diff.external=",
        "-c",
        "core.pager=cat",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let flags = ["--no-color", "--no-ext-diff", "--no-textconv"];
    match request {
        GitRequest::Show { rev, .. } => {
            args.push("show".into());
            args.extend(flags.iter().map(|s| s.to_string()));
            args.push("--end-of-options".into());
            args.push(rev.to_string());
        }
        GitRequest::DiffRange { from, to, .. } => {
            args.push("diff".into());
            args.extend(flags.iter().map(|s| s.to_string()));
            args.push("--end-of-options".into());
            args.push(format!("{from}..{to}"));
        }
        GitRequest::DiffWorkingTree { .. } => {
            args.push("diff".into());
            args.extend(flags.iter().map(|s| s.to_string()));
            args.push("HEAD".into());
        }
    }
    if let Some(spec) = pathspec {
        args.push("--".into());
        args.push(spec.to_string());
    }
    args
}

/// Runs `git -C <root> <args>` with the environment cleared to
/// [`capped_env`], no shell, stdin closed, killed on drop and after
/// [`TIMEOUT`]; stdout read in pieces up to `max` bytes, the marker
/// included: at `max` less the marker the process is killed and the marker
/// appended (decision 6.8). stderr is read up to 4 KiB, only its first
/// line kept.
pub async fn run_capped(
    git: &Path,
    root: &Path,
    args: &[String],
    max: usize,
    marker: &str,
) -> Result<Capped, CappedError> {
    run_capped_with(git, root, args, max, marker, TIMEOUT).await
}

/// As [`run_capped`], with its timeout (tests).
pub async fn run_capped_with(
    git: &Path,
    root: &Path,
    args: &[String],
    max: usize,
    marker: &str,
    timeout: Duration,
) -> Result<Capped, CappedError> {
    use tokio::io::AsyncReadExt;
    let mut cmd = tokio::process::Command::new(git);
    cmd.env_clear()
        .envs(capped_env())
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| CappedError::Io(e.to_string()))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let limit = max.saturating_sub(marker.len());
    // stderr is read to its end, so the child never blocks on a full
    // pipe, keeping the first 4 KiB.
    let err_task = tokio::spawn(async move {
        let mut kept = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let room = 4096usize.saturating_sub(kept.len());
            kept.extend_from_slice(&buf[..n.min(room)]);
        }
        kept
    });
    let run = async {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut cut = false;
        loop {
            let n = stdout
                .read(&mut buf)
                .await
                .map_err(|e| CappedError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            let room = limit - out.len();
            if n >= room {
                out.extend_from_slice(&buf[..room]);
                cut = true;
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        if cut {
            let _ = child.start_kill();
        }
        let status = child
            .wait()
            .await
            .map_err(|e| CappedError::Io(e.to_string()))?;
        Ok::<_, CappedError>((out, cut, status.code()))
    };
    let (mut out, cut, code) = match tokio::time::timeout(timeout, run).await {
        Ok(r) => r?,
        Err(_) => {
            // `child` is dropped with the future: kill_on_drop kills it.
            err_task.abort();
            return Err(CappedError::Timeout);
        }
    };
    let err = tokio::time::timeout(Duration::from_secs(1), err_task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    if cut {
        out.extend_from_slice(marker.as_bytes());
    }
    let stderr_line: String = String::from_utf8_lossy(&err)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect();
    Ok(Capped {
        stdout: out,
        cut,
        code: if cut { None } else { code },
        stderr_line,
    })
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

    // Plan 20261028_05, step 5: the capped runner, with stand-ins for git.

    /// An executable script `body` in a new folder.
    fn stand_in(body: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("git");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn the_childs_environment_holds_only_the_fixed_variables() {
        let (dir, git) = stand_in("env");
        let out = run_capped(&git, dir.path(), &[], 1 << 20, "[cut]")
            .await
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let mut keys: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, _)| k))
            // The shell's own.
            .filter(|k| !["PWD", "SHLVL", "_", "OLDPWD"].contains(k))
            .collect();
        keys.sort();
        let mut want = vec![
            "GIT_CONFIG_NOSYSTEM",
            "GIT_LITERAL_PATHSPECS",
            "GIT_OPTIONAL_LOCKS",
            "GIT_TERMINAL_PROMPT",
            "LC_ALL",
        ];
        for k in ["HOME", "PATH", "TZ"] {
            if std::env::var(k).is_ok() {
                want.push(k);
            }
        }
        want.sort();
        assert_eq!(keys, want, "{text}");
        assert!(text.contains("GIT_OPTIONAL_LOCKS=0"));
        assert!(!out.cut);
        assert_eq!(out.code, Some(0));
    }

    #[tokio::test]
    async fn a_long_output_is_cut_with_the_marker_and_a_slow_one_times_out() {
        // 9 MiB of output, cut at 8 MiB with the marker.
        let (dir, git) = stand_in("head -c 9437184 /dev/zero");
        let marker = "\n[flankr: cut at 8 MiB]\n";
        let out = run_capped(&git, dir.path(), &[], 8 << 20, marker)
            .await
            .unwrap();
        assert!(out.cut);
        assert_eq!(out.stdout.len(), 8 << 20);
        assert!(out.stdout.ends_with(marker.as_bytes()));
        // A stand-in that sleeps past the timeout is killed.
        let (dir, git) = stand_in("exec sleep 6");
        let started = std::time::Instant::now();
        let err = run_capped_with(
            &git,
            dir.path(),
            &[],
            1024,
            marker,
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert_eq!(err, CappedError::Timeout);
        assert!(started.elapsed() < Duration::from_secs(3));
        // A failure's first stderr line, at most 200 characters.
        let (dir, git) = stand_in(&format!(
            "echo '{}' >&2; echo second >&2; exit 3",
            "x".repeat(300)
        ));
        let out = run_capped(&git, dir.path(), &[], 1024, marker)
            .await
            .unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stderr_line, "x".repeat(200));
    }

    #[test]
    fn the_argv_is_fixed_and_options_stop_before_the_revs() {
        let r = libflankr_blob::GitRequest::parse("show HEAD~1 -- a.txt").unwrap();
        let args = show_diff_args(&r, Some("./a.txt"));
        let tail: Vec<&str> = args.iter().skip(9).map(String::as_str).collect();
        assert_eq!(
            tail,
            [
                "show",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--end-of-options",
                "HEAD~1",
                "--",
                "./a.txt"
            ]
        );
        assert!(args.contains(&"diff.external=".to_string()));
        assert!(args.contains(&"core.pager=cat".to_string()));
        let r = libflankr_blob::GitRequest::parse("diff").unwrap();
        let args = show_diff_args(&r, None);
        assert_eq!(args.last().unwrap(), "HEAD");
        assert!(!args.contains(&"--end-of-options".to_string()));
        let r = libflankr_blob::GitRequest::parse("diff HEAD~2..HEAD").unwrap();
        assert_eq!(show_diff_args(&r, None).last().unwrap(), "HEAD~2..HEAD");
    }
}
