//! `SETTINGS_COPY.md` into `settings_copy.rs`
//! (docs/design/20261024_04_MARKDOWN_RENDERER_PLAN.md, decision 6, and
//! CHANGES.md 2026-10-24 13:02 EDT, vendor additions).
//!
//! `build.rs` includes this file, and so do the library's tests, which run
//! its unit tests. Each `# [Copy] <key>` object has an optional `title`, a
//! `text` (Markdown, which may hold `{{ [abc] name }}` slots), and one
//! member per slot it declares. `# [Copy] <key>.<vendor>` (the vendor
//! lower-cased) is a vendor subkey: on that vendor's phones its text is
//! appended after the base key's.
//!
//! The build fails when a key the code uses is missing, a slot is not
//! declared, a subkey has no base key, or a block does not parse.

use std::collections::BTreeMap;

/// The tag that marks a piece of copy.
pub const TAG: &str = "Copy";

/// One key, with its vendor additions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub title: Option<String>,
    pub text: String,
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
        if raw.title.is_some() {
            return Err(format!("`{name}`: a vendor subkey has no title"));
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
        let mut slots = Vec::new();
        for child in &node.nodes {
            let Some(member) = child.as_member() else {
                continue;
            };
            let value = member.rhs.as_ref().map(|t| t.render()).unwrap_or_default();
            match member.name.as_str() {
                "title" => title = Some(value.trim().to_string()),
                "text" => text = Some(value.trim().to_string()),
                slot => slots.push(slot.to_string()),
            }
        }
        let text = text.ok_or_else(|| format!("`{head}` has no `text`"))?;
        out.push(Raw {
            key,
            vendor,
            title,
            text,
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

/// `settings_copy.rs` for `md`: one `pub const` per key, of the type
/// `CopyText` the including module defines. Fails when a key in `used` is
/// missing, or as [`parse`] does.
pub fn generate(md: &str, used: &[String]) -> Result<String, String> {
    let entries = parse(md)?;
    if let Some(missing) = used.iter().find(|k| !entries.iter().any(|e| &e.key == *k)) {
        return Err(format!(
            "there is no `# [Copy] {missing}`, which the code uses"
        ));
    }
    let mut out = String::from("// Generated by build.rs from SETTINGS_COPY.md: do not edit.\n");
    for e in &entries {
        let vendors: Vec<String> = e
            .vendors
            .iter()
            .map(|(v, t)| format!("({v:?}, {t:?})"))
            .collect();
        out.push_str(&format!(
            "\npub const {}: CopyText = CopyText {{\n    key: {:?},\n    title: {:?},\n    text: {:?},\n    slots: &{:?},\n    vendors: &[{}],\n}};\n",
            e.key.to_ascii_uppercase(),
            e.key,
            e.title,
            e.text,
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
    fn used_keys_are_the_consts_named_after_copy() {
        let src = "let a = copy::UNPAIR.fill(None, &[]); copy::CopyText; copy::BATTERY.text(None); copy::UNPAIR".to_string();
        assert_eq!(used_keys(&[src]), ["battery", "unpair"]);
    }
}
