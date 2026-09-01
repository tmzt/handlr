//! Command policy helpers shared by the transport permission gate and the
//! speech-to-speech processor.
//!
//! Agents configured with `allow_arbitrary_commands` may receive free-form
//! spoken instructions that are not on their allow list. Those strings are
//! typed verbatim into the agent's prompt, so they are restricted to a
//! conservative ASCII whitelist before they are sent.

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
            let err = validate_arbitrary_command(injection)
                .unwrap_err_or_panic(injection);
            assert!(
                err.contains("is not permitted"),
                "expected rejection for {injection:?}, got: {err}"
            );
        }
    }

    #[test]
    fn test_validate_arbitrary_command_rejects_empty_and_overlong() {
        assert!(validate_arbitrary_command("   ").unwrap_err().contains("empty"));
        assert!(validate_arbitrary_command(" . , ").unwrap_err().contains("empty"));
        let long = "a ".repeat(MAX_ARBITRARY_COMMAND_LEN);
        assert!(validate_arbitrary_command(&long).unwrap_err().contains("exceeds"));
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
