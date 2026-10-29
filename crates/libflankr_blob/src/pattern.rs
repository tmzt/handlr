//! The `read` pattern syntax and the always-refused names (decision 4.9),
//! moved here from the daemon's `src/llm/planning.rs` and `src/project.rs`
//! (docs/design/20261027_04_GENERAL_AGENT_PLANNING_PLAN.md, the 11:20 EDT
//! amendment, item 4) so the daemon and the app match paths with one piece
//! of code.

/// One `read` pattern: `/`-separated segments matched against a
/// root-relative path's segments. `**` as a whole segment is zero or more
/// segments; `*` within a segment is any run of characters but `/`;
/// everything else is literal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadPattern {
    /// As written.
    pub text: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Segment {
    /// `**`: zero or more segments.
    AnyDepth,
    /// One segment, `*` its only wildcard.
    Name(String),
}

impl ReadPattern {
    /// `text` as a pattern, or why it is refused.
    pub fn parse(text: &str) -> Result<ReadPattern, String> {
        let refuse = |why: &str| Err(format!("read pattern \"{text}\" is refused: {why}"));
        if text.is_empty() {
            return refuse("it is empty");
        }
        if let Some(c) = text
            .chars()
            .find(|c| matches!(c, '?' | '[' | ']' | '{' | '}' | '!' | '\\'))
        {
            return refuse(&format!("'{c}' is not part of the pattern syntax"));
        }
        if text.starts_with('/') {
            return refuse("it starts with '/'; patterns are relative to the project's root");
        }
        let mut segments = Vec::new();
        for segment in text.split('/') {
            match segment {
                "" => return refuse("it has an empty segment"),
                "." | ".." => return refuse("'.' and '..' are not allowed"),
                "**" => segments.push(Segment::AnyDepth),
                s if s.contains("**") => return refuse("'**' must be a whole segment"),
                s => segments.push(Segment::Name(s.to_string())),
            }
        }
        Ok(ReadPattern {
            text: text.to_string(),
            segments,
        })
    }

    /// Whether the root-relative `path` (`/`-separated) matches.
    pub fn matches(&self, path: &str) -> bool {
        let names: Vec<&str> = path.split('/').collect();
        match_segments(&self.segments, &names)
    }
}

fn match_segments(pattern: &[Segment], names: &[&str]) -> bool {
    match pattern.split_first() {
        None => names.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=names.len()).any(|skip| match_segments(rest, &names[skip..]))
        }
        Some((Segment::Name(p), rest)) => match names.split_first() {
            Some((name, names)) => match_name(p, name) && match_segments(rest, names),
            None => false,
        },
    }
}

/// One name against one segment, `*` being any run of characters.
fn match_name(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // No `*`: the whole name.
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// Whether a root-relative path is refused whatever the patterns say:
/// any segment `.private`, `.git`, `.env` or `.env.*`, or `.` and `..`,
/// or an empty segment.
pub fn always_refused(path: &str) -> bool {
    path.split('/').any(|name| {
        matches!(name, ".private" | ".git" | ".env" | "." | ".." | "") || name.starts_with(".env.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_outside_the_syntax_are_refused() {
        for ok in ["*.md", "docs/**/*.md", "docs/**"] {
            assert!(ReadPattern::parse(ok).is_ok(), "{ok}");
        }
        let err = |t: &str| ReadPattern::parse(t).unwrap_err();
        assert!(err("a?.md").contains("'?'"));
        for bad in ["[ab].md", "{a,b}.md", "!x.md", "a\\b", ""] {
            assert!(ReadPattern::parse(bad).is_err(), "{bad}");
        }
        assert!(err("/etc/*").contains("starts with '/'"));
        assert!(err("a**/b").contains("whole segment"));
        assert!(err("docs/**x").contains("whole segment"));
        assert!(err("a//b").contains("empty segment"));
        assert!(err("../x.md").contains("not allowed"));
        assert_eq!(ReadPattern::parse("*.md").unwrap().text, "*.md");
    }

    #[test]
    fn patterns_match_by_segment() {
        let p = |t: &str| ReadPattern::parse(t).unwrap();
        let top = p("*.md");
        assert!(top.matches("README.md"));
        assert!(!top.matches("docs/README.md"), "the top folder only");
        assert!(!top.matches("README.mdx"));
        let docs = p("docs/**/*.md");
        for path in ["docs/x.md", "docs/design/x.md", "docs/a/b/c.md"] {
            assert!(docs.matches(path), "{path}");
        }
        for path in ["docs/x.txt", "src/docs/x.md", "docs", "x.md"] {
            assert!(!docs.matches(path), "{path}");
        }
        let all = p("**/*.md");
        assert!(all.matches("README.md") && all.matches("src/a/README.md"));
        let tree = p("docs/**");
        assert!(tree.matches("docs/x.txt") && tree.matches("docs/a/b"));
        assert!(!tree.matches("src/x"));
        let mid = p("src/*_PLAN*.md");
        assert!(mid.matches("src/20261027_04_X_PLAN.md"));
        assert!(mid.matches("src/a_PLAN.md"));
        assert!(!mid.matches("src/a_PLA.md"));
        assert!(p("a*a").matches("aa") && !p("a*a").matches("a"));
        assert!(p("Cargo.toml").matches("Cargo.toml") && !p("Cargo.toml").matches("Cargo.lock"));
    }

    #[test]
    fn the_refused_names_hold_whatever_the_patterns_say() {
        for path in [
            ".env",
            ".env.local",
            ".private/key.md",
            ".git/config",
            "docs/.git/x",
            "../x.md",
            "docs/../x",
            "./x",
            "a//b",
            "",
        ] {
            assert!(always_refused(path), "{path}");
        }
        for path in ["README.md", "docs/design/x.md", ".envrc", "env/.gitignore"] {
            assert!(!always_refused(path), "{path}");
        }
    }
}
