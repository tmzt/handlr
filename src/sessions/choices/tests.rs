//! The key plan, the screen checks and the result comparison (plan
//! 20261029_01, step 4). Questions are made up.

use std::sync::Mutex;

use libflankr_proto::common::ChoiceOption;

use super::*;

fn opt(label: &str) -> ChoiceOption {
    ChoiceOption::new(label, "", None)
}

fn single(text: &str, labels: &[&str]) -> ChoiceQuestion {
    ChoiceQuestion::new(
        "",
        text,
        false,
        true,
        labels.iter().map(|l| opt(l)).collect(),
    )
}

fn multi(text: &str, labels: &[&str]) -> ChoiceQuestion {
    ChoiceQuestion::new(
        "",
        text,
        true,
        true,
        labels.iter().map(|l| opt(l)).collect(),
    )
}

fn preview(text: &str, labels: &[&str]) -> ChoiceQuestion {
    let options = labels
        .iter()
        .map(|l| ChoiceOption::new(*l, "", Some(format!("[{l}]"))))
        .collect();
    ChoiceQuestion::new("", text, false, false, options)
}

fn set(questions: Vec<ChoiceQuestion>) -> ChoiceSet {
    ChoiceSet::new("toolu_1", questions, ASK_SOURCE)
}

fn pick(k: &[usize]) -> ChoiceAnswer {
    ChoiceAnswer {
        picked: k.to_vec(),
        other: None,
    }
}

fn other(picked: &[usize], text: &str) -> ChoiceAnswer {
    ChoiceAnswer {
        picked: picked.to_vec(),
        other: Some(text.to_string()),
    }
}

/// The keys and texts of `steps`, checks shown as `?<n>` and `?review`.
fn shape(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|s| match s {
            Step::Question {
                index,
                other_empty,
                first_focused,
                ..
            } => format!(
                "?{}{}{}",
                index + 1,
                if *other_empty { "o" } else { "" },
                if *first_focused { "f" } else { "" }
            ),
            Step::Review => "?review".into(),
            Step::Key(k) => k.clone(),
            Step::Text(t) => format!("'{t}'"),
        })
        .collect()
}

#[test]
fn case_a_single_select_one_question_sends_the_digit_and_no_review() {
    let s = set(vec![single("Which colour?", &["Teal", "Amber", "Plum"])]);
    let steps = plan(&s, &[pick(&[1])]).unwrap();
    assert_eq!(shape(&steps), ["?1", "2"]);
    match &steps[0] {
        Step::Question { needle, .. } => assert_eq!(needle, "Which colour?"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn case_b_other_text_focuses_the_input_types_and_enters() {
    let s = set(vec![single("Which colour?", &["Teal", "Amber"])]);
    let steps = plan(&s, &[other(&[], "  warm orange ")]).unwrap();
    assert_eq!(shape(&steps), ["?1o", "3", "'warm orange'", "enter"]);
}

#[test]
fn case_c_multi_select_toggles_then_goes_down_to_the_button_and_reviews() {
    let s = set(vec![multi("Which rows?", &["Files", "Folders", "Links"])]);
    let steps = plan(&s, &[pick(&[0, 2])]).unwrap();
    assert_eq!(
        shape(&steps),
        [
            "?1f", "1", "3", "down", "down", "down", "down", "enter", "?review", "1"
        ]
    );
}

#[test]
fn case_d_multi_select_with_other_types_in_the_other_row() {
    let s = set(vec![multi("Which rows?", &["Files", "Folders", "Links"])]);
    let steps = plan(&s, &[other(&[1], "Tags")]).unwrap();
    assert_eq!(
        shape(&steps),
        [
            "?1of", "2", "down", "down", "down", "'Tags'", "down", "enter", "?review", "1"
        ]
    );
    // Other alone is a multi answer too.
    let steps = plan(&s, &[other(&[], "Tags")]).unwrap();
    assert_eq!(
        shape(&steps),
        [
            "?1of", "down", "down", "down", "'Tags'", "down", "enter", "?review", "1"
        ]
    );
}

#[test]
fn case_e_preview_question_focuses_then_enters() {
    let s = set(vec![preview("Which layout?", &["Stacked", "Grid"])]);
    let steps = plan(&s, &[pick(&[1])]).unwrap();
    assert_eq!(shape(&steps), ["?1", "2", "enter"]);
    // No Other on a preview question.
    assert!(matches!(
        plan(&s, &[other(&[], "x")]),
        Err(PlanError::Invalid(_))
    ));
}

#[test]
fn case_f_three_questions_mixing_a_b_and_c_end_on_the_review() {
    let s = set(vec![
        single("Which colour?", &["Teal", "Amber"]),
        single("Which size?", &["Small", "Large"]),
        multi("Which rows?", &["Files", "Folders"]),
    ]);
    let steps = plan(&s, &[pick(&[0]), other(&[], "Medium"), pick(&[1])]).unwrap();
    assert_eq!(
        shape(&steps),
        [
            "?1", "1", "?2o", "3", "'Medium'", "enter", "?3f", "2", "down", "down", "down",
            "enter", "?review", "1"
        ]
    );
    // Two single-select questions review too.
    let s = set(vec![
        single("Which colour?", &["Teal", "Amber"]),
        single("Which size?", &["Small", "Large"]),
    ]);
    let steps = plan(&s, &[pick(&[1]), pick(&[0])]).unwrap();
    assert_eq!(shape(&steps), ["?1", "2", "?2", "1", "?review", "1"]);
    // Never escape, Tab or the side arrows.
    for step in &steps {
        if let Step::Key(k) = step {
            assert!(!matches!(
                k.as_str(),
                "escape" | "esc" | "tab" | "left" | "right"
            ));
        }
    }
}

#[test]
fn answers_that_do_not_fit_are_invalid_and_other_sources_unsupported() {
    let s = set(vec![single("Which colour?", &["Teal", "Amber"])]);
    for bad in [
        vec![],
        vec![pick(&[])],
        vec![pick(&[0, 1])],
        vec![pick(&[2])],
        vec![other(&[0], "x")],
        vec![other(&[], "  ")],
        vec![pick(&[0]), pick(&[0])],
    ] {
        assert!(
            matches!(plan(&s, &bad), Err(PlanError::Invalid(_))),
            "{bad:?}"
        );
    }
    let m = set(vec![multi("Which rows?", &["Files", "Folders"])]);
    assert!(matches!(
        plan(&m, &[pick(&[0, 0])]),
        Err(PlanError::Invalid(_))
    ));
    assert!(matches!(
        plan(&m, &[ChoiceAnswer::default()]),
        Err(PlanError::Invalid(_))
    ));
    let codex = ChoiceSet::new("c1", s.questions.clone(), "codex/ask");
    assert!(matches!(
        plan(&codex, &[pick(&[0])]),
        Err(PlanError::Unsupported(_))
    ));
}

/// Recorded frames of Claude Code's question screen, as finding 3 gives
/// its strings (the Mac check, step 7, confirms them on a real screen).
fn frame_single(question: &str, labels: &[&str], other_text: Option<&str>) -> String {
    let mut f = format!("│ ☐ Colour  ✔ Submit →\n│\n│ {question}\n│\n");
    for (i, l) in labels.iter().enumerate() {
        let pointer = if i == 0 { "❯" } else { " " };
        f.push_str(&format!("│ {pointer} {}. {l}\n", i + 1));
    }
    let other = other_text.unwrap_or("Type something.");
    f.push_str(&format!("│   {}. {other}\n", labels.len() + 1));
    f.push_str("│ ─────────────\n│   Chat about this\n│\n");
    f.push_str("│ Enter to select · ↑/↓ to navigate · Esc to cancel\n");
    f
}

const REVIEW_FRAME: &str = "Review your answers\n\n ● Which colour?\n   → Amber\n\nReady to submit your answers?\n\n❯ 1. Submit answers\n  2. Cancel\n";

#[test]
fn the_checks_fold_the_gutter_and_whitespace() {
    let frame = frame_single(
        "Which colour should the badge be, given the bar\n│ is teal?",
        &["Teal", "Amber"],
        None,
    );
    let step = Step::Question {
        index: 0,
        needle: needle("Which colour should the badge be, given the bar is teal?"),
        other_empty: true,
        first_focused: true,
    };
    assert_eq!(unmet(&step, &frame), None);
    let typed = frame_single(
        "Which colour should the badge be, given the bar is teal?",
        &["Teal", "Amber"],
        Some("orange"),
    );
    assert_eq!(
        unmet(&step, &typed).as_deref(),
        Some("the Other row of question 1 already has text")
    );
    let step2 = Step::Question {
        index: 1,
        needle: needle("Which size?"),
        other_empty: false,
        first_focused: false,
    };
    assert_eq!(
        unmet(&step2, &frame).as_deref(),
        Some("question 2 did not appear")
    );
    assert_eq!(unmet(&Step::Review, REVIEW_FRAME), None);
    assert_eq!(
        unmet(&Step::Review, &frame).as_deref(),
        Some("the review did not appear")
    );
}

/// A pane that shows `frames[i]` after `i` keys and texts, and logs them.
struct Scripted {
    frames: Vec<String>,
    log: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(frames: Vec<String>) -> Self {
        Scripted {
            frames,
            log: Mutex::new(Vec::new()),
        }
    }
    fn sent(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

#[async_trait]
impl Pane for Scripted {
    async fn screen(&self) -> Result<String, String> {
        let n = self.log.lock().unwrap().len();
        Ok(self.frames[n.min(self.frames.len() - 1)].clone())
    }
    async fn key(&self, key: &str) -> Result<(), String> {
        self.log.lock().unwrap().push(key.to_string());
        Ok(())
    }
    async fn text(&self, text: &str) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("'{text}'"));
        Ok(())
    }
}

fn fast() -> Timing {
    Timing {
        gap: Duration::from_millis(1),
        poll: Duration::from_millis(5),
        check_for: Duration::from_millis(50),
    }
}

#[tokio::test]
async fn frames_that_match_are_driven_through_and_a_missing_question_stops() {
    let s = set(vec![
        single("Which colour?", &["Teal", "Amber"]),
        single("Which size?", &["Small", "Large"]),
    ]);
    let steps = plan(&s, &[pick(&[1]), pick(&[0])]).unwrap();
    let q1 = frame_single("Which colour?", &["Teal", "Amber"], None);
    let q2 = frame_single("Which size?", &["Small", "Large"], None);
    let pane = Scripted::new(vec![q1.clone(), q2, REVIEW_FRAME.to_string()]);
    drive(&pane, &steps, &fast()).await.unwrap();
    assert_eq!(pane.sent(), ["2", "1", "1"]);

    // Question 2 never appears: stopped after question 1's key.
    let pane = Scripted::new(vec![q1.clone()]);
    let err = drive(&pane, &steps, &fast()).await.unwrap_err();
    assert_eq!(
        err,
        DriveError::Screen {
            step: "question 2 did not appear".into(),
            sent: 1
        }
    );
    assert_eq!(pane.sent(), ["2"]);
}

#[tokio::test]
async fn an_other_row_with_text_is_refused_before_any_key() {
    let s = set(vec![single("Which colour?", &["Teal", "Amber"])]);
    let steps = plan(&s, &[other(&[], "orange")]).unwrap();
    let pane = Scripted::new(vec![frame_single(
        "Which colour?",
        &["Teal", "Amber"],
        Some("something typed on the Mac"),
    )]);
    let err = drive(&pane, &steps, &fast()).await.unwrap_err();
    assert!(matches!(err, DriveError::Screen { sent: 0, .. }), "{err:?}");
    assert!(pane.sent().is_empty());
}

fn answered(pairs: &[(&str, &str)]) -> ChoiceOutcome {
    ChoiceOutcome {
        answers: pairs
            .iter()
            .map(|(q, a)| (q.to_string(), a.to_string()))
            .collect(),
        ..ChoiceOutcome::default()
    }
}

#[test]
fn the_result_is_compared_by_label_text_and_set() {
    let s = set(vec![
        ChoiceQuestion::new(
            "Colour",
            "Which colour?",
            false,
            true,
            vec![opt("Teal"), opt("Amber")],
        ),
        ChoiceQuestion::new(
            "Rows",
            "Which rows?",
            true,
            true,
            vec![opt("Files"), opt("Folders, links"), opt("\"Quoted\"")],
        ),
    ]);
    let sent = [pick(&[1]), other(&[2, 1], "Tags, too")];
    // Multi: toggled order, JSON-quoted labels, the Other text last.
    let ok = answered(&[
        ("Which colour?", "Amber"),
        (
            "Which rows?",
            "\"\\\"Quoted\\\"\", \"Folders, links\", Tags, too",
        ),
    ]);
    assert_eq!(compare(&s, &sent, &ok), ChoiceAnswerResult::Answered);
    let reordered = answered(&[
        ("Which colour?", "Amber"),
        (
            "Which rows?",
            "\"Folders, links\", \"\\\"Quoted\\\"\", Tags, too",
        ),
    ]);
    assert_eq!(compare(&s, &sent, &reordered), ChoiceAnswerResult::Answered);
    let wrong = answered(&[("Which colour?", "Teal"), ("Which rows?", "Files")]);
    assert_eq!(
        compare(&s, &sent, &wrong),
        ChoiceAnswerResult::Mismatch {
            got: vec!["Colour: Teal".into(), "Rows: Files".into()]
        }
    );
    let missing = answered(&[("Which colour?", "Amber")]);
    assert_eq!(
        compare(&s, &sent, &missing),
        ChoiceAnswerResult::Mismatch {
            got: vec!["Colour: Amber".into(), "Rows: (no answer)".into()]
        }
    );
    let rejected = ChoiceOutcome {
        rejected: true,
        ..ChoiceOutcome::default()
    };
    assert_eq!(
        compare(&s, &sent, &rejected),
        ChoiceAnswerResult::Mismatch {
            got: vec!["Closed on the Mac".into()]
        }
    );
}

#[test]
fn versions_outside_the_tested_range_are_untested() {
    assert!(tested(Some("2.1.227")));
    assert!(tested(Some("2.1.312")));
    assert!(tested(Some("2.1.300")));
    assert!(!tested(Some("2.1.313")));
    assert!(!tested(Some("2.2.0")));
    assert!(!tested(Some("2.1.226")));
    assert!(!tested(None));
    assert!(!tested(Some("next")));
}

#[tokio::test]
async fn the_result_wait_gives_up() {
    let none = await_result(|| None, Duration::from_millis(30), Duration::from_millis(5)).await;
    assert!(none.is_none());
    let some = await_result(
        || Some(ChoiceOutcome::default()),
        Duration::from_millis(30),
        Duration::from_millis(5),
    )
    .await;
    assert!(some.is_some());
}
