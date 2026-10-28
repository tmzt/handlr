//! The copy files in `copy/` (`SETTINGS_COPY.md`, `PAIRING_COPY.md`) into
//! one `copy.rs` (docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md,
//! decision 6, and CHANGES.md 2026-10-24 13:02 EDT, vendor additions).
//!
//! `build.rs` includes this file, and so do the library's tests, which run
//! its unit tests. Each `# [Copy] <key>` object has an optional `title`, a
//! `text` (Markdown, which may hold `{{ [abc] name }}` slots), optional
//! `enabled_text` and `disabled_text` (a row's one-line status, by whether
//! its feature is on), an optional `link` (a Markdown link, drawn last,
//! after any vendor addition), and one member per slot it declares. `# [Copy] <key>.<vendor>` (the vendor
//! lower-cased) is a vendor subkey: on that vendor's phones its text is
//! appended after the base key's.
//!
//! The build fails when a key the code uses is missing, a key is defined
//! twice (in one file or across them), a slot is not declared, a subkey
//! has no base key, or a block does not parse.

use std::collections::BTreeMap;

/// The tag that marks a piece of copy.
pub const TAG: &str = "Copy";

/// One key, with its vendor additions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub title: Option<String>,
    pub text: String,
    /// A row's status line while its feature is on, and while it is off.
    pub enabled_text: Option<String>,
    pub disabled_text: Option<String>,
    /// A Markdown link, drawn last.
    pub link: Option<String>,
    /// The slots it declares, its subkeys' included.
    pub slots: Vec<String>,
    /// `(vendor, text)`, appended on that vendor's phones.
    pub vendors: Vec<(String, String)>,
}

/// An object as written, before subkeys are joined to their keys.
struct Raw {
    key: String,
    vendor: Option<String>,
    title: Option<String>,
    text: String,
    enabled_text: Option<String>,
    disabled_text: Option<String>,
    link: Option<String>,
    slots: Vec<String>,
}

/// The slot names in `text`, in order.
pub fn slots_in(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or("a `{{` is not closed")?;
        let inner = after[..end].trim();
        let name = match inner.strip_prefix('[').and_then(|s| s.split_once(']')) {
            Some((_, name)) => name.trim(),
            None => inner,
        };
        out.push(name.to_string());
        rest = &after[end + 2..];
    }
    Ok(out)
}

fn is_key(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Every piece of copy in `md`, its subkeys joined to their keys.
pub fn parse(md: &str) -> Result<Vec<Entry>, String> {
    let stream = libtypedmd::parse_with(md, &libtypedmd::ParseOptions::strict())
        .map_err(|e| format!("does not parse: {e:?}"))?;
    let mut raws = Vec::new();
    for document in &stream.documents {
        collect(&document.nodes, &mut raws)?;
    }
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut order = Vec::new();
    for raw in raws.iter().filter(|r| r.vendor.is_none()) {
        let used = slots_in(&raw.text)
            .and_then(|mut s| {
                s.extend(slots_in(raw.title.as_deref().unwrap_or(""))?);
                Ok(s)
            })
            .map_err(|e| format!("`{}`: {e}", raw.key))?;
        if let Some(slot) = used.iter().find(|s| !raw.slots.contains(s)) {
            return Err(format!("`{}`: the slot `{slot}` is not declared", raw.key));
        }
        if entries.contains_key(&raw.key) {
            return Err(format!("`{}` is defined twice", raw.key));
        }
        order.push(raw.key.clone());
        entries.insert(
            raw.key.clone(),
            Entry {
                key: raw.key.clone(),
                title: raw.title.clone(),
                enabled_text: raw.enabled_text.clone(),
                disabled_text: raw.disabled_text.clone(),
                link: raw.link.clone(),
                text: raw.text.clone(),
                slots: raw.slots.clone(),
                vendors: Vec::new(),
            },
        );
    }
    for raw in &raws {
        let Some(vendor) = &raw.vendor else {
            continue;
        };
        let name = format!("{}.{vendor}", raw.key);
        let base = entries
            .get_mut(&raw.key)
            .ok_or_else(|| format!("`{name}` has no base key `{}`", raw.key))?;
        if raw.title.is_some()
            || raw.enabled_text.is_some()
            || raw.disabled_text.is_some()
            || raw.link.is_some()
        {
            return Err(format!("`{name}`: a vendor subkey has only a text"));
        }
        let used = slots_in(&raw.text).map_err(|e| format!("`{name}`: {e}"))?;
        if let Some(slot) = used
            .iter()
            .find(|s| !raw.slots.contains(s) && !base.slots.contains(s))
        {
            return Err(format!("`{name}`: the slot `{slot}` is not declared"));
        }
        if base.vendors.iter().any(|(v, _)| v == vendor) {
            return Err(format!("`{name}` is defined twice"));
        }
        for slot in &raw.slots {
            if !base.slots.contains(slot) {
                base.slots.push(slot.clone());
            }
        }
        base.vendors.push((vendor.clone(), raw.text.clone()));
    }
    Ok(order
        .into_iter()
        .filter_map(|k| entries.remove(&k))
        .collect())
}

/// Copy objects among `nodes`, at any depth.
fn collect(nodes: &[libtypedmd::Node], out: &mut Vec<Raw>) -> Result<(), String> {
    for node in nodes {
        let Some(object) = node.as_object() else {
            continue;
        };
        if object.tag.as_deref() != Some(TAG) {
            collect(&node.nodes, out)?;
            continue;
        }
        let head = object.head.render().trim().to_string();
        let (key, vendor) = match head.split_once('.') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (head.clone(), None),
        };
        if !is_key(&key) || vendor.as_deref().is_some_and(|v| !is_key(v)) {
            return Err(format!(
                "`{head}`: a key is lower-case letters, digits and `_`, and a vendor is too"
            ));
        }
        let mut title = None;
        let mut text = None;
        let mut enabled_text = None;
        let mut disabled_text = None;
        let mut link = None;
        let mut slots = Vec::new();
        for child in &node.nodes {
            let Some(member) = child.as_member() else {
                continue;
            };
            let value = member.rhs.as_ref().map(|t| t.render()).unwrap_or_default();
            match member.name.as_str() {
                "title" => title = Some(value.trim().to_string()),
                "text" => text = Some(value.trim().to_string()),
                "enabled_text" => enabled_text = Some(value.trim().to_string()),
                "disabled_text" => disabled_text = Some(value.trim().to_string()),
                "link" => link = Some(value.trim().to_string()),
                slot => slots.push(slot.to_string()),
            }
        }
        let text = text.ok_or_else(|| format!("`{head}` has no `text`"))?;
        out.push(Raw {
            key,
            vendor,
            title,
            text,
            enabled_text,
            disabled_text,
            link,
            slots,
        });
    }
    Ok(())
}

/// The keys `sources` use, as `copy::<KEY>`, lower-cased.
pub fn used_keys(sources: &[String]) -> Vec<String> {
    let mut keys = Vec::new();
    for source in sources {
        let mut rest = source.as_str();
        while let Some(at) = rest.find("copy::") {
            rest = &rest[at + "copy::".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            // A const is all capitals; a type or function is not.
            let next = rest[name.len()..].chars().next();
            if name.is_empty() || next.is_some_and(|c| c.is_alphanumeric()) {
                continue;
            }
            let key = name.to_ascii_lowercase();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys.sort();
    keys
}

/// `copy.rs` for one file `md`, as [`generate_files`] does (the tests').
#[allow(dead_code)]
pub fn generate(md: &str, used: &[String]) -> Result<String, String> {
    generate_files(&[("COPY.md".to_string(), md.to_string())], used)
}

/// `copy.rs` for `files` (each its name and text): one `pub const` per key,
/// of the type `CopyText` the including module defines, in file order.
/// Fails, naming the file, as [`parse`] does; when a key is in two files;
/// or when a key in `used` is in none.
pub fn generate_files(files: &[(String, String)], used: &[String]) -> Result<String, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut from: Vec<&str> = Vec::new();
    for (name, md) in files {
        for entry in parse(md).map_err(|e| format!("{name}: {e}"))? {
            if let Some(i) = entries.iter().position(|e| e.key == entry.key) {
                return Err(format!(
                    "`{}` is defined in both {} and {name}",
                    entry.key, from[i]
                ));
            }
            entries.push(entry);
            from.push(name);
        }
    }
    if let Some(missing) = used.iter().find(|k| !entries.iter().any(|e| &e.key == *k)) {
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        return Err(format!(
            "there is no `# [Copy] {missing}` in {}, which the code uses",
            names.join(" or ")
        ));
    }
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let mut out = format!(
        "// Generated by build.rs from {}: do not edit.\n",
        names.join(", ")
    );
    for e in &entries {
        let vendors: Vec<String> = e
            .vendors
            .iter()
            .map(|(v, t)| format!("({v:?}, {t:?})"))
            .collect();
        out.push_str(&format!(
            "\npub const {}: CopyText = CopyText {{\n    key: {:?},\n    title: {:?},\n    text: {:?},\n    enabled_text: {:?},\n    disabled_text: {:?},\n    link: {:?},\n    slots: &{:?},\n    vendors: &[{}],\n}};\n",
            e.key.to_ascii_uppercase(),
            e.key,
            e.title,
            e.text,
            e.enabled_text,
            e.disabled_text,
            e.link,
            e.slots,
            vendors.join(", "),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "Prose is ignored.\n\n# [Copy] unpair\n\n- title: Unpair\n- [abc] daemon: the daemon's name\n- text: |\n    Unpair from {{ [abc] daemon }}?\n\n# [Copy] battery\n\n- text: |\n    Battery **text**.\n\n# [Copy] battery.samsung\n\n- text: |\n    One UI too.\n";

    fn used(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn keys_titles_slots_and_vendor_subkeys_parse() {
        let entries = parse(MD).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "unpair");
        assert_eq!(entries[0].title.as_deref(), Some("Unpair"));
        assert_eq!(entries[0].text, "Unpair from {{ [abc] daemon }}?");
        assert_eq!(entries[0].slots, ["daemon"]);
        assert_eq!(entries[1].text, "Battery **text**.");
        assert_eq!(
            entries[1].vendors,
            [("samsung".to_string(), "One UI too.".to_string())]
        );
        let code = generate(MD, &used(&["unpair", "battery"])).unwrap();
        assert!(code.contains("pub const UNPAIR: CopyText"), "{code}");
        assert!(code.contains("vendors: &[(\"samsung\", \"One UI too.\")]"));
    }

    #[test]
    fn the_build_fails_for_a_missing_used_key() {
        let err = generate(MD, &used(&["unpair", "delete_data"])).unwrap_err();
        assert!(err.contains("delete_data"), "{err}");
    }

    #[test]
    fn the_build_fails_for_an_undeclared_slot() {
        let md = "# [Copy] a\n\n- text: |\n    Hi {{ [abc] who }}.\n";
        assert!(parse(md).unwrap_err().contains("`who` is not declared"));
        let md = "# [Copy] a\n\n- text: |\n    Hi.\n\n# [Copy] a.google\n\n- text: |\n    And {{ [#] n }}.\n";
        assert!(parse(md).unwrap_err().contains("`n` is not declared"));
        let md = "# [Copy] a\n\n- text: |\n    Hi {{ [abc] who\n";
        assert!(parse(md).is_err());
    }

    #[test]
    fn the_build_fails_for_a_subkey_without_its_key_or_a_bad_block() {
        let md = "# [Copy] a.samsung\n\n- text: |\n    x\n";
        assert!(parse(md).unwrap_err().contains("no base key `a`"));
        let md = "# [Copy] a\n\n- title: no text\n";
        assert!(parse(md).unwrap_err().contains("no `text`"));
        let md = "# [Copy] Bad-Key\n\n- text: |\n    x\n";
        assert!(parse(md).is_err());
    }

    #[test]
    fn several_files_make_one_module_and_a_key_is_defined_once() {
        let files = |b: &str| {
            vec![
                ("SETTINGS_COPY.md".to_string(), MD.to_string()),
                ("PAIRING_COPY.md".to_string(), b.to_string()),
            ]
        };
        let pairing = "# [Copy] pairing_done\n\n- text: Done\n";
        let code = generate_files(&files(pairing), &used(&["unpair", "pairing_done"])).unwrap();
        assert!(code.contains("pub const UNPAIR: CopyText"), "{code}");
        assert!(code.contains("pub const PAIRING_DONE: CopyText"), "{code}");
        assert!(
            code.contains("from SETTINGS_COPY.md, PAIRING_COPY.md"),
            "{code}"
        );
        // In both files: refused, naming them.
        let twice = "# [Copy] battery\n\n- text: Again\n";
        let err = generate_files(&files(twice), &[]).unwrap_err();
        assert!(
            err.contains("`battery` is defined in both SETTINGS_COPY.md and PAIRING_COPY.md"),
            "{err}"
        );
        // A bad block names its file; a missing key names them all.
        let err = generate_files(&files("# [Copy] a\n\n- title: x\n"), &[]).unwrap_err();
        assert!(err.starts_with("PAIRING_COPY.md: "), "{err}");
        let err = generate_files(&files(pairing), &used(&["nowhere"])).unwrap_err();
        assert!(
            err.contains("in SETTINGS_COPY.md or PAIRING_COPY.md"),
            "{err}"
        );
    }

    #[test]
    fn used_keys_are_the_consts_named_after_copy() {
        let src = "let a = copy::UNPAIR.fill(None, &[]); copy::CopyText; copy::BATTERY.text(None); copy::UNPAIR".to_string();
        assert_eq!(used_keys(&[src]), ["battery", "unpair"]);
    }
}
