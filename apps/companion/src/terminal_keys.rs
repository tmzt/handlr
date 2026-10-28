//! Typed keys as `terminal.input`'s `keys` (plan 20261027_07, decision 6,
//! step 6's app half).
//!
//! The daemon reads `keys` as whitespace-separated tokens: a named key
//! (`enter`, `escape`, `tab`, `backspace`, `space`, `up`, `down`, `left`,
//! `right`), `ctrl+<a-z>`, or printable text, one key per character. A
//! literal space is the `space` token, since whitespace separates them.
//! The app sends every printable character as its own one-character token,
//! so a typed word such as "tab" is never read as the Tab key. herdr's
//! `pane.send_keys` takes the same tokens, one key each.

use libmorph::Key;

/// The token for one typed character, or `None` for a control character
/// the daemon does not take.
pub fn char_token(c: char) -> Option<String> {
    let token = match c {
        ' ' => "space".to_string(),
        '\r' | '\n' => "enter".to_string(),
        '\t' => "tab".to_string(),
        '\u{8}' | '\u{7f}' => "backspace".to_string(),
        '\u{1b}' => "escape".to_string(),
        // Ctrl+A to Ctrl+Z, but for those above (Ctrl+H, Ctrl+I, Ctrl+J
        // and Ctrl+M are Backspace, Tab and Enter).
        '\u{1}'..='\u{1a}' => format!("ctrl+{}", (b'a' + c as u8 - 1) as char),
        c if c.is_control() => return None,
        c if c.is_whitespace() => "space".to_string(),
        c => c.to_string(),
    };
    Some(token)
}

/// The tokens for `text`, joined with single spaces; characters with no
/// token are left out.
pub fn text_keys(text: &str) -> String {
    text.chars()
        .filter_map(char_token)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A physical key's token: a character's, or Escape's. `None` for the
/// system Back and for what the daemon does not take.
pub fn key_token(key: Key) -> Option<String> {
    match key {
        Key::Character(c) => char_token(c),
        Key::Escape => Some("escape".to_string()),
        _ => None,
    }
}

/// The keys that turn the soft keyboard's text `old` into `new`: a
/// Backspace for each character after their common start, then the new
/// characters. The soft keyboard reports the whole text after each edit
/// (a composition or autocorrect replaces a word), so this is how its
/// edits reach the pane.
pub fn edit_keys(old: &str, new: &str) -> String {
    let common = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let removed = old.chars().count() - common;
    let added: String = new.chars().skip(common).collect();
    let mut tokens: Vec<String> = std::iter::repeat_n("backspace".to_string(), removed).collect();
    let added = text_keys(&added);
    if !added.is_empty() {
        tokens.push(added);
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_character_is_its_own_token_and_a_space_is_space() {
        assert_eq!(text_keys("ls -la"), "l s space - l a");
        // A typed word is letters, never the named key.
        assert_eq!(text_keys("tab"), "t a b");
        assert_eq!(text_keys("a+b/c"), "a + b / c");
        assert_eq!(text_keys("ü"), "ü");
    }

    #[test]
    fn named_keys_and_control_characters() {
        assert_eq!(text_keys("ls\n"), "l s enter");
        assert_eq!(char_token('\r').as_deref(), Some("enter"));
        assert_eq!(char_token('\t').as_deref(), Some("tab"));
        assert_eq!(char_token('\u{8}').as_deref(), Some("backspace"));
        assert_eq!(char_token('\u{7f}').as_deref(), Some("backspace"));
        assert_eq!(char_token('\u{1b}').as_deref(), Some("escape"));
        assert_eq!(char_token('\u{3}').as_deref(), Some("ctrl+c"));
        assert_eq!(char_token('\u{1}').as_deref(), Some("ctrl+a"));
        assert_eq!(char_token('\u{1a}').as_deref(), Some("ctrl+z"));
        // Other control characters are not sent.
        assert_eq!(char_token('\u{0}'), None);
        assert_eq!(char_token('\u{1c}'), None);
        assert_eq!(key_token(Key::Escape).as_deref(), Some("escape"));
        assert_eq!(key_token(Key::Character('x')).as_deref(), Some("x"));
        assert_eq!(key_token(Key::Back), None);
    }

    #[test]
    fn the_soft_keyboards_edits_become_keys() {
        assert_eq!(edit_keys("", "l"), "l");
        assert_eq!(edit_keys("l", "ls"), "s");
        assert_eq!(edit_keys("ls", "ls\n"), "enter");
        assert_eq!(edit_keys("ls", "l"), "backspace");
        // Autocorrect replaces a word: back to where they differ, retyped.
        assert_eq!(
            edit_keys("git sttus", "git status"),
            "backspace backspace backspace a t u s"
        );
        assert_eq!(edit_keys("same", "same"), "");
    }
}
