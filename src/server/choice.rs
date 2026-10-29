//! `choice.answer` (docs/design/20261029_01_CHOICE_QUESTIONS_PLAN.md,
//! decisions 2, 5 and 6): a companion's answer to the choice question open
//! in a pane, given by the agent's own keys and checked against the result
//! the agent writes.

use std::sync::Arc;

use async_trait::async_trait;
use libflankr_proto::ErrorObject;
use libflankr_proto::common::{ChoiceAnswerParams, ChoiceAnswerResult};
use libflankr_proto::envelope::codes;
use libherdr::HerdrClient;
use libherdr::protocol::{HerdrRequest, ReadSource};
use serde_json::{Value, json};

use super::Daemon;
use super::conn::{Conn, ok, params, resolve_target, snapshot};
use crate::core::agents::{self, extract_agent_name_for_pane};
use crate::core::policy;
use crate::sessions::choices::{self, DriveError, PlanError, Timing};

/// Lines asked of `pane.read`; the visible frame is what comes back.
const SCREEN_LINES: usize = 200;

/// herdr's `pane.send_text`: literal text, no Enter. libherdr has no
/// request of its own for it, so a custom one (finding 5).
pub fn send_text_request(pane_id: &str, text: &str) -> HerdrRequest {
    HerdrRequest::Custom(json!({
        "method": "pane.send_text",
        "params": {"pane_id": pane_id, "text": text},
    }))
}

/// A herdr pane as the driver sees it.
struct HerdrPane<'a> {
    client: &'a HerdrClient,
    pane_id: &'a str,
}

#[async_trait]
impl choices::Pane for HerdrPane<'_> {
    async fn screen(&self) -> Result<String, String> {
        let read = self
            .client
            .inspect_pane_from(self.pane_id, SCREEN_LINES, ReadSource::Visible)
            .await
            .map_err(|e| e.to_string())?;
        Ok(agents::pane_text(&read).unwrap_or_default().to_string())
    }

    async fn key(&self, key: &str) -> Result<(), String> {
        self.client
            .send_keys(self.pane_id, key)
            .await
            .map(drop)
            .map_err(|e| e.to_string())
    }

    async fn text(&self, text: &str) -> Result<(), String> {
        self.client
            .send_request(&send_text_request(self.pane_id, text))
            .await
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// The pane's entry in [`Daemon::choice_answering`], removed when
/// dropped.
struct Answering<'a>(&'a Daemon, String);

impl<'a> Answering<'a> {
    fn take(daemon: &'a Daemon, pane_id: &str) -> Option<Self> {
        let mut set = daemon.choice_answering.lock().unwrap();
        set.insert(pane_id.to_string())
            .then(|| Answering(daemon, pane_id.to_string()))
    }
}

impl Drop for Answering<'_> {
    fn drop(&mut self) {
        self.0.choice_answering.lock().unwrap().remove(&self.1);
    }
}

fn not_permitted(reason: impl Into<String>) -> ErrorObject {
    ErrorObject::new(codes::NOT_PERMITTED, reason)
}

/// `choice.answer`, for companions only (the dispatcher gives anyone else
/// `METHOD_NOT_FOUND`).
pub(super) async fn answer(conn: &Arc<Conn>, raw: Value) -> Result<Value, ErrorObject> {
    let mut p: ChoiceAnswerParams = params(raw)?;
    let daemon = &conn.daemon;
    let snap = snapshot(conn).await?;
    let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
    let agent = extract_agent_name_for_pane(&snap, &pane_id);
    let principal = conn.principal();
    // Decision 6: the agent takes messages and this principal reaches it.
    policy::check_answer(principal, &daemon.agents, &agent, &pane_id)
        .map_err(|denied| not_permitted(denied.to_string()))?;
    let pane = agents::agents(&snap)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, &pane_id))
        .ok_or_else(|| ErrorObject::new(codes::UNKNOWN_AGENT, format!("no pane {pane_id}")))?;
    // Session files are read on this machine only.
    let local = !matches!(
        daemon.client.transport(),
        libherdr::transport::HerdrTransport::RemoteSsh { .. }
    );
    let open = local
        .then(|| daemon.sessions.open_choice(&pane))
        .flatten()
        .filter(|o| o.set.id == p.id)
        .ok_or_else(|| {
            ErrorObject::new(
                codes::CHOICE_STALE,
                format!("{} is not the open choice question of {pane_id}", p.id),
            )
        })?;
    if !choices::supported(&open.set.source) {
        return Err(ErrorObject::new(
            codes::CHOICE_UNSUPPORTED,
            format!("no driver for {}", open.set.source),
        ));
    }
    // Other texts as messages are checked: one line, no command prefix,
    // the paths in the agent's project. The cleaned text is what is typed.
    for answer in &mut p.answers {
        if let Some(text) = &answer.other {
            let checked = policy::check_message(
                principal,
                &daemon.agents,
                &agent,
                &pane_id,
                text,
                pane.cwd.as_deref(),
            )
            .map_err(|denied| not_permitted(denied.to_string()))?;
            answer.other = Some(checked.command);
        }
    }
    let steps = choices::plan(&open.set, &p.answers).map_err(|e| match e {
        PlanError::Invalid(why) => ErrorObject::new(codes::INVALID_PARAMS, why),
        PlanError::Unsupported(why) => ErrorObject::new(codes::CHOICE_UNSUPPORTED, why),
    })?;
    let Some(_answering) = Answering::take(daemon, &pane_id) else {
        return Err(not_permitted(format!(
            "an answer to {pane_id}'s question is already being sent"
        )));
    };
    let version = crate::sessions::claude::call_version(&open.located.path, &p.id);
    let untested = !choices::tested(version.as_deref());
    log::info!(
        "Answering choice question {} in pane '{pane_id}' (Claude Code {}{}) by the tap",
        p.id,
        version.as_deref().unwrap_or("of unknown version"),
        if untested { ", untested" } else { "" }
    );
    let herdr = HerdrPane {
        client: &daemon.client,
        pane_id: &pane_id,
    };
    if let Err(e) = choices::drive(&herdr, &steps, &Timing::default()).await {
        return Err(match e {
            DriveError::Screen { step, sent } => {
                log::warn!(
                    "Choice question {} in pane '{pane_id}': {step} ({sent} keys sent); left for the Mac",
                    p.id
                );
                ErrorObject::new(codes::SCREEN_MISMATCH, step)
            }
            DriveError::Herdr(e) => {
                ErrorObject::new(codes::HERDR_UNAVAILABLE, format!("herdr: {e}"))
            }
        });
    }
    let sessions = Arc::clone(&daemon.sessions);
    let id = p.id.clone();
    let outcome = choices::await_result(
        || sessions.choice_result(&pane, &id),
        choices::RESULT_WAIT,
        choices::RESULT_POLL,
    )
    .await;
    let result = match outcome {
        Some(outcome) => choices::compare(&open.set, &p.answers, &outcome),
        None => ChoiceAnswerResult::NoResult,
    };
    log::info!(
        "Choice question {} in pane '{pane_id}': {:?}{}",
        p.id,
        result,
        if untested {
            format!(
                " (Claude Code {} is untested)",
                version.as_deref().unwrap_or("of unknown version")
            )
        } else {
            String::new()
        }
    );
    ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_text_is_herdrs_pane_send_text() {
        let HerdrRequest::Custom(v) = send_text_request("w1:p2", "warm orange") else {
            panic!("a custom request");
        };
        assert_eq!(
            v,
            json!({"method": "pane.send_text", "params": {"pane_id": "w1:p2", "text": "warm orange"}})
        );
        let rpc = send_text_request("p", "t").to_jsonrpc();
        assert_eq!(rpc.method, "pane.send_text");
        assert_eq!(rpc.params, json!({"pane_id": "p", "text": "t"}));
    }
}
