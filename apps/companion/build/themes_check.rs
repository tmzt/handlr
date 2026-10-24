//! `themes.toml`: its themes, parsed and checked
//! (docs/design/20261024_05_THEMES_PLAN.md, the 15:33 EDT amendment, and
//! CHANGES.md 2026-10-24, "a theme's missing roles come from the light or
//! dark default").
//!
//! `build.rs` includes this file to check the shipped `themes.toml` with
//! [`check`]; the app includes it too (`src/theme.rs`), to parse the
//! shipped text at start and the user's `themes.toml` beside
//! `settings.toml` with [`parse`], and to run these unit tests.
//!
//! Each theme is a table `[theme.<id>]` with a `name`, a `dark` flag, and
//! its colour roles. A colour is `"#RRGGBB"`, `"#RRGGBBAA"`, or an array of
//! three or four numbers from 0 to 1 (so *Dark (gray)* can hold today's
//! values exactly). The first dark theme and the first light theme are the
//! defaults and set every role; any other theme may leave roles out, and
//! each one left out is its kind's default's.
//!
//! [`check`] fails for a TOML error, a theme without `name` or `dark`, an
//! unknown key, a bad colour, a repeated id or name, a default missing a
//! role, or a file without both a light and a dark theme. [`parse`] keeps
//! the themes that are well formed and reports the others.

macro_rules! roles {
    ($($role:ident => $variant:ident: $doc:literal,)*) => {
        /// A theme's colours, one per role.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Colors {
            $(#[doc = $doc] pub $role: [f32; 4],)*
        }

        /// A colour role, for colours kept in state (a note, a line) and
        /// looked up as they are drawn.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Role {
            $(#[doc = $doc] $variant,)*
        }

        /// The roles' names in `themes.toml`, in [`Role`]'s order.
        pub const ROLES: &[&str] = &[$(stringify!($role),)*];

        impl Role {
            /// Every role, in [`ROLES`]' order.
            pub const ALL: &[Role] = &[$(Role::$variant,)*];
        }

        impl Colors {
            /// The colour of `role`.
            pub fn get(&self, role: Role) -> [f32; 4] {
                match role {
                    $(Role::$variant => self.$role,)*
                }
            }

            fn set(&mut self, role: Role, c: [f32; 4]) {
                match role {
                    $(Role::$variant => self.$role = c,)*
                }
            }

            /// All transparent: the start of a theme being filled in.
            const fn clear() -> Self {
                Colors {
                    $($role: [0.0; 4],)*
                }
            }
        }
    };
}

roles! {
    bg => Bg: "The screens' background.",
    header => Header: "The headers, and the turn sheet.",
    card => Card: "Cards and modal frames.",
    band => Band: "Bands: the turn strip, the input box, a toggle's track.",
    input => Input: "The dictation card.",
    overlay => Overlay: "The shade under the turn sheet.",
    text => Text: "Body text on the overview, the pairing screen and settings.",
    fg => Fg: "The agent view's text, headings and button labels.",
    dim => Dim: "Dim text: section titles, chevrons, tool lines.",
    muted => Muted: "Secondary text: status lines, notes.",
    code => Code: "The pairing screen's device code.",
    link => Link: "Links.",
    spoken => Spoken: "What was said, and Gemini's lines.",
    accent => Accent: "An approval's accent, and the input box's amber.",
    question => Question: "A question's accent.",
    green => Green: "Done, added lines.",
    red => Red: "Errors, removed lines, danger rows.",
    yellow => Yellow: "Working.",
    warn => Warn: "Warnings.",
    recording => Recording: "The record button while it records.",
    approve => Approve: "The Approve button.",
    deny => Deny: "The Deny button.",
    message => Message: "The message and Pair buttons.",
    back => Back: "The Back, OK and Cancel buttons.",
    button => Button: "A settings row's action button.",
    toggle => Toggle: "A toggle's track while it is on.",
    scrollbar => Scrollbar: "The settings' scroll indicator.",
    placeholder => Placeholder: "A placeholder's bars.",
    shimmer => Shimmer: "The shimmer over the placeholders.",
    shadow => Shadow: "Cards' shadows.",
    session_1 => Session1: "Session colour 1: the first hue, light.",
    session_2 => Session2: "Session colour 2: the first hue, dark.",
    session_3 => Session3: "Session colour 3: the second hue, light.",
    session_4 => Session4: "Session colour 4: the second hue, dark.",
    session_5 => Session5: "Session colour 5: the third hue, light.",
    session_6 => Session6: "Session colour 6: the third hue, dark.",
    session_7 => Session7: "Session colour 7: the fourth hue, light.",
    session_8 => Session8: "Session colour 8: the fourth hue, dark.",
    session_9 => Session9: "Session colour 9: the fifth hue, light.",
    session_10 => Session10: "Session colour 10: the fifth hue, dark.",
    session_11 => Session11: "Session colour 11: the sixth hue, light.",
    session_12 => Session12: "Session colour 12: the sixth hue, dark.",
    session_13 => Session13: "Session colour 13: the seventh hue, light.",
    session_14 => Session14: "Session colour 14: the seventh hue, dark.",
    session_15 => Session15: "Session colour 15: the eighth hue, light.",
    session_16 => Session16: "Session colour 16: the eighth hue, dark.",
}

/// The session colours' roles, `session_1` to `session_16`, in order
/// (plan 20261022_03, the L3 amendment of 2026-10-24 15:56 EDT).
pub const SESSION_ROLES: [Role; 16] = [
    Role::Session1,
    Role::Session2,
    Role::Session3,
    Role::Session4,
    Role::Session5,
    Role::Session6,
    Role::Session7,
    Role::Session8,
    Role::Session9,
    Role::Session10,
    Role::Session11,
    Role::Session12,
    Role::Session13,
    Role::Session14,
    Role::Session15,
    Role::Session16,
];

impl Colors {
    /// Session colour `index` (0 to 15, as the client's hash gives it):
    /// the role `session_<index + 1>`.
    pub fn session(&self, index: u8) -> [f32; 4] {
        self.get(SESSION_ROLES[usize::from(index) % SESSION_ROLES.len()])
    }
}

/// A complete theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    /// A dark theme (`dark = true`) or a light one.
    pub dark: bool,
    pub colors: Colors,
}

/// A theme as written: the roles it sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Partial {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub set: Vec<(Role, [f32; 4])>,
}

impl Partial {
    /// The roles it leaves out.
    pub fn missing(&self) -> Vec<&'static str> {
        Role::ALL
            .iter()
            .enumerate()
            .filter(|(_, r)| !self.set.iter().any(|(s, _)| s == *r))
            .map(|(i, _)| ROLES[i])
            .collect()
    }

    /// The theme, its missing roles taken from `default`.
    pub fn complete(&self, default: &Colors) -> Theme {
        let mut colors = *default;
        for &(role, c) in &self.set {
            colors.set(role, c);
        }
        Theme {
            id: self.id.clone(),
            name: self.name.clone(),
            dark: self.dark,
            colors,
        }
    }

    /// The theme, when it sets every role.
    fn whole(&self) -> Result<Theme, String> {
        let missing = self.missing();
        if !missing.is_empty() {
            return Err(format!(
                "theme.{}: the default {} theme sets every role; missing {}",
                self.id,
                if self.dark { "dark" } else { "light" },
                missing.join(", ")
            ));
        }
        Ok(self.complete(&Colors::clear()))
    }
}

/// `"#RRGGBB"`, `"#RRGGBBAA"`, or `[r, g, b]` / `[r, g, b, a]` from 0 to 1.
pub fn colour(value: &toml::Value) -> Result<[f32; 4], String> {
    match value {
        toml::Value::String(s) => {
            let hex = s
                .strip_prefix('#')
                .filter(|h| (h.len() == 6 || h.len() == 8) && h.is_ascii())
                .ok_or_else(|| format!("{s:?} is not #RRGGBB or #RRGGBBAA"))?;
            let mut c = [1.0f32; 4];
            for (i, slot) in c.iter_mut().enumerate().take(hex.len() / 2) {
                let byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
                    .map_err(|_| format!("{s:?} is not #RRGGBB or #RRGGBBAA"))?;
                *slot = byte as f32 / 255.0;
            }
            Ok(c)
        }
        toml::Value::Array(items) if items.len() == 3 || items.len() == 4 => {
            let mut c = [1.0f32; 4];
            for (slot, item) in c.iter_mut().zip(items) {
                let v = match item {
                    toml::Value::Float(f) => *f,
                    toml::Value::Integer(i) => *i as f64,
                    _ => return Err(format!("{value} holds a non-number")),
                };
                if !(0.0..=1.0).contains(&v) {
                    return Err(format!("{value}: {v} is not from 0 to 1"));
                }
                *slot = v as f32;
            }
            Ok(c)
        }
        _ => Err(format!(
            "{value} is not a colour (\"#RRGGBB\", \"#RRGGBBAA\" or [r, g, b, a])"
        )),
    }
}

/// One theme's table.
fn theme(id: &str, value: &toml::Value) -> Result<Partial, String> {
    let at = |e: String| format!("theme.{id}: {e}");
    let table = value.as_table().ok_or_else(|| at("not a table".into()))?;
    let name = match table.get("name") {
        Some(toml::Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(_) => return Err(at("`name` is not a non-empty string".into())),
        None => return Err(at("no `name`".into())),
    };
    let dark = match table.get("dark") {
        Some(toml::Value::Boolean(b)) => *b,
        Some(_) => return Err(at("`dark` is not true or false".into())),
        None => return Err(at("no `dark`".into())),
    };
    let mut set = Vec::new();
    for (key, v) in table {
        if key == "name" || key == "dark" {
            continue;
        }
        let i = ROLES
            .iter()
            .position(|r| r == key)
            .ok_or_else(|| at(format!("`{key}` is not a role")))?;
        let c = colour(v).map_err(|e| at(format!("{key}: {e}")))?;
        set.push((Role::ALL[i], c));
    }
    Ok(Partial {
        id: id.to_string(),
        name,
        dark,
        set,
    })
}

/// The themes in `text`, in file order, and an error for each theme that
/// is not well formed (skipped). `Err` when the text is not TOML, or has no
/// `theme` table.
pub fn parse(text: &str) -> Result<(Vec<Partial>, Vec<String>), String> {
    let doc: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
    let mut themes: Vec<Partial> = Vec::new();
    let mut errors = Vec::new();
    for (key, _) in doc.iter().filter(|(k, _)| *k != "theme") {
        errors.push(format!("`{key}`: only `theme.<id>` tables belong here"));
    }
    let Some(table) = doc.get("theme") else {
        return Ok((themes, errors));
    };
    let table = table
        .as_table()
        .ok_or_else(|| "`theme` is not a table".to_string())?;
    for (id, value) in table {
        match theme(id, value) {
            Ok(t) if themes.iter().any(|o| o.name == t.name) => {
                errors.push(format!("theme.{id}: the name {:?} is taken", t.name));
            }
            Ok(t) => themes.push(t),
            Err(e) => errors.push(e),
        }
    }
    Ok((themes, errors))
}

/// The defaults of `themes`: its first dark theme and its first light one,
/// each setting every role.
pub fn defaults(themes: &[Partial]) -> Result<(Theme, Theme), String> {
    let first = |dark: bool| {
        themes
            .iter()
            .find(|t| t.dark == dark)
            .ok_or_else(|| format!("no {} theme", if dark { "dark" } else { "light" }))
            .and_then(Partial::whole)
    };
    Ok((first(true)?, first(false)?))
}

/// The shipped file's check: every theme well formed, ids and names
/// unique, a dark and a light default that set every role. The themes,
/// completed, in file order.
pub fn check(text: &str) -> Result<Vec<Theme>, String> {
    // A repeated table is a TOML error ("duplicate key"), so a repeated id
    // fails here.
    let (themes, errors) = parse(text)?;
    if let Some(e) = errors.into_iter().next() {
        return Err(e);
    }
    let (dark, light) = defaults(&themes)?;
    Ok(themes
        .iter()
        .map(|t| t.complete(if t.dark { &dark.colors } else { &light.colors }))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_role(id: &str, name: &str, dark: bool, hex: &str) -> String {
        let mut s = format!("[theme.{id}]\nname = \"{name}\"\ndark = {dark}\n");
        for r in ROLES {
            s.push_str(&format!("{r} = \"{hex}\"\n"));
        }
        s
    }

    fn two() -> String {
        every_role("d", "D", true, "#102030") + &every_role("l", "L", false, "#F0F0F0")
    }

    #[test]
    fn colours_parse_as_hex_or_numbers() {
        let v = |s: &str| -> toml::Value {
            toml::from_str::<toml::Table>(&format!("c = {s}")).unwrap()["c"].clone()
        };
        assert_eq!(colour(&v("\"#FF0000\"")).unwrap(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(colour(&v("\"#00000080\"")).unwrap()[3], 128.0 / 255.0);
        assert_eq!(
            colour(&v("[0.03, 0.03, 0.04]")).unwrap(),
            [0.03, 0.03, 0.04, 1.0]
        );
        assert_eq!(
            colour(&v("[0, 0, 0, 0.55]")).unwrap(),
            [0.0, 0.0, 0.0, 0.55]
        );
        for bad in [
            "\"FF0000\"",
            "\"#FF00\"",
            "\"#GG0000\"",
            "[1.5, 0, 0]",
            "[0, 0]",
            "3",
            "\"#ÿÿÿ\"",
        ] {
            assert!(colour(&v(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_whole_file_checks_in_order() {
        let themes = check(&two()).unwrap();
        let ids: Vec<&str> = themes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["d", "l"]);
        assert!(themes[0].dark && !themes[1].dark);
        assert_eq!(
            themes[1].colors.bg,
            [240.0 / 255.0, 240.0 / 255.0, 240.0 / 255.0, 1.0]
        );
    }

    #[test]
    fn a_default_missing_a_role_fails_the_check() {
        let text = two().replace("shimmer = \"#102030\"\n", "");
        let e = check(&text).unwrap_err();
        assert!(e.contains("theme.d") && e.contains("shimmer"), "{e}");
    }

    #[test]
    fn a_bad_colour_an_unknown_role_a_repeat_or_no_dark_fails_the_check() {
        let bad = two().replace("bg = \"#102030\"", "bg = \"#1020\"");
        assert!(check(&bad).unwrap_err().contains("theme.d: bg"));
        let unknown = two() + "[theme.x]\nname = \"X\"\ndark = true\nbackground = \"#000000\"\n";
        assert!(
            check(&unknown)
                .unwrap_err()
                .contains("`background` is not a role")
        );
        let repeat = two() + &every_role("d", "D again", true, "#000000");
        assert!(check(&repeat).is_err(), "a repeated id");
        let named = two() + "[theme.x]\nname = \"D\"\ndark = true\n";
        assert!(check(&named).unwrap_err().contains("taken"));
        let no_dark = two() + "[theme.x]\nname = \"X\"\nbg = \"#000000\"\n";
        assert!(check(&no_dark).unwrap_err().contains("no `dark`"));
        let only_dark = every_role("d", "D", true, "#102030");
        assert!(check(&only_dark).unwrap_err().contains("no light theme"));
    }

    #[test]
    fn missing_roles_come_from_the_default_of_its_kind() {
        let text = two()
            + "[theme.dd]\nname = \"DD\"\ndark = true\nbg = \"#000000\"\n"
            + "[theme.ll]\nname = \"LL\"\ndark = false\ntext = \"#000000\"\n";
        let themes = check(&text).unwrap();
        let (d, l) = (&themes[0].colors, &themes[1].colors);
        let dd = &themes[2].colors;
        assert_eq!(dd.bg, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            Colors { bg: d.bg, ..*dd },
            *d,
            "the rest is the dark default's"
        );
        let ll = &themes[3].colors;
        assert_eq!(ll.text, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            Colors {
                text: l.text,
                ..*ll
            },
            *l,
            "the rest is the light default's"
        );
    }

    #[test]
    fn parse_skips_bad_themes_and_keeps_the_rest() {
        let text = "[theme.a]\nname = \"A\"\ndark = true\nbg = \"#000000\"\n\
                    [theme.b]\nname = \"B\"\nbg = \"#000000\"\n\
                    [theme.c]\nname = \"C\"\ndark = false\nbg = \"nope\"\n";
        let (themes, errors) = parse(text).unwrap();
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].id, "a");
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(parse("not toml = = 3").is_err());
    }
}
