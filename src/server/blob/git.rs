//! The `git` namespace, daemon to phone (docs/design/20261028_05_BINARY_FILES_PLAN.md,
//! decisions 6.5-6.10): `show` and `diff` by a fixed argv with validated
//! revs, a cleared environment, 5 s, cut at 8 MiB; not limited by the
//! `read` patterns (the owner's ruling, 6.9).

use std::path::Path;
use std::sync::Arc;

use libflankr_blob::{Code, GIT_CUT_MARKER, GIT_MAX, GitRequest};

use super::super::conn::Conn;
use super::{Refusal, local_root, session};
use crate::git::CappedError;

/// What a REQUEST is answered with: (name, MIME, bytes, mtime in ms).
pub type Served = super::files::Served;

/// The empty output's text (decision 6.10).
pub const NO_CHANGES: &str = "(no changes)\n";

/// `request` in `session`'s repository.
pub(super) async fn request(
    conn: &Arc<Conn>,
    session_id: &str,
    request: &str,
    max_size: u64,
) -> Result<Served, Refusal> {
    let Some(git) = crate::git::binary() else {
        return Err((Code::UNSUPPORTED, "git is not installed".into()));
    };
    let parsed = GitRequest::parse(request)
        .map_err(|code| (code, "not a request flankr serves".to_string()))?;
    let s = session(conn, session_id).await?;
    let root = local_root(conn, &s)?;
    let Ok(_running) = Arc::clone(&conn.daemon.blob.git_running).try_acquire_owned() else {
        return Err((Code::BUSY, "git is busy".into()));
    };
    let started = std::time::Instant::now();
    let served = run(git, Path::new(&root), &parsed, max_size).await;
    match &served {
        Ok((name, _, bytes, _)) => log::info!(
            "git: '{name}' for pane '{}': exit 0, {} ms, {} bytes{}",
            s.pane_id,
            started.elapsed().as_millis(),
            bytes.len(),
            if bytes.ends_with(GIT_CUT_MARKER.as_bytes()) {
                ", cut"
            } else {
                ""
            }
        ),
        Err((code, reason)) => log::info!(
            "git: '{parsed}' for pane '{}': {code} ({reason}), {} ms",
            s.pane_id,
            started.elapsed().as_millis()
        ),
    }
    served
}

/// Runs `request` in `root` (decision 6.7), its path checked as a
/// message's paths are (6.5).
pub async fn run(
    git: &Path,
    root: &Path,
    request: &GitRequest,
    max_size: u64,
) -> Result<Served, Refusal> {
    let pathspec = match request.path() {
        Some(path) => Some(
            crate::git::check_path(path, &root.to_string_lossy())
                .map_err(|e| (Code::NOT_PERMITTED, e))?,
        ),
        None => None,
    };
    let args = crate::git::show_diff_args(request, pathspec.as_deref());
    let max = GIT_MAX.min(max_size.max(GIT_CUT_MARKER.len() as u64 + 1)) as usize;
    let out = crate::git::run_capped(git, root, &args, max, GIT_CUT_MARKER)
        .await
        .map_err(|e| match e {
            CappedError::Timeout => (Code::TIMEOUT, "git took longer than 5 s".to_string()),
            CappedError::Io(why) => (Code::IO, why),
        })?;
    if !out.cut && out.code != Some(0) {
        // `show` exits 128 ("fatal: not a git repository"); `diff`
        // outside a work tree exits 129 ("warning: Not a git repository").
        if matches!(out.code, Some(128 | 129))
            && out
                .stderr_line
                .to_ascii_lowercase()
                .contains("not a git repository")
        {
            return Err((Code::NOT_A_REPOSITORY, out.stderr_line));
        }
        return Err((Code::FAILED, out.stderr_line));
    }
    let mut bytes = out.stdout;
    if bytes.is_empty() {
        bytes = NO_CHANGES.as_bytes().to_vec();
    }
    Ok((request.to_string(), "text/x-diff", bytes, 0))
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use super::*;

    fn git() -> &'static Path {
        crate::git::binary().expect("git on PATH for these tests")
    }

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new(git())
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    /// A repository with three commits of `a.txt`.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        sh(dir.path(), &["init", "-q", "-b", "main"]);
        for i in 1..=3 {
            std::fs::write(dir.path().join("a.txt"), format!("line {i}\n")).unwrap();
            sh(dir.path(), &["add", "-A"]);
            sh(dir.path(), &["commit", "-q", "-m", &format!("commit {i}")]);
        }
        dir
    }

    async fn ask(root: &Path, request: &str) -> Result<String, Code> {
        let parsed = GitRequest::parse(request)?;
        run(git(), root, &parsed, GIT_MAX)
            .await
            .map(|(name, mime, bytes, _)| {
                assert_eq!(mime, "text/x-diff");
                assert_eq!(name, parsed.to_string());
                String::from_utf8(bytes).unwrap()
            })
            .map_err(|(code, _)| code)
    }

    #[tokio::test]
    async fn show_and_diff_in_a_repository() {
        let dir = repo();
        let root = dir.path();
        let head = sh(root, &["rev-parse", "HEAD"]);
        let short = &head.trim()[..8];
        let shown = ask(root, &format!("show {short}")).await.unwrap();
        assert!(
            shown.contains("commit 3") && shown.contains("+line 3"),
            "{shown}"
        );
        let one = ask(root, "show HEAD~1 -- a.txt").await.unwrap();
        assert!(one.contains("+line 2"), "{one}");
        let range = ask(root, "diff HEAD~2..HEAD").await.unwrap();
        assert!(
            range.contains("-line 1") && range.contains("+line 3"),
            "{range}"
        );
        assert_eq!(ask(root, "diff").await.unwrap(), NO_CHANGES);
        // An edited file and a staged one.
        std::fs::write(root.join("a.txt"), "edited\n").unwrap();
        std::fs::write(root.join("b.txt"), "staged\n").unwrap();
        sh(root, &["add", "b.txt"]);
        let diff = ask(root, "diff").await.unwrap();
        assert!(
            diff.contains("+edited") && diff.contains("+staged"),
            "{diff}"
        );
        let one = ask(root, "diff -- b.txt").await.unwrap();
        assert!(one.contains("+staged") && !one.contains("+edited"), "{one}");
        // A path not matching any `read` pattern is served (6.9): none apply.
        assert!(ask(root, "show HEAD -- a.txt").await.is_ok());
    }

    #[tokio::test]
    async fn refusals() {
        let dir = repo();
        let root = dir.path();
        for bad in [
            "show --output=/tmp/x",
            "show main",
            "show HEAD^",
            "show HEAD~1000",
            "show abc",
            "diff a..b..c",
            "show  HEAD",
            "show HEAD ",
        ] {
            assert_eq!(
                ask(root, bad).await.unwrap_err(),
                Code::BAD_REQUEST,
                "{bad}"
            );
        }
        assert_eq!(
            ask(root, "show HEAD -- ../x").await.unwrap_err(),
            Code::NOT_PERMITTED
        );
        assert_eq!(
            ask(root, "show HEAD -- ~/x").await.unwrap_err(),
            Code::NOT_PERMITTED
        );
        // A hash that is no commit.
        assert_eq!(
            ask(root, "show 1234567abc").await.unwrap_err(),
            Code::FAILED
        );
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(
            ask(outside.path(), "diff").await.unwrap_err(),
            Code::NOT_A_REPOSITORY
        );
        assert_eq!(
            ask(outside.path(), "show HEAD").await.unwrap_err(),
            Code::NOT_A_REPOSITORY
        );
    }

    #[tokio::test]
    async fn an_external_diff_and_a_textconv_driver_never_run() {
        let dir = repo();
        let root = dir.path();
        let marker = root.join("ran");
        let script = root.join("tool.sh");
        std::fs::write(&script, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config = root.join(".git/config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str(&format!(
            "[diff]\n\texternal = {s}\n[diff \"conv\"]\n\ttextconv = {s}\n",
            s = script.display()
        ));
        std::fs::write(&config, text).unwrap();
        std::fs::write(root.join(".gitattributes"), "*.txt diff=conv\n").unwrap();
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        assert!(ask(root, "diff").await.unwrap().contains("+changed"));
        assert!(ask(root, "show HEAD").await.is_ok());
        assert!(ask(root, "diff HEAD~1..HEAD").await.is_ok());
        assert!(!marker.exists(), "a configured tool ran");
    }

    #[tokio::test]
    async fn a_long_output_is_cut_and_served() {
        let dir = repo();
        let root = dir.path();
        // About 9 MiB of changed lines.
        let line = "x".repeat(99) + "\n";
        std::fs::write(root.join("a.txt"), line.repeat(95_000)).unwrap();
        let out = ask(root, "diff").await.unwrap();
        assert_eq!(out.len() as u64, GIT_MAX);
        assert!(out.ends_with(GIT_CUT_MARKER));
    }
}
