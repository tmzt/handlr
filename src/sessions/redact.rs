//! Redaction of session text before it leaves handlr (plan 20261022_03,
//! decision 7): Gemini's context, `agents.list` and `session.peek` only
//! ever see redacted text.
//!
//! What is redacted, in this order:
//! 1. PEM blocks, `-----BEGIN …-----` to `-----END …-----` (to the end of
//!    the text when the end is missing);
//! 2. on a line that mentions a path under `.private/` or a `.env` file,
//!    whatever follows the path (after a `=` or `:` when there is one);
//! 3. API keys: `AIza…`, `sk-…`, `ghp_…` (and GitHub's other token
//!    prefixes), `xox…`;
//! 4. JWTs: three base64url segments, the first starting `eyJ`;
//! 5. runs of more than 40 hex characters, and runs of more than 40
//!    base64 characters with a random-looking stretch in them.
//!
//! Each is replaced by [`REDACTED`]. The rules are deliberately broad: a
//! false positive costs a few words of context, a false negative a secret.

/// What a redacted secret is replaced with.
pub const REDACTED: &str = "[redacted]";

/// Runs longer than this many characters are redacted (rule 5).
const LONG_RUN: usize = 40;

/// Redact `text` by every rule.
pub fn redact(text: &str) -> String {
    let text = redact_pem(text);
    let text = redact_secret_paths(&text);
    let text = redact_api_keys(&text);
    let text = redact_jwts(&text);
    redact_long_runs(&text)
}

/// Whether `text` mentions a path under `.private/` or a `.env` file.
pub fn mentions_secret_path(text: &str) -> bool {
    secret_path_mention(text).is_some()
}

/// Replace each `(start, end)` byte span of `text` with [`REDACTED`].
/// Spans are sorted and do not overlap; every bound is a char boundary.
fn replace_spans(text: &str, spans: &[(usize, usize)]) -> String {
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for &(start, end) in spans {
        out.push_str(&text[at..start]);
        out.push_str(REDACTED);
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

// ── 1. PEM blocks ────────────────────────────────────────────────────────

fn redact_pem(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const END: &str = "-----END ";
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find(BEGIN) {
        let start = from + rel;
        let end = match text[start + BEGIN.len()..].find(END) {
            Some(rel_end) => {
                let end_marker = start + BEGIN.len() + rel_end + END.len();
                // The END line's closing dashes, else the end of its line.
                match text[end_marker..].find("-----") {
                    Some(close) if !text[end_marker..end_marker + close].contains('\n') => {
                        end_marker + close + 5
                    }
                    _ => text[end_marker..]
                        .find('\n')
                        .map_or(text.len(), |nl| end_marker + nl),
                }
            }
            None => text.len(),
        };
        spans.push((start, end));
        from = end;
    }
    replace_spans(text, &spans)
}

// ── 2. Lines that mention .private/ or .env ─────────────────────────────

/// Where a path ends: at whitespace, a quote, a bracket or a separator.
fn ends_path(c: char) -> bool {
    c.is_whitespace() || "\"'`()[]{}<>,;:=".contains(c)
}

/// The byte range of the first `.private/…` or `.env` path in `line`.
fn secret_path_mention(line: &str) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut best: Option<usize> = None;
    if let Some(i) = line.find(".private/") {
        best = Some(i);
    }
    let mut from = 0;
    while let Some(rel) = line[from..].find(".env") {
        let i = from + rel;
        let after = bytes.get(i + 4).copied();
        // `.env`, `.env.local`, `/.env"`; not `.environment` or `.envrc`.
        let file_end = after.is_none_or(|b| !(b as char).is_ascii_alphanumeric() && b != b'_');
        if file_end {
            best = Some(best.map_or(i, |b| b.min(i)));
            break;
        }
        from = i + 4;
    }
    let start = best?;
    let end = line[start..]
        .char_indices()
        .find(|&(_, c)| ends_path(c))
        .map_or(line.len(), |(i, _)| start + i);
    Some((start, end))
}

fn redact_secret_paths(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        match secret_path_mention(body) {
            Some((_, end)) if body[end..].chars().any(|c| c.is_alphanumeric()) => {
                let rest = &body[end..];
                let keep = rest.find(['=', ':']).map_or(0, |i| i + 1);
                out.push_str(&body[..end + keep]);
                out.push(' ');
                out.push_str(REDACTED);
            }
            _ => out.push_str(body),
        }
        out.push_str(newline);
    }
    out
}

// ── 3. API keys ──────────────────────────────────────────────────────────

fn is_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Known key prefixes, the fewest key characters that must follow, and
/// whether those must include a digit.
const KEY_PREFIXES: &[(&str, usize, bool)] = &[
    ("AIza", 30, false),
    ("sk-", 20, true),
    ("ghp_", 20, false),
    ("gho_", 20, false),
    ("ghu_", 20, false),
    ("ghs_", 20, false),
    ("ghr_", 20, false),
    ("github_pat_", 20, false),
    ("xoxa-", 10, true),
    ("xoxb-", 10, true),
    ("xoxe-", 10, true),
    ("xoxp-", 10, true),
    ("xoxr-", 10, true),
    ("xoxs-", 10, true),
];

fn redact_api_keys(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for &(prefix, min_body, needs_digit) in KEY_PREFIXES {
        let mut from = 0;
        while let Some(rel) = text[from..].find(prefix) {
            let start = from + rel;
            let body_start = start + prefix.len();
            from = body_start;
            if start > 0 && is_key_char(bytes[start - 1]) {
                continue;
            }
            let body_end = bytes[body_start..]
                .iter()
                .position(|&b| !is_key_char(b))
                .map_or(bytes.len(), |p| body_start + p);
            let body = &bytes[body_start..body_end];
            if body.len() >= min_body && (!needs_digit || body.iter().any(u8::is_ascii_digit)) {
                spans.push((start, body_end));
                from = body_end;
            }
        }
    }
    merge(&mut spans);
    replace_spans(text, &spans)
}

/// Sort `spans` and merge the ones that overlap.
fn merge(spans: &mut Vec<(usize, usize)>) {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for &(s, e) in spans.iter() {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    *spans = merged;
}

// ── 4. JWTs ──────────────────────────────────────────────────────────────

fn is_b64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn redact_jwts(text: &str) -> String {
    let bytes = text.as_bytes();
    let run = |from: usize| {
        bytes[from..]
            .iter()
            .position(|&b| !is_b64url(b))
            .map_or(bytes.len(), |p| from + p)
    };
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find("eyJ") {
        let start = from + rel;
        from = start + 3;
        if start > 0 && is_b64url(bytes[start - 1]) {
            continue;
        }
        let head_end = run(start);
        if head_end - start < 10 || bytes.get(head_end) != Some(&b'.') {
            continue;
        }
        let body_end = run(head_end + 1);
        if body_end - head_end - 1 < 10 || bytes.get(body_end) != Some(&b'.') {
            continue;
        }
        let end = run(body_end + 1);
        spans.push((start, end));
        from = end;
    }
    replace_spans(text, &spans)
}

// ── 5. Long hex and base64 runs ──────────────────────────────────────────

fn is_b64(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"+/=_-".contains(&b)
}

/// Whether a base64 run looks random rather than like a path or a name: a
/// stretch of at least 20 characters between `/`, `_` and `-` that mixes
/// capitals, small letters and digits.
fn looks_random(run: &[u8]) -> bool {
    run.split(|b| b"/_-".contains(b)).any(|seg| {
        seg.len() >= 20
            && seg.iter().any(u8::is_ascii_uppercase)
            && seg.iter().any(u8::is_ascii_lowercase)
            && seg.iter().any(u8::is_ascii_digit)
    })
}

fn redact_long_runs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_b64(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_b64(bytes[i]) {
            i += 1;
        }
        let run = &bytes[start..i];
        if run.len() > LONG_RUN && looks_random(run) {
            spans.push((start, i));
            continue;
        }
        // Hex runs inside the base64 run.
        let mut j = 0;
        while j < run.len() {
            if !run[j].is_ascii_hexdigit() {
                j += 1;
                continue;
            }
            let hex_start = j;
            while j < run.len() && run[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j - hex_start > LONG_RUN {
                spans.push((start + hex_start, start + j));
            }
        }
    }
    replace_spans(text, &spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic secrets: the right shapes, made-up characters.
    const GOOGLE: &str = "AIzaSyD0nTuSeTh1sFakeKeyAbCdEfGhIjKlMn";
    const OPENAI: &str = "sk-proj-0a1B2c3D4e5F6g7H8i9J0kLmNoPq";
    const ANTHROPIC: &str = "sk-ant-api03-Zz9Yy8Xx7Ww6Vv5Uu4Tt3Ss2";
    const GITHUB: &str = "ghp_0123456789abcdefABCDEF0123456789abcd";
    const SLACK: &str = "xoxb-1234567890-0987654321-AbCdEfGhIjKl";
    const JWT: &str = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlLWZha2UtMDAx";

    fn gone(text: &str, secret: &str) {
        let out = redact(text);
        assert!(!out.contains(secret), "{secret} survived in {out:?}");
        assert!(out.contains(REDACTED), "nothing redacted in {out:?}");
    }

    #[test]
    fn google_keys_are_redacted() {
        gone(&format!("key = {GOOGLE} ok"), GOOGLE);
        assert_eq!(redact(&format!("({GOOGLE})")), "([redacted])");
    }

    #[test]
    fn sk_keys_are_redacted() {
        gone(&format!("export OPENAI_API_KEY={OPENAI}"), OPENAI);
        gone(&format!("the key {ANTHROPIC} works"), ANTHROPIC);
        // Words that merely contain or start with sk- stay.
        let plain = "a task-notification, and sk-learn-is-a-library-name";
        assert_eq!(redact(plain), plain);
    }

    #[test]
    fn github_tokens_are_redacted() {
        gone(&format!("token: {GITHUB}"), GITHUB);
        let pat = "github_pat_11ABCDEFG0123456789_abcdefghijklmnop";
        gone(&format!("GH_TOKEN={pat}"), pat);
    }

    #[test]
    fn slack_tokens_are_redacted() {
        gone(&format!("SLACK={SLACK}"), SLACK);
    }

    #[test]
    fn pem_blocks_are_redacted() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nZmFrZQ==\n-----END OPENSSH PRIVATE KEY-----";
        let out = redact(&format!("here:\n{pem}\nafter"));
        assert_eq!(out, "here:\n[redacted]\nafter");
        // Cut off before its end: redacted to the end.
        let out = redact("x -----BEGIN RSA PRIVATE KEY-----\nMIIEfake");
        assert_eq!(out, "x [redacted]");
    }

    #[test]
    fn jwts_are_redacted() {
        gone(&format!("Authorization: Bearer {JWT}"), JWT);
        assert_eq!(redact(&format!("t={JWT}.")), "t=[redacted].");
    }

    #[test]
    fn long_hex_runs_are_redacted() {
        let hex = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        gone(&format!("sha256 {hex}"), hex);
        // A 40-character git hash is not more than 40.
        let sha1 = "1dada86a1dada86a1dada86a1dada86a1dada86a";
        assert_eq!(sha1.len(), 40);
        assert_eq!(redact(sha1), sha1);
        // UUIDs stay.
        let uuid = "00000000-0000-4000-8000-000000000001";
        assert_eq!(redact(uuid), uuid);
    }

    #[test]
    fn long_base64_runs_are_redacted() {
        let b64 = "Zm9vYmFyQmF6UXV1eDEyMzQ1Njc4OTBhYmNkZWZnaGlqa2xtbm9w";
        gone(&format!("secret: {b64}"), b64);
        // Paths and names do not look random.
        for plain in [
            "/work/example/target/debug/deps/handlr-5f3c2a1b9d8e7f6a",
            "/Users/someone/src/project/docs/design/plans",
            "send-usage-to-the-claude-agent-in-the-project",
        ] {
            assert_eq!(redact(plain), plain);
        }
    }

    #[test]
    fn secret_path_lines_lose_their_values() {
        assert_eq!(
            redact(".private/gemini_api_key: not-a-real-value"),
            ".private/gemini_api_key: [redacted]"
        );
        assert_eq!(
            redact("cat /work/app/.env\nnext line"),
            "cat /work/app/.env\nnext line"
        );
        assert_eq!(
            redact("from .env: GEMINI_API_KEY=abc"),
            "from .env: [redacted]"
        );
        assert_eq!(
            redact("I read .env.local and it says hello"),
            "I read .env.local [redacted]"
        );
        // Only the line that mentions it.
        assert_eq!(
            redact("x\nsee .private/token value here\ny"),
            "x\nsee .private/token [redacted]\ny"
        );
        // Not a .env file.
        let plain = "std::env and self.environment and .envrc are fine";
        assert_eq!(redact(plain), plain);
        // Trailing punctuation alone is kept.
        assert_eq!(
            redact(r#"{"file_path":"/x/.env"}"#),
            r#"{"file_path":"/x/.env"}"#
        );
        assert!(mentions_secret_path("Read /x/.private/key"));
        assert!(!mentions_secret_path("an environment"));
    }

    #[test]
    fn plain_text_is_unchanged() {
        let text = "I updated src/transport.rs and ran cargo test -p handlr. Shall I commit?";
        assert_eq!(redact(text), text);
        assert_eq!(redact("héllo — ok?"), "héllo — ok?");
    }
}
