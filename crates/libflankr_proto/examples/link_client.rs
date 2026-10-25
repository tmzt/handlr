//! A flankr link client, for pairing, reconnecting and revocation
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, step 3, as amended
//! at 23:07 EDT: pairing by public keys).
//!
//! ```text
//! cargo run -p libflankr_proto --features link --example link_client -- \
//!     --key <file> [--name <name>] [--protocol flankr.harness] [--wait] \
//!     [<pairing URL or string>]
//! ```
//!
//! With a pairing URL or string, it uses the key in `--key` (or makes one),
//! stores the server's key and addresses there (mode 0600), and prints its
//! device code for `flankr pair`. It then connects (`IK`, `hello` in the
//! handshake) every 2 s until the server accepts its key, which it does once
//! the code is pasted. Without one, it reads `--key` and connects once. Either
//! way it then runs `agents.list`. `--wait` then stays connected, printing
//! what arrives, until the server closes the connection (as it does when the
//! entry is deleted).
//!
//! Each link message is one JSON-RPC object, with no newline.
//!
//! The crate's dev-dependencies give Tokio without `net` or `time`, so the
//! TCP connection is a std `TcpStream`, bridged to an in-memory Tokio
//! duplex by two threads, and the 2 s wait is a thread sleep.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use libflankr_proto::common::{HelloParams, methods, protocols};
use libflankr_proto::link::{
    DeviceCode, LinkError, LinkStream, PairingPayload, StaticKeypair, generate_keypair, reconnect,
};
use libflankr_proto::{Id, Incoming, Request};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often to try again while the server does not know the key yet.
const RETRY: Duration = Duration::from_secs(2);

struct Args {
    key: PathBuf,
    name: String,
    protocol: String,
    wait: bool,
    pairing: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let usage = "usage: link_client --key <file> [--name <name>] [--protocol <name>] [--wait] \
                 [<pairing URL or string>]";
    let mut key = None;
    let mut name = "link-client".to_string();
    let mut protocol = protocols::HARNESS.to_string();
    let mut wait = false;
    let mut pairing = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--key" => key = args.next().map(PathBuf::from),
            "--name" => name = args.next().ok_or(usage)?,
            "--protocol" => protocol = args.next().ok_or(usage)?,
            "--wait" => wait = true,
            _ if !arg.starts_with("--") && pairing.is_none() => pairing = Some(arg),
            _ => return Err(usage.to_string()),
        }
    }
    Ok(Args {
        key: key.ok_or(usage)?,
        name,
        protocol,
        wait,
        pairing,
    })
}

/// What the key file keeps: this client's private key and name, and the
/// server.
struct Stored {
    private: [u8; 32],
    server: [u8; 32],
    name: String,
    addresses: Vec<String>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

impl Stored {
    fn save(&self, path: &Path) -> Result<(), String> {
        let text = format!(
            "# flankr link_client key file\nprivate = {}\nserver = {}\nname = {}\naddresses = {}\n",
            hex(&self.private),
            hex(&self.server),
            self.name,
            self.addresses.join(",")
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        // An existing file keeps its mode; make sure it is private.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn load(path: &Path) -> Result<Stored, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!("{} must be mode 0600", path.display()));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let field = |name: &str| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == name)
                .map(|(_, v)| v.trim().to_string())
                .ok_or_else(|| format!("{}: no {name}", path.display()))
        };
        let bad = |name: &str| format!("{}: bad {name}", path.display());
        Ok(Stored {
            private: unhex(&field("private")?).ok_or_else(|| bad("private"))?,
            server: unhex(&field("server")?).ok_or_else(|| bad("server"))?,
            name: field("name")?,
            addresses: field("addresses")?
                .split(',')
                .filter(|a| !a.is_empty())
                .map(str::to_string)
                .collect(),
        })
    }
}

/// Connect to the first address that answers, bridged to a Tokio duplex.
fn connect(addresses: &[String]) -> Result<DuplexStream, String> {
    let mut last = "no addresses".to_string();
    for address in addresses {
        let resolved = match address.to_socket_addrs() {
            Ok(resolved) => resolved,
            Err(e) => {
                last = format!("{address}: {e}");
                continue;
            }
        };
        for addr in resolved {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(tcp) => {
                    eprintln!("connected to {address} ({addr})");
                    return bridge(tcp).map_err(|e| e.to_string());
                }
                Err(e) => last = format!("{address}: {e}"),
            }
        }
    }
    Err(format!("could not connect: {last}"))
}

fn bridge(tcp: TcpStream) -> std::io::Result<DuplexStream> {
    let _ = tcp.set_nodelay(true);
    let (ours, theirs) = tokio::io::duplex(1 << 17);
    let (mut from_tcp, mut to_tcp) = tokio::io::split(theirs);
    let handle = tokio::runtime::Handle::current();
    let mut tcp_read = tcp.try_clone()?;
    let mut tcp_write = tcp;
    let h = handle.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            match tcp_read.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if h.block_on(to_tcp.write_all(&buf[..n])).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = h.block_on(to_tcp.shutdown());
    });
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            match handle.block_on(from_tcp.read(&mut buf)) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tcp_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = tcp_write.shutdown(Shutdown::Both);
    });
    Ok(ours)
}

fn hello(protocol: &str) -> Vec<u8> {
    let request = Request::new(
        Id::Number(0),
        methods::HELLO,
        HelloParams {
            protocol: protocol.to_string(),
            version: 1,
            client: "link_client".to_string(),
        },
    );
    serde_json::to_vec(&request).expect("a request serializes")
}

/// Read until the response to `id`, printing notifications.
async fn response(stream: &mut LinkStream<DuplexStream>, id: i64) -> Result<Value, String> {
    loop {
        let message = stream.recv_message().await.map_err(|e| e.to_string())?;
        match serde_json::from_slice::<Incoming>(&message) {
            Ok(Incoming::Response(r)) if r.id == Some(Id::Number(id)) => {
                return match r.error {
                    Some(error) => Err(format!("error {}: {}", error.code, error.message)),
                    None => Ok(r.result.unwrap_or(Value::Null)),
                };
            }
            Ok(Incoming::Response(r)) if r.id.is_none() => {
                return Err(format!("error: {:?}", r.error));
            }
            _ => println!("{}", String::from_utf8_lossy(&message)),
        }
    }
}

/// Connects once with `IK`.
async fn connect_once(
    stored: &Stored,
    first: &[u8],
) -> Result<LinkStream<DuplexStream>, LinkError> {
    let io = connect(&stored.addresses).map_err(|e| LinkError::Io(std::io::Error::other(e)))?;
    let keypair = StaticKeypair::from_private(stored.private)?;
    // No client-side timeout: this crate's Tokio has no `time` feature.
    // The server times a handshake out at 10 s.
    reconnect(io, &keypair, &stored.server, Some(first)).await
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let first = hello(&args.protocol);
    let mut stream = match &args.pairing {
        Some(text) => {
            let payload = PairingPayload::decode(text).map_err(|e| e.to_string())?;
            // Keep this client's key and name if it has them.
            let (private, name) = match Stored::load(&args.key) {
                Ok(stored) => (stored.private, stored.name),
                Err(_) => (
                    generate_keypair().map_err(|e| e.to_string())?.private,
                    args.name.clone(),
                ),
            };
            let stored = Stored {
                private,
                server: payload.server_key,
                name,
                addresses: payload.addresses.clone(),
            };
            stored.save(&args.key)?;
            let code = DeviceCode {
                key: libflankr_proto::link::public_key(&stored.private)
                    .map_err(|e| e.to_string())?,
                name: stored.name.clone(),
            }
            .encode(&payload.server_key)
            .map_err(|e| e.to_string())?;
            eprintln!("key stored in {}", args.key.display());
            eprintln!("Paste this code into flankr pair:");
            println!("{code}");
            loop {
                match connect_once(&stored, &first).await {
                    Ok(stream) => break stream,
                    Err(e) => {
                        eprintln!("not accepted yet ({e}); trying again in 2 s");
                        std::thread::sleep(RETRY);
                    }
                }
            }
        }
        None => {
            let stored = Stored::load(&args.key)?;
            let stream = connect_once(&stored, &first).await.map_err(|e| {
                format!(
                    "reconnecting failed (is {} still paired?): {e}",
                    stored.name
                )
            })?;
            eprintln!("reconnected as {}", stored.name);
            stream
        }
    };

    let hello = response(&mut stream, 0).await?;
    println!("hello: {hello}");
    let list = Request::new(Id::Number(1), methods::AGENTS_LIST, json!({}));
    stream
        .send_message(&serde_json::to_vec(&list).map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())?;
    let agents = response(&mut stream, 1).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&agents).unwrap_or_default()
    );

    if args.wait {
        eprintln!("waiting; the server closes the connection when the entry is deleted");
        loop {
            match stream.recv_message().await {
                Ok(message) => println!("{}", String::from_utf8_lossy(&message)),
                Err(e) => {
                    println!("connection closed by the server ({e})");
                    break;
                }
            }
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let code = match run().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("link_client: {e}");
            1
        }
    };
    // The bridge threads may still be blocked on the socket.
    std::process::exit(code);
}
