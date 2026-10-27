//! The link listener: Noise over TCP for paired devices
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, decisions 4–6 and
//! the amendments of 11:58, 18:20 and 23:07 EDT; the rules for step 3 in
//! CHANGES.md, 2026-10-22 18:24 EDT).
//!
//! * By default the link binds only `127.0.0.1:7347`. `[link] listen`
//!   replaces that and turns off the Tailscale check.
//! * Every connection is `IK`. A handshake must finish within
//!   [`HANDSHAKE_TIMEOUT`], and a key not in `auth.toml` gets no reply.
//! * Pairing moves only public keys: `link.pair_begin` gives the pairing
//!   URL (the server's key and addresses) and holds nothing pending;
//!   `link.pair_add` checks a device code against the server's key and
//!   appends the device to `auth.toml`. The device's first `hello` after
//!   that is answered with `paired: true`, and `link.pair_done` goes to the
//!   connection that added it.
//! * `link.unpair`, over a paired link only, removes the calling device's
//!   entry from `auth.toml` and closes its connections once the answer is
//!   queued (plan 20261024_02, decision 6).
//! * A handshake's first message may only be `hello`; anything else closes
//!   the connection without a reply.
//! * `auth.toml` is read for every new connection, and on each change the
//!   daemon's file watcher reports (plan 20261027_05, decision 2) and every
//!   `revoke_interval` as a slow fallback, open connections whose key is no
//!   longer in it are closed.
//! * Sockets get `SO_KEEPALIVE` with the OS's timers; there is no
//!   application idle timeout.
//! * Each link message is one whole JSON-RPC object, without a newline.

pub mod auth;
pub mod keyfile;
pub mod tailscale;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libflankr_proto::common::{
    LinkPairAddParams, LinkPairAddResult, LinkPairBeginResult, LinkPairDone,
};
use libflankr_proto::link::{
    DeviceCode, KEY_LEN, PairingPayload, PairingStringError, StaticKeypair, accept,
};
use libflankr_proto::{Incoming, Request};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use self::auth::{AuthEntry, AuthFile};
use self::tailscale::Tailscale;
use super::Daemon;
use crate::config::LinkConfig;

/// The link's port, when an address is given without one.
pub const DEFAULT_PORT: u16 = 7347;
/// The default listener.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7347";
/// How long a handshake may take.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often open connections are checked against `auth.toml`.
pub const DEFAULT_REVOKE_INTERVAL: Duration = Duration::from_secs(60);

/// Where the link keeps its files, where it listens, and how often it
/// checks for revocations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSettings {
    pub key_path: PathBuf,
    pub auth_path: PathBuf,
    /// `None`: the default listener and the Tailscale check.
    pub listen: Option<Vec<String>>,
    pub revoke_interval: Duration,
}

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("flankr")
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|_| PathBuf::from(p)),
        None => PathBuf::from(p),
    }
}

impl LinkSettings {
    /// From `[link]`; files default to `~/.config/flankr/`.
    pub fn from_config(config: &LinkConfig) -> Self {
        LinkSettings {
            key_path: config
                .key
                .as_deref()
                .map(expand_tilde)
                .unwrap_or_else(|| config_dir().join("link_key")),
            auth_path: config
                .auth
                .as_deref()
                .map(expand_tilde)
                .unwrap_or_else(|| config_dir().join("auth.toml")),
            listen: config.listen.clone(),
            revoke_interval: config
                .revoke_check_secs
                .map(|s| Duration::from_secs(s.max(1)))
                .unwrap_or(DEFAULT_REVOKE_INTERVAL),
        }
    }

    /// Whether the link is on: anything but an explicit empty list.
    pub fn enabled(&self) -> bool {
        self.listen.as_ref().is_none_or(|l| !l.is_empty())
    }
}

/// `<addr>:<port>`, adding port 7347 to a bare address.
pub fn normalize_listen(addr: &str) -> String {
    let addr = addr.trim();
    if addr.parse::<SocketAddr>().is_ok() {
        return addr.to_string();
    }
    if let Ok(ip) = addr.parse::<std::net::IpAddr>() {
        return SocketAddr::new(ip, DEFAULT_PORT).to_string();
    }
    match addr.rsplit_once(':') {
        Some((_, port)) if port.parse::<u16>().is_ok() => addr.to_string(),
        _ => format!("{addr}:{DEFAULT_PORT}"),
    }
}

/// A device `link.pair_add` added that has not said `hello` yet.
struct Awaiting {
    name: String,
    /// To the connection that added it.
    done: oneshot::Sender<LinkPairDone>,
}

/// An open link connection, for the revocation check.
struct OpenConn {
    key: [u8; KEY_LEN],
    name: String,
    close: CancellationToken,
}

/// A device added by `link.pair_add`.
pub struct Added {
    pub result: LinkPairAddResult,
    /// Resolves when the device's first `hello` is answered.
    pub done: oneshot::Receiver<LinkPairDone>,
}

/// The link's shared state.
pub struct Link {
    settings: LinkSettings,
    keypair: StaticKeypair,
    /// Devices added and not yet heard from, by key.
    awaiting: Mutex<HashMap<[u8; KEY_LEN], Awaiting>>,
    conns: Mutex<HashMap<u64, OpenConn>>,
    next_conn: AtomicU64,
    /// Serializes writes to `auth.toml`.
    auth_lock: Mutex<()>,
    /// The addresses of an explicit `listen`, as bound.
    bound: Mutex<Vec<String>>,
}

/// A device name: 1 to 64 of ASCII letters, digits, `.`, `_` and `-`.
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Link {
    /// Load (or create) the server key. Refuses a key file that others may
    /// read.
    pub fn open(settings: LinkSettings) -> Result<Arc<Link>, String> {
        let keypair = keyfile::load_or_create(&settings.key_path)?;
        Ok(Arc::new(Link {
            settings,
            keypair,
            awaiting: Mutex::new(HashMap::new()),
            conns: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            auth_lock: Mutex::new(()),
            bound: Mutex::new(Vec::new()),
        }))
    }

    pub fn settings(&self) -> &LinkSettings {
        &self.settings
    }

    /// The server's static public key.
    pub fn public_key(&self) -> [u8; KEY_LEN] {
        self.keypair.public
    }

    /// The paired clients, as `auth.toml` says now.
    pub fn auth(&self) -> Result<AuthFile, String> {
        AuthFile::load(&self.settings.auth_path)
    }

    /// Bind the listeners. With no explicit `listen`, also checks Tailscale
    /// and logs the forward command if it is missing.
    pub async fn bind(&self) -> Result<Vec<TcpListener>, String> {
        let Some(listen) = &self.settings.listen else {
            let listener = TcpListener::bind(DEFAULT_LISTEN)
                .await
                .map_err(|e| format!("link: bind {DEFAULT_LISTEN}: {e}"))?;
            log::info!("Link listening on {DEFAULT_LISTEN}");
            match tailscale::detect().await {
                ts @ Tailscale::NoForward { .. } => {
                    log::warn!("{}", ts.warning().unwrap_or_default());
                }
                Tailscale::Forwarded { dns_name } => log::info!(
                    "Tailscale forwards {}:7347 to the link",
                    dns_name.as_deref().unwrap_or("this machine")
                ),
                Tailscale::NotRunning => {}
            }
            return Ok(vec![listener]);
        };
        let mut listeners = Vec::new();
        let mut bound = Vec::new();
        for addr in listen {
            let addr = normalize_listen(addr);
            let listener = TcpListener::bind(&addr)
                .await
                .map_err(|e| format!("link: bind {addr}: {e}"))?;
            // Port 0 asks the OS for one; advertise the one it gave.
            let advertised = match listener.local_addr() {
                Ok(local) if addr.ends_with(":0") => local.to_string(),
                _ => addr.clone(),
            };
            log::info!("Link listening on {advertised}");
            bound.push(advertised);
            listeners.push(listener);
        }
        *self.bound.lock().unwrap() = bound;
        Ok(listeners)
    }

    /// The addresses a pairing string carries now, and any warning.
    async fn addresses(&self) -> (Vec<String>, Vec<String>) {
        if self.settings.listen.is_some() {
            return (self.bound.lock().unwrap().clone(), Vec::new());
        }
        let ts = tailscale::detect().await;
        (ts.addresses(), ts.warning().into_iter().collect())
    }

    /// `link.pair_begin`: the pairing URL and string. They hold only the
    /// server's public key and addresses, and nothing is held pending.
    pub async fn pair_begin(&self) -> Result<LinkPairBeginResult, String> {
        let (addresses, warnings) = self.addresses().await;
        if addresses.is_empty() {
            return Err("the link has no address to pair with".to_string());
        }
        let payload = PairingPayload {
            server_key: self.keypair.public,
            addresses: addresses.clone(),
        };
        let pairing = payload.encode().map_err(|e| e.to_string())?;
        let url = payload.url().map_err(|e| e.to_string())?;
        Ok(LinkPairBeginResult {
            url,
            pairing,
            addresses,
            warnings,
        })
    }

    /// `link.pair_add`: checks `code` against the server's key and appends
    /// the device to `auth.toml`, under its own name or `name`, for
    /// `protocols` (by default the companion). A name in use is refused
    /// unless `replace`, which swaps the entry and closes the old key's
    /// connections; a key already paired under another name is refused.
    pub fn pair_add(&self, params: LinkPairAddParams) -> Result<Added, String> {
        let code = DeviceCode::decode(&params.code, &self.keypair.public).map_err(code_error)?;
        let name = match params.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => code.name.clone(),
        };
        if !valid_name(&name) {
            return Err(format!(
                "'{name}' is not a valid name: use 1 to 64 letters, digits, '.', '_' or '-'"
            ));
        }
        let protocols = params
            .protocols
            .unwrap_or_else(|| vec![auth::COMPANION.to_string()]);
        let protocols: Vec<String> = protocols
            .iter()
            .map(|p| auth::short_protocol(p.trim()).to_ascii_lowercase())
            .collect();
        if protocols.is_empty() {
            return Err("give at least one protocol".to_string());
        }
        if let Some(bad) = protocols
            .iter()
            .find(|p| ![auth::COMPANION, auth::HARNESS].contains(&p.as_str()))
        {
            return Err(format!(
                "unknown protocol '{bad}'; use companion or harness"
            ));
        }

        let entry = AuthEntry::new(&name, &code.key, protocols.clone());
        let replaced = {
            let _guard = self.auth_lock.lock().unwrap();
            auth::add(&self.settings.auth_path, entry, params.replace).map_err(|e| e.to_string())?
        };
        if let Some(old) = replaced.and_then(|old| old.public_key())
            && old != code.key
        {
            self.close_where(|c| c.key == old, "its entry was replaced");
            self.awaiting.lock().unwrap().remove(&old);
        }
        let (done_tx, done) = oneshot::channel();
        self.awaiting.lock().unwrap().insert(
            code.key,
            Awaiting {
                name: name.clone(),
                done: done_tx,
            },
        );
        log::info!("Added '{name}' ({}) to auth.toml", protocols.join(", "));
        Ok(Added {
            result: LinkPairAddResult { name, protocols },
            done,
        })
    }

    /// A connection with `key` had its `hello` accepted. Returns whether
    /// it is the first since `link.pair_add` added the key, and if so tells
    /// the connection that added it.
    pub fn first_hello(&self, key: &[u8; KEY_LEN]) -> bool {
        let Some(awaiting) = self.awaiting.lock().unwrap().remove(key) else {
            return false;
        };
        log::info!("'{}' connected for the first time", awaiting.name);
        let _ = awaiting.done.send(LinkPairDone {
            name: awaiting.name,
            paired: true,
            error: None,
        });
        true
    }

    /// `link.unpair` (plan 20261024_02, decision 6): removes the entry with
    /// `key` from `auth.toml`, so the key no longer connects. Its open
    /// connections are closed by [`Link::close_key`], once the answer is on
    /// its way. Returns the name it was paired as.
    pub fn unpair(&self, key: &[u8; KEY_LEN]) -> Result<Option<String>, String> {
        let removed = {
            let _guard = self.auth_lock.lock().unwrap();
            auth::remove(&self.settings.auth_path, key)?
        };
        self.awaiting.lock().unwrap().remove(key);
        if let Some(entry) = &removed {
            log::info!("Removed '{}' from auth.toml: it unpaired", entry.name);
        }
        Ok(removed.map(|e| e.name))
    }

    /// Close every open connection with `key`.
    pub fn close_key(&self, key: &[u8; KEY_LEN], why: &str) {
        self.close_where(|c| &c.key == key, why);
    }

    fn register(&self, key: [u8; KEY_LEN], name: &str) -> (u64, CancellationToken) {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let close = CancellationToken::new();
        self.conns.lock().unwrap().insert(
            id,
            OpenConn {
                key,
                name: name.to_string(),
                close: close.clone(),
            },
        );
        (id, close)
    }

    fn unregister(&self, id: u64) {
        self.conns.lock().unwrap().remove(&id);
    }

    fn close_where(&self, mut pred: impl FnMut(&OpenConn) -> bool, why: &str) {
        for conn in self.conns.lock().unwrap().values() {
            if pred(conn) && !conn.close.is_cancelled() {
                log::info!("Closing the link connection of '{}': {why}", conn.name);
                conn.close.cancel();
            }
        }
    }

    /// Close open connections whose key is no longer in `auth.toml`. A file
    /// that cannot be read closes them all.
    pub fn check_revocations(&self) {
        match self.auth() {
            Ok(file) => self.close_where(|c| file.find_key(&c.key).is_none(), "its entry is gone"),
            Err(e) => {
                log::error!("auth.toml: {e}; closing every link connection");
                self.close_where(|_| true, "auth.toml is unreadable");
            }
        }
    }

    /// How many link connections are open.
    pub fn open_connections(&self) -> usize {
        self.conns.lock().unwrap().len()
    }
}

/// Serve link connections on `listeners`, and run the revocation check.
pub fn spawn(daemon: Arc<Daemon>, listeners: Vec<TcpListener>) {
    let Some(link) = daemon.link.clone() else {
        return;
    };
    // On each change to auth.toml the file watcher reports (plan
    // 20261027_05, decision 2), and every `revoke_interval` as a slow
    // fallback in case the watcher fails.
    let mut changes = daemon.file_watcher().and_then(|files| {
        files
            .subscribe(std::slice::from_ref(&link.settings.auth_path))
            .inspect_err(|e| {
                log::warn!(
                    "{} can't be watched ({e}); revocations are checked every {:?}",
                    link.settings.auth_path.display(),
                    link.settings.revoke_interval
                )
            })
            .ok()
    });
    let checker = Arc::clone(&link);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(checker.settings.revoke_interval);
        tick.tick().await;
        loop {
            match &mut changes {
                Some(sub) => {
                    tokio::select! {
                        _ = tick.tick() => {}
                        alive = sub.changed() => {
                            if !alive {
                                changes = None;
                            }
                        }
                    }
                }
                None => {
                    tick.tick().await;
                }
            }
            checker.check_revocations();
        }
    });
    for listener in listeners {
        let daemon = Arc::clone(&daemon);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        tokio::spawn(handle(Arc::clone(&daemon), stream, peer));
                    }
                    Err(e) => {
                        log::warn!("link accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });
    }
}

/// Whether `bytes` is a `hello` request.
fn is_hello(bytes: &[u8]) -> bool {
    matches!(
        serde_json::from_slice::<Incoming>(bytes),
        Ok(Incoming::Request(Request { ref method, .. })) if method == libflankr_proto::common::methods::HELLO
    )
}

async fn handle(daemon: Arc<Daemon>, stream: TcpStream, peer: SocketAddr) {
    let Some(link) = daemon.link.clone() else {
        return;
    };
    if let Err(e) = socket2::SockRef::from(&stream).set_keepalive(true) {
        log::warn!("link {peer}: SO_KEEPALIVE: {e}");
    }

    let mut known: Option<AuthEntry> = None;
    let accepted = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        accept(stream, &link.keypair, |key| {
            known = match link.auth() {
                Ok(file) => file.find_key(key).cloned(),
                Err(e) => {
                    log::error!("auth.toml: {e}");
                    None
                }
            };
            known.is_some()
        }),
    )
    .await;
    let accepted = match accepted {
        Ok(Ok(accepted)) => accepted,
        Ok(Err(e)) => {
            log::info!("Refused a link connection from {peer}: {e}");
            return;
        }
        Err(_) => {
            log::info!("Refused a link connection from {peer}: the handshake timed out");
            return;
        }
    };
    let Some(entry) = known else {
        return;
    };

    // Only hello may ride in a handshake: anything else could be a replay.
    if let Some(first) = &accepted.first_message
        && !is_hello(first)
    {
        log::warn!(
            "Closed the link connection of '{}' from {peer}: its first message was not hello",
            entry.name
        );
        return;
    }

    let (conn_id, close) = link.register(accepted.peer_key, &entry.name);
    log::info!("Link connection from '{}' ({peer})", entry.name);
    super::conn::handle_link(
        Arc::clone(&daemon),
        accepted.stream,
        accepted.first_message,
        entry,
        accepted.peer_key,
        close,
    )
    .await;
    link.unregister(conn_id);
}

/// What `link.pair_add` says of a device code it refused.
fn code_error(e: PairingStringError) -> String {
    match e {
        PairingStringError::BadChecksum => {
            "that code does not match this server: it was made for another server, or mistyped"
                .to_string()
        }
        PairingStringError::BadPrefix => {
            "that is not a device code: a device code starts with flankrkey1".to_string()
        }
        PairingStringError::BadName => {
            "the device's name in that code is not valid; pass --name".to_string()
        }
        other => format!("that is not a valid device code: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_addresses_get_the_default_port() {
        assert_eq!(normalize_listen("127.0.0.1"), "127.0.0.1:7347");
        assert_eq!(normalize_listen("100.64.0.1:9000"), "100.64.0.1:9000");
        assert_eq!(normalize_listen("::1"), "[::1]:7347");
        assert_eq!(normalize_listen("[::1]:80"), "[::1]:80");
        assert_eq!(normalize_listen("mini.ts.net"), "mini.ts.net:7347");
        assert_eq!(normalize_listen("mini.ts.net:1"), "mini.ts.net:1");
    }

    #[test]
    fn names() {
        assert!(valid_name("s23"));
        assert!(valid_name("t3-code.mini_1"));
        assert!(!valid_name(""));
        assert!(!valid_name("has space"));
        assert!(!valid_name(&"x".repeat(65)));
    }

    #[test]
    fn settings_default_to_the_config_dir_and_an_empty_list_is_off() {
        let s = LinkSettings::from_config(&LinkConfig::default());
        assert!(s.key_path.ends_with(".config/flankr/link_key"));
        assert!(s.auth_path.ends_with(".config/flankr/auth.toml"));
        assert_eq!(s.revoke_interval, DEFAULT_REVOKE_INTERVAL);
        assert!(s.enabled());
        let off = LinkSettings::from_config(&LinkConfig {
            listen: Some(vec![]),
            ..Default::default()
        });
        assert!(!off.enabled());
    }

    #[test]
    fn the_link_does_not_open_with_a_key_at_0644() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let settings = LinkSettings {
            key_path: dir.path().join("link_key"),
            auth_path: dir.path().join("auth.toml"),
            listen: Some(vec!["127.0.0.1:0".to_string()]),
            revoke_interval: DEFAULT_REVOKE_INTERVAL,
        };
        let first = Link::open(settings.clone()).unwrap().public_key();
        assert_eq!(Link::open(settings.clone()).unwrap().public_key(), first);
        std::fs::set_permissions(&settings.key_path, std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let err = Link::open(settings).err().unwrap();
        assert!(err.contains("0644"), "{err}");
    }

    #[test]
    fn only_hello_is_a_first_message() {
        assert!(is_hello(
            br#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{}}"#
        ));
        assert!(!is_hello(
            br#"{"jsonrpc":"2.0","id":1,"method":"agents.send","params":{}}"#
        ));
        assert!(!is_hello(b"not json"));
    }
}
