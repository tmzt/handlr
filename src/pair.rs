//! `flankr pair`: put the daemon in pair mode for this run, print its
//! pairing URL, its QR code as a `data:` URL and its fingerprint, wait for
//! the phone's pairing request (or a pasted device code), print the six
//! pairing words, and add the device only when the owner types `yes`
//! (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md, the amendments of
//! 14:37, 14:40, 14:42 and 14:44 EDT; earlier, plan 20261022_02, decision
//! 5, and plan 20261022_06, step 3).
//!
//! Only public keys move: the URL holds the server's key, addresses and
//! this run's offer; the phone's request (or its device code) its key and
//! name.
//!
//! * The fingerprint is printed as its `3f9a 12c4` label and six words.
//! * The run makes one attempt: once a request is held, any answer but
//!   `yes`, the phone's Cancel, or the offer expiring ends it with a
//!   non-zero status and nothing added.
//! * When the offer expires (60 s by default, `--timeout` 1 to 300), the
//!   run just exits non-zero, ending an open prompt line and printing
//!   nothing else.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use libflankr_proto::common::{HelloParams, LinkPairBeginResult, methods, protocols};
use libflankr_proto::link::{
    PAIR_TIMEOUT_MAX_SECS, PAIRING_PREFIX, PAIRING_URL_SCHEME, PairingPayload, fingerprint_text,
    fingerprint_words_text,
};
use libflankr_proto::{ErrorObject, Id, Incoming, Notification, Request};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::server::conn::MAX_LINE;
use crate::server::link::pair_mode::{
    PairAnswerParams, PairAnswerResult, PairBeginExtra, PairBeginParams, PairCodeParams, PairEnded,
    PairRequestInfo, methods as pair_methods,
};

/// What `flankr pair` asks once a request is held.
pub const PROMPT: &str = "Do the six words on the phone match? Type yes to pair: ";

/// What `flankr pair` says of a pasted server address.
pub const NOT_A_DEVICE_CODE: &str =
    "That is this server's pairing URL; paste it on the device, then paste the device's code here.";

/// What `flankr pair` shows for the server.
#[derive(Debug)]
pub struct Shown {
    /// `flankr://<host>:<port>/pair#flankr1…`.
    pub url: String,
    /// The PNG of the URL's QR code.
    pub png: Vec<u8>,
    /// Whether some address can be reached from another machine.
    pub remote: bool,
    /// The daemon's fingerprint label, `3f9a 12c4`.
    pub fingerprint: String,
    /// The daemon's fingerprint as six words.
    pub fingerprint_words: String,
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
            fingerprint: fingerprint_text(&payload.server_key),
            fingerprint_words: fingerprint_words_text(&payload.server_key),
        })
    }

    /// The line naming the daemon, printed after the QR code.
    pub fn identity(&self) -> String {
        format!(
            "Daemon fingerprint: {} ({})",
            self.fingerprint, self.fingerprint_words
        )
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

/// What `flankr pair` was asked for.
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
    /// How long the offer lasts, 1 to 300 seconds; the daemon's default
    /// (60) when `None`.
    pub timeout_secs: Option<u16>,
}

/// `--timeout`'s value: whole seconds from 1 to 300.
pub fn parse_timeout(value: &str) -> Result<u16, String> {
    match value.trim().parse::<u64>() {
        Ok(secs) if (1..=u64::from(PAIR_TIMEOUT_MAX_SECS)).contains(&secs) => Ok(secs as u16),
        Ok(secs) if secs > u64::from(PAIR_TIMEOUT_MAX_SECS) => Err(format!(
            "--timeout {secs} is too long: a pairing offer lasts at most {PAIR_TIMEOUT_MAX_SECS} seconds"
        )),
        _ => Err(format!(
            "--timeout takes whole seconds from 1 to {PAIR_TIMEOUT_MAX_SECS}, not '{value}'"
        )),
    }
}

/// Whether the owner's answer is `yes`: exactly that word, in any case,
/// with surrounding space.
pub fn is_yes(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case("yes")
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
                "connect {}: {e} (is `flankr serve` running?)",
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

    /// The next notification. Cancel-safe while no call is waiting.
    async fn note(&mut self) -> Result<Notification, String> {
        if !self.notes.is_empty() {
            return Ok(self.notes.remove(0));
        }
        loop {
            if let Incoming::Notification(note) = self.next().await? {
                return Ok(note);
            }
        }
    }
}

/// What ended the wait for a request, or the owner's answer.
enum Wait {
    Request(PairRequestInfo),
    Ended(PairEnded),
    /// The offer's time is up, by this side's clock.
    TimeUp,
}

/// Reads standard input line by line on its own thread. The channel
/// closes at end of input.
fn stdin_lines() -> mpsc::UnboundedReceiver<String> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        loop {
            let mut line = String::new();
            match stdin.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    rx
}

/// Run `flankr pair` on the terminal. Returns the process exit code.
pub async fn run(socket: &Path, options: PairOptions) -> Result<i32, String> {
    run_with(socket, options, stdin_lines(), std::io::stdout()).await
}

/// Run `flankr pair`, reading the owner's lines from `input` and printing
/// to `out`; warnings and errors go to standard error. Returns the exit
/// code: 0 once a device is added, 1 otherwise.
pub async fn run_with<W: Write + Send>(
    socket: &Path,
    options: PairOptions,
    mut input: mpsc::UnboundedReceiver<String>,
    mut out: W,
) -> Result<i32, String> {
    let mut daemon = Daemon::connect(socket).await?;
    let params = PairBeginParams {
        timeout_secs: options.timeout_secs,
    };
    let begun = match daemon.call(methods::LINK_PAIR_BEGIN, params).await? {
        Ok(result) => result,
        Err(error) => {
            eprintln!("flankr pair: {}", error.message);
            return Ok(1);
        }
    };
    let extra: PairBeginExtra =
        serde_json::from_value(begun.clone()).map_err(|e| format!("unexpected reply: {e}"))?;
    let begun: LinkPairBeginResult =
        serde_json::from_value(begun).map_err(|e| format!("unexpected reply: {e}"))?;
    show(&mut out, &begun, options.qr_png.as_deref())?;
    let secs = extra.expires_in_secs;
    say(
        &mut out,
        &format!(
            "Scan the QR code with the phone, or paste its device code here. \
             The link expires in {secs} s.\n"
        ),
    )?;
    // The daemon ends the run when the offer expires; this side's clock,
    // a little later, is only a fallback.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(u64::from(secs) + 2);

    // Wait for the run's one request: the phone's, or a pasted code.
    let mut pasted = options.code.clone();
    let mut input_open = true;
    let wait = loop {
        let line = match pasted.take() {
            Some(code) => Some(code),
            None => tokio::select! {
                note = daemon.note() => match pair_note(note?) {
                    Some(wait) => break wait,
                    None => continue,
                },
                line = input.recv(), if input_open => match line {
                    Some(line) => Some(line),
                    None => {
                        input_open = false;
                        None
                    }
                },
                _ = tokio::time::sleep_until(deadline) => break Wait::TimeUp,
            },
        };
        let Some(line) = line else { continue };
        let code = match Pasted::classify(&line) {
            Pasted::Empty => continue,
            Pasted::ServerAddress => {
                say(&mut out, &format!("{NOT_A_DEVICE_CODE}\n"))?;
                if options.code.is_some() {
                    return Ok(1);
                }
                continue;
            }
            Pasted::Code(code) => code,
        };
        match daemon
            .call(pair_methods::LINK_PAIR_CODE, PairCodeParams { code })
            .await?
        {
            Ok(info) => {
                break Wait::Request(
                    serde_json::from_value(info).map_err(|e| format!("unexpected reply: {e}"))?,
                );
            }
            Err(error) => {
                eprintln!("flankr pair: {}", error.message);
                if options.code.is_some() {
                    return Ok(1);
                }
            }
        }
    };
    let request = match wait {
        Wait::Request(request) => request,
        // Expired: just exit (the 14:42 EDT amendment).
        Wait::Ended(_) | Wait::TimeUp => return Ok(1),
    };

    say(
        &mut out,
        &format!(
            "Pairing request from {}\nPairing words: {}\n{PROMPT}",
            request.name, request.words
        ),
    )?;
    let answer = loop {
        tokio::select! {
            note = daemon.note() => match pair_note(note?) {
                Some(Wait::Ended(ended)) => {
                    say(&mut out, "\n")?;
                    if ended.reason == "cancelled" {
                        eprintln!("The phone cancelled the pairing; nothing was added.");
                    }
                    return Ok(1);
                }
                _ => continue,
            },
            line = input.recv() => break line.as_deref().is_some_and(is_yes),
            _ = tokio::time::sleep_until(deadline) => {
                say(&mut out, "\n")?;
                return Ok(1);
            }
        }
    };
    let params = PairAnswerParams {
        yes: answer,
        name: options.name.clone(),
        protocols: options.protocols.clone(),
        replace: options.replace,
    };
    let answered = match daemon.call(pair_methods::LINK_PAIR_ANSWER, params).await? {
        Ok(result) => serde_json::from_value::<PairAnswerResult>(result)
            .map_err(|e| format!("unexpected reply: {e}"))?,
        Err(error) => {
            eprintln!("flankr pair: {}", error.message);
            return Ok(1);
        }
    };
    match (answered.added, answered.name) {
        (true, Some(name)) => {
            let protocols = answered.protocols.unwrap_or_default().join(", ");
            say(&mut out, &format!("Added {name} ({protocols})\n"))?;
            Ok(0)
        }
        _ => {
            say(&mut out, "Not paired: the answer was not yes.\n")?;
            Ok(1)
        }
    }
}

/// A pair-mode notification, if `note` is one.
fn pair_note(note: Notification) -> Option<Wait> {
    match note.method.as_str() {
        pair_methods::LINK_PAIR_REQUEST => {
            serde_json::from_value(note.params).ok().map(Wait::Request)
        }
        pair_methods::LINK_PAIR_ENDED => serde_json::from_value(note.params).ok().map(Wait::Ended),
        _ => None,
    }
}

/// Writes `text` to `out` and flushes it.
fn say(out: &mut impl Write, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("write: {e}"))
}

fn show(
    out: &mut impl Write,
    result: &LinkPairBeginResult,
    qr_png: Option<&Path>,
) -> Result<(), String> {
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    let shown = Shown::new(&result.pairing)?;
    if !shown.remote {
        eprintln!(
            "warning: every address is this machine's own; a phone cannot reach it \
             (run tailscale serve, or pass --listen to flankr serve)"
        );
    }
    say(
        out,
        &format!(
            "{}\n\n{}\n\n{}\n\n",
            shown.url,
            shown.data_url(),
            shown.identity()
        ),
    )?;
    if let Some(path) = qr_png {
        write_png(path, &shown.png)?;
        say(out, &format!("Wrote {}\n", path.display()))?;
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
    use libflankr_proto::link::generate_keypair;

    fn pairing(addresses: &[&str]) -> String {
        PairingPayload {
            server_key: generate_keypair().unwrap().public,
            addresses: addresses.iter().map(|a| a.to_string()).collect(),
            offer: Some([7; 16]),
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
            format!("flankr://m4.tail1a2b3c.ts.net:7347/pair#{pairing}")
        );
        assert!(shown.remote);
        assert_eq!(shown.png, qrcodeurl::png(&shown.url).unwrap());
        assert!(
            shown
                .data_url()
                .starts_with("data:image/png;base64,iVBORw0KGgo")
        );
        assert_eq!(
            PairingPayload::decode(&shown.url).unwrap(),
            PairingPayload::decode(&pairing).unwrap()
        );
    }

    #[test]
    fn the_fingerprint_label_and_words_come_from_the_server_key() {
        let pairing = PairingPayload {
            server_key: [0u8; 32],
            addresses: vec!["m4.tail1a2b3c.ts.net:7347".to_string()],
            offer: Some([0; 16]),
        }
        .encode()
        .unwrap();
        let shown = Shown::new(&pairing).unwrap();
        // libflankr_proto's fixed vectors for the all-zero key.
        assert_eq!(shown.fingerprint, "320b 5ea9");
        assert_eq!(
            shown.fingerprint_words,
            "craft foot price devote father throw"
        );
        assert_eq!(
            shown.identity(),
            "Daemon fingerprint: 320b 5ea9 (craft foot price devote father throw)"
        );
    }

    #[test]
    fn the_timeout_is_1_to_300_seconds() {
        assert_eq!(parse_timeout("1"), Ok(1));
        assert_eq!(parse_timeout("60"), Ok(60));
        assert_eq!(parse_timeout(" 300 "), Ok(300));
        let err = parse_timeout("301").unwrap_err();
        assert!(err.contains("at most 300"), "{err}");
        assert!(parse_timeout("100000").unwrap_err().contains("at most 300"));
        for bad in ["0", "-1", "1.5", "", "a minute"] {
            let err = parse_timeout(bad).unwrap_err();
            assert!(err.contains("1 to 300"), "{bad}: {err}");
        }
    }

    #[test]
    fn only_yes_pairs() {
        for yes in ["yes", "YES", " Yes \n", "yEs\r\n"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["y", "yes please", "no", "", "\n", "ye s", "yess"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    #[test]
    fn loopback_alone_is_not_remote() {
        let shown = Shown::new(&pairing(&["127.0.0.1:7347"])).unwrap();
        assert!(!shown.remote);
        assert!(shown.url.starts_with("flankr://127.0.0.1:7347/pair#"));
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
            Pasted::classify(" flankrkey1abc\n"),
            Pasted::Code("flankrkey1abc".to_string())
        );
    }
}
