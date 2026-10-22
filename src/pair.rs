//! `handlr pair`: ask the running daemon for a pending pairing, show it as
//! a QR code and a `handlr1…` string, and wait for the device
//! (plan 20261022_02, decision 5, and the amendments of 11:58 and 18:20
//! EDT).

use std::path::Path;

use futures_util::StreamExt;
use libhandlr_proto::common::{
    methods, protocols, HelloParams, LinkPairBeginParams, LinkPairBeginResult, LinkPairDone,
};
use libhandlr_proto::{Id, Incoming, Request};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::server::conn::MAX_LINE;

/// The QR code of `text`, in Unicode half blocks, light on dark.
pub fn qr_text(text: &str) -> Result<String, String> {
    let code = QrCode::new(text.as_bytes()).map_err(|e| format!("QR code: {e}"))?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .build())
}

/// Run `handlr pair`. Returns the process exit code.
pub async fn run(socket: &Path, params: LinkPairBeginParams) -> Result<i32, String> {
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        format!(
            "connect {}: {e} (is `handlr serve` running?)",
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
            client: "pair".to_string(),
        },
    );
    let begin = Request::new(Id::Number(1), methods::LINK_PAIR_BEGIN, params);
    for message in [serde_json::to_string(&hello), serde_json::to_string(&begin)] {
        let mut line = message.map_err(|e| e.to_string())?;
        line.push('\n');
        write
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
    }

    while let Some(line) = lines.next().await {
        let line = line.map_err(|e| format!("read: {e}"))?;
        match serde_json::from_str::<Incoming>(&line) {
            Ok(Incoming::Response(response)) => {
                if let Some(error) = response.error {
                    eprintln!("handlr pair: {}", error.message);
                    return Ok(1);
                }
                if response.id != Some(Id::Number(1)) {
                    continue;
                }
                let result: LinkPairBeginResult =
                    serde_json::from_value(response.result.unwrap_or_default())
                        .map_err(|e| format!("unexpected reply: {e}"))?;
                show(&result)?;
            }
            Ok(Incoming::Notification(note)) if note.method == methods::LINK_PAIR_DONE => {
                let done: LinkPairDone = serde_json::from_value(note.params)
                    .map_err(|e| format!("unexpected notification: {e}"))?;
                if done.paired {
                    println!("Paired {}", done.name);
                    return Ok(0);
                }
                eprintln!(
                    "Pairing {} failed: {}",
                    done.name,
                    done.error.as_deref().unwrap_or("unknown error")
                );
                return Ok(1);
            }
            _ => {}
        }
    }
    Err("the daemon closed the connection".to_string())
}

fn show(result: &LinkPairBeginResult) -> Result<(), String> {
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", qr_text(&result.pairing)?);
    println!("{}", result.pairing);
    println!();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!(
        "Waiting for {} to pair at {} (expires in {} s)...",
        result.name,
        result.addresses.join(", "),
        result.expiry.saturating_sub(now)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairing_string_fits_a_qr_code() {
        let text = format!("handlr1{}", "z".repeat(200));
        let qr = qr_text(&text).unwrap();
        assert!(qr.lines().count() > 10);
        assert!(qr.contains('█') || qr.contains('▀') || qr.contains('▄'));
    }
}
