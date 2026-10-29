//! Choice questions in Claude Code's reader (plan 20261029_01, step 2),
//! over JSONL lines written from finding 1's shapes. The questions are
//! made up; no transcript text is used.

use libherdr::protocol::AgentStatus;
use serde_json::{Value, json};

use super::*;
use crate::sessions::links::Linker;
use crate::sessions::sections::{self, Builder, Live, SLOT_QUESTION};
use crate::sessions::{LineReader, pending, turn};
use libflankr_proto::common::{CardKind, Section, Serial, Turn};

const ID: &str = "toolu_01TESTCHOICE";

fn reader() -> ClaudeCode {
    ClaudeCode::new(std::path::PathBuf::from("/nonexistent"))
}

fn ask_line(input: Value) -> String {
    json!({
        "type": "assistant", "timestamp": "2026-10-29T04:11:00.000Z", "isSidechain": false,
        "version": "2.1.310",
        "message": {"role": "assistant", "type": "message", "content": [
            {"type": "tool_use", "id": ID, "name": "AskUserQuestion", "input": input}
        ]}
    })
    .to_string()
}

fn result_line(content: &str, structured: Value, is_error: bool) -> String {
    let mut block = json!({"type": "tool_result", "tool_use_id": ID, "content": content});
    if is_error {
        block["is_error"] = json!(true);
    }
    json!({
        "type": "user", "timestamp": "2026-10-29T04:12:00.000Z", "isSidechain": false,
        "message": {"role": "user", "content": [block]},
        "toolUseResult": structured
    })
    .to_string()
}

fn two_questions() -> Value {
    json!({"questions": [
        {"question": "Which colour should the badge be?", "header": "Colour",
         "multiSelect": false, "options": [
            {"label": "Teal", "description": "Matches the bar"},
            {"label": "Amber", "description": "Stands out"}]},
        {"question": "Which rows get icons?", "header": "Icons",
         "multiSelect": true, "options": [
            {"label": "Files", "description": "File rows"},
            {"label": "Folders", "description": "Folder rows"},
            {"label": "Links", "description": "Link rows"}]}
    ]})
}

fn prompt_line() -> String {
    json!({"type": "user", "timestamp": "2026-10-29T04:10:00.000Z", "isSidechain": false,
        "message": {"role": "user", "content": "style the badges"}, "origin": {"kind": "human"}})
    .to_string()
}

fn items_of(lines: &[String]) -> Vec<Item> {
    let r = reader();
    lines.iter().flat_map(|l| r.parse_line(l)).collect()
}

/// The cards of `lines` read as one file, with `live` built from the
/// pending question when there is one.
fn cards_of(lines: &[String], status: &AgentStatus) -> Vec<Section> {
    let r = reader();
    let linker = Linker::new(None, None);
    let mut builder = Builder::new(7, linker.clone());
    let mut placed = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        placed.extend(builder.feed(n as u32 + 1, r.parse_line(line)));
    }
    let items: Vec<Item> = placed.iter().map(|p| p.item.clone()).collect();
    let q = pending::pending_question(&items, status, false, None);
    let live = Live {
        name: "Claude",
        status,
        quiet: false,
        pending_question: q.as_ref(),
        blocked_at: None,
    };
    sections::cards(7, &placed, &[], Some(&live), &linker)
        .into_iter()
        .filter(|s| {
            s.card
                .as_ref()
                .is_some_and(|c| c.card == CardKind::Question)
        })
        .collect()
}

#[test]
fn an_open_call_is_the_pending_question_with_its_choices() {
    let lines = [prompt_line(), ask_line(two_questions())];
    let items = items_of(&lines);
    let q = pending::pending_question(&items, &AgentStatus::Blocked, false, None).unwrap();
    assert_eq!(q.text, "Which colour should the badge be?");
    assert_eq!(q.ts, "2026-10-29T04:11:00.000Z");
    assert!(q.answers.is_empty() && !q.about_plan);
    let set = q.choices.unwrap();
    assert_eq!(set.id, ID);
    assert_eq!(set.source, "claude_code/AskUserQuestion");
    assert_eq!(set.questions.len(), 2);
    assert_eq!(set.questions[0].header, "Colour");
    assert!(!set.questions[0].multi && set.questions[0].other);
    assert_eq!(set.questions[0].options[1].label, "Amber");
    assert_eq!(set.questions[0].options[1].detail, "Stands out");
    assert!(set.questions[1].multi && set.questions[1].other);
    assert_eq!(set.questions[1].options.len(), 3);
    assert_eq!(
        turn::turn(&items, &AgentStatus::Blocked, false),
        Turn::Asking {
            question: "Which colour should the badge be?".into()
        }
    );
    // Working with the file not quiet: not yet waiting.
    assert!(pending::pending_question(&items, &AgentStatus::Working, false, None).is_none());
    assert!(pending::pending_question(&items, &AgentStatus::Working, true, None).is_some());
    // The live card is at the call's line.
    let cards = cards_of(&lines, &AgentStatus::Blocked);
    assert_eq!(cards.len(), 1);
    let body = cards[0].card.as_ref().unwrap();
    assert!(body.live);
    assert_eq!(body.title, "Claude is asking");
    assert_eq!(body.text, "Which colour should the badge be?");
    assert_eq!(cards[0].serial, Serial::new(7, 2, SLOT_QUESTION));
}

#[test]
fn an_answered_call_is_not_pending_and_its_card_lists_the_answers() {
    let structured = json!({
        "questions": two_questions()["questions"],
        "answers": {
            "Which colour should the badge be?": "Amber",
            "Which rows get icons?": "Files, Links"
        },
        "annotations": {}
    });
    let content = "Your questions have been answered: \"Which colour should the badge be?\"=\"Amber\", \"Which rows get icons?\"=\"Files, Links\". You can now continue with these answers in mind.";
    let lines = [
        prompt_line(),
        ask_line(two_questions()),
        result_line(content, structured, false),
    ];
    let items = items_of(&lines);
    assert!(pending::pending_question(&items, &AgentStatus::Blocked, false, None).is_none());
    assert!(!matches!(
        turn::turn(&items, &AgentStatus::Blocked, false),
        Turn::Asking { .. }
    ));
    let cards = cards_of(&lines, &AgentStatus::Blocked);
    assert_eq!(cards.len(), 1);
    let body = cards[0].card.as_ref().unwrap();
    assert!(!body.live);
    assert_eq!(body.text, "Colour: Amber\nIcons: Files, Links");
}

#[test]
fn a_rejected_call_says_closed_on_the_mac() {
    let content = "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.";
    let lines = [
        prompt_line(),
        ask_line(two_questions()),
        result_line(content, json!("Error: rejected"), true),
    ];
    let items = items_of(&lines);
    assert!(
        items
            .last()
            .unwrap()
            .choice_result
            .as_ref()
            .unwrap()
            .rejected
    );
    assert!(pending::pending_question(&items, &AgentStatus::Idle, false, None).is_none());
    let cards = cards_of(&lines, &AgentStatus::Idle);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].card.as_ref().unwrap().text, "Closed on the Mac");
}

#[test]
fn the_extended_form_has_no_choices() {
    let mut titled = two_questions();
    titled["title"] = json!("A survey");
    let mut text_kind = two_questions();
    text_kind["questions"][1]["kind"] = json!("text");
    let mut choice_kind = two_questions();
    choice_kind["questions"][0]["kind"] = json!("choice");
    for (input, has) in [(titled, false), (text_kind, false), (choice_kind, true)] {
        let items = items_of(&[ask_line(input)]);
        assert_eq!(items[0].choices.is_some(), has);
        assert_eq!(items[0].kind, EntryKind::ToolUse);
    }
    // Out of bounds: one option only, or five questions.
    let mut one = two_questions();
    one["questions"][0]["options"] = json!([{"label": "Only", "description": ""}]);
    assert!(items_of(&[ask_line(one)])[0].choices.is_none());
    let q = two_questions()["questions"][0].clone();
    let five = json!({"questions": [q, q, q, q, q]});
    assert!(items_of(&[ask_line(five)])[0].choices.is_none());
    // An open call with no choices is not a choice question.
    let mut titled = two_questions();
    titled["title"] = json!("A survey");
    let items = items_of(&[prompt_line(), ask_line(titled)]);
    assert!(pending::pending_question(&items, &AgentStatus::Blocked, false, None).is_none());
}

#[test]
fn a_preview_question_has_no_other_and_a_long_input_is_parsed_whole() {
    let preview = "    let layout = Layout::stacked(width, height, gap);\n".repeat(35);
    assert!(preview.chars().count() > 1000);
    let input = json!({"questions": [
        {"question": "Which layout reads best?", "header": "Layout", "multiSelect": false,
         "options": [
            {"label": "Stacked", "description": "One above the other", "preview": preview},
            {"label": "Side by side", "description": "Two columns", "preview": preview},
            {"label": "Tabs", "description": "One at a time", "preview": preview},
            {"label": "Grid", "description": "Two by two", "preview": preview}]},
        {"question": "Which extras?", "header": "Extras", "multiSelect": true,
         "options": [
            {"label": "Shadows", "description": "", "preview": "x"},
            {"label": "Borders", "description": ""}]}
    ]});
    assert!(input.to_string().chars().count() > 6000);
    let items = items_of(&[ask_line(input)]);
    // The item's text is cut; the choices are not.
    assert!(items[0].text.chars().count() <= TOOL_TEXT_MAX);
    let set = items[0].choices.as_ref().unwrap();
    assert!(!set.questions[0].other, "a single-select preview question");
    assert!(set.questions[1].other, "a multi-select one keeps Other");
    assert_eq!(set.questions[0].options.len(), 4);
    assert_eq!(set.questions[0].options[3].label, "Grid");
    assert_eq!(
        set.questions[0].options[3].preview.as_deref(),
        Some(preview.as_str())
    );
}

#[test]
fn a_null_tool_use_result_falls_back_to_the_contents_pairs() {
    let content = "The user answered: \"Which colour should the badge be?\"=\"a warm \\\"sunset\\\" orange\", \"Which rows get icons?\"=\"Folders\". Read the answers carefully — the user typed some of them.";
    let lines = [
        prompt_line(),
        ask_line(two_questions()),
        result_line(content, Value::Null, false),
    ];
    let items = items_of(&lines);
    let outcome = items.last().unwrap().choice_result.clone().unwrap();
    assert!(outcome.answers.is_empty() && outcome.content.is_some());
    let set = items[1].choices.as_ref().unwrap();
    assert_eq!(
        outcome.answers_for(set),
        vec![
            Some("a warm \\\"sunset\\\" orange".to_string()),
            Some("Folders".to_string())
        ]
    );
    let cards = cards_of(&lines, &AgentStatus::Idle);
    assert_eq!(
        cards[0].card.as_ref().unwrap().text,
        "Colour: a warm \\\"sunset\\\" orange\nIcons: Folders"
    );
    // A preview answer's note is not part of the answer.
    let preview = "Your questions have been answered: \"Which colour should the badge be?\"=\"Teal\" selected preview:\n[teal]\n, \"Which rows get icons?\"=\"Files\". You can now continue with these answers in mind.";
    let outcome = choice_outcome(None, preview, false).unwrap();
    assert_eq!(
        outcome.answers_for(set),
        vec![Some("Teal".to_string()), Some("Files".to_string())]
    );
    // A plain result is not a choice outcome.
    assert!(choice_outcome(None, "ok", false).is_none());
    assert!(choice_outcome(None, "error: exit 1", true).is_none());
}
