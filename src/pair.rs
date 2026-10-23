//! `handlr pair`: print the daemon's pairing URL and its QR code as a
//! `data:` URL, read the code the device shows, add the device with
//! `link.pair_add`, and wait for it to connect (plan 20261022_02, decision
//! 5 as amended at 23:07 EDT; plan 20261022_06, step 3 as amended at 22:55
//! and 23:07 EDT).
//!
//! Only public keys move: the URL holds the server's key and addresses,
//! and the device code the device's key and name.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use libhandlr_proto::common::{
    methods, protocols, HelloParams, LinkPairAddParams, LinkPairAddResult, LinkPairBeginParams,
    LinkPairBeginResult, LinkPairDone,
};
use libhandlr_proto::link::{PairingPayload, PAIRING_PREFIX, PAIRING_URL_SCHEME};
use libhandlr_proto::{ErrorObject, Id, Incoming, Notification, Request};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::server::conn::MAX_LINE;

/// The prompt for the device's code.
pub const PROMPT: &str = "Paste the pairing code from the device: ";

/// What `handlr pair` says of a pasted server address.
pub const NOT_A_DEVICE_CODE: &str =
    "That is this server's pairing URL; paste it on the device, then paste the device's code here.";

/// What `handlr pair` shows for the server.
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

/// What `handlr pair` was asked for.
#[derive(Debug, Clone, Default)]
pub struct PairOptions {
    /// Pair as this name instead of the device's own.
    pub name: Option<String>,
    /// `companion`, `harness`; the daemon defaults to `companion`.
    pub protocols: Option<Vec<String>>,
    /// Replace an entry of the same name.
    pub replace: bool,
    /// The device code, instead of prompting for it.
    pub code: Option<String>,
    /// Also write the QR code here, never over an existing file.
    pub qr_png: Option<PathBuf>,
}

/// What a pasted line is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pasted {
    /// Nothing: stop.
    Empty,
    /// A server's pairing URL, data URL or string, which belongs on the
    /// device.
    ServerAddress,
    /// Anything else, sent to the daemon as a device code.
    Code(String),
}

impl Pasted {
    pub fn classify(line: &str) -> Pasted {
        let line = line.trim();
        if line.is_empty() {
            Pasted::Empty
        } else if line.starts_with(&format!("{PAIRING_URL_SCHEME}://"))
            || line.starts_with("data:")
            || line.starts_with(PAIRING_PREFIX)
        {
            Pasted::ServerAddress
        } else {
            Pasted::Code(line.to_string())
        }
    }
}

/// The daemon-socket connection.
struct Daemon {
    write: OwnedWriteHalf,
    lines: FramedRead<OwnedReadHalf, LinesCodec>,
    next_id: i64,
    /// Notifications that arrived while waiting for a response.
    notes: Vec<Notification>,
}

impl Daemon {
    async fn connect(socket: &Path) -> Result<Daemon, String> {
        let stream = UnixStream::connect(socket).await.map_err(|e| {
            format!(
                "connect {}: {e} (is `handlr serve` running?)",
                socket.display()
            )
        })?;
        let (read, write) = stream.into_split();
        let mut daemon = Daemon {
            write,
            lines: FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE)),
            next_id: 0,
            notes: Vec::new(),
        };
        let hello = HelloParams {
            protocol: protocols::HARNESS.to_string(),
            version: 1,
            client: "pair".to_string(),
        };
        daemon
            .call(methods::HELLO, hello)
            .await?
            .map_err(|e| format!("hello: {}", e.message))?;
        Ok(daemon)
    }

    async fn next(&mut self) -> Result<Incoming, String> {
        loop {
            let line = self
                .lines
                .next()
                .await
                .ok_or("the daemon closed the connection")?
                .map_err(|e| format!("read: {e}"))?;
            if let Ok(message) = serde_json::from_str::<Incoming>(&line) {
                return Ok(message);
            }
        }
    }

    /// One request; the outer error is the connection's, the inner the
    /// daemon's answer.
    async fn call(
        &mut self,
        method: &str,
        params: impl serde::Serialize,
    ) -> Result<Result<Value, ErrorObject>, String> {
        let id = Id::Number(self.next_id);
        self.next_id += 1;
        let mut line = serde_json::to_string(&Request::new(id.clone(), method, params))
            .map_err(|e| e.to_string())?;
        line.push('\n');
        self.write
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
        loop {
            match self.next().await? {
                Incoming::Response(response) if response.id.as_ref() == Some(&id) => {
                    return Ok(match response.error {
                        Some(error) => Err(error),
                        None => Ok(response.result.unwrap_or_default()),
                    });
                }
                Incoming::Response(response) if response.id.is_none() => {
                    let message = response
                        .error
                        .map(|e| e.message)
                        .unwrap_or_else(|| "a response with no id".to_string());
                    return Err(message);
                }
                Incoming::Notification(note) => self.notes.push(note),
                _ => {}
            }
        }
    }

    /// Waits for `link.pair_done` for `name`.
    async fn pair_done(&mut self, name: &str) -> Result<LinkPairDone, String> {
        loop {
            let note = if !self.notes.is_empty() {
                self.notes.remove(0)
            } else {
                match self.next().await? {
                    Incoming::Notification(note) => note,
                    _ => continue,
                }
            };
            if note.method != methods::LINK_PAIR_DONE {
                continue;
            }
            let done: LinkPairDone = serde_json::from_value(note.params)
                .map_err(|e| format!("unexpected notification: {e}"))?;
            if done.name == name {
                return Ok(done);
            }
        }
    }
}

/// Run `handlr pair`. Returns the process exit code.
pub async fn run(socket: &Path, options: PairOptions) -> Result<i32, String> {
    let mut daemon = Daemon::connect(socket).await?;
    let begun = match daemon
        .call(methods::LINK_PAIR_BEGIN, LinkPairBeginParams {})
        .await?
    {
        Ok(result) => result,
        Err(error) => {
            eprintln!("handlr pair: {}", error.message);
            return Ok(1);
        }
    };
    let begun: LinkPairBeginResult =
        serde_json::from_value(begun).map_err(|e| format!("unexpected reply: {e}"))?;
    show(&begun, options.qr_png.as_deref())?;

    let added = loop {
        let line = match &options.code {
            Some(code) => code.clone(),
            None => match prompt().await? {
                Some(line) => line,
                None => return Ok(1),
            },
        };
        let code = match Pasted::classify(&line) {
            Pasted::Empty => return Ok(1),
            Pasted::ServerAddress => {
                println!("{NOT_A_DEVICE_CODE}");
                if options.code.is_some() {
                    return Ok(1);
                }
                continue;
            }
            Pasted::Code(code) => code,
        };
        let params = LinkPairAddParams {
            code,
            name: options.name.clone(),
            protocols: options.protocols.clone(),
            replace: options.replace,
        };
        match daemon.call(methods::LINK_PAIR_ADD, params).await? {
            Ok(result) => {
                break serde_json::from_value::<LinkPairAddResult>(result)
                    .map_err(|e| format!("unexpected reply: {e}"))?
            }
            Err(error) => {
                eprintln!("handlr pair: {}", error.message);
                if options.code.is_some() {
                    return Ok(1);
                }
            }
        }
    };

    println!("Added {} ({})", added.name, added.protocols.join(", "));
    println!("Waiting for {} to connect (Ctrl-C to stop)...", added.name);
    let done = daemon.pair_done(&added.name).await?;
    if done.paired {
        println!("{} connected", done.name);
        return Ok(0);
    }
    eprintln!(
        "{} did not connect: {}",
        done.name,
        done.error.as_deref().unwrap_or("unknown error")
    );
    Ok(1)
}

/// Prompts for the device code and reads one line from standard input,
/// off the runtime. `None` at end of input.
async fn prompt() -> Result<Option<String>, String> {
    print!("{PROMPT}");
    let _ = std::io::stdout().flush();
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(line)),
            Err(e) => Err(format!("read standard input: {e}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
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
    use libhandlr_proto::link::generate_keypair;

    fn pairing(addresses: &[&str]) -> String {
        PairingPayload {
            server_key: generate_keypair().unwrap().public,
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

    #[test]
    fn a_pasted_server_address_is_not_a_device_code() {
        let pairing = pairing(&["m4.tail1a2b3c.ts.net:7347"]);
        let shown = Shown::new(&pairing).unwrap();
        for server in [&pairing, &shown.url, &shown.data_url()] {
            assert_eq!(
                Pasted::classify(&format!(" {server}\n")),
                Pasted::ServerAddress
            );
        }
        assert_eq!(Pasted::classify("  \n"), Pasted::Empty);
        assert_eq!(
            Pasted::classify(" handlrkey1abc\n"),
            Pasted::Code("handlrkey1abc".to_string())
        );
    }
}
