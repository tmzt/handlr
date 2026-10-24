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
    /// A row's one-line status while its feature is on, and while it is
    /// off (`enabled_text`, `disabled_text`).
    pub enabled_text: Option<&'static str>,
    pub disabled_text: Option<&'static str>,
    /// A Markdown link, drawn last, after any vendor addition.
    pub link: Option<&'static str>,
    pub slots: &'static [&'static str],
    /// `(vendor, text)`: appended on that vendor's phones.
    pub vendors: &'static [(&'static str, &'static str)],
}

impl CopyText {
    /// A row's status line for its feature being `on`, if it has one.
    pub fn status(&self, on: bool) -> Option<&'static str> {
        if on {
            self.enabled_text
        } else {
            self.disabled_text
        }
    }

    /// A row's label or a modal's title: its title, else its text.
    pub fn label(&self) -> &'static str {
        self.title.unwrap_or(self.text)
    }

    /// Its text, then the addition for `vendor` when there is one, then its
    /// link last.
    pub fn body(&self, vendor: Option<&str>) -> String {
        let addition = vendor.and_then(|v| {
            self.vendors
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(v))
                .map(|(_, text)| *text)
        });
        let mut out = self.text.to_string();
        if let Some(more) = addition {
            out.push_str("\n\n");
            out.push_str(more);
        }
        if let Some(link) = self.link {
            out.push_str("\n\n");
            out.push_str(link);
        }
        out
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
        assert!(base.starts_with(BACKGROUND.text));
        assert!(!base.contains("Freecess"), "One UI's text is Samsung's");
        let samsung = BACKGROUND.body(Some("samsung"));
        assert!(
            samsung.starts_with(BACKGROUND.text),
            "appended, never replacing"
        );
        assert!(samsung.contains("Freecess"));
        // The link is last, after the vendor's addition.
        let link = BACKGROUND.link.expect("a link");
        assert!(link.starts_with("[Android documentation]("));
        assert!(base.ends_with(link) && samsung.ends_with(link));
        assert!(samsung.find("Freecess") < samsung.find(link));
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
