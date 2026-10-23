//! JSON-RPC 2.0 framing and handlr's error codes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `jsonrpc` member: always `"2.0"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct V2;

impl Serialize for V2 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for V2 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        if v == "2.0" {
            Ok(V2)
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported jsonrpc version {v:?}"
            )))
        }
    }
}

/// A request id: a string or an integer, echoed back as it came.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: V2,
    pub id: Id,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: Id, method: &str, params: impl Serialize) -> Self {
        Request {
            jsonrpc: V2,
            id,
            method: method.to_string(),
            params: serde_json::to_value(params).unwrap_or(Value::Null),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: V2,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Notification {
    pub fn new(method: &str, params: impl Serialize) -> Self {
        Notification {
            jsonrpc: V2,
            method: method.to_string(),
            params: serde_json::to_value(params).unwrap_or(Value::Null),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: V2,
    /// `null` only when the request's id could not be read.
    pub id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

impl Response {
    pub fn ok(id: Id, result: impl Serialize) -> Self {
        Response {
            jsonrpc: V2,
            id: Some(id),
            result: Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            error: None,
        }
    }

    pub fn err(id: Option<Id>, error: ErrorObject) -> Self {
        Response {
            jsonrpc: V2,
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorObject {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        ErrorObject {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// Anything that can arrive on a connection. A request has an `id` and a
/// `method`, a notification only a `method`, a response an `id` and a
/// `result` or `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Request(Request),
    Notification(Notification),
    Response(Response),
}

/// handlr's error codes (plan 20261022_01, decision 7, and the CHANGES.md
/// entry on confirmation nonces), and the JSON-RPC codes it uses.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;

    /// Not permitted, with the policy's reason.
    pub const NOT_PERMITTED: i64 = -32001;
    /// No such agent or pane.
    pub const UNKNOWN_AGENT: i64 = -32002;
    /// The request was cancelled.
    pub const CANCELLED: i64 = -32004;
    /// herdr could not be reached.
    pub const HERDR_UNAVAILABLE: i64 = -32005;
    /// No such confirmation, or it is no longer pending.
    pub const CONFIRMATION_NOT_FOUND: i64 = -32006;
    /// This principal may not approve the confirmation.
    pub const MAY_NOT_APPROVE: i64 = -32007;
    /// The confirmation nonce does not match.
    pub const BAD_NONCE: i64 = -32008;
    /// The first request must be `hello`.
    pub const HELLO_REQUIRED: i64 = -32010;
    /// The protocol or version in `hello` is not served.
    pub const UNSUPPORTED_PROTOCOL: i64 = -32011;
    /// A service the request needs is not there: Gemini has no key, a
    /// prompt did not load, or the call failed or timed out (plan
    /// 20261023_05, section 4).
    pub const UNAVAILABLE: i64 = -32015;
    /// A companion turn is already running in this session
    /// (plan 20261022_02, decision 9).
    pub const BUSY: i64 = -32020;

    /// Every code above but `UNAVAILABLE`, for checking they are distinct.
    /// `UNAVAILABLE` came in 0.1.1, which is additive; growing this array
    /// would change its type.
    pub const ALL: [i64; 15] = [
        PARSE_ERROR,
        INVALID_REQUEST,
        METHOD_NOT_FOUND,
        INVALID_PARAMS,
        INTERNAL_ERROR,
        NOT_PERMITTED,
        UNKNOWN_AGENT,
        CANCELLED,
        HERDR_UNAVAILABLE,
        CONFIRMATION_NOT_FOUND,
        MAY_NOT_APPROVE,
        BAD_NONCE,
        HELLO_REQUIRED,
        UNSUPPORTED_PROTOCOL,
        BUSY,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_round_trip_with_either_kind_of_id() {
        for id in [Id::Number(7), Id::String("a1".to_string())] {
            let req = Request::new(id.clone(), "agents.list", json!({}));
            let text = serde_json::to_string(&req).unwrap();
            let back: Request = serde_json::from_str(&text).unwrap();
            assert_eq!(back, req);
            assert_eq!(back.id, id);
        }
    }

    #[test]
    fn the_version_must_be_2_0() {
        let bad = r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#;
        assert!(serde_json::from_str::<Request>(bad).is_err());
    }

    #[test]
    fn incoming_tells_requests_notifications_and_responses_apart() {
        let req: Incoming =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert!(matches!(req, Incoming::Request(_)));
        let note: Incoming =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"confirm.resolved","params":{}}"#)
                .unwrap();
        assert!(matches!(note, Incoming::Notification(_)));
        let resp: Incoming =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":"x","result":{}}"#).unwrap();
        assert!(matches!(resp, Incoming::Response(_)));
    }

    #[test]
    fn responses_carry_a_result_or_an_error() {
        let ok = serde_json::to_value(Response::ok(Id::Number(1), json!({"x": 1}))).unwrap();
        assert_eq!(ok, json!({"jsonrpc": "2.0", "id": 1, "result": {"x": 1}}));
        let err = serde_json::to_value(Response::err(
            None,
            ErrorObject::new(codes::HELLO_REQUIRED, "hello required"),
        ))
        .unwrap();
        assert_eq!(
            err,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32010, "message": "hello required"}})
        );
    }

    #[test]
    fn error_codes_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<i64> = codes::ALL
            .iter()
            .copied()
            .filter(|c| seen.insert(*c))
            .collect();
        assert_eq!(unique.len(), codes::ALL.len());
        assert!(!codes::ALL.contains(&codes::UNAVAILABLE));
    }
}
