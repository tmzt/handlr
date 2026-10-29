//! The app's copy, from the files in `copy/` (`SETTINGS_COPY.md`,
//! `PAIRING_COPY.md`) at build time
//! (docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decision 6).
//!
//! `build.rs` generates one `pub const` per `# [Copy] <key>` of every file, a
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

include!(concat!(env!("OUT_DIR"), "/copy.rs"));

/// The copy files, as `build.rs` reads them, for the tests.
#[cfg(test)]
pub const FILES: [(&str, &str); 3] = [
    (
        "copy/DRAWER_COPY.md",
        include_str!("../../copy/DRAWER_COPY.md"),
    ),
    (
        "copy/PAIRING_COPY.md",
        include_str!("../../copy/PAIRING_COPY.md"),
    ),
    (
        "copy/SETTINGS_COPY.md",
        include_str!("../../copy/SETTINGS_COPY.md"),
    ),
];

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
        let text = REMOVE_REMOTE_NEXT.fill(None, &[("next", "m4")]);
        assert!(text.contains("connects to m4 next."), "{text}");
        assert!(!text.contains("{{"));
        assert_eq!(REMOVE_REMOTE_NEXT.slots, ["next"]);
        assert_eq!(REMOVE_REMOTE_OFFLINE.slots, ["name", "device"]);
        // Without its value, a slot is left as written.
        assert!(REMOVE_REMOTE_NEXT.fill(None, &[]).contains("{{"));
    }

    #[test]
    fn the_shipped_files_generate() {
        let keys = |name: &str| -> Vec<String> {
            let (_, md) = FILES.iter().find(|(n, _)| n.ends_with(name)).expect(name);
            copy_gen::parse(md)
                .expect("it parses")
                .into_iter()
                .map(|e| e.key)
                .collect()
        };
        let settings = keys("SETTINGS_COPY.md");
        assert!(settings.iter().any(|k| k == "background"));
        // The notification's templates, which notify.rs fills.
        assert!(settings.iter().any(|k| k.starts_with("notification_")));
        assert!(settings.iter().any(|k| k == "remote_details_fingerprint"));
        // The pairing window's keys are in their own file, and only there.
        let pairing = keys("PAIRING_COPY.md");
        assert!(!pairing.is_empty());
        assert!(
            pairing
                .iter()
                .all(|k| k.starts_with("pairing_") || k.starts_with("remote_add")),
            "{pairing:?}"
        );
        assert!(pairing.iter().any(|k| k == "remote_add"));
        assert!(
            !settings
                .iter()
                .any(|k| k.starts_with("pairing_") || k.starts_with("remote_add")),
            "{settings:?}"
        );
        // Both, as build.rs reads them (in name order), into one module.
        let files: Vec<(String, String)> = FILES
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect();
        let code = copy_gen::generate_files(&files, &[]).expect("one module");
        assert!(code.contains("pub const PAIRING_DONE: CopyText"));
        assert!(code.contains("pub const REMOTES_EMPTY: CopyText"));
        assert!(code.contains("pub const DRAWER_CONNECTING: CopyText"));
        assert_eq!(
            DRAWER_UNREACHABLE.fill(None, &[("nickname", "m4"), ("seconds", "8")]),
            "Can't reach m4 · retrying in 8 s"
        );
        // The generated consts are the files' text.
        assert_eq!(REMOTES_EMPTY.text, "No remotes paired");
        assert_eq!(PAIRING_FINGERPRINT.text, "Daemon fingerprint");
    }

    #[test]
    fn there_is_no_copy_left_at_the_old_path() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!dir.join("SETTINGS_COPY.md").exists());
        let mut found: Vec<String> = std::fs::read_dir(dir.join("copy"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "DRAWER_COPY.md",
                "PAIRING_COPY.md",
                "PREVIEW_COPY.md",
                "SETTINGS_COPY.md"
            ]
        );
    }
}
