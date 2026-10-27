//! `SUGGESTIONS.md`: the rules that pick a pending question's answers,
//! read with libtypedmd (docs/design/20261027_06_SUGGESTION_RULES_PLAN.md).
//!
//! A `# [Rule] <name>` object has `when` (the words a question must have),
//! an optional `unless` (words it must not have) and `offers` (the answers'
//! names). A `# [Answer] <name>` object has `label`, `style`, exactly one
//! of `sends` and `starts`, and optional `command` and `answers` flags. The
//! texts' `{{ [abc] name }}` slots are `detected_plan_id`, `agent` and
//! `project` (decision 4).
//!
//! The rules are tried in order and the first that matches gives the
//! answers (decision 2). An object with an error is left out and the rest
//! load (decision 6). A user file is merged over the shipped one on the
//! parsed objects (the 14:13 EDT amendment): an object with the same tag
//! and name replaces it where it stands, a new rule goes before the
//! shipped ones, a new answer is added, and a user object with an error is
//! left out, the shipped one it would have replaced staying.

use std::collections::BTreeMap;

use libtypedmd::{Node, NodeKind, Text, TextPart};

use crate::sessions::links::{bare, detected_plan_id, words_of};

/// The shipped rules, compiled in as `ADDITIONAL_PROMPTS.md` is.
pub const SUGGESTIONS_RAW: &str = include_str!("../SUGGESTIONS.md");

/// The tags of the two kinds of object.
pub const RULE_TAG: &str = "Rule";
pub const ANSWER_TAG: &str = "Answer";

/// The rule whose match marks a question `about_plan` for older apps
/// (decision 5).
pub const PLAN_RULE: &str = "plan";

/// The styles an answer may have (decision 3).
pub const STYLES: [&str; 5] = ["approve", "deny", "green", "link", "yellow"];

/// The slots an answer's text may use (decision 4).
pub const SLOTS: [&str; 3] = ["detected_plan_id", "agent", "project"];

/// The longest text an answer sends or starts, in characters, after
/// replacement (decision 3).
pub const TEXT_MAX: usize = 200;

/// What is offered when no rule loads (decision 6): yes and no, as the
/// app's fallback.
const FALLBACK: &str = "# [Rule] question\n\n- offers: yes, no\n\n\
# [Answer] yes\n\n- label: Yes\n- style: approve\n- sends: yes\n- answers: true\n\n\
# [Answer] no\n\n- label: No\n- style: deny\n- sends: no\n- answers: true\n";

/// Words to look for: every group must be present, and a group is present
/// when one of its alternatives is. An alternative is a word, or words
/// that must follow each other.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Words {
    groups: Vec<Vec<Vec<String>>>,
}

impl Words {
    fn parse(text: &str) -> Result<Words, String> {
        let mut groups = Vec::new();
        for group in text.split(',') {
            let mut alternatives = Vec::new();
            for alternative in group.split('|') {
                // Split as a question's words are, so "don't" matches.
                let words: Vec<String> = words_of(alternative).into_iter().map(bare).collect();
                if words.is_empty() || words.iter().any(String::is_empty) {
                    return Err(format!("`{}` has an empty word", text.trim()));
                }
                alternatives.push(words);
            }
            groups.push(alternatives);
        }
        Ok(Words { groups })
    }

    /// Whether every group has an alternative among `words` (bare).
    fn all_in(&self, words: &[String]) -> bool {
        self.groups.iter().all(|group| {
            group.iter().any(|phrase| {
                words
                    .windows(phrase.len())
                    .any(|w| w.iter().zip(phrase).all(|(a, b)| a == b))
            })
        })
    }

    /// Whether any group has an alternative among `words` (for `unless`).
    fn any_in(&self, words: &[String]) -> bool {
        self.groups.iter().any(|group| {
            Words {
                groups: vec![group.clone()],
            }
            .all_in(words)
        })
    }
}

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    when: Option<Words>,
    unless: Option<Words>,
    /// The answers' names, in order.
    pub offers: Vec<String>,
}

impl Rule {
    /// Whether the rule takes a question of these (bare) words.
    fn matches(&self, words: &[String]) -> bool {
        self.when.as_ref().is_none_or(|w| w.all_in(words))
            && !self.unless.as_ref().is_some_and(|w| w.any_in(words))
    }
}

/// A piece of an answer's text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Literal(String),
    Slot(String),
}

/// One answer, its text unfilled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerDef {
    pub name: String,
    pub label: String,
    pub style: String,
    text: Vec<Piece>,
    /// Its text starts the message card rather than being sent.
    pub starts: bool,
    pub command: bool,
    pub answers: bool,
}

/// An answer as sent with a question: its text filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filled {
    pub label: String,
    pub style: String,
    pub sends: Option<String>,
    pub starts: Option<String>,
    pub command: bool,
    pub answers: bool,
}

/// The values the slots are filled from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Who<'a> {
    /// The agent's spoken name ("Claude").
    pub agent: &'a str,
    /// The project's name ("flankr").
    pub project: &'a str,
}

/// The answers a question gets, and the rule that gave them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub rule: String,
    /// `detected_plan_id` for the question.
    pub plan: String,
    pub answers: Vec<Filled>,
}

impl Offered {
    /// The question is about a plan: the `plan` rule matched.
    pub fn about_plan(&self) -> bool {
        self.rule == PLAN_RULE
    }
}

/// The rules and answers of one file, or of the merged files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suggestions {
    rules: Vec<Rule>,
    answers: BTreeMap<String, AnswerDef>,
}

/// One object as compiled, before the rules are checked against the
/// answers.
#[derive(Debug, Clone)]
enum Compiled {
    Rule(Rule),
    Answer(AnswerDef),
}

/// An object of a file: its tag, its name, and it compiled or why not.
struct Parsed {
    tag: &'static str,
    name: String,
    compiled: Result<Compiled, String>,
}

/// The `[Rule]` and `[Answer]` objects in `text`, in order, at any depth.
/// A second object with a tag and name already seen is an error.
fn objects(text: &str) -> Vec<Parsed> {
    fn walk(nodes: &[Node], out: &mut Vec<Parsed>) {
        for node in nodes {
            let Some(object) = node.as_object() else {
                continue;
            };
            let tag = match object.tag.as_deref() {
                Some(RULE_TAG) => RULE_TAG,
                Some(ANSWER_TAG) => ANSWER_TAG,
                _ => {
                    walk(&node.nodes, out);
                    continue;
                }
            };
            let name = object.head.render().trim().to_string();
            let compiled = if name.is_empty() {
                Err("it has no name".to_string())
            } else if out.iter().any(|p| p.tag == tag && p.name == name) {
                Err("defined twice; the first is kept".to_string())
            } else if tag == RULE_TAG {
                rule(&name, &node.nodes).map(Compiled::Rule)
            } else {
                answer(&name, &node.nodes).map(Compiled::Answer)
            };
            out.push(Parsed {
                tag,
                name,
                compiled,
            });
        }
    }
    let stream = libtypedmd::parse(text);
    let mut out = Vec::new();
    for document in &stream.documents {
        walk(&document.nodes, &mut out);
    }
    out
}

/// The members of an object, by name, each once.
fn members<'a>(
    children: &'a [Node],
    allowed: &[&str],
) -> Result<BTreeMap<&'a str, &'a Text>, String> {
    static EMPTY: std::sync::OnceLock<Text> = std::sync::OnceLock::new();
    let mut out = BTreeMap::new();
    for child in children {
        let NodeKind::Member(member) = &child.kind else {
            continue;
        };
        let name = member.name.as_str();
        if !allowed.contains(&name) {
            return Err(format!("unknown key `{name}`"));
        }
        let rhs = member
            .rhs
            .as_ref()
            .unwrap_or_else(|| EMPTY.get_or_init(Text::empty));
        if out.insert(name, rhs).is_some() {
            return Err(format!("`{name}` is given twice"));
        }
    }
    Ok(out)
}

/// A member's value as plain text, with no slots.
fn plain(key: &str, text: &Text) -> Result<String, String> {
    if text.parts().iter().any(|p| matches!(p, TextPart::Slot(_))) {
        return Err(format!("`{key}` may not have a slot"));
    }
    Ok(text.render().trim().to_string())
}

fn flag(key: &str, text: Option<&&Text>) -> Result<bool, String> {
    match text.map(|t| plain(key, t)).transpose()?.as_deref() {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        Some(other) => Err(format!("`{key}` is `{other}`, not true or false")),
    }
}

fn rule(name: &str, children: &[Node]) -> Result<Rule, String> {
    let m = members(children, &["when", "unless", "offers"])?;
    let words = |key: &str| {
        m.get(key)
            .map(|t| plain(key, t).and_then(|s| Words::parse(&s)))
            .transpose()
    };
    let offers: Vec<String> = m
        .get("offers")
        .map(|t| plain("offers", t))
        .transpose()?
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if offers.is_empty() {
        return Err("it offers no answers".to_string());
    }
    Ok(Rule {
        name: name.to_string(),
        when: words("when")?,
        unless: words("unless")?,
        offers,
    })
}

fn answer(name: &str, children: &[Node]) -> Result<AnswerDef, String> {
    let m = members(
        children,
        &["label", "style", "sends", "starts", "command", "answers"],
    )?;
    let label = m
        .get("label")
        .map(|t| plain("label", t))
        .transpose()?
        .filter(|l| !l.is_empty())
        .ok_or("it has no `label`")?;
    let style = m
        .get("style")
        .map(|t| plain("style", t))
        .transpose()?
        .ok_or("it has no `style`")?;
    if !STYLES.contains(&style.as_str()) {
        return Err(format!(
            "style `{style}` is not one of {}",
            STYLES.join(", ")
        ));
    }
    let (text, starts) = match (m.get("sends"), m.get("starts")) {
        (Some(t), None) => (*t, false),
        (None, Some(t)) => (*t, true),
        _ => return Err("it needs exactly one of `sends` and `starts`".to_string()),
    };
    let mut pieces = Vec::new();
    for part in text.parts() {
        match part {
            TextPart::Literal(s) => pieces.push(Piece::Literal(s.clone())),
            TextPart::Slot(slot) => {
                let expr = slot.expr.trim();
                if !SLOTS.contains(&expr) {
                    return Err(format!("unknown slot `{expr}`"));
                }
                pieces.push(Piece::Slot(expr.to_string()));
            }
        }
    }
    let command = flag("command", m.get("command"))?;
    if command && starts {
        return Err("`command` needs `sends`".to_string());
    }
    Ok(AnswerDef {
        name: name.to_string(),
        label,
        style,
        text: pieces,
        starts,
        command,
        answers: flag("answers", m.get("answers"))?,
    })
}

impl Suggestions {
    /// Every rule and answer in `text`. Errors are per object: an object
    /// with one is left out and the rest load.
    pub fn parse(text: &str) -> (Suggestions, Vec<String>) {
        Self::merge(text, None)
    }

    /// The shipped `SUGGESTIONS.md`.
    pub fn shipped() -> (Suggestions, Vec<String>) {
        Self::parse(SUGGESTIONS_RAW)
    }

    /// Yes and no, for when no rule loads.
    pub fn fallback() -> Suggestions {
        Self::parse(FALLBACK).0
    }

    /// `base` with `user` merged over it on the parsed objects (the 14:13
    /// EDT amendment). Errors in the user file are named as such.
    pub fn merge(base: &str, user: Option<&str>) -> (Suggestions, Vec<String>) {
        let mut errors = Vec::new();
        let mut report = |file: &str, p: &Parsed, e: &str| {
            let kind = if p.tag == RULE_TAG { "rule" } else { "answer" };
            errors.push(format!("{file}{kind} `{}`: {e}", p.name));
        };
        // The shipped objects in order; each with the user's replacement,
        // tried first.
        let mut slots: Vec<(Parsed, Option<Compiled>)> = Vec::new();
        for p in objects(base) {
            if let Err(e) = &p.compiled {
                report("", &p, e);
            }
            slots.push((p, None));
        }
        let mut new_rules: Vec<Compiled> = Vec::new();
        let mut new_answers: Vec<Compiled> = Vec::new();
        for p in objects(user.unwrap_or("")) {
            let compiled = match &p.compiled {
                Ok(c) => c.clone(),
                Err(e) => {
                    report("the user file's ", &p, &format!("{e}; left out"));
                    continue;
                }
            };
            match slots
                .iter_mut()
                .find(|(b, _)| b.tag == p.tag && b.name == p.name)
            {
                Some((_, replacement)) => *replacement = Some(compiled),
                None if p.tag == RULE_TAG => new_rules.push(compiled),
                None => new_answers.push(compiled),
            }
        }
        // The answers first: the rules are checked against them.
        let mut answers = BTreeMap::new();
        let base_answers = slots.iter().filter(|(b, _)| b.tag == ANSWER_TAG);
        for (b, replacement) in base_answers {
            let chosen = replacement.clone().or_else(|| b.compiled.clone().ok());
            if let Some(Compiled::Answer(a)) = chosen {
                answers.insert(a.name.clone(), a);
            }
        }
        for c in new_answers {
            if let Compiled::Answer(a) = c {
                answers.insert(a.name.clone(), a);
            }
        }
        let unknown = |r: &Rule| r.offers.iter().find(|o| !answers.contains_key(*o)).cloned();
        let mut rules = Vec::new();
        let mut errors_after = Vec::new();
        for c in new_rules {
            if let Compiled::Rule(r) = c {
                match unknown(&r) {
                    Some(o) => errors_after.push(format!(
                        "the user file's rule `{}`: offers `{o}`, which is not defined; left out",
                        r.name
                    )),
                    None => rules.push(r),
                }
            }
        }
        for (b, replacement) in slots.into_iter().filter(|(b, _)| b.tag == RULE_TAG) {
            if let Some(Compiled::Rule(r)) = replacement {
                match unknown(&r) {
                    Some(o) => errors_after.push(format!(
                        "the user file's rule `{}`: offers `{o}`, which is not defined; \
                         the shipped one stays",
                        r.name
                    )),
                    None => {
                        rules.push(r);
                        continue;
                    }
                }
            }
            if let Ok(Compiled::Rule(r)) = b.compiled {
                match unknown(&r) {
                    Some(o) => errors_after.push(format!(
                        "rule `{}`: offers `{o}`, which is not defined",
                        r.name
                    )),
                    None => rules.push(r),
                }
            }
        }
        errors.extend(errors_after);
        (Suggestions { rules, answers }, errors)
    }

    /// The rules, in the order they are tried.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn answer(&self, name: &str) -> Option<&AnswerDef> {
        self.answers.get(name)
    }

    /// The first rule that takes `question`.
    pub fn rule_for(&self, question: &str) -> Option<&Rule> {
        let words: Vec<String> = words_of(question).into_iter().map(bare).collect();
        self.rules.iter().find(|r| r.matches(&words))
    }

    /// The answers `question` gets, filled: `context` is the rest of its
    /// reply, where a plan id the question lacks may be. `None` when no
    /// rule takes it.
    pub fn offer(&self, question: &str, context: &str, who: Who) -> Option<Offered> {
        let rule = self.rule_for(question)?;
        let plan = detected_plan_id(question, context);
        let answers = rule
            .offers
            .iter()
            .filter_map(|name| self.answers.get(name))
            .map(|a| a.fill(&plan, who))
            .collect();
        Some(Offered {
            rule: rule.name.clone(),
            plan,
            answers,
        })
    }
}

impl AnswerDef {
    /// The answer with its text filled and cut to [`TEXT_MAX`].
    pub fn fill(&self, detected_plan_id: &str, who: Who) -> Filled {
        let mut text = String::new();
        for piece in &self.text {
            match piece {
                Piece::Literal(s) => text.push_str(s),
                Piece::Slot(name) => text.push_str(match name.as_str() {
                    "detected_plan_id" => detected_plan_id,
                    "agent" => who.agent,
                    _ => who.project,
                }),
            }
        }
        let text: String = text.trim().chars().take(TEXT_MAX).collect();
        Filled {
            label: self.label.clone(),
            style: self.style.clone(),
            sends: (!self.starts).then(|| text.clone()),
            starts: self.starts.then_some(text),
            command: self.command,
            answers: self.answers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHO: Who = Who {
        agent: "Claude",
        project: "flankr",
    };

    fn labels(s: &Suggestions, question: &str, context: &str) -> Vec<String> {
        s.offer(question, context, WHO)
            .map(|o| o.answers.into_iter().map(|a| a.label).collect())
            .unwrap_or_default()
    }

    fn sends(s: &Suggestions, question: &str, context: &str, label: &str) -> Option<String> {
        let o = s.offer(question, context, WHO)?;
        let a = o.answers.into_iter().find(|a| a.label == label)?;
        a.sends.or(a.starts)
    }

    #[test]
    fn the_shipped_file_parses() {
        let (s, errors) = Suggestions::shipped();
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<&str> = s.rules().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["plan", "question"]);
        for a in ["yes", "no", "approve", "discuss", "implement"] {
            assert!(s.answer(a).is_some(), "{a}");
        }
        let discuss = s.answer("discuss").unwrap();
        assert!(discuss.starts && !discuss.answers && !discuss.command);
        assert_eq!(discuss.style, "link");
        let approve = s.answer("approve").unwrap();
        assert!(!approve.starts && approve.answers);
    }

    #[test]
    fn the_plan_rule_matches_plan_questions_only() {
        let (s, _) = Suggestions::shipped();
        for q in [
            "Approve plan 03?",
            "Approve the draft?",
            "Do you APPROVE, the plan?",
        ] {
            assert_eq!(s.rule_for(q).unwrap().name, "plan", "{q}");
            assert_eq!(labels(&s, q, ""), ["approve", "discuss", "implement"]);
        }
        for q in [
            "Restart the daemon on the plan 05 build?",
            "Is the planet planning approved?",
            "Install it on the S23?",
        ] {
            assert_eq!(s.rule_for(q).unwrap().name, "question", "{q}");
            assert_eq!(labels(&s, q, ""), ["Yes", "No"]);
        }
        let o = s.offer("Approve plan 03?", "", WHO).unwrap();
        assert!(o.about_plan());
        assert!(!s.offer("Push it?", "", WHO).unwrap().about_plan());
    }

    #[test]
    fn the_answers_are_filled() {
        let (s, _) = Suggestions::shipped();
        let o = s.offer("Approve plan 03?", "", WHO).unwrap();
        let yes_no = s.offer("Push it?", "", WHO).unwrap();
        assert_eq!(
            o.answers,
            vec![
                Filled {
                    label: "approve".into(),
                    style: "green".into(),
                    sends: Some("Approve plan 03".into()),
                    starts: None,
                    command: false,
                    answers: true,
                },
                Filled {
                    label: "discuss".into(),
                    style: "link".into(),
                    sends: None,
                    starts: Some("Discuss plan 03:".into()),
                    command: false,
                    answers: false,
                },
                Filled {
                    label: "implement".into(),
                    style: "yellow".into(),
                    sends: Some("Approve plan 03 and start building it".into()),
                    starts: None,
                    command: false,
                    answers: true,
                },
            ]
        );
        assert_eq!(yes_no.answers[0].sends.as_deref(), Some("yes"));
        assert_eq!(yes_no.answers[0].style, "approve");
        assert_eq!(yes_no.answers[1].sends.as_deref(), Some("no"));
        assert_eq!(yes_no.answers[1].style, "deny");
    }

    #[test]
    fn detected_plan_id_from_the_question_the_reply_or_neither() {
        let (s, _) = Suggestions::shipped();
        let approve = |q: &str, c: &str| sends(&s, q, c, "approve").unwrap();
        assert_eq!(approve("Approve plan 04?", ""), "Approve plan 04");
        assert_eq!(
            approve("Approve the draft?", "I wrote plan 20261027_06 as a draft."),
            "Approve plan 20261027_06"
        );
        assert_eq!(
            approve("Approve the draft?", "Nothing numbered."),
            "Approve the draft"
        );
        assert_eq!(approve("Shall I approve the plan?", ""), "Approve the plan");
        assert_eq!(
            approve("Approve plan 03 or plan 04?", ""),
            "Approve the plan"
        );
        assert_eq!(
            s.offer("Approve the draft?", "", WHO).unwrap().plan,
            "the draft"
        );
    }

    #[test]
    fn unless_stops_a_match_and_a_rule_without_when_takes_the_rest() {
        let text = "# [Rule] push\n\n- when: push | install\n- unless: not, never | don't\n\
                    - offers: go\n\n# [Rule] rest\n\n- offers: go\n\n\
                    # [Answer] go\n\n- label: go\n- style: green\n- sends: go {{ [abc] agent }} in {{ [abc] project }}\n";
        let (s, errors) = Suggestions::parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(s.rule_for("Push it?").unwrap().name, "push");
        assert_eq!(s.rule_for("Install it now?").unwrap().name, "push");
        assert_eq!(s.rule_for("Push it, or not?").unwrap().name, "rest");
        assert_eq!(s.rule_for("Never push it?").unwrap().name, "rest");
        assert_eq!(s.rule_for("Don't install?").unwrap().name, "rest");
        assert_eq!(s.rule_for("Anything?").unwrap().name, "rest");
        assert_eq!(
            sends(&s, "Push?", "", "go").as_deref(),
            Some("go Claude in flankr")
        );
    }

    #[test]
    fn a_phrase_must_be_whole_and_in_order() {
        let text = "# [Rule] r\n\n- when: go ahead\n- offers: a\n\n\
                    # [Answer] a\n\n- label: a\n- style: link\n- sends: a\n";
        let (s, errors) = Suggestions::parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(s.rule_for("Shall I go ahead?").is_some());
        assert!(s.rule_for("Ahead, or go?").is_none());
    }

    #[test]
    fn each_error_is_left_out_and_reported() {
        let text = "\
# [Rule] unknown_key\n\n- when: x\n- offer: a\n\n\
# [Rule] undefined\n\n- offers: a, missing\n\n\
# [Rule] empty_word\n\n- when: x,, y\n- offers: a\n\n\
# [Rule] good\n\n- offers: a\n\n\
# [Answer] a\n\n- label: A\n- style: green\n- sends: a\n\n\
# [Answer] slot\n\n- label: S\n- style: green\n- sends: {{ [abc] who }}\n\n\
# [Answer] both\n\n- label: B\n- style: green\n- sends: b\n- starts: b\n\n\
# [Answer] neither\n\n- label: N\n- style: green\n\n\
# [Answer] style\n\n- label: S\n- style: purple\n- sends: s\n\n\
# [Answer] flag\n\n- label: F\n- style: green\n- sends: f\n- answers: yes\n\n\
# [Answer] a\n\n- label: again\n- style: green\n- sends: again\n";
        let (s, errors) = Suggestions::parse(text);
        let names: Vec<&str> = s.rules().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["good"]);
        assert_eq!(s.answer("a").unwrap().label, "A");
        for gone in ["slot", "both", "neither", "style", "flag"] {
            assert!(s.answer(gone).is_none(), "{gone}");
        }
        let expect = [
            ("rule `unknown_key`", "unknown key `offer`"),
            ("rule `undefined`", "`missing`"),
            ("rule `empty_word`", "empty word"),
            ("answer `slot`", "unknown slot `who`"),
            ("answer `both`", "exactly one of"),
            ("answer `neither`", "exactly one of"),
            ("answer `style`", "purple"),
            ("answer `flag`", "not true or false"),
            ("answer `a`", "defined twice"),
        ];
        assert_eq!(errors.len(), expect.len(), "{errors:#?}");
        for (what, why) in expect {
            assert!(
                errors.iter().any(|e| e.contains(what) && e.contains(why)),
                "{what}: {why} in {errors:#?}"
            );
        }
    }

    #[test]
    fn a_command_answer_needs_sends() {
        let text = "# [Answer] c\n\n- label: cont\n- style: yellow\n- starts: continue\n- command: true\n\n\
                    # [Answer] d\n\n- label: cont\n- style: yellow\n- sends: continue\n- command: true\n";
        let (s, errors) = Suggestions::parse(text);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(s.answer("c").is_none());
        assert!(s.answer("d").unwrap().command);
    }

    #[test]
    fn a_sent_text_is_cut_to_200_characters() {
        let text = format!(
            "# [Rule] r\n\n- offers: a\n\n# [Answer] a\n\n- label: a\n- style: green\n- sends: {} {{{{ [abc] detected_plan_id }}}}\n",
            "x".repeat(198)
        );
        let (s, errors) = Suggestions::parse(&text);
        assert!(errors.is_empty(), "{errors:?}");
        let sent = sends(&s, "Approve plan 04?", "", "a").unwrap();
        assert_eq!(sent.chars().count(), TEXT_MAX);
        assert!(sent.ends_with(" p"));
    }

    #[test]
    fn the_fallback_is_yes_and_no() {
        let s = Suggestions::fallback();
        assert_eq!(labels(&s, "Anything?", ""), ["Yes", "No"]);
        let (none, _) = Suggestions::parse("Nothing here.");
        assert!(none.rules().is_empty());
    }

    const USER: &str = "\
# [Answer] approve\n\n- label: approve\n- style: green\n- sends: LGTM, {{ [abc] detected_plan_id }}\n- answers: true\n\n\
# [Rule] push\n\n- when: push\n- offers: push_it, no\n\n\
# [Answer] push_it\n\n- label: push it\n- style: green\n- sends: push it\n- answers: true\n\n\
# [Answer] discuss\n\n- label: discuss\n- style: link\n- sends: a\n- starts: b\n\n\
# [Rule] plan\n\n- when: approve, plan\n- offers: approve, nowhere\n";

    #[test]
    fn a_user_file_merges_over_the_shipped_one() {
        let (s, errors) = Suggestions::merge(SUGGESTIONS_RAW, Some(USER));
        // The new rule first, then the shipped ones in their order; the
        // broken replacement of `plan` leaves the shipped one.
        let names: Vec<&str> = s.rules().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["push", "plan", "question"]);
        assert_eq!(
            sends(&s, "Approve plan 04?", "", "approve").as_deref(),
            Some("LGTM, plan 04")
        );
        // The shipped `plan` still takes "draft".
        assert_eq!(s.rule_for("Approve the draft?").unwrap().name, "plan");
        assert_eq!(labels(&s, "Push it to main?", ""), ["push it", "No"]);
        // The broken discuss: the shipped one stays.
        assert!(s.answer("discuss").unwrap().starts);
        assert_eq!(
            sends(&s, "Approve plan 04?", "", "discuss").as_deref(),
            Some("Discuss plan 04:")
        );
        assert_eq!(errors.len(), 2, "{errors:#?}");
        assert!(errors.iter().all(|e| e.starts_with("the user file's")));
        assert!(errors.iter().any(|e| e.contains("`discuss`")));
        assert!(
            errors
                .iter()
                .any(|e| e.contains("`plan`") && e.contains("the shipped one stays"))
        );
    }
}
