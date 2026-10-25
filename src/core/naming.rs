//! What flankr calls an agent when it speaks or shows it
//! (docs/design/20261022_05_SPOKEN_NAMES_PLAN.md, decisions 3–10).
//!
//! A pure module: a herdr snapshot and a [`NamingConfig`] go in, names come
//! out. Every name has a *shown* form (labels as written, "Claude in
//! libmorph, tab -mb") and a *spoken* form (spoken tab forms and spelling,
//! "Claude in libmorph, tab em bee"). Neither ever contains a pane id.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::config::{NamingConfig, ProjectFrom};
use crate::core::agents::{self, AgentRef};

/// The English names of the letters `a` to `z`, written so a TTS says the
/// name and not a sound (decision 5). Not the military alphabet.
pub const LETTER_NAMES: [&str; 26] = [
    "ay",
    "bee",
    "see",
    "dee",
    "ee",
    "ef",
    "jee",
    "aitch",
    "eye",
    "jay",
    "kay",
    "el",
    "em",
    "en",
    "oh",
    "pee",
    "cue",
    "ar",
    "ess",
    "tee",
    "you",
    "vee",
    "double you",
    "ex",
    "why",
    "zee",
];

/// The agent name herdr reports for a pane that runs no known agent.
const SHELL: &str = "shell";

// ── Words ───────────────────────────────────────────────────────────────

/// The spoken name of one letter: the configured override, else the default.
/// `None` for anything that is not an ASCII letter.
pub fn letter_name(letter: char, config: &NamingConfig) -> Option<String> {
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let lower = letter.to_ascii_lowercase();
    let configured = config.letters.iter().find_map(|(key, name)| {
        let mut chars = key.trim().chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.to_ascii_lowercase() == lower => Some(name.trim().to_string()),
            _ => None,
        }
    });
    Some(configured.unwrap_or_else(|| LETTER_NAMES[(lower as u8 - b'a') as usize].to_string()))
}

/// Number of ASCII letters in `word`.
fn letter_count(word: &str) -> usize {
    word.chars().filter(|c| c.is_ascii_alphabetic()).count()
}

/// Whether `word` is short enough that its spelled form and capitals belong
/// in the vocabulary: at least one letter, at most `spell_max_len`.
fn is_short(word: &str, config: &NamingConfig) -> bool {
    let n = letter_count(word);
    n > 0 && n <= config.spell_max_len
}

/// Whether `word` is spoken letter by letter: short, and containing none of
/// the configured vowels.
pub fn should_spell(word: &str, config: &NamingConfig) -> bool {
    let vowels = config.vowels.to_lowercase();
    is_short(word, config) && !word.to_lowercase().chars().any(|c| vowels.contains(c))
}

/// `word` letter by letter: letters by their names, every run of other
/// characters (digits) left as it is. `kp2` -> "kay pee 2".
pub fn spell(word: &str, config: &NamingConfig) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in word.chars() {
        match letter_name(c, config) {
            Some(name) => {
                if !run.is_empty() {
                    out.push(std::mem::take(&mut run));
                }
                out.push(name);
            }
            None => run.push(c),
        }
    }
    if !run.is_empty() {
        out.push(run);
    }
    out.join(" ")
}

/// How one word is said: spelled when [`should_spell`], else as written.
pub fn spoken_word(word: &str, config: &NamingConfig) -> String {
    if should_spell(word, config) {
        spell(word, config)
    } else {
        word.to_string()
    }
}

/// The words of a name part: split on whitespace, `-` and `_`.
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|w| !w.is_empty())
        .collect()
}

/// How a name part is said: each word through [`spoken_word`].
pub fn speak(text: &str, config: &NamingConfig) -> String {
    words(text)
        .iter()
        .map(|w| spoken_word(w, config))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A tab label's word form (decision 4): without a leading number, without
/// leading or trailing `-`, other `-` read as a space. `17-hr` -> `hr`.
/// `None` when nothing is left (a label that is only a number, or empty).
pub fn tab_word_form(label: &str) -> Option<String> {
    let rest = label
        .trim()
        .trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.trim_matches(|c: char| c == '-' || c.is_whitespace());
    let form = words(rest).join(" ");
    (!form.is_empty()).then_some(form)
}

/// Capitalise a name for speech, as agent names are proper nouns here.
pub fn capitalise(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => {
            first.to_uppercase().collect::<String>() + chars.as_str()
        }
        _ => trimmed.to_string(),
    }
}

fn alias_for<'c>(config: &'c NamingConfig, key: &str) -> Option<&'c str> {
    let key = key.trim();
    config
        .aliases
        .iter()
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

// ── Name parts ──────────────────────────────────────────────────────────

/// One part of a name (the agent, its project or its tab) in both forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePart {
    /// As written: the label, project or agent name, or an alias value.
    pub shown: String,
    /// As said: spoken forms and spelling, or an alias value.
    pub spoken: String,
    /// The text the words came from, before spelling (`hr` for `17-hr`).
    /// Empty for an alias.
    pub word_form: String,
    /// The alias value that replaced this part, if one did.
    pub alias: Option<String>,
}

impl NamePart {
    fn aliased(value: &str) -> Self {
        Self {
            shown: value.to_string(),
            spoken: value.to_string(),
            word_form: String::new(),
            alias: Some(value.to_string()),
        }
    }
}

fn agent_part(agent: &str, config: &NamingConfig) -> NamePart {
    if let Some(alias) = alias_for(config, agent) {
        return NamePart::aliased(alias);
    }
    NamePart {
        shown: capitalise(agent),
        spoken: capitalise(&speak(agent, config)),
        word_form: agent.trim().to_string(),
        alias: None,
    }
}

fn project_part(project: &str, config: &NamingConfig) -> NamePart {
    if let Some(alias) = alias_for(config, project) {
        return NamePart::aliased(alias);
    }
    NamePart {
        shown: project.trim().to_string(),
        spoken: speak(project, config),
        word_form: project.trim().to_string(),
        alias: None,
    }
}

/// The tab part, without the word "tab": the label (or its alias), else its
/// number. `None` when the tab has neither.
fn tab_part(label: Option<&str>, number: Option<u32>, config: &NamingConfig) -> Option<NamePart> {
    let label = label.map(str::trim).filter(|l| !l.is_empty());
    if let Some(alias) = label.and_then(|l| alias_for(config, l)) {
        return Some(NamePart::aliased(alias));
    }
    if let Some(form) = label.and_then(tab_word_form) {
        return Some(NamePart {
            shown: label.unwrap_or_default().to_string(),
            spoken: speak(&form, config),
            word_form: form,
            alias: None,
        });
    }
    // Only a number: `tab <label>`, as the tab bar shows it. Empty: `tab
    // <number>`.
    let number = label
        .map(|l| l.trim_matches('-').to_string())
        .filter(|l| !l.is_empty())
        .or_else(|| number.map(|n| n.to_string()))?;
    Some(NamePart {
        shown: number.clone(),
        spoken: number.clone(),
        word_form: number,
        alias: None,
    })
}

// ── Names ───────────────────────────────────────────────────────────────

/// One pane with the names it is shown and spoken by.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedAgent {
    pub agent: AgentRef,
    /// The shown form of the shortest unique name ("Claude in libmorph, tab -mb").
    pub shown: String,
    /// The spoken form of the same name ("Claude in libmorph, tab em bee").
    pub spoken: String,
    pub agent_part: NamePart,
    /// The project by decision 3: the workspace label, or the cwd's basename
    /// when another agent shares that label.
    pub project: Option<NamePart>,
    pub tab: Option<NamePart>,
}

impl NamedAgent {
    fn is_shell(&self) -> bool {
        self.agent.agent == SHELL
    }
}

/// Why a target did not resolve to one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Nothing matched.
    NotFound { target: String },
    /// More than one pane matched; their shown names.
    Ambiguous {
        target: String,
        candidates: Vec<String>,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { target } => write!(f, "no agent matching '{target}'"),
            Self::Ambiguous { target, candidates } => write!(
                f,
                "'{target}' matches more than one agent: {}",
                candidates.join("; ")
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// Every pane in a snapshot, named.
#[derive(Debug, Clone)]
pub struct Names {
    pub agents: Vec<NamedAgent>,
    workspace_labels: Vec<String>,
    config: NamingConfig,
}

impl Names {
    /// Name every pane in `snapshot`.
    ///
    /// Agents are named among agents and shells among shells, so a shell in
    /// a project does not push its agent to a longer name; a shell's project
    /// rung reads "the flankr shell".
    pub fn from_snapshot(snapshot: &serde_json::Value, config: &NamingConfig) -> Self {
        let labels = agents::workspace_labels(snapshot);
        let refs = agents::agents(snapshot);

        // Decision 3: how many agents (of the same class) share each label.
        let mut label_count: HashMap<(bool, String), usize> = HashMap::new();
        for r in &refs {
            if let Some(label) = workspace_label(r, &labels) {
                *label_count
                    .entry((r.agent == SHELL, label.to_lowercase()))
                    .or_default() += 1;
            }
        }

        let mut named: Vec<NamedAgent> = refs
            .into_iter()
            .map(|r| {
                let label = workspace_label(&r, &labels);
                let shared = label.is_some_and(|l| {
                    label_count
                        .get(&(r.agent == SHELL, l.to_lowercase()))
                        .copied()
                        .unwrap_or(0)
                        > 1
                });
                let basename = r.cwd.as_deref().and_then(cwd_basename);
                let project = match (config.project_from, label, basename) {
                    (ProjectFrom::Directory, l, None) => l.map(str::to_string),
                    (ProjectFrom::Directory, _, base) => base,
                    (ProjectFrom::Workspace, Some(_), Some(base)) if shared => Some(base),
                    (ProjectFrom::Workspace, Some(l), _) => Some(l.to_string()),
                    (ProjectFrom::Workspace, None, base) => base,
                };
                NamedAgent {
                    agent_part: agent_part(&r.agent, config),
                    project: project.as_deref().map(|p| project_part(p, config)),
                    tab: tab_part(r.tab_label.as_deref(), r.tab_number, config),
                    shown: String::new(),
                    spoken: String::new(),
                    agent: r,
                }
            })
            .collect();

        let ladder: Vec<(String, String)> = (0..named.len())
            .map(|i| {
                let peers: Vec<&NamedAgent> = named
                    .iter()
                    .filter(|p| p.is_shell() == named[i].is_shell())
                    .collect();
                ladder_name(&named[i], &peers)
            })
            .collect();
        for (agent, (shown, spoken)) in named.iter_mut().zip(ladder) {
            agent.shown = shown;
            agent.spoken = spoken;
        }

        let mut workspace_labels: Vec<String> = labels.into_values().collect();
        workspace_labels.sort();
        Self {
            agents: named,
            workspace_labels,
            config: config.clone(),
        }
    }

    /// The named pane with this id (short or fully qualified).
    pub fn get(&self, pane_id: &str) -> Option<&NamedAgent> {
        self.agents
            .iter()
            .find(|a| crate::watch::pane_ids_match(&a.agent.pane_id, pane_id))
    }

    /// The words a speech recogniser should expect (decision 9 as amended):
    /// agent names, projects, cwd basenames and workspace labels; tab labels
    /// as written and their spoken forms; alias values; and for every word
    /// of at most `spell_max_len` letters, its capitals and spelled form.
    /// Each appears once, compared case-sensitively.
    pub fn vocabulary(&self) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let mut push = |s: &str| {
            let s = s.trim();
            if !s.is_empty() && seen.insert(s.to_string()) {
                out.push(s.to_string());
            }
        };
        let mut short_words: Vec<String> = Vec::new();
        for a in &self.agents {
            for part in [Some(&a.agent_part), a.project.as_ref(), a.tab.as_ref()]
                .into_iter()
                .flatten()
            {
                push(&part.shown);
                push(&part.spoken);
                push(&part.word_form);
                short_words.extend(
                    words(&part.word_form)
                        .into_iter()
                        .filter(|w| is_short(w, &self.config))
                        .map(str::to_string),
                );
            }
            if let Some(base) = a.agent.cwd.as_deref().and_then(cwd_basename) {
                push(&base);
            }
        }
        for label in &self.workspace_labels {
            push(label);
        }
        for value in self.config.aliases.values() {
            push(value);
        }
        for word in &short_words {
            push(&word.to_uppercase());
            push(&spell(word, &self.config));
        }
        out
    }

    /// Resolve `target` to one pane (decision 10), trying in order: a pane
    /// id; a tab label as written; an alias value; a spoken form or its
    /// capitals (including a short word's spelled form); a tab number.
    /// Case-insensitive. Matches among agents are preferred over matches
    /// among shells at each step.
    pub fn resolve(&self, target: &str) -> Result<&NamedAgent, ResolveError> {
        let raw = target.trim();
        let not_found = || ResolveError::NotFound {
            target: raw.to_string(),
        };
        if raw.is_empty() {
            return Err(not_found());
        }
        if let Some(a) = self
            .agents
            .iter()
            .find(|a| crate::watch::pane_ids_match(&a.agent.pane_id, raw))
        {
            return Ok(a);
        }

        let full = normalise(raw);
        let mut targets = vec![full.clone()];
        if let Some(rest) = full.strip_prefix("tab ") {
            targets.push(rest.trim().to_string());
        }

        type Step = fn(&NamedAgent, &str, &NamingConfig) -> bool;
        let steps: [Step; 4] = [
            |a, t, _| {
                a.agent
                    .tab_label
                    .as_deref()
                    .is_some_and(|l| normalise(l) == t)
            },
            |a, t, _| {
                [Some(&a.agent_part), a.project.as_ref(), a.tab.as_ref()]
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.alias.as_deref())
                    .any(|v| normalise(v) == t)
            },
            matches_spoken,
            |a, t, _| {
                t.parse::<u32>()
                    .is_ok_and(|n| a.agent.tab_number == Some(n))
            },
        ];
        for step in steps {
            let hits: Vec<&NamedAgent> = self
                .agents
                .iter()
                .filter(|a| targets.iter().any(|t| step(a, t, &self.config)))
                .collect();
            let agents: Vec<&NamedAgent> = hits.iter().copied().filter(|a| !a.is_shell()).collect();
            let hits = if agents.is_empty() { hits } else { agents };
            match hits.as_slice() {
                [] => continue,
                [one] => return Ok(one),
                many => {
                    return Err(ResolveError::Ambiguous {
                        target: raw.to_string(),
                        candidates: many.iter().map(|a| a.shown.clone()).collect(),
                    });
                }
            }
        }
        Err(not_found())
    }
}

impl Names {
    /// The agents an utterance names (plan 20261022_03, decision 7): by a
    /// full name, or a project or tab part, shown, spoken or aliased, said
    /// as whole words. The agent part alone ("Claude") names an agent only
    /// when no other agent has it. One-word parts shorter than three
    /// characters are not matched on their own.
    pub fn named_in(&self, utterance: &str) -> Vec<&NamedAgent> {
        let said = word_tokens(utterance);
        if said.is_empty() {
            return Vec::new();
        }
        let mut agent_parts: HashMap<String, usize> = HashMap::new();
        for a in self.agents.iter().filter(|a| !a.is_shell()) {
            *agent_parts
                .entry(a.agent_part.spoken.to_lowercase())
                .or_default() += 1;
        }
        self.agents
            .iter()
            .filter(|a| {
                let mut phrases: Vec<&str> = vec![&a.shown, &a.spoken];
                let part_forms = |p: &'_ NamePart| -> Vec<String> {
                    let mut forms = vec![p.shown.clone(), p.spoken.clone(), p.word_form.clone()];
                    forms.extend(p.alias.clone());
                    forms
                };
                let mut owned: Vec<String> = Vec::new();
                for part in [a.project.as_ref(), a.tab.as_ref()].into_iter().flatten() {
                    owned.extend(part_forms(part));
                }
                let unique = agent_parts
                    .get(&a.agent_part.spoken.to_lowercase())
                    .is_some_and(|n| *n == 1);
                if unique && !a.is_shell() {
                    owned.extend(part_forms(&a.agent_part));
                }
                phrases.extend(owned.iter().map(String::as_str));
                phrases.into_iter().any(|p| {
                    let words = word_tokens(p);
                    let long_enough =
                        words.len() > 1 || words.first().is_some_and(|w| w.len() >= 3);
                    long_enough && said.windows(words.len()).any(|w| w == words.as_slice())
                })
            })
            .collect()
    }
}

/// Lowercase letter-and-digit words.
fn word_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Name every pane in `snapshot` and resolve `target` to its pane id.
pub fn resolve_pane_id(
    snapshot: &serde_json::Value,
    config: &NamingConfig,
    target: &str,
) -> Result<String, ResolveError> {
    Names::from_snapshot(snapshot, config)
        .resolve(target)
        .map(|a| a.agent.pane_id.clone())
}

/// Step 4 of resolution: the full shown or spoken name, or any part's
/// shown, spoken or word form, its capitals, or its words all spelled.
fn matches_spoken(a: &NamedAgent, target: &str, config: &NamingConfig) -> bool {
    if normalise(&a.shown) == target || normalise(&a.spoken) == target {
        return true;
    }
    let compact_target = compact(target);
    [Some(&a.agent_part), a.project.as_ref(), a.tab.as_ref()]
        .into_iter()
        .flatten()
        .any(|p| {
            let spelled = words(&p.word_form)
                .iter()
                .map(|w| spell(w, config))
                .collect::<Vec<_>>()
                .join(" ");
            [&p.shown, &p.spoken, &p.word_form, &spelled]
                .iter()
                .any(|form| !form.is_empty() && normalise(form) == target)
                // Capitals as STT may write them: "HR", "H R", "H.R."
                || (!p.word_form.is_empty()
                    && compact(&p.word_form) == compact_target)
        })
}

/// Lowercase, collapse whitespace, drop trailing sentence punctuation.
fn normalise(s: &str) -> String {
    let s = s
        .trim()
        .trim_end_matches(['.', ',', '!', '?'])
        .to_lowercase();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Letters and digits only, lowercased.
fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn workspace_label<'l>(r: &AgentRef, labels: &'l HashMap<String, String>) -> Option<&'l str> {
    r.workspace_id
        .as_deref()
        .and_then(|ws| labels.get(ws))
        .map(String::as_str)
}

fn cwd_basename(cwd: &str) -> Option<String> {
    cwd.trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Decision 6: the shortest rung of the ladder that is unique among `peers`
/// (which include `target`), as (shown, spoken).
fn ladder_name(target: &NamedAgent, peers: &[&NamedAgent]) -> (String, String) {
    let others: Vec<&NamedAgent> = peers
        .iter()
        .copied()
        .filter(|p| p.agent.pane_id != target.agent.pane_id)
        .collect();
    let same = |a: Option<&NamePart>, b: Option<&NamePart>| match (a, b) {
        (Some(a), Some(b)) => a.spoken.eq_ignore_ascii_case(&b.spoken),
        _ => false,
    };
    let agent = &target.agent_part;

    // 1. The agent.
    if !others
        .iter()
        .any(|p| same(Some(&p.agent_part), Some(agent)))
    {
        return (agent.shown.clone(), agent.spoken.clone());
    }

    let project = target.project.as_ref();
    if let Some(project) = project {
        // 2. The project.
        if !others
            .iter()
            .any(|p| same(p.project.as_ref(), Some(project)))
        {
            let noun = if target.is_shell() { "shell" } else { "agent" };
            return (
                format!("the {} {noun}", project.shown),
                format!("the {} {noun}", project.spoken),
            );
        }
        // 3. The agent in the project.
        let rival = |p: &&NamedAgent| {
            same(Some(&p.agent_part), Some(agent)) && same(p.project.as_ref(), Some(project))
        };
        if !others.iter().any(rival) {
            return (
                format!("{} in {}", agent.shown, project.shown),
                format!("{} in {}", agent.spoken, project.spoken),
            );
        }
    }

    // The agents still sharing agent and project with the target.
    let group: Vec<&NamedAgent> = others
        .iter()
        .copied()
        .filter(|p| {
            same(Some(&p.agent_part), Some(agent))
                && match (p.project.as_ref(), project) {
                    (None, None) => true,
                    (a, b) => same(a, b),
                }
        })
        .collect();
    let (base_shown, base_spoken) = match project {
        Some(p) => (
            format!("{} in {}", agent.shown, p.shown),
            format!("{} in {}", agent.spoken, p.spoken),
        ),
        None => (agent.shown.clone(), agent.spoken.clone()),
    };

    // 4. The tab.
    if let Some(tab) = target.tab.as_ref()
        && !group.iter().any(|p| same(p.tab.as_ref(), Some(tab)))
    {
        return (
            format!("{base_shown}, tab {}", tab.shown),
            format!("{base_spoken}, tab {}", tab.spoken),
        );
    }

    // 5. The tab number.
    let number = target.agent.tab_number;
    if let Some(n) = number
        && !group.iter().any(|p| p.agent.tab_number == Some(n))
    {
        return (
            format!("{base_shown}, tab {n}"),
            format!("{base_spoken}, tab {n}"),
        );
    }

    // Past the ladder (two such agents in one tab): their order among the
    // peers, so the name is still never a pane id.
    let ordinal = peers
        .iter()
        .filter(|p| {
            p.agent.pane_id == target.agent.pane_id
                || (group.iter().any(|g| g.agent.pane_id == p.agent.pane_id)
                    && p.agent.tab_number == number)
        })
        .position(|p| p.agent.pane_id == target.agent.pane_id)
        .unwrap_or(0)
        + 1;
    let tab = number.map(|n| format!(", tab {n}")).unwrap_or_default();
    (
        format!("{base_shown}{tab}, number {ordinal}"),
        format!("{base_spoken}{tab}, number {ordinal}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// The shape of the 2026-10-22 snapshot: all 11 Claude panes in
    /// workspace w2, "cc-highbay", with top-level tabs. Plus one shell.
    fn highbay() -> Value {
        let tabs = [
            ("w2:pZ", "17-fl", 17, "/Users/tmeade/src/flankr"),
            ("w2:p2", "16-stt", 16, "/Users/tmeade/src/libvoicexn"),
            ("w2:p3", "-ad", 3, "/Users/tmeade/src/libmorph"),
            ("w2:p4", "-mb", 4, "/Users/tmeade/src/libmorph"),
            ("w2:p5", "-bb", 5, "/Users/tmeade/src/bubble"),
            ("w2:p6", "ctrls", 6, "/Users/tmeade/src/controls"),
            ("w2:p7", "-sigsp", 7, "/Users/tmeade/src/sigsp"),
            ("w2:p8", "-sigcc", 8, "/Users/tmeade/src/sigcc"),
            ("w2:p9", "5-kp", 9, "/Users/tmeade/src/keypad"),
            ("w2:pA", "2", 2, "/Users/tmeade/src/highbay"),
            ("w2:pB", "1", 1, "/Users/tmeade/src/herdr"),
        ];
        let mut tab_list: Vec<Value> = Vec::new();
        let mut panes: Vec<Value> = Vec::new();
        for (i, (pane, label, number, cwd)) in tabs.iter().enumerate() {
            let tab_id = format!("w2:t{}", i + 1);
            tab_list.push(json!({
                "tab_id": tab_id, "workspace_id": "w2", "number": number,
                "label": label, "focused": false, "pane_count": 1,
                "agent_status": "idle"
            }));
            panes.push(json!({
                "pane_id": pane, "tab_id": tab_id, "workspace_id": "w2",
                "cwd": cwd, "agent": "claude", "agent_status": "idle",
                "focused": false, "terminal_title": "claude"
            }));
        }
        tab_list.push(json!({"tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "1"}));
        panes.push(json!({
            "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1",
            "cwd": "/Users/tmeade"
        }));
        json!({"result": {"snapshot": {
            "agents": [], "focused_pane_id": "w2:pZ", "focused_tab_id": "w2:t1",
            "focused_workspace_id": "w2", "layouts": [], "protocol": 1, "version": "x",
            "workspaces": [
                {"workspace_id": "w1", "label": "home", "number": 1, "active_tab_id": "w1:t1"},
                {"workspace_id": "w2", "label": "cc-highbay", "number": 2, "active_tab_id": "w2:t1"}
            ],
            "tabs": tab_list,
            "panes": panes
        }}})
    }

    fn cfg() -> NamingConfig {
        NamingConfig::default()
    }

    fn names(snap: &Value) -> Names {
        Names::from_snapshot(snap, &cfg())
    }

    #[test]
    fn utterances_name_agents_by_their_parts() {
        let snap = highbay();
        let n = names(&snap);
        let ids = |said: &str| -> Vec<String> {
            n.named_in(said)
                .into_iter()
                .map(|a| a.agent.pane_id.clone())
                .collect()
        };
        assert_eq!(ids("What is the flankr agent waiting on?"), ["w2:pZ"]);
        assert_eq!(ids("and flankr?"), ["w2:pZ"]);
        // Two agents share the project.
        assert_eq!(ids("check libmorph"), ["w2:p3", "w2:p4"]);
        assert_eq!(ids("what about tab em bee"), ["w2:p4"]);
        // Every agent is Claude: the agent part alone names none.
        assert!(ids("what is claude doing").is_empty());
        // Words, not substrings; short parts alone do not count.
        assert!(ids("flankrs everywhere").is_empty());
        assert!(ids("fl").is_empty());
        assert!(ids("").is_empty());
    }

    fn contains_pane_id(name: &str, pane_id: &str) -> bool {
        let short = pane_id.rsplit(':').next().unwrap_or(pane_id);
        name.contains(pane_id)
            || name
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| w == short)
    }

    // ── step 1: names ──

    #[test]
    fn every_claude_is_named_without_a_pane_id() {
        let n = names(&highbay());
        let claudes: Vec<&NamedAgent> = n.agents.iter().filter(|a| !a.is_shell()).collect();
        assert_eq!(claudes.len(), 11);
        for a in &n.agents {
            for name in [&a.shown, &a.spoken] {
                assert!(
                    !contains_pane_id(name, &a.agent.pane_id),
                    "{name:?} names pane {}",
                    a.agent.pane_id
                );
                assert!(!name.is_empty());
            }
        }
        // All share cc-highbay, so the project is the cwd's basename.
        assert_eq!(n.get("w2:pZ").unwrap().shown, "the flankr agent");
        assert_eq!(n.get("w2:p2").unwrap().spoken, "the libvoicexn agent");
        // The lone shell is "Shell".
        assert_eq!(n.get("w1:p1").unwrap().shown, "Shell");
    }

    #[test]
    fn libmorph_panes_are_named_by_tab() {
        let n = names(&highbay());
        let mb = n.get("w2:p4").unwrap();
        assert_eq!(mb.shown, "Claude in libmorph, tab -mb");
        assert_eq!(mb.spoken, "Claude in libmorph, tab em bee");
        let ad = n.get("w2:p3").unwrap();
        assert_eq!(ad.shown, "Claude in libmorph, tab -ad");
        // `ad` has a vowel, so it is read as written.
        assert_eq!(ad.spoken, "Claude in libmorph, tab ad");
    }

    #[test]
    fn workspace_label_is_the_project_when_not_shared() {
        let snap = json!({"snapshot": {
            "workspaces": [
                {"workspace_id": "w1", "label": "flankr"},
                {"workspace_id": "w2", "label": "highbay"}
            ],
            "panes": [
                {"pane_id": "w1:p6", "agent": "claude", "workspace_id": "w1", "cwd": "/src/x"},
                {"pane_id": "w2:p1", "agent": "claude", "workspace_id": "w2", "cwd": "/src/x"}
            ]
        }});
        let workspace = NamingConfig {
            project_from: ProjectFrom::Workspace,
            ..NamingConfig::default()
        };
        let n = Names::from_snapshot(&snap, &workspace);
        assert_eq!(n.get("w1:p6").unwrap().shown, "the flankr agent");
        assert_eq!(n.get("w2:p1").unwrap().shown, "the highbay agent");
    }

    #[test]
    fn the_directory_is_the_project_by_default() {
        // One workspace for everything, as the owner uses herdr: its label
        // says nothing, the directories do.
        let snap = json!({"snapshot": {
            "workspaces": [{"workspace_id": "w2", "label": "cc-highbay"}],
            "panes": [
                {"pane_id": "w2:pZ", "agent": "claude", "workspace_id": "w2", "cwd": "/src/flankr"},
                {"pane_id": "w2:pR", "agent": "claude", "workspace_id": "w2", "cwd": "/src/libmorph"}
            ]
        }});
        let n = names(&snap);
        assert_eq!(
            n.get("w2:pZ").unwrap().project.as_ref().unwrap().shown,
            "flankr"
        );
        assert_eq!(
            n.get("w2:pR").unwrap().project.as_ref().unwrap().shown,
            "libmorph"
        );
        // No cwd: the label, rather than nothing.
        let lone = json!({"snapshot": {
            "workspaces": [{"workspace_id": "w1", "label": "docs"}],
            "panes": [{"pane_id": "w1:p1", "agent": "claude", "workspace_id": "w1"}]
        }});
        assert_eq!(
            names(&lone)
                .get("w1:p1")
                .unwrap()
                .project
                .as_ref()
                .unwrap()
                .shown,
            "docs"
        );
    }

    #[test]
    fn ladder_rungs() {
        // Lone agent: its name.
        let snap = json!({"snapshot": {"panes": [
            {"pane_id": "w1:p1", "agent": "claude", "cwd": "/src/flankr"},
            {"pane_id": "w1:p2", "agent": "agy", "cwd": "/src/flankr"},
            {"pane_id": "w1:p3", "agent": "claude", "cwd": "/src/highbay"},
            {"pane_id": "w1:p4", "agent": "agy", "cwd": "/src/other"}
        ]}});
        let n = names(&snap);
        // Claude twice and flankr holds two agents: rung 3.
        assert_eq!(n.get("p1").unwrap().shown, "Claude in flankr");
        // highbay holds only one agent: rung 2.
        assert_eq!(n.get("p3").unwrap().shown, "the highbay agent");
        assert_eq!(n.get("p2").unwrap().shown, "Agy in flankr");

        // Same agent, project and tab label: rung 5, the tab number.
        let snap = json!({"snapshot": {
            "tabs": [
                {"tab_id": "t1", "number": 24, "label": "dev"},
                {"tab_id": "t2", "number": 25, "label": "dev"},
                {"tab_id": "t3", "number": 26, "label": "dev"}
            ],
            "panes": [
                {"pane_id": "w1:p1", "agent": "claude", "cwd": "/src/libmorph", "tab_id": "t1"},
                {"pane_id": "w1:p2", "agent": "claude", "cwd": "/src/libmorph", "tab_id": "t2"},
                {"pane_id": "w1:p3", "agent": "claude", "cwd": "/src/libmorph", "tab_id": "t3"},
                {"pane_id": "w1:p4", "agent": "claude", "cwd": "/src/libmorph", "tab_id": "t3"}
            ]
        }});
        let n = names(&snap);
        assert_eq!(n.get("p1").unwrap().shown, "Claude in libmorph, tab 24");
        assert_eq!(n.get("p2").unwrap().spoken, "Claude in libmorph, tab 25");
        // Two in one tab: past the ladder, still no pane id.
        assert_eq!(
            n.get("p3").unwrap().shown,
            "Claude in libmorph, tab 26, number 1"
        );
        assert_eq!(
            n.get("p4").unwrap().shown,
            "Claude in libmorph, tab 26, number 2"
        );
    }

    #[test]
    fn numeric_or_empty_tab_label() {
        let snap = json!({"snapshot": {
            "tabs": [
                {"tab_id": "t1", "number": 9, "label": "2"},
                {"tab_id": "t2", "number": 7, "label": ""}
            ],
            "panes": [
                {"pane_id": "w1:p1", "agent": "claude", "cwd": "/src/work", "tab_id": "t1"},
                {"pane_id": "w1:p2", "agent": "claude", "cwd": "/src/work", "tab_id": "t2"}
            ]
        }});
        let n = names(&snap);
        assert_eq!(n.get("p1").unwrap().spoken, "Claude in work, tab 2");
        assert_eq!(n.get("p2").unwrap().shown, "Claude in work, tab 7");
    }

    #[test]
    fn tab_word_forms() {
        assert_eq!(tab_word_form("17-hr").as_deref(), Some("hr"));
        assert_eq!(tab_word_form("-mb").as_deref(), Some("mb"));
        assert_eq!(tab_word_form("5-kp").as_deref(), Some("kp"));
        assert_eq!(tab_word_form("ctrls").as_deref(), Some("ctrls"));
        assert_eq!(tab_word_form("-sig-sp-").as_deref(), Some("sig sp"));
        assert_eq!(tab_word_form("2"), None);
        assert_eq!(tab_word_form(""), None);
        assert_eq!(tab_word_form("-"), None);
    }

    #[test]
    fn spelling_examples() {
        let c = cfg();
        assert_eq!(spoken_word("hr", &c), "aitch ar");
        assert_eq!(spoken_word("mb", &c), "em bee");
        assert_eq!(spoken_word("kp", &c), "kay pee");
        assert_eq!(spoken_word("stt", &c), "ess tee tee");
        assert_eq!(spoken_word("ctrls", &c), "see tee ar el ess");
        for word in ["ad", "bubble", "sigsp", "libmorph"] {
            assert_eq!(spoken_word(word, &c), word);
        }
        // Digits are left as they are.
        assert_eq!(spell("kp2", &c), "kay pee 2");
        assert_eq!(spoken_word("42", &c), "42");
    }

    #[test]
    fn all_letter_names() {
        let c = cfg();
        let expected = [
            "ay",
            "bee",
            "see",
            "dee",
            "ee",
            "ef",
            "jee",
            "aitch",
            "eye",
            "jay",
            "kay",
            "el",
            "em",
            "en",
            "oh",
            "pee",
            "cue",
            "ar",
            "ess",
            "tee",
            "you",
            "vee",
            "double you",
            "ex",
            "why",
            "zee",
        ];
        for (i, name) in expected.iter().enumerate() {
            let letter = (b'a' + i as u8) as char;
            assert_eq!(letter_name(letter, &c).as_deref(), Some(*name));
            assert_eq!(
                letter_name(letter.to_ascii_uppercase(), &c).as_deref(),
                Some(*name)
            );
        }
        assert_eq!(letter_name('3', &c), None);
    }

    #[test]
    fn seven_letter_word_without_vowel_is_read_as_written() {
        assert_eq!(spoken_word("bcdfghj", &cfg()), "bcdfghj");
        assert_eq!(spoken_word("bcdfgh", &cfg()), "bee see dee ef jee aitch");
    }

    // ── step 1: resolution ──

    #[test]
    fn fl_resolves_to_flankr() {
        let n = names(&highbay());
        for target in [
            "fl", "FL", "17-fl", "ef el", "tab fl", "w2:pZ", "pZ", "F.L.",
        ] {
            assert_eq!(
                n.resolve(target).map(|a| a.agent.pane_id.as_str()),
                Ok("w2:pZ"),
                "{target}"
            );
        }
        assert_eq!(
            resolve_pane_id(&highbay(), &cfg(), "the flankr agent").as_deref(),
            Ok("w2:pZ")
        );
    }

    #[test]
    fn ad_resolves_spelled_or_capitals() {
        let n = names(&highbay());
        for target in ["ay dee", "AD", "-ad", "ad"] {
            assert_eq!(
                n.resolve(target).map(|a| a.agent.pane_id.as_str()),
                Ok("w2:p3"),
                "{target}"
            );
        }
        assert_eq!(
            n.resolve("em bee").map(|a| a.agent.pane_id.as_str()),
            Ok("w2:p4")
        );
    }

    #[test]
    fn tab_number_resolves() {
        let n = names(&highbay());
        // Tab 16's label is "16-stt"; "16" only matches its number.
        assert_eq!(
            n.resolve("16").map(|a| a.agent.pane_id.as_str()),
            Ok("w2:p2")
        );
        // Label "1" exists in both workspaces; the agent is preferred over
        // the shell.
        assert_eq!(
            n.resolve("1").map(|a| a.agent.pane_id.as_str()),
            Ok("w2:pB")
        );
    }

    #[test]
    fn ambiguous_label_lists_candidates() {
        let snap = json!({"snapshot": {
            "workspaces": [
                {"workspace_id": "w1", "label": "alpha"},
                {"workspace_id": "w2", "label": "beta"}
            ],
            "tabs": [
                {"tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "dev"},
                {"tab_id": "w2:t1", "workspace_id": "w2", "number": 1, "label": "dev"}
            ],
            "panes": [
                {"pane_id": "w1:p1", "agent": "claude", "workspace_id": "w1", "tab_id": "w1:t1"},
                {"pane_id": "w2:p1", "agent": "claude", "workspace_id": "w2", "tab_id": "w2:t1"}
            ]
        }});
        let n = names(&snap);
        let err = n.resolve("dev").unwrap_err();
        assert_eq!(
            err,
            ResolveError::Ambiguous {
                target: "dev".to_string(),
                candidates: vec!["the alpha agent".to_string(), "the beta agent".to_string()],
            }
        );
        assert!(err.to_string().contains("the alpha agent"));
        assert!(!err.to_string().contains("w1:p1"));
        assert_eq!(
            n.resolve("nope"),
            Err(ResolveError::NotFound {
                target: "nope".to_string()
            })
        );
    }

    // ── step 1: vocabulary ──

    #[test]
    fn vocabulary_has_labels_spoken_forms_capitals_and_spellings() {
        let vocab = names(&highbay()).vocabulary();
        for word in [
            "Claude",
            "flankr",
            "libmorph",
            "cc-highbay",
            "17-fl",
            "fl",
            "ef el",
            "FL",
            "-mb",
            "em bee",
            "MB",
            "CTRLS",
            "see tee ar el ess",
            "AD",
            "ay dee",
        ] {
            assert!(vocab.iter().any(|v| v == word), "missing {word:?}");
        }
        assert!(!vocab.iter().any(|v| v.contains("pZ")));
        let unique: HashSet<&String> = vocab.iter().collect();
        assert_eq!(unique.len(), vocab.len());
    }

    #[test]
    fn vocabulary_spells_sy() {
        let snap = json!({"snapshot": {
            "tabs": [{"tab_id": "t1", "number": 3, "label": "-sy"}],
            "panes": [{"pane_id": "w1:p1", "agent": "claude", "cwd": "/src/x", "tab_id": "t1"}]
        }});
        let n = names(&snap);
        let vocab = n.vocabulary();
        assert!(vocab.iter().any(|v| v == "ess why"));
        assert!(vocab.iter().any(|v| v == "SY"));
        // Spoken as written: `y` is a vowel.
        assert_eq!(n.agents[0].tab.as_ref().unwrap().spoken, "sy");
        assert_eq!(
            n.resolve("ess why").map(|a| a.agent.pane_id.as_str()),
            Ok("w1:p1")
        );
    }

    // ── step 2: configuration ──

    #[test]
    fn letter_override_changes_spelling() {
        let mut c = cfg();
        c.letters.insert("w".to_string(), "dub".to_string());
        c.letters.insert("H".to_string(), "haitch".to_string());
        assert_eq!(spoken_word("wd", &c), "dub dee");
        assert_eq!(spoken_word("hr", &c), "haitch ar");
        assert_eq!(letter_name('x', &c).as_deref(), Some("ex"));
    }

    #[test]
    fn spell_max_len_zero_turns_spelling_off() {
        let mut c = cfg();
        c.spell_max_len = 0;
        assert_eq!(spoken_word("hr", &c), "hr");
        let n = Names::from_snapshot(&highbay(), &c);
        assert_eq!(n.get("w2:p4").unwrap().spoken, "Claude in libmorph, tab mb");
        assert!(!n.vocabulary().iter().any(|v| v == "em bee"));
    }

    #[test]
    fn vowels_are_configurable() {
        let mut c = cfg();
        c.vowels = "aeiou".to_string();
        assert_eq!(spoken_word("sy", &c), "ess why");
    }

    #[test]
    fn alias_replaces_tab_name_in_both_forms() {
        let mut c = cfg();
        c.aliases
            .insert("CTRLS".to_string(), "controls".to_string());
        c.aliases.insert("-mb".to_string(), "morph bee".to_string());
        let n = Names::from_snapshot(&highbay(), &c);
        let mb = n.get("w2:p4").unwrap();
        assert_eq!(mb.shown, "Claude in libmorph, tab morph bee");
        assert_eq!(mb.spoken, "Claude in libmorph, tab morph bee");
        let vocab = n.vocabulary();
        assert!(vocab.iter().any(|v| v == "controls"));
        assert!(vocab.iter().any(|v| v == "morph bee"));
        // Alias values are not spelled.
        assert!(!vocab.iter().any(|v| v == "see tee ar el ess"));
        assert_eq!(
            n.resolve("Morph Bee").map(|a| a.agent.pane_id.as_str()),
            Ok("w2:p4")
        );
        assert_eq!(
            n.resolve("controls").map(|a| a.agent.pane_id.as_str()),
            Ok("w2:p6")
        );
    }

    #[test]
    fn alias_replaces_agent_and_project() {
        let mut c = cfg();
        c.aliases
            .insert("agy".to_string(), "antigravity".to_string());
        c.aliases.insert("highbay".to_string(), "bay".to_string());
        let snap = json!({"snapshot": {"panes": [
            {"pane_id": "w1:p1", "agent": "agy", "cwd": "/src/a"},
            {"pane_id": "w1:p2", "agent": "claude", "cwd": "/src/highbay"},
            {"pane_id": "w1:p3", "agent": "claude", "cwd": "/src/other"}
        ]}});
        let n = Names::from_snapshot(&snap, &c);
        assert_eq!(n.get("p1").unwrap().shown, "antigravity");
        assert_eq!(n.get("p2").unwrap().spoken, "the bay agent");
    }
}
