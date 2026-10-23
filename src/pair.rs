//! `handlr pair`: ask the running daemon for a pending pairing, print its
//! pairing URL and the URL's QR code as a `data:` URL, and wait for the
//! device (plan 20261022_02, decision 5, and the amendments of 11:58 and
//! 18:20 EDT; plan 20261022_06, step 3 and the 22:55 EDT amendment).
//!
//! The pairing URL carries the single-use secret: nothing here logs it.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use futures_util::StreamExt;
use libhandlr_proto::common::{
    methods, protocols, HelloParams, LinkPairBeginParams, LinkPairBeginResult, LinkPairDone,
};
use libhandlr_proto::link::PairingPayload;
use libhandlr_proto::{Id, Incoming, Request};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::server::conn::MAX_LINE;

/// What `handlr pair` shows for a pending pairing.
#[derive(Debug)]
pub struct Shown {
    /// `handlr://<host>:<port>/pair#handlr1…`.
    pub url: String,
    /// The PNG of the URL's QR code.
    pub png: Vec<u8>,
    /// Whether some address can be reached from another machine.
    pub remote: bool,
}

impl Shown {
    pub fn new(pairing: &str) -> Result<Shown, String> {
        let payload = PairingPayload::decode(pairing).map_err(|e| e.to_string())?;
        let url = payload.url().map_err(|e| e.to_string())?;
        let png = qrcodeurl::png(&url).map_err(|e| format!("QR code: {e}"))?;
        Ok(Shown {
            url,
            png,
            remote: payload.has_remote_address(),
        })
    }

    /// The QR code as `data:image/png;base64,…`.
    pub fn data_url(&self) -> String {
        use base64::Engine;
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&self.png)
        )
    }
}

/// Run `handlr pair`. Returns the process exit code. With `qr_png`, the QR
/// code is also written there, mode 0600, never over an existing file.
pub async fn run(
    socket: &Path,
    params: LinkPairBeginParams,
    qr_png: Option<&Path>,
) -> Result<i32, String> {
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
                show(&result, qr_png)?;
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

fn show(result: &LinkPairBeginResult, qr_png: Option<&Path>) -> Result<(), String> {
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    let shown = Shown::new(&result.pairing)?;
    if !shown.remote {
        eprintln!(
            "warning: every address is this machine's own; a phone cannot reach it \
             (run tailscale serve, or pass --listen to handlr serve)"
        );
    }
    println!("{}", shown.url);
    println!();
    println!("{}", shown.data_url());
    println!();
    if let Some(path) = qr_png {
        write_png(path, &shown.png)?;
        println!("Wrote {}", path.display());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!(
        "Waiting for {} to pair at {} (single use, expires in {} s)...",
        result.name,
        result.addresses.join(", "),
        result.expiry.saturating_sub(now)
    );
    Ok(())
}

/// Writes `png` to a new file at `path`, mode 0600.
fn write_png(path: &Path, png: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(png)
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use libhandlr_proto::link::{generate_keypair, generate_secret};

    fn pairing(addresses: &[&str]) -> String {
        PairingPayload {
            server_key: generate_keypair().unwrap().public,
            secret: generate_secret().unwrap(),
            expiry: 1_792_000_000,
            name: "phone".into(),
            addresses: addresses.iter().map(|a| a.to_string()).collect(),
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn the_url_names_the_tailscale_address_and_the_png_carries_it() {
        let pairing = pairing(&["m4.tail1a2b3c.ts.net:7347", "127.0.0.1:7347"]);
        let shown = Shown::new(&pairing).unwrap();
        assert_eq!(
            shown.url,
            format!("handlr://m4.tail1a2b3c.ts.net:7347/pair#{pairing}")
        );
        assert!(shown.remote);
        assert_eq!(shown.png, qrcodeurl::png(&shown.url).unwrap());
        assert!(shown
            .data_url()
            .starts_with("data:image/png;base64,iVBORw0KGgo"));
        assert_eq!(
            PairingPayload::decode(&shown.url).unwrap(),
            PairingPayload::decode(&pairing).unwrap()
        );
    }

    #[test]
    fn loopback_alone_is_not_remote() {
        let shown = Shown::new(&pairing(&["127.0.0.1:7347"])).unwrap();
        assert!(!shown.remote);
        assert!(shown.url.starts_with("handlr://127.0.0.1:7347/pair#"));
    }

    #[test]
    fn the_png_file_is_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("qr.png");
        write_png(&path, b"png").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(write_png(&path, b"other").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"png");
    }
}
