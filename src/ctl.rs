//! `flankr ctl`: send one JSON-RPC request to the daemon and print what
//! comes back (plan 20261022_01, decision 1). A debugging client.
//!
//! The result is printed, then any notifications until the request is done.
//! An `agents.send` that comes back `pending` is done when its
//! `confirm.resolved` arrives. `--follow` keeps printing notifications.
//!
//! For `confirm.reply`, params may carry `token` (and `handle`) as read off
//! `flankr serve`'s terminal. They are turned into the `nonce` here and
//! never sent.
//!
//! `--handle <text>` adds a `handle` param. For `confirm.status` the
//! result says how it matched (`matched`: `id`, `exact`, `coverage` or
//! `entropy`, with `bits`); an ambiguous handle (`-32009`) and a miss
//! print the pending handles (`data.pending`, newest first).
//!
//! `--client <name>` sets the client label (default `ctl`), and so the
//! principal, `local:<name>`. A requester cannot approve its own command,
//! so trying the confirmation flow by hand takes two names.

use std::path::Path;

use futures_util::StreamExt;
use libflankr_proto::common::{HelloParams, methods, protocols};
use libflankr_proto::{Id, Incoming, Request};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::core::confirm::nonce;
use crate::server::conn::MAX_LINE;

/// Replace `token` (and `handle`) in `confirm.reply` params with the nonce.
pub fn token_to_nonce(method: &str, mut params: Value) -> Result<Value, String> {
    if method != methods::CONFIRM_REPLY {
        return Ok(params);
    }
    let Some(obj) = params.as_object_mut() else {
        return Ok(params);
    };
    let Some(token) = obj.remove("token") else {
        return Ok(params);
    };
    let token = token.as_str().ok_or("token must be a string")?.to_string();
    let handle = obj
        .remove("handle")
        .and_then(|h| h.as_str().map(str::to_string));
    let confirm_id = obj
        .get("confirm_id")
        .and_then(|v| v.as_u64())
        .ok_or("confirm.reply needs a numeric confirm_id")?;
    obj.insert(
        "nonce".to_string(),
        Value::String(nonce(&token, confirm_id, handle.as_deref())),
    );
    Ok(params)
}

/// Run one request. Returns the process exit code.
pub async fn run(
    socket: &Path,
    client: &str,
    method: &str,
    params: Value,
    follow: bool,
) -> Result<i32, String> {
    let params = token_to_nonce(method, params)?;
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        format!(
            "connect {}: {e} (is `flankr serve` running?)",
            socket.display()
        )
    })?;
    let (read, mut write) = stream.into_split();
    let mut lines = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE));

    let hello = Request::new(
        Id::Number(0),
        methods::HELLO,
        HelloParams {
            protocol: protocols::HARNESS.to_string(),
            version: 1,
            client: client.to_string(),
        },
    );
    let request = Request::new(Id::Number(1), method, params);
    for message in [&hello, &request] {
        let mut line = serde_json::to_string(message).map_err(|e| e.to_string())?;
        line.push('\n');
        write
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
    }

    let mut code = 0;
    let mut waiting_for: Option<u64> = None;
    let mut answered = false;
    while let Some(line) = lines.next().await {
        let line = line.map_err(|e| format!("read: {e}"))?;
        match serde_json::from_str::<Incoming>(&line) {
            Ok(Incoming::Response(response)) => {
                if let Some(error) = response.error {
                    println!("{}", serde_json::to_string(&error).unwrap_or_default());
                    if response.id != Some(Id::Number(1)) || !follow {
                        return Ok(1);
                    }
                    code = 1;
                    answered = true;
                } else if response.id == Some(Id::Number(1)) {
                    let result = response.result.unwrap_or(Value::Null);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    );
                    if result.get("status").and_then(|s| s.as_str()) == Some("pending") {
                        waiting_for = result.get("confirm_id").and_then(|v| v.as_u64());
                    }
                    answered = true;
                }
            }
            Ok(Incoming::Notification(note)) => {
                println!("{}", serde_json::to_string(&note).unwrap_or_default());
                let resolves = note.method == methods::CONFIRM_RESOLVED
                    && note.params.get("confirm_id").and_then(|v| v.as_u64()) == waiting_for;
                if resolves {
                    waiting_for = None;
                }
            }
            Ok(Incoming::Request(_)) | Err(_) => {}
        }
        if answered && waiting_for.is_none() && !follow {
            return Ok(code);
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_token_becomes_a_nonce_and_is_not_sent() {
        let params = token_to_nonce(
            methods::CONFIRM_REPLY,
            json!({"confirm_id": 3, "approve": true, "token": "a-b-c", "handle": "keep-going"}),
        )
        .unwrap();
        assert_eq!(params.get("token"), None);
        assert_eq!(params.get("handle"), None);
        assert_eq!(params["nonce"], nonce("a-b-c", 3, Some("keep-going")));
        // Other methods pass through untouched.
        let other = token_to_nonce(methods::AGENTS_LIST, json!({"token": "x"})).unwrap();
        assert_eq!(other, json!({"token": "x"}));
    }
}
