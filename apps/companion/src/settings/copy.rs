//! The settings screen's copy, from `SETTINGS_COPY.md` at build time
//! (docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decision 6).
//!
//! `build.rs` generates one `pub const` per `# [Copy] <key>`, a
//! [`CopyText`], named as the key in capitals; a key the code names here
//! and the file lacks fails the build. A vendor subkey's text is appended
//! after its key's on that vendor's phones ([`CopyText::body`]), with the
//! vendor from [`super::build_info`].

/// One piece of copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyText {
    pub key: &'static str,
    pub title: Option<&'static str>,
    /// Markdown, with its slots unfilled.
    pub text: &'static str,
    pub slots: &'static [&'static str],
    /// `(vendor, text)`: appended on that vendor's phones.
    pub vendors: &'static [(&'static str, &'static str)],
}

impl CopyText {
    /// A row's label or a modal's title: its title, else its text.
    pub fn label(&self) -> &'static str {
        self.title.unwrap_or(self.text)
    }

    /// Its text, and after it the addition for `vendor` when there is one.
    pub fn body(&self, vendor: Option<&str>) -> String {
        let addition = vendor.and_then(|v| {
            self.vendors
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(v))
                .map(|(_, text)| *text)
        });
        match addition {
            Some(more) => format!("{}\n\n{more}", self.text),
            None => self.text.to_string(),
        }
    }

    /// [`CopyText::body`] with each `{{ [sigil] name }}` replaced from
    /// `values`. A slot without a value is left as written.
    pub fn fill(&self, vendor: Option<&str>, values: &[(&str, &str)]) -> String {
        let text = self.body(vendor);
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                break;
            };
            out.push_str(&rest[..start]);
            let inner = after[..end].trim();
            let name = match inner.strip_prefix('[').and_then(|s| s.split_once(']')) {
                Some((_, name)) => name.trim(),
                None => inner,
            };
            match values.iter().find(|(n, _)| *n == name) {
                Some((_, value)) => out.push_str(value),
                None => {
                    log::warn!("settings copy `{}`: no value for `{name}`", self.key);
                    out.push_str(&rest[start..start + 2 + end + 2]);
                }
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }
}

include!(concat!(env!("OUT_DIR"), "/settings_copy.rs"));

/// The generator's unit tests, run with the library's.
#[cfg(test)]
#[path = "../../build/copy_gen.rs"]
mod copy_gen;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vendor_addition_is_appended_and_none_is_the_base_text() {
        let base = BACKGROUND.body(None);
        assert_eq!(base, BACKGROUND.text);
        assert!(!base.contains("Freecess"), "One UI's text is Samsung's");
        let samsung = BACKGROUND.body(Some("samsung"));
        assert!(samsung.starts_with(&base), "appended, never replacing");
        assert!(samsung.contains("Freecess"));
        assert_eq!(BACKGROUND.body(Some("Samsung")), samsung);
        assert_eq!(BACKGROUND.body(Some("google")), base);
    }

    #[test]
    fn slots_fill() {
        let text = UNPAIR.fill(None, &[("daemon", "m4")]);
        assert!(text.contains("from m4?"), "{text}");
        assert!(!text.contains("{{"));
        assert_eq!(UNPAIR.slots, ["daemon"]);
        // Without its value, a slot is left as written.
        assert!(UNPAIR.fill(None, &[]).contains("{{"));
    }

    #[test]
    fn the_shipped_file_generates() {
        let md = include_str!("../../SETTINGS_COPY.md");
        let entries = copy_gen::parse(md).expect("it parses");
        assert!(entries.iter().any(|e| e.key == "background"));
        // The notification's templates, which notify.rs fills.
        assert!(entries.iter().any(|e| e.key.starts_with("notification_")));
    }
}
