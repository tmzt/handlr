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

/// Characters permitted in an arbitrary command, in addition to ASCII
/// alphanumerics and spaces.
const EXTRA_ALLOWED_CHARS: &[char] = &['.', ',', '\'', '?', ':'];

fn is_allowed_arbitrary_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == ' ' || EXTRA_ALLOWED_CHARS.contains(&c)
}

/// Validate a free-form command for an agent with `allow_arbitrary_commands`.
///
/// Returns the cleaned command (surrounding whitespace and trailing sentence
/// punctuation removed) or a human-readable rejection reason that is surfaced
/// back to the model as the tool error.
///
/// Rejects, in order: non-ASCII text, embedded control characters (which would
/// otherwise inject a second line of input into the pane), `/` and `-` (so a
/// spoken phrase can never become a slash command or a CLI flag), empty or
/// overlong strings, and anything outside the character whitelist.
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
    if trimmed.contains('/') || trimmed.contains('-') {
        return Err(format!(
            "Arbitrary command '{}' rejected: '/' and '-' are not permitted in arbitrary commands.",
            trimmed
        ));
    }

    let cleaned = trimmed.trim_end_matches(['.', ',']).trim();
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
    if let Some(bad) = cleaned.chars().find(|c| !is_allowed_arbitrary_char(*c)) {
        return Err(format!(
            "Arbitrary command '{}' rejected: character '{}' is not permitted. Only letters, digits, spaces and . , ' ? : are allowed.",
            cleaned, bad
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
    fn test_validate_arbitrary_command_rejects_slash_and_dash() {
        let err = validate_arbitrary_command("cat /etc/passwd").unwrap_err();
        assert!(err.contains("'/' and '-' are not permitted"));
        let err = validate_arbitrary_command("cargo test --all").unwrap_err();
        assert!(err.contains("'/' and '-' are not permitted"));
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
    fn test_validate_arbitrary_command_rejects_shell_metacharacters() {
        for injection in [
            "run tests; rm everything",
            "run tests && quit",
            "run tests | tee out",
            "echo $HOME",
            "echo `whoami`",
            "run tests > out.txt",
            "run tests (again)",
            "run tests {x}",
            "run tests \\ more",
            "run tests \"quoted\"",
        ] {
            let err = validate_arbitrary_command(injection).unwrap_err_or_panic(injection);
            assert!(
                err.contains("is not permitted"),
                "expected rejection for {injection:?}, got: {err}"
            );
        }
    }

    #[test]
    fn test_validate_arbitrary_command_rejects_empty_and_overlong() {
        assert!(validate_arbitrary_command("   ")
            .unwrap_err()
            .contains("empty"));
        assert!(validate_arbitrary_command(" . , ")
            .unwrap_err()
            .contains("empty"));
        let long = "a ".repeat(MAX_ARBITRARY_COMMAND_LEN);
        assert!(validate_arbitrary_command(&long)
            .unwrap_err()
            .contains("exceeds"));
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

    /// Small helper so the injection table above reads cleanly.
    trait UnwrapErrOrPanic {
        fn unwrap_err_or_panic(self, ctx: &str) -> String;
    }
    impl UnwrapErrOrPanic for Result<String, String> {
        fn unwrap_err_or_panic(self, ctx: &str) -> String {
            match self {
                Ok(v) => panic!("expected {ctx:?} to be rejected, but it was accepted as {v:?}"),
                Err(e) => e,
            }
        }
    }
}
