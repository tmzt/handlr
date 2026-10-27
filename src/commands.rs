//! Command policy helpers shared by the transport permission gate and the
//! speech-to-speech processor.
//!
//! Two related concerns live here:
//!
//! * **Arbitrary command validation** — agents configured with
//!   `allow_arbitrary_commands` may receive free-form spoken instructions that
//!   are not on their allow list. Those strings are typed verbatim into the
//!   agent's prompt, so they are restricted to a conservative ASCII whitelist.
//! * **Confirmation request parsing** — the mechanical gate is armed from the
//!   assistant's own "Should I send ... ?" turn, so the pane and command it
//!   named can be bound to the gate and re-checked at execution time.

/// Maximum length of an arbitrary (non allow-listed) command.
pub const MAX_ARBITRARY_COMMAND_LEN: usize = 256;

/// Maximum number of threshold-triggering symbols permitted before an
/// arbitrary command or message is rejected.
pub const SYMBOL_THRESHOLD: usize = 0;

/// Characters permitted in an arbitrary command, in addition to ASCII
/// alphanumerics and spaces.
const EXTRA_ALLOWED_CHARS: &[char] = &['.', ',', '\'', '?', ':', '_'];

fn is_allowed_char(c: char, allow_multiline: bool) -> bool {
    c.is_ascii_alphanumeric()
        || c == ' '
        || (allow_multiline && (c == '\n' || c == '\r' || c == '\t'))
        || EXTRA_ALLOWED_CHARS.contains(&c)
}

fn is_allowed_arbitrary_char(c: char) -> bool {
    is_allowed_char(c, false)
}

/// Count symbols that trigger the rejection threshold.
///
/// A single symbol followed by a space (or trailing sentence punctuation at the
/// end of the text), such as `. `, `, `, `! `, `? `, `: `, or `- `, does not
/// trigger the threshold.
/// Embedded symbols (not followed by space, such as in paths or flags) or runs
/// of consecutive symbols do trigger the threshold.
pub fn count_triggering_symbols(s: &str) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut count = 0;
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() && !c.is_whitespace() && c != '_' {
            // An apostrophe in a contraction (e.g. don't, it's) does not trigger the threshold.
            if c == '\''
                && i > 0
                && chars[i - 1].is_alphabetic()
                && i + 1 < chars.len()
                && chars[i + 1].is_alphabetic()
            {
                continue;
            }
            let is_sentence_end =
                (i + 1 == chars.len()) && matches!(c, '.' | '?' | '!' | ',' | ':');
            let followed_by_space =
                is_sentence_end || (i + 1 < chars.len() && chars[i + 1].is_whitespace());
            let preceded_by_symbol =
                i > 0 && !chars[i - 1].is_alphanumeric() && !chars[i - 1].is_whitespace();
            if !followed_by_space || preceded_by_symbol {
                count += 1;
            }
        }
    }
    count
}

/// Find the first symbol that triggers the threshold, if any.
pub fn first_triggering_symbol(s: &str) -> Option<char> {
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() && !c.is_whitespace() && c != '_' {
            if c == '\''
                && i > 0
                && chars[i - 1].is_alphabetic()
                && i + 1 < chars.len()
                && chars[i + 1].is_alphabetic()
            {
                continue;
            }
            let is_sentence_end =
                (i + 1 == chars.len()) && matches!(c, '.' | '?' | '!' | ',' | ':');
            let followed_by_space =
                is_sentence_end || (i + 1 < chars.len() && chars[i + 1].is_whitespace());
            let preceded_by_symbol =
                i > 0 && !chars[i - 1].is_alphanumeric() && !chars[i - 1].is_whitespace();
            if !followed_by_space || preceded_by_symbol {
                return Some(c);
            }
        }
    }
    None
}

/// Remove unallowed symbols from the text, keeping ASCII alphanumeric
/// characters, spaces, and allowed punctuation (`. , ' ? :`).
pub fn filter_symbols(s: &str) -> String {
    s.chars()
        .filter(|&c| is_allowed_arbitrary_char(c))
        .collect()
}

/// Remove unallowed symbols from message or general agent text, keeping ASCII
/// alphanumeric characters, spaces, newlines, tabs, and allowed punctuation (`. , ' ? :`).
pub fn filter_message_symbols(s: &str) -> String {
    s.chars().filter(|&c| is_allowed_char(c, true)).collect()
}

/// Validate a free-form command for an agent with `allow_arbitrary_commands`.
///
/// Returns the cleaned command (surrounding whitespace and trailing sentence
/// punctuation removed, and unallowed symbols stripped) or a human-readable
/// rejection reason that is surfaced back to the model as the tool error.
///
/// Rejects: non-ASCII text, embedded control characters (which would otherwise
/// inject a second line of input into the pane), excessive threshold-triggering
/// symbols (over [`SYMBOL_THRESHOLD`]), and empty or overlong strings.
pub fn validate_arbitrary_command(cmd: &str) -> Result<String, String> {
    let trimmed = cmd.trim();

    if !trimmed.is_ascii() {
        return Err(format!(
            "Arbitrary command '{}' rejected: must be ASCII only.",
            trimmed
        ));
    }
    if let Some(bad) = trimmed.chars().find(|c| c.is_ascii_control()) {
        return Err(format!(
            "Arbitrary command '{}' rejected: control characters (found {:?}) are not permitted.",
            trimmed.escape_debug(),
            bad
        ));
    }

    let trigger_count = count_triggering_symbols(trimmed);
    if trigger_count > SYMBOL_THRESHOLD {
        let bad = first_triggering_symbol(trimmed).unwrap_or('?');
        return Err(format!(
            "Arbitrary command '{trimmed}' rejected: too many symbols (found '{bad}', limit is {SYMBOL_THRESHOLD})."
        ));
    }

    let filtered = filter_symbols(trimmed);
    let folded = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = folded.trim_end_matches(['.', ',']).trim();
    if !cleaned.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err("Arbitrary command cannot be empty.".to_string());
    }
    if cleaned.len() > MAX_ARBITRARY_COMMAND_LEN {
        return Err(format!(
            "Arbitrary command rejected: {} characters exceeds the {} character limit.",
            cleaned.len(),
            MAX_ARBITRARY_COMMAND_LEN
        ));
    }

    Ok(cleaned.to_string())
}

/// Maximum length of a dictated message (plan 20261023_02, decision 6 as
/// amended at 01:58 EDT).
pub const MAX_MESSAGE_LEN: usize = 4000;

/// The characters that make a text's first word a command in a coding
/// agent's prompt: `/` (a slash command), `!` (Claude Code's shell mode)
/// and `#` (older Claude Code's memory).
pub const COMMAND_PREFIXES: &[char] = &['/', '!', '#'];

/// Validate a message to a coding agent, typed in the app or sent by the
/// general agent (plan 20261023_02, decision 6, and plan 20261027_04,
/// decision 1, as changed in CHANGES.md 2026-10-27): the symbol ban is now
/// only on the first word, which may not start with one of
/// [`COMMAND_PREFIXES`], since a command must be on the agent's allow list.
/// After it any printable text is kept, paths and flags included; paths
/// are checked against the project by [`check_paths`].
///
/// Returns the cleaned message (runs of spaces folded to one, surrounding
/// space and trailing `.` and `,` removed) or the reason it was refused. Line
/// breaks, tabs and every other control character are refused, never folded,
/// so a message is always one line of input.
pub fn validate_message(text: &str) -> Result<String, String> {
    if let Some(bad) = text.chars().find(|c| c.is_control()) {
        return Err(format!(
            "Message rejected: control characters (found {bad:?}) are not permitted."
        ));
    }
    let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = folded.trim_end_matches(['.', ',']).trim_end();
    if !cleaned.chars().any(|c| c.is_alphanumeric()) {
        return Err("Message cannot be empty.".to_string());
    }
    if let Some(c) = cleaned
        .chars()
        .next()
        .filter(|c| COMMAND_PREFIXES.contains(c))
    {
        return Err(format!(
            "Message rejected: it starts with '{c}', which makes it a command; only the agent's allowed commands may be sent."
        ));
    }
    let len = cleaned.chars().count();
    if len > MAX_MESSAGE_LEN {
        return Err(format!(
            "Message rejected: {len} characters exceeds the {MAX_MESSAGE_LEN} character limit."
        ));
    }
    Ok(cleaned.to_string())
}

/// At most this many paths are checked in one message; more is refused.
pub const MAX_PATHS: usize = 32;
/// A path longer than this many characters is refused.
pub const MAX_PATH_LEN: usize = 512;
/// A path with more than this many `/`-separated parts is refused.
pub const MAX_PATH_PARTS: usize = 32;
/// A path with more than this many `..` parts is refused.
pub const MAX_PARENT_PARTS: usize = 4;

/// The words of `text` that look like paths: those holding a `/`, or
/// starting with `~`, once quotes, brackets and trailing sentence
/// punctuation are taken off. URLs (`://`) are not paths.
pub fn path_words(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| "\"'`()[]{}<>".contains(c))
                .trim_end_matches(['.', ',', ';', ':', '?', '!'])
        })
        .filter(|w| (w.contains('/') || w.starts_with('~')) && !w.contains("://"))
        .collect()
}

/// Check every path in a message to an agent (the owner, 2026-10-27:
/// "paths should be resolved and checked to see if they fall outside of the
/// focused repo, at least for now"): each is resolved against `root`, the
/// agent's project folder (`~` is the home folder), and must stay inside it.
/// Resolution is by the text first, with bounds against pathological paths
/// ([`MAX_PATHS`], [`MAX_PATH_LEN`], [`MAX_PATH_PARTS`],
/// [`MAX_PARENT_PARTS`]); a path that exists is then resolved on disk too,
/// so a symbolic link out of the project is caught. Without a `root`, a
/// message holding a path is refused.
pub fn check_paths(text: &str, root: Option<&str>) -> Result<(), String> {
    use std::path::{Component, Path, PathBuf};
    let words = path_words(text);
    if words.is_empty() {
        return Ok(());
    }
    if words.len() > MAX_PATHS {
        return Err(format!(
            "Message rejected: {} paths, more than the {MAX_PATHS} allowed.",
            words.len()
        ));
    }
    let Some(root) = root.filter(|r| r.starts_with('/')) else {
        return Err(format!(
            "Message rejected: the agent's project folder is unknown, so the path '{}' can't be checked.",
            words[0]
        ));
    };
    // The root by the text, and on disk when it is on this machine.
    let root_text = normalise(Path::new(root)).unwrap_or_else(|| PathBuf::from(root));
    let root_disk = std::fs::canonicalize(&root_text).ok();
    let home = std::env::var("HOME").ok();
    for word in words {
        let outside =
            || format!("Message rejected: the path '{word}' is outside the project {root}.");
        if word.chars().count() > MAX_PATH_LEN {
            return Err(format!(
                "Message rejected: a path is longer than {MAX_PATH_LEN} characters."
            ));
        }
        let parts = word.split('/').filter(|p| !p.is_empty()).count();
        let parents = word.split('/').filter(|p| *p == "..").count();
        if parts > MAX_PATH_PARTS || parents > MAX_PARENT_PARTS {
            return Err(format!(
                "Message rejected: the path '{word}' has too many parts (at most {MAX_PATH_PARTS}, and {MAX_PARENT_PARTS} '..')."
            ));
        }
        let joined = match word.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => match &home {
                Some(home) => PathBuf::from(format!("{home}{rest}")),
                None => return Err(outside()),
            },
            // `~user/...`: another user's home.
            Some(_) => return Err(outside()),
            None if word.starts_with('/') => PathBuf::from(word),
            None => root_text.join(word),
        };
        let resolved = normalise(&joined).ok_or_else(outside)?;
        if !resolved.starts_with(&root_text) {
            return Err(outside());
        }
        if let (Some(root_disk), Ok(real)) = (&root_disk, std::fs::canonicalize(&resolved))
            && !real.starts_with(root_disk)
        {
            return Err(outside());
        }
    }
    return Ok(());

    /// `path` with `.` and `..` resolved by the text alone; `None` when a
    /// `..` climbs above `/`.
    fn normalise(path: &Path) -> Option<PathBuf> {
        let mut out = PathBuf::new();
        for part in path.components() {
            match part {
                Component::RootDir | Component::Prefix(_) => out.push(part.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !out.pop() || out.as_os_str().is_empty() {
                        return None;
                    }
                }
                Component::Normal(p) => out.push(p),
            }
        }
        Some(out)
    }
}

/// Validate general agent chat input (plan 20261024_03 as amended): at
/// most [`MAX_MESSAGE_LEN`] characters, line breaks and tabs kept. It goes
/// to the general agent, never to a pane, so symbols are kept too (CHANGES.md
/// 2026-10-27, plan 20261027_04): what the general agent then sends to an
/// agent is checked by [`validate_message`] and [`check_paths`].
pub fn validate_general_input(text: &str) -> Result<String, String> {
    if let Some(bad) = text
        .chars()
        .find(|c| c.is_control() && *c != '\n' && *c != '\r' && *c != '\t')
    {
        return Err(format!(
            "Input rejected: control characters (found {bad:?}) are not permitted."
        ));
    }

    let mut lines = Vec::new();
    for line in text.lines() {
        let folded = line
            .split(' ')
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(folded);
    }
    let joined = lines.join("\n");
    let cleaned = joined.trim();
    if !cleaned.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err("Input cannot be empty.".to_string());
    }
    let len = cleaned.chars().count();
    if len > MAX_MESSAGE_LEN {
        return Err(format!(
            "Input rejected: {len} characters exceeds the {MAX_MESSAGE_LEN} character limit."
        ));
    }
    Ok(cleaned.to_string())
}

/// Normalize a command so the phrasing used when confirmation was requested
/// matches the string later handed to `run_command`: lowercased, trailing
/// sentence punctuation dropped, and internal whitespace collapsed.
pub fn normalize_command_for_match(cmd: &str) -> String {
    cmd.trim()
        .to_lowercase()
        .trim_end_matches(['.', ',', '!', '?'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Candidate spellings of a command to test against an armed confirmation.
///
/// The gate is armed from the assistant's spoken phrasing, which may differ
/// cosmetically from the string it later passes to `run_command` — most often a
/// slash command announced as "usage" and sent as `/usage`. Trying each variant
/// keeps the binding meaningful (an unrelated command still fails to match)
/// without blocking a command the user really did confirm.
pub fn confirmation_match_candidates(final_cmd: &str, raw_cmd: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for base in [
        normalize_command_for_match(final_cmd),
        final_cmd.to_string(),
        raw_cmd.to_string(),
    ] {
        let normalized = normalize_command_for_match(&base);
        let unslashed = normalized.trim_start_matches('/').to_string();
        let slashed = format!("/{unslashed}");
        for candidate in [base, normalized, unslashed, slashed] {
            if !candidate.is_empty() && !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

/// A parsed "Should I send X to pane Y?" turn from the assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationRequest {
    /// Pane the assistant named, if it gave a concrete pane id (e.g. `w2:p1`).
    pub pane_id: Option<String>,
    /// Command the assistant named, already normalized for gate matching.
    pub command: Option<String>,
}

/// Parse an assistant reply that asks the user to confirm sending a command.
///
/// Returns `None` when the reply is not a confirmation request. When it is, any
/// field the reply did not state unambiguously is left as `None`, which the
/// gate treats as "unbound" — a missed extraction degrades to the previous
/// permissive behaviour rather than blocking a legitimate command.
pub fn parse_confirmation_request(reply: &str) -> Option<ConfirmationRequest> {
    let lower = reply.to_lowercase();
    let is_asking = reply.trim_end().ends_with('?')
        || lower.contains("should i")
        || lower.contains("confirm")
        || lower.contains("are you sure");
    if !is_asking {
        return None;
    }

    let command = extract_quoted(reply)
        .or_else(|| extract_after_send(reply))
        .map(|c| normalize_command_for_match(&c))
        .filter(|c| !c.is_empty());

    Some(ConfirmationRequest {
        pane_id: extract_pane_id(reply),
        command,
    })
}

/// Pull the first quoted span out of a reply, skipping apostrophes that are
/// part of a word (`Claude's`) rather than a quote delimiter.
fn extract_quoted(reply: &str) -> Option<String> {
    const PAIRS: &[(char, char)] = &[
        ('"', '"'),
        ('\u{201C}', '\u{201D}'),
        ('\u{2018}', '\u{2019}'),
        ('`', '`'),
        ('\'', '\''),
    ];

    for (open, close) in PAIRS {
        let mut search_from = 0usize;
        while let Some(rel) = reply[search_from..].find(*open) {
            let start = search_from + rel;
            let before = reply[..start].chars().next_back();
            search_from = start + open.len_utf8();
            if before.is_some_and(|c| c.is_alphanumeric()) {
                continue; // contraction or possessive, not an opening quote
            }
            let rest = &reply[search_from..];
            let Some(end) = rest.find(*close) else { break };
            let after = rest[end + close.len_utf8()..].chars().next();
            if after.is_some_and(|c| c.is_alphanumeric()) {
                continue; // closing quote sits mid-word; treat as apostrophe
            }
            let inner = rest[..end].trim();
            if !inner.is_empty() {
                return Some(inner.to_string());
            }
        }
    }
    None
}

/// Fallback extraction for unquoted commands: the span between `send ` and the
/// target clause (`to`/`into`/`in`).
fn extract_after_send(reply: &str) -> Option<String> {
    const SEND: &str = "send ";
    let lower = reply.to_lowercase();
    // `to_lowercase` can change byte length, so index into the lowercased copy only.
    let start = lower.find(SEND)? + SEND.len();
    let rest = &lower[start..];

    let end = [" to ", " into ", " in "]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());

    let mut candidate = rest[..end].trim().trim_end_matches('?').trim();
    for filler in [
        "the message ",
        "the command ",
        "the input ",
        "message ",
        "command ",
        "input ",
    ] {
        if let Some(stripped) = candidate.strip_prefix(filler) {
            candidate = stripped.trim();
            break;
        }
    }

    // A long span means the sentence was not a simple "send X to Y"; leave the
    // gate unbound rather than binding it to a mis-parse.
    if candidate.is_empty() || candidate.split_whitespace().count() > 24 {
        None
    } else {
        Some(candidate.to_string())
    }
}

/// Find a concrete pane id (`w2:p1`) mentioned in the reply.
fn extract_pane_id(reply: &str) -> Option<String> {
    reply
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '!' | '"' | '\'' | '(' | ')'))
        .map(|token| token.trim_matches(|c: char| matches!(c, '.' | '?' | ':' | ';')))
        .find(|token| {
            token.contains(':')
                && token.len() >= 3
                && token.chars().all(|c| c.is_ascii_alphanumeric() || c == ':')
        })
        .map(|token| token.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_arbitrary_command_accepts_plain_speech() {
        assert_eq!(
            validate_arbitrary_command("  please run the tests, then report back.  ").unwrap(),
            "please run the tests, then report back"
        );
        assert_eq!(
            validate_arbitrary_command("what is the status of task 3?").unwrap(),
            "what is the status of task 3?"
        );
        assert_eq!(
            validate_arbitrary_command("don't stop, keep going").unwrap(),
            "don't stop, keep going"
        );
    }

    #[test]
    fn test_validate_arbitrary_command_removes_symbols_and_rejects_excessive_symbols() {
        // Single symbols followed by space are removed without triggering threshold.
        assert_eq!(
            validate_arbitrary_command("hello! world").unwrap(),
            "hello world"
        );
        assert_eq!(
            validate_arbitrary_command("don't stop - keep going").unwrap(),
            "don't stop keep going"
        );
        assert_eq!(
            validate_arbitrary_command("step 1; step 2: done.").unwrap(),
            "step 1 step 2: done"
        );

        // Runs or embedded symbols (such as paths or flags) trigger the threshold and are rejected.
        let err = validate_arbitrary_command("cat /etc/passwd").unwrap_err();
        assert!(err.contains("too many symbols"), "got: {err}");
        let err = validate_arbitrary_command("cargo test --all").unwrap_err();
        assert!(err.contains("too many symbols"), "got: {err}");
        let err = validate_arbitrary_command("!@#$%^&*()").unwrap_err();
        assert!(err.contains("too many symbols"), "got: {err}");
    }

    #[test]
    fn test_validate_arbitrary_command_rejects_non_ascii() {
        let err = validate_arbitrary_command("你好").unwrap_err();
        assert!(err.contains("must be ASCII only"));
    }

    #[test]
    fn test_validate_arbitrary_command_rejects_control_chars() {
        // A newline would submit a second line of input into the pane.
        let err = validate_arbitrary_command("continue\nrm everything").unwrap_err();
        assert!(err.contains("control characters"), "got: {err}");
        let err = validate_arbitrary_command("continue\r\nthen quit").unwrap_err();
        assert!(err.contains("control characters"), "got: {err}");
        let err = validate_arbitrary_command("clear\u{1b}[2J").unwrap_err();
        assert!(err.contains("control characters"), "got: {err}");
    }

    #[test]
    fn test_validate_arbitrary_command_rejects_empty_and_overlong() {
        assert!(
            validate_arbitrary_command("   ")
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            validate_arbitrary_command(" . , ")
                .unwrap_err()
                .contains("empty")
        );
        let long = "a ".repeat(MAX_ARBITRARY_COMMAND_LEN);
        assert!(
            validate_arbitrary_command(&long)
                .unwrap_err()
                .contains("exceeds")
        );
    }

    #[test]
    fn test_validate_message_accepts_strict_text_and_folds_spaces() {
        assert_eq!(
            validate_message("  Please run   the tests,  then tell me what failed.  ").unwrap(),
            "Please run the tests, then tell me what failed"
        );
        assert_eq!(
            validate_message("Is it ready? Yes: it's ready").unwrap(),
            "Is it ready? Yes: it's ready"
        );
    }

    #[test]
    fn test_validate_message_keeps_symbols_after_the_first_word() {
        assert_eq!(
            validate_message("Hello! Look in src/main.rs, then run cargo test --all.").unwrap(),
            "Hello! Look in src/main.rs, then run cargo test --all"
        );
        assert_eq!(
            validate_message("say \"hi\" (twice); ok?").unwrap(),
            "say \"hi\" (twice); ok?"
        );
        assert_eq!(validate_message("café crème").unwrap(), "café crème");
        // The first word may not be a command.
        for bad in ["/compact", "  /usage now", "!rm -rf /", "#remember this"] {
            let err = validate_message(bad).unwrap_err();
            assert!(err.contains("makes it a command"), "{bad:?}: {err}");
        }
        // A command character later on is only text.
        assert_eq!(
            validate_message("please run /compact later").unwrap(),
            "please run /compact later"
        );
    }

    #[test]
    fn test_check_paths_keeps_paths_inside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("docs/design")).unwrap();
        std::fs::write(root.join("docs/design/x.md"), "x").unwrap();
        let root_s = root.to_str().unwrap();
        let ok = |t: &str| check_paths(t, Some(root_s));
        // Relative, ./, new files, and absolute inside the root.
        assert!(ok("read docs/design/x.md and fix src/new.rs").is_ok());
        assert!(ok("see ./docs/design/x.md, then docs/../README.md.").is_ok());
        assert!(ok(&format!("open {root_s}/docs/design/x.md")).is_ok());
        assert!(ok("plain words, no paths; and/or both").is_ok());
        assert!(ok("see https://example.com/a/b").is_ok());
        // Outside: climbing out, absolute elsewhere, another home.
        for bad in [
            "read ../other/secret.md",
            "read docs/../../x",
            "cat /etc/passwd",
            "run /compact later",
            "look at ~other/notes",
            "open `../x`",
        ] {
            let err = ok(bad).unwrap_err();
            assert!(err.contains("outside the project"), "{bad:?}: {err}");
        }
        // No root: a path can't be checked; plain text still can.
        assert!(
            check_paths("read docs/x.md", None)
                .unwrap_err()
                .contains("unknown")
        );
        assert!(check_paths("no paths here", None).is_ok());
    }

    #[test]
    fn test_check_paths_bounds_pathological_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("a/b/c/d/e/f");
        std::fs::create_dir_all(&root).unwrap();
        let root_s = root.to_str().unwrap();
        // Four `..` that stay inside are fine; five are refused even inside.
        assert!(check_paths("x/y/z/w/../../../../ok.md", Some(root_s)).is_ok());
        let err = check_paths("v/x/y/z/w/../../../../../ok.md", Some(root_s)).unwrap_err();
        assert!(err.contains("too many parts"), "{err}");
        let deep = vec!["d"; MAX_PATH_PARTS + 1].join("/");
        assert!(
            check_paths(&deep, Some(root_s))
                .unwrap_err()
                .contains("too many parts")
        );
        let long = format!("x/{}", "a".repeat(MAX_PATH_LEN));
        assert!(
            check_paths(&long, Some(root_s))
                .unwrap_err()
                .contains("longer than")
        );
        let many = vec!["a/b"; MAX_PATHS + 1].join(" ");
        assert!(
            check_paths(&many, Some(root_s))
                .unwrap_err()
                .contains("paths, more than")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_paths_follows_a_link_out_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("secret.md"), "s").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.md"), root.join("link.md")).unwrap();
        let err = check_paths("read ./link.md", Some(root.to_str().unwrap())).unwrap_err();
        assert!(err.contains("outside the project"), "{err}");
    }

    #[test]
    fn test_count_triggering_symbols() {
        // Normal sentence with period and comma: 0 triggering symbols.
        assert_eq!(
            count_triggering_symbols("Hello, this is a test. Here is another sentence."),
            0
        );
        // Exclamation, question mark, colon followed by space: 0 triggering symbols.
        assert_eq!(
            count_triggering_symbols("Hey! Look at this: is it ready? Yes!"),
            0
        );
        // Contractions with apostrophe: 0 triggering symbols.
        assert_eq!(count_triggering_symbols("don't stop, it's fine."), 0);
        // Single hyphen followed by space: 0 triggering symbols.
        assert_eq!(count_triggering_symbols("item 1 - item 2"), 0);
        // Runs of symbols or symbols without space: trigger threshold.
        assert_eq!(count_triggering_symbols("cat /etc/passwd/root"), 3);
        assert_eq!(count_triggering_symbols("cargo test --all"), 2);
    }

    #[test]
    fn test_validate_message_refuses_control_characters() {
        for bad in [
            "one\ntwo",
            "one\r\ntwo",
            "one\ttwo",
            "clear\u{1b}[2J",
            "x\u{85}y",
        ] {
            let err = validate_message(bad).unwrap_err();
            assert!(err.contains("control characters"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn test_validate_general_input_accepts_multiline_and_preserves_punctuation() {
        assert_eq!(
            validate_general_input("line one\nline two\nline three.").unwrap(),
            "line one\nline two\nline three."
        );
        assert_eq!(
            validate_general_input("line one\r\nline two?").unwrap(),
            "line one\nline two?"
        );
        assert_eq!(
            validate_general_input("Claude_Continue Send").unwrap(),
            "Claude_Continue Send"
        );
        // Symbols are kept: the input goes to the general agent, not a pane.
        assert_eq!(
            validate_general_input("read docs/design/x.md --all \"now\"").unwrap(),
            "read docs/design/x.md --all \"now\""
        );
        for bad in ["clear\u{1b}[2J", "x\u{85}y", "a\x00b", "a\x07b"] {
            let err = validate_general_input(bad).unwrap_err();
            assert!(err.contains("control characters"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn test_validate_message_takes_4000_characters_and_not_4001() {
        let at_cap = "a".repeat(MAX_MESSAGE_LEN);
        assert_eq!(validate_message(&at_cap).unwrap(), at_cap);
        let err = validate_message(&"a".repeat(MAX_MESSAGE_LEN + 1)).unwrap_err();
        assert!(err.contains("4001 characters exceeds the 4000"), "{err}");
        // Folding comes first: spaces that fold away do not count.
        let padded = format!("{}   {}", "a".repeat(1999), "b".repeat(2000));
        assert_eq!(validate_message(&padded).unwrap().len(), MAX_MESSAGE_LEN);
        assert!(validate_message(" . , ").unwrap_err().contains("empty"));
    }

    #[test]
    fn test_normalize_command_for_match() {
        assert_eq!(
            normalize_command_for_match("  Run   The Tests.  "),
            "run the tests"
        );
        assert_eq!(normalize_command_for_match("/Usage"), "/usage");
        assert_eq!(normalize_command_for_match("continue?"), "continue");
    }

    #[test]
    fn test_confirmation_match_candidates_bridge_slash_command_phrasing() {
        // Announced as "usage", sent as "/usage".
        let candidates = confirmation_match_candidates("/usage", "/usage");
        assert!(candidates.contains(&"usage".to_string()));
        assert!(candidates.contains(&"/usage".to_string()));

        // Announced as "/compact", sent as "compact".
        let candidates = confirmation_match_candidates("compact", "compact");
        assert!(candidates.contains(&"/compact".to_string()));

        // An unrelated command never produces the confirmed spelling.
        let candidates = confirmation_match_candidates("delete the repo", "delete the repo.");
        assert!(!candidates.contains(&"please run the tests".to_string()));
    }

    #[test]
    fn test_parse_confirmation_request_quoted_command_with_pane() {
        let req = parse_confirmation_request(
            "Should I send 'please check test results' to Claude in pane w2:p1?",
        )
        .expect("should parse as a confirmation request");
        assert_eq!(req.command.as_deref(), Some("please check test results"));
        assert_eq!(req.pane_id.as_deref(), Some("w2:p1"));
    }

    #[test]
    fn test_parse_confirmation_request_unquoted_slash_command() {
        let req = parse_confirmation_request("Should I send /usage to Claude in Highbay?")
            .expect("should parse as a confirmation request");
        assert_eq!(req.command.as_deref(), Some("/usage"));
        assert_eq!(req.pane_id, None);
    }

    #[test]
    fn test_parse_confirmation_request_strips_filler_and_target_clause() {
        let req = parse_confirmation_request("Should I send the message run the tests to Agy?")
            .expect("should parse as a confirmation request");
        assert_eq!(req.command.as_deref(), Some("run the tests"));
    }

    #[test]
    fn test_parse_confirmation_request_ignores_possessive_apostrophe() {
        let req =
            parse_confirmation_request("Should I send /compact to Claude's session in Highbay?")
                .expect("should parse as a confirmation request");
        assert_eq!(req.command.as_deref(), Some("/compact"));
    }

    #[test]
    fn test_parse_confirmation_request_non_question_returns_none() {
        assert!(parse_confirmation_request("Sent /usage to Claude in Highbay.").is_none());
        assert!(parse_confirmation_request("Agy is running the test suite.").is_none());
    }

    #[test]
    fn test_parse_confirmation_request_unparsable_command_leaves_gate_unbound() {
        let req = parse_confirmation_request("Are you sure?").expect("is a confirmation request");
        assert_eq!(req.command, None);
        assert_eq!(req.pane_id, None);
    }
}
