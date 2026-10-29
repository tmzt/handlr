//! A `git` namespace request (decisions 6.5 and 6.6).

use crate::Code;

/// A rev of decision 6.6: a full or short hash, or `HEAD~N`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Rev {
    /// 7-40 lowercase hex digits.
    Hash(String),
    /// `HEAD` (0) or `HEAD~N`, `N` in `1..=999`.
    Head(u16),
}

/// The largest `N` of `HEAD~N`.
pub const HEAD_MAX: u16 = 999;

impl Rev {
    /// `^[0-9a-f]{7,40}$`, `HEAD`, or `HEAD~N` with `N` in `0..=999`
    /// written without leading zeros; nothing else.
    pub fn parse(s: &str) -> Option<Rev> {
        if (7..=40).contains(&s.len()) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Some(Rev::Hash(s.to_string()));
        }
        if s == "HEAD" {
            return Some(Rev::Head(0));
        }
        let n = s.strip_prefix("HEAD~")?;
        if n.is_empty()
            || n.len() > 3
            || !n.bytes().all(|b| b.is_ascii_digit())
            || (n.len() > 1 && n.starts_with('0'))
        {
            return None;
        }
        let n: u16 = n.parse().ok()?;
        (n <= HEAD_MAX).then_some(Rev::Head(n))
    }
}

impl std::fmt::Display for Rev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rev::Hash(h) => f.write_str(h),
            Rev::Head(0) => f.write_str("HEAD"),
            Rev::Head(n) => write!(f, "HEAD~{n}"),
        }
    }
}

/// A `git` request, parsed and validated; `Display` writes it back,
/// normalised (`HEAD~0` as `HEAD`). The path is as the phone wrote it; the
/// daemon checks it against the project (`not_permitted`) before use.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GitRequest {
    /// `show <rev>`, `show <rev> -- <path>`.
    Show { rev: Rev, path: Option<String> },
    /// `diff <rev>..<rev>`, `diff <rev>..<rev> -- <path>`.
    DiffRange {
        from: Rev,
        to: Rev,
        path: Option<String>,
    },
    /// `diff`, `diff -- <path>`: the working tree and index against `HEAD`.
    DiffWorkingTree { path: Option<String> },
}

impl GitRequest {
    /// One of decision 6.5's six forms, words separated by single spaces
    /// and nothing else; anything else is [`Code::BAD_REQUEST`].
    pub fn parse(s: &str) -> Result<GitRequest, Code> {
        let bad = Err(Code::BAD_REQUEST);
        let words: Vec<&str> = s.split(' ').collect();
        if words.iter().any(|w| w.is_empty()) {
            return bad;
        }
        // `-- <path>` at the end, else no path.
        let (words, path) = match words.as_slice() {
            [head @ .., "--", path] => (head, Some(path.to_string())),
            all => (all, None),
        };
        if words.contains(&"--")
            || path
                .as_deref()
                .is_some_and(|p| p.chars().any(char::is_control))
        {
            return bad;
        }
        match words {
            ["show", rev] => match Rev::parse(rev) {
                Some(rev) => Ok(GitRequest::Show { rev, path }),
                None => bad,
            },
            ["diff"] => Ok(GitRequest::DiffWorkingTree { path }),
            ["diff", range] => {
                let Some((from, to)) = range.split_once("..") else {
                    return bad;
                };
                match (Rev::parse(from), Rev::parse(to)) {
                    (Some(from), Some(to)) => Ok(GitRequest::DiffRange { from, to, path }),
                    _ => bad,
                }
            }
            _ => bad,
        }
    }

    /// The path the request is limited to, as written.
    pub fn path(&self) -> Option<&str> {
        match self {
            GitRequest::Show { path, .. }
            | GitRequest::DiffRange { path, .. }
            | GitRequest::DiffWorkingTree { path } => path.as_deref(),
        }
    }
}

impl std::fmt::Display for GitRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitRequest::Show { rev, .. } => write!(f, "show {rev}")?,
            GitRequest::DiffRange { from, to, .. } => write!(f, "diff {from}..{to}")?,
            GitRequest::DiffWorkingTree { .. } => f.write_str("diff")?,
        }
        if let Some(path) = self.path() {
            write!(f, " -- {path}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_forms_parse_and_write_back() {
        for s in [
            "show 1a2b3c4d",
            "show HEAD~1 -- a.txt",
            "diff HEAD~2..HEAD",
            "diff 1a2b3c4..HEAD -- src/a.rs",
            "diff",
            "diff -- docs",
            "show 0123456789abcdef0123456789abcdef01234567",
        ] {
            assert_eq!(GitRequest::parse(s).unwrap().to_string(), s, "{s}");
        }
        assert_eq!(
            GitRequest::parse("show HEAD~0").unwrap().to_string(),
            "show HEAD"
        );
        assert_eq!(
            GitRequest::parse("show HEAD~1 -- a.txt").unwrap(),
            GitRequest::Show {
                rev: Rev::Head(1),
                path: Some("a.txt".into())
            }
        );
        assert_eq!(GitRequest::parse("diff -- x").unwrap().path(), Some("x"));
    }

    #[test]
    fn everything_else_is_bad_request() {
        for s in [
            "show --output=/tmp/x",
            "show main",
            "show HEAD^",
            "show HEAD~1000",
            "show HEAD~01",
            "show HEAD~",
            "show abc",
            "show ABCDEF12",
            "show @",
            "show HEAD:a.txt",
            "show 0123456789abcdef0123456789abcdef012345678",
            "diff a..b..c",
            "diff HEAD...HEAD~1",
            "diff HEAD~1..",
            "diff HEAD",
            "show  1a2b3c4",
            "show 1a2b3c4 ",
            " diff",
            "diff --",
            "diff -- a -- b",
            "show 1a2b3c4 -- a b",
            "show 1a2b3c4 a.txt",
            "log",
            "",
            "show 1a2b3c4 -- a\tb",
            "diff -- a\u{7}",
        ] {
            assert_eq!(GitRequest::parse(s), Err(Code::BAD_REQUEST), "{s:?}");
        }
    }
}
