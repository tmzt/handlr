//! The extractor behind `build.rs`: herdr's protocol types, out of herdr's
//! own source, into one generated module
//! (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md, decision 2, and
//! the 23:42 EDT amendment, item 3).
//!
//! It is a plain module, used by `build.rs` and by `tests/extract.rs` (which
//! runs it on a broken copy of herdr's source), so it takes a source root
//! and returns the generated text or the error the build prints.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit_mut::VisitMut;
use syn::{Attribute, Item, Meta, Path as SynPath, punctuated::Punctuated};

/// herdr's files read, relative to the submodule's root.
pub const WIRE: &str = "src/protocol/wire.rs";
pub const ENDPOINT: &str = "src/protocol/endpoint.rs";
pub const INPUT_MODEL: &str = "src/input/model.rs";
pub const SCHEMA_COMMON: &str = "src/api/schema/common.rs";
pub const CONFIG_MODEL: &str = "src/config/model.rs";

/// Every file read, for `cargo:rerun-if-changed`.
pub const SOURCES: [&str; 5] = [WIRE, ENDPOINT, INPUT_MODEL, SCHEMA_COMMON, CONFIG_MODEL];

/// The free functions kept from `wire.rs` and `endpoint.rs`.
const KEPT_FNS: [&str; 7] = [
    "serialize_graphics_bytes",
    "deserialize_graphics_bytes",
    "deserialize_client_shell_agent_status",
    "default_true",
    "write_message",
    "read_message",
    "read_exact_or_eof",
];

/// The trait impls kept, as (trait's last segment, self type).
const KEPT_IMPLS: [(&str, &str); 4] = [
    ("Clone", "CellData"),
    ("Display", "FramingError"),
    ("Error", "FramingError"),
    ("From", "FramingError"),
];

/// The types defined outside the protocol files that wire types name, as
/// (file, type, the `crate::` path herdr names it by).
const OUTSIDE: [(&str, &str, &str); 3] = [
    (
        INPUT_MODEL,
        "WindowsKeyRecord",
        "crate::input::WindowsKeyRecord",
    ),
    (
        SCHEMA_COMMON,
        "AgentStatus",
        "crate::api::schema::AgentStatus",
    ),
    (
        CONFIG_MODEL,
        "ToastHerdrPosition",
        "crate::config::ToastHerdrPosition",
    ),
];

/// Items whose absence means the extraction no longer matches herdr.
const REQUIRED: [&str; 24] = [
    "PROTOCOL_VERSION",
    "MAX_FRAME_SIZE",
    "MAX_GRAPHICS_FRAME_SIZE",
    "ENDPOINT_PROTOCOL_GENERATION",
    "ENDPOINT_HELLO_KIND",
    "ENDPOINT_WELCOME_KIND",
    "SNAPSHOT_CODEC_V1",
    "SURFACE_CODEC_V1",
    "INPUT_CODEC_V1",
    "BLOB_CODEC_V1",
    "ClientMessage",
    "ServerMessage",
    "ClientPaneInputEvent",
    "ClientSurfaceSize",
    "CellData",
    "CursorState",
    "FrameData",
    "PaneSurfaceFrame",
    "PaneSurfacePatch",
    "PaneSurfacePane",
    "ClientShellSnapshot",
    "EndpointClientHello",
    "EndpointServerWelcome",
    "FramingError",
];

/// Why the extraction failed: the message `build.rs` panics with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractError(pub String);

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The generated module's text and what went into it.
#[derive(Debug, Clone)]
pub struct Extracted {
    /// The module body, formatted by prettyplease.
    pub code: String,
    /// herdr's `PROTOCOL_VERSION` as written in `wire.rs`.
    pub protocol_version: u32,
    /// The names of the items kept, sorted.
    pub items: BTreeSet<String>,
}

/// The submodule's root: `HERDR_SRC` when set, else `vendor/herdr` two
/// levels above the crate.
pub fn herdr_root(manifest_dir: &Path, herdr_src: Option<&str>) -> PathBuf {
    match herdr_src {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => manifest_dir.join("../../vendor/herdr"),
    }
}

/// Extract herdr's protocol types from the herdr checkout at `root`.
pub fn extract(root: &Path) -> Result<Extracted, ExtractError> {
    let wire = parse(root, WIRE)?;
    let endpoint = parse(root, ENDPOINT)?;

    let mut uses: Vec<Item> = Vec::new();
    let mut seen_uses = BTreeSet::new();
    let mut items: Vec<(String, Item)> = Vec::new();
    let mut kept_impls = BTreeSet::new();

    for ast in [wire, endpoint] {
        for item in ast.items {
            let Some(item) = keep_cfg(item) else { continue };
            match item {
                Item::Use(u) => {
                    let text = u.tree.to_token_stream().to_string();
                    let root = text.split_whitespace().next().unwrap_or("").to_owned();
                    if (root == "std" || root == "serde") && seen_uses.insert(text) {
                        uses.push(Item::Use(u));
                    }
                }
                Item::Const(c) => items.push((c.ident.to_string(), Item::Const(c))),
                Item::Type(t) => items.push((t.ident.to_string(), Item::Type(t))),
                Item::Struct(mut s) => {
                    if derives_serde(&s.attrs) || s.ident == "FramingError" {
                        clean_derives(&mut s.attrs);
                        s.fields = keep_cfg_fields(s.fields);
                        items.push((s.ident.to_string(), Item::Struct(s)));
                    }
                }
                Item::Enum(mut e) => {
                    if derives_serde(&e.attrs) || e.ident == "FramingError" {
                        clean_derives(&mut e.attrs);
                        e.variants = e
                            .variants
                            .into_iter()
                            .filter_map(|mut v| {
                                v.attrs = keep_cfg_attrs(v.attrs)?;
                                v.fields = keep_cfg_fields(v.fields);
                                Some(v)
                            })
                            .collect();
                        items.push((e.ident.to_string(), Item::Enum(e)));
                    }
                }
                Item::Fn(f) => {
                    let name = f.sig.ident.to_string();
                    if KEPT_FNS.contains(&name.as_str()) {
                        items.push((name, Item::Fn(f)));
                    }
                }
                Item::Impl(i) => {
                    let Some((_, trait_path, _)) = &i.trait_ else {
                        continue;
                    };
                    let trait_name = last_segment(trait_path);
                    let self_name = i.self_ty.to_token_stream().to_string();
                    if let Some(&(t, s)) = KEPT_IMPLS
                        .iter()
                        .find(|(t, s)| *t == trait_name && *s == self_name)
                    {
                        kept_impls.insert((t, s));
                        items.push((format!("impl {t} for {s}"), Item::Impl(i)));
                    }
                }
                _ => {}
            }
        }
    }

    // The outside types, from their own files.
    let mut renames: Vec<(String, String)> = Vec::new();
    for (file, name, herdr_path) in OUTSIDE {
        let ast = parse(root, file)?;
        let found = ast.items.into_iter().find_map(|item| match item {
            Item::Struct(s) if s.ident == name => Some(Item::Struct(s)),
            Item::Enum(e) if e.ident == name => Some(Item::Enum(e)),
            _ => None,
        });
        let Some(mut item) = found else {
            return Err(ExtractError(format!(
                "libherdr_term: `{name}` is no longer defined in herdr's {file}; \
                 find where herdr moved it and map it in crates/libherdr_term/build/extract.rs (OUTSIDE)"
            )));
        };
        match &mut item {
            Item::Struct(s) => clean_derives(&mut s.attrs),
            Item::Enum(e) => clean_derives(&mut e.attrs),
            _ => unreachable!(),
        }
        items.push((name.to_owned(), item));
        renames.push((herdr_path.replace(' ', ""), name.to_owned()));
    }

    let names: BTreeSet<String> = items.iter().map(|(n, _)| n.clone()).collect();

    for name in REQUIRED {
        if !names.contains(name) {
            return Err(ExtractError(format!(
                "libherdr_term: herdr's protocol no longer defines `{name}` where build.rs looks for it \
                 ({WIRE} or {ENDPOINT}); map it in crates/libherdr_term/build/extract.rs"
            )));
        }
    }
    for name in KEPT_FNS {
        if !names.contains(name) {
            return Err(ExtractError(format!(
                "libherdr_term: herdr's protocol no longer defines the function `{name}`; \
                 map it in crates/libherdr_term/build/extract.rs (KEPT_FNS)"
            )));
        }
    }
    for (t, s) in KEPT_IMPLS {
        if !kept_impls.contains(&(t, s)) {
            return Err(ExtractError(format!(
                "libherdr_term: herdr's protocol no longer has `impl {t} for {s}`; \
                 map it in crates/libherdr_term/build/extract.rs (KEPT_IMPLS)"
            )));
        }
    }

    // Rewrite herdr's crate paths to the flattened module, then refuse any
    // that remain.
    let mut rewriter = Rewriter {
        renames,
        local: names.clone(),
    };
    for (_, item) in items.iter_mut() {
        rewriter.visit_item_mut(item);
    }
    for (name, item) in &items {
        if let Some(path) = find_crate_path(item.to_token_stream()) {
            return Err(ExtractError(format!(
                "libherdr_term: herdr's `{name}` names `{path}`, which build.rs does not extract; \
                 map it in crates/libherdr_term/build/extract.rs (OUTSIDE)"
            )));
        }
    }

    let protocol_version = items
        .iter()
        .find_map(|(name, item)| match item {
            Item::Const(c) if name == "PROTOCOL_VERSION" => match &*c.expr {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(i),
                    ..
                }) => i.base10_parse::<u32>().ok(),
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(|| {
            ExtractError(format!(
                "libherdr_term: herdr's PROTOCOL_VERSION in {WIRE} is not an integer literal; \
                 update crates/libherdr_term/build/extract.rs"
            ))
        })?;

    let file = syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: uses
            .into_iter()
            .chain(items.into_iter().map(|(_, item)| item))
            .collect(),
    };
    let header = format!(
        "// Generated by crates/libherdr_term/build.rs from herdr's source\n\
         // ({WIRE}, {ENDPOINT}, {INPUT_MODEL}, {SCHEMA_COMMON}, {CONFIG_MODEL}).\n\
         // herdr is Apache-2.0; see crates/libherdr_term/README.md. Do not edit.\n\
         // herdr PROTOCOL_VERSION = {protocol_version}.\n\n"
    );
    Ok(Extracted {
        code: header + &prettyplease::unparse(&file),
        protocol_version,
        items: names,
    })
}

fn parse(root: &Path, rel: &str) -> Result<syn::File, ExtractError> {
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        ExtractError(format!(
            "libherdr_term: cannot read herdr's {rel} at {} ({e}). \
             Run `git submodule update --init vendor/herdr`, or point HERDR_SRC at a herdr checkout.",
            path.display()
        ))
    })?;
    syn::parse_file(&text)
        .map_err(|e| ExtractError(format!("libherdr_term: cannot parse herdr's {rel}: {e}")))
}

/// The value of a `cfg` predicate on flankr's targets (unix, never herdr's
/// `test`): `Some(true)`, `Some(false)`, or `None` when it names something
/// else (the attribute is then left for rustc).
fn eval_cfg(meta: &Meta) -> Option<bool> {
    match meta {
        Meta::Path(p) => {
            if p.is_ident("test") || p.is_ident("windows") {
                Some(false)
            } else if p.is_ident("unix") {
                Some(true)
            } else {
                None
            }
        }
        Meta::List(list) => {
            let name = list.path.get_ident()?.to_string();
            let args = list
                .parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated)
                .ok()?;
            let values: Vec<Option<bool>> = args.iter().map(eval_cfg).collect();
            match name.as_str() {
                "not" if values.len() == 1 => values[0].map(|v| !v),
                "any" => {
                    if values.contains(&Some(true)) {
                        Some(true)
                    } else if values.iter().all(|v| *v == Some(false)) {
                        Some(false)
                    } else {
                        None
                    }
                }
                "all" => {
                    if values.contains(&Some(false)) {
                        Some(false)
                    } else if values.iter().all(|v| *v == Some(true)) {
                        Some(true)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        Meta::NameValue(_) => None,
    }
}

/// `attrs` without the `cfg`s that hold, or `None` when one does not.
fn keep_cfg_attrs(attrs: Vec<Attribute>) -> Option<Vec<Attribute>> {
    let mut out = Vec::with_capacity(attrs.len());
    for attr in attrs {
        if attr.path().is_ident("cfg") {
            let value = attr.parse_args::<Meta>().ok().and_then(|m| eval_cfg(&m));
            match value {
                Some(false) => return None,
                Some(true) => continue,
                None => {}
            }
        }
        out.push(attr);
    }
    Some(out)
}

fn keep_cfg(item: Item) -> Option<Item> {
    macro_rules! with_attrs {
        ($i:ident, $v:ident) => {{
            let mut $i = $i;
            $i.attrs = keep_cfg_attrs(std::mem::take(&mut $i.attrs))?;
            Some(Item::$v($i))
        }};
    }
    match item {
        Item::Use(i) => with_attrs!(i, Use),
        Item::Const(i) => with_attrs!(i, Const),
        Item::Type(i) => with_attrs!(i, Type),
        Item::Struct(i) => with_attrs!(i, Struct),
        Item::Enum(i) => with_attrs!(i, Enum),
        Item::Fn(i) => with_attrs!(i, Fn),
        Item::Impl(i) => with_attrs!(i, Impl),
        Item::Mod(i) => with_attrs!(i, Mod),
        other => Some(other),
    }
}

fn keep_cfg_fields(fields: syn::Fields) -> syn::Fields {
    let filter =
        |p: Punctuated<syn::Field, syn::Token![,]>| -> Punctuated<syn::Field, syn::Token![,]> {
            p.into_iter()
                .filter_map(|mut f| {
                    f.attrs = keep_cfg_attrs(std::mem::take(&mut f.attrs))?;
                    Some(f)
                })
                .collect()
        };
    match fields {
        syn::Fields::Named(mut n) => {
            n.named = filter(n.named);
            syn::Fields::Named(n)
        }
        syn::Fields::Unnamed(mut u) => {
            u.unnamed = filter(u.unnamed);
            syn::Fields::Unnamed(u)
        }
        syn::Fields::Unit => syn::Fields::Unit,
    }
}

fn derive_paths(attr: &Attribute) -> Option<Vec<SynPath>> {
    if !attr.path().is_ident("derive") {
        return None;
    }
    attr.parse_args_with(Punctuated::<SynPath, syn::Token![,]>::parse_terminated)
        .ok()
        .map(|p| p.into_iter().collect())
}

fn last_segment(path: &SynPath) -> String {
    path.segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

fn derives_serde(attrs: &[Attribute]) -> bool {
    attrs.iter().filter_map(derive_paths).flatten().any(|p| {
        let last = last_segment(&p);
        last == "Serialize" || last == "Deserialize"
    })
}

/// Drop `bincode::Decode`/`Encode` (flankr decodes through serde only) and
/// `schemars::JsonSchema` from derives.
fn clean_derives(attrs: &mut [Attribute]) {
    for attr in attrs.iter_mut() {
        let Some(paths) = derive_paths(attr) else {
            continue;
        };
        let kept: Vec<SynPath> = paths
            .into_iter()
            .filter(|p| {
                let first = p
                    .segments
                    .first()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default();
                !(p.segments.len() > 1 && (first == "bincode" || first == "schemars"))
            })
            .collect();
        *attr = syn::parse_quote!(#[derive(#(#kept),*)]);
    }
}

/// Rewrites `crate::…::Name` paths of the mapped outside types, and
/// `crate::protocol::Name` / `super::Name` of kept items, to `Name`.
struct Rewriter {
    renames: Vec<(String, String)>,
    local: BTreeSet<String>,
}

impl Rewriter {
    fn target(&self, path: &SynPath) -> Option<(usize, String)> {
        let idents: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        // The longest mapped prefix wins: `crate::api::schema::AgentStatus::Idle`
        // keeps its `::Idle`.
        for (from, to) in &self.renames {
            let from: Vec<&str> = from.split("::").collect();
            if idents.len() >= from.len() && idents.iter().zip(&from).all(|(a, b)| a == b) {
                return Some((from.len(), to.clone()));
            }
        }
        let prefix = match idents.first().map(String::as_str) {
            Some("super") => 1,
            Some("crate") if idents.get(1).map(String::as_str) == Some("protocol") => 2,
            _ => return None,
        };
        let name = idents.get(prefix)?;
        if self.local.contains(name) {
            Some((prefix + 1, name.clone()))
        } else {
            None
        }
    }
}

impl VisitMut for Rewriter {
    fn visit_path_mut(&mut self, path: &mut SynPath) {
        if let Some((consumed, to)) = self.target(path) {
            let last = path.segments[consumed - 1].clone();
            let rest: Vec<syn::PathSegment> =
                path.segments.iter().skip(consumed).cloned().collect();
            let mut segments = Punctuated::new();
            segments.push(syn::PathSegment {
                ident: syn::Ident::new(&to, last.ident.span()),
                arguments: last.arguments,
            });
            for s in rest {
                segments.push(s);
            }
            path.leading_colon = None;
            path.segments = segments;
        }
        syn::visit_mut::visit_path_mut(self, path);
    }
}

/// The first `crate::…` (or `super::…`) path left anywhere in `tokens`,
/// macros included.
fn find_crate_path(tokens: TokenStream) -> Option<String> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    for (i, tt) in tokens.iter().enumerate() {
        match tt {
            TokenTree::Ident(id) if id == "crate" || id == "super" => {
                // Collect `crate :: a :: b`; a bare `crate` (as in
                // `pub(crate)`) is not a path.
                let mut path = id.to_string();
                let mut j = i + 1;
                while let (
                    Some(TokenTree::Punct(a)),
                    Some(TokenTree::Punct(b)),
                    Some(TokenTree::Ident(n)),
                ) = (tokens.get(j), tokens.get(j + 1), tokens.get(j + 2))
                {
                    if a.as_char() != ':' || b.as_char() != ':' {
                        break;
                    }
                    path.push_str("::");
                    path.push_str(&n.to_string());
                    j += 3;
                }
                if j > i + 1 {
                    return Some(path);
                }
            }
            TokenTree::Group(g) => {
                if let Some(path) = find_crate_path(g.stream()) {
                    return Some(path);
                }
            }
            _ => {}
        }
    }
    None
}
