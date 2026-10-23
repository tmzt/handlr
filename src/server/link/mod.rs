//! The link listener: Noise over TCP for paired devices
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, decisions 4–6 and
//! the amendments of 11:58 and 18:20 EDT; the rules for step 3 in
//! CHANGES.md, 2026-10-22 18:24 EDT).
//!
//! * By default the link binds only `127.0.0.1:7347`. `[link] listen`
//!   replaces that and turns off the Tailscale check.
//! * A handshake must finish within [`HANDSHAKE_TIMEOUT`].
//! * A pairing secret is burned on first use, and its expiry is checked
//!   before it is looked up.
//! * A handshake's first message may only be `hello`; anything else closes
//!   the connection without a reply.
//! * `auth.toml` is read for every new connection, and every
//!   `revoke_interval` open connections whose key is no longer in it are
//!   closed.
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libhandlr_proto::common::{LinkPairBeginParams, LinkPairBeginResult, LinkPairDone};
use libhandlr_proto::link::{
    accept, generate_secret, AcceptedMode, PairingId, PairingPayload, StaticKeypair, KEY_LEN,
};
use libhandlr_proto::{Incoming, Request};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use self::auth::{AddError, AuthEntry, AuthFile};
use self::tailscale::Tailscale;
use super::Daemon;
use crate::config::LinkConfig;

/// The link's port, when an address is given without one.
pub const DEFAULT_PORT: u16 = 7347;
/// The default listener.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7347";
/// How long a handshake may take.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a pairing secret lives unless `--ttl` says otherwise.
pub const DEFAULT_PAIR_TTL: u64 = 300;
/// The longest `--ttl`.
pub const MAX_PAIR_TTL: u64 = 3600;
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
    PathBuf::from(home).join(".config").join("handlr")
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
    /// From `[link]`; files default to `~/.config/handlr/`.
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

/// A pairing that `link.pair_begin` opened and no device has used yet.
struct Pending {
    secret: [u8; KEY_LEN],
    name: String,
    protocols: Vec<String>,
    replace: bool,
    expiry: u64,
    done: oneshot::Sender<LinkPairDone>,
}

/// An open link connection, for the revocation check.
struct OpenConn {
    key: [u8; KEY_LEN],
    name: String,
    close: CancellationToken,
}

/// A pairing begun by `link.pair_begin`.
pub struct Begun {
    pub id: PairingId,
    pub result: LinkPairBeginResult,
    /// Resolves when a device pairs, the pairing fails, or it expires.
    pub done: oneshot::Receiver<LinkPairDone>,
}

/// The link's shared state.
pub struct Link {
    settings: LinkSettings,
    keypair: StaticKeypair,
    pending: Mutex<HashMap<PairingId, Pending>>,
    conns: Mutex<HashMap<u64, OpenConn>>,
    next_conn: AtomicU64,
    /// Serializes writes to `auth.toml`.
    auth_lock: Mutex<()>,
    /// The addresses of an explicit `listen`, as bound.
    bound: Mutex<Vec<String>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
            pending: Mutex::new(HashMap::new()),
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

    /// `link.pair_begin`: a pending pairing.
    pub async fn pair_begin(
        self: &Arc<Self>,
        params: LinkPairBeginParams,
    ) -> Result<Begun, String> {
        let name = params.name.trim().to_string();
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
        let ttl = params.ttl.unwrap_or(DEFAULT_PAIR_TTL);
        if !(1..=MAX_PAIR_TTL).contains(&ttl) {
            return Err(format!("ttl must be 1 to {MAX_PAIR_TTL} seconds"));
        }
        let file = self.auth()?;
        if !params.replace && file.find_name(&name).is_some() {
            return Err(AddError::NameInUse(name).to_string());
        }
        let (addresses, warnings) = self.addresses().await;
        if addresses.is_empty() {
            return Err("the link has no address to pair with".to_string());
        }

        let expiry = now_secs() + ttl;
        let (done_tx, done) = oneshot::channel();
        let (id, secret) = {
            let mut pending = self.pending.lock().unwrap();
            if pending.values().any(|p| p.name == name) {
                return Err(format!("a pairing for '{name}' is already waiting"));
            }
            let (id, secret) = loop {
                let secret = generate_secret().map_err(|e| e.to_string())?;
                let id = PairingId::from_secret(&secret);
                if !pending.contains_key(&id) {
                    break (id, secret);
                }
            };
            pending.insert(
                id,
                Pending {
                    secret,
                    name: name.clone(),
                    protocols,
                    replace: params.replace,
                    expiry,
                    done: done_tx,
                },
            );
            (id, secret)
        };
        let payload = PairingPayload {
            server_key: self.keypair.public,
            secret,
            expiry: u32::try_from(expiry).unwrap_or(u32::MAX),
            name: name.clone(),
            addresses: addresses.clone(),
        };
        let pairing = match payload.encode() {
            Ok(pairing) => pairing,
            Err(e) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(e.to_string());
            }
        };

        // Expire it, and tell whoever waits.
        let link = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(ttl)).await;
            if let Some(p) = link.pending.lock().unwrap().remove(&id) {
                log::info!("The pairing for '{}' expired", p.name);
                let _ = p.done.send(LinkPairDone {
                    name: p.name,
                    paired: false,
                    error: Some("the pairing expired".to_string()),
                });
            }
        });
        log::info!("Pairing for '{name}' waits for {ttl} s");
        Ok(Begun {
            id,
            result: LinkPairBeginResult {
                pairing,
                name,
                expiry,
                addresses,
                warnings,
            },
            done,
        })
    }

    /// Withdraw pairings whose `handlr pair` went away.
    pub fn withdraw(&self, ids: &[PairingId]) {
        let mut pending = self.pending.lock().unwrap();
        for id in ids {
            if let Some(p) = pending.remove(id) {
                log::info!("The pairing for '{}' was withdrawn", p.name);
            }
        }
    }

    /// Take (and so burn) the pending pairing `id`. An expired one is
    /// burned too, and not handed out.
    fn take_pending(&self, id: PairingId) -> Option<Pending> {
        let p = self.pending.lock().unwrap().remove(&id)?;
        if now_secs() >= p.expiry {
            log::info!("Refused an expired pairing for '{}'", p.name);
            let _ = p.done.send(LinkPairDone {
                name: p.name,
                paired: false,
                error: Some("the pairing expired".to_string()),
            });
            return None;
        }
        Some(p)
    }

    /// Record a completed pairing in `auth.toml`, closing the old key's
    /// connections on a replace.
    fn complete(&self, p: &Pending, key: &[u8; KEY_LEN]) -> Result<AuthEntry, String> {
        let entry = AuthEntry::new(&p.name, key, p.protocols.clone());
        let replaced = {
            let _guard = self.auth_lock.lock().unwrap();
            auth::add(&self.settings.auth_path, entry.clone(), p.replace)
                .map_err(|e| e.to_string())?
        };
        if let Some(old) = replaced.and_then(|old| old.public_key()) {
            if &old != key {
                self.close_where(|c| c.key == old, "its entry was replaced");
            }
        }
        // Read back, so `agents` and `approve` kept by a replace apply.
        Ok(self
            .auth()
            .ok()
            .and_then(|f| f.find_key(key).cloned())
            .unwrap_or(entry))
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
    let checker = Arc::clone(&link);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(checker.settings.revoke_interval);
        tick.tick().await;
        loop {
            tick.tick().await;
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
        Ok(Incoming::Request(Request { ref method, .. })) if method == libhandlr_proto::common::methods::HELLO
    )
}

async fn handle(daemon: Arc<Daemon>, stream: TcpStream, peer: SocketAddr) {
    let Some(link) = daemon.link.clone() else {
        return;
    };
    if let Err(e) = socket2::SockRef::from(&stream).set_keepalive(true) {
        log::warn!("link {peer}: SO_KEEPALIVE: {e}");
    }

    let mut taken: Option<Pending> = None;
    let mut known: Option<AuthEntry> = None;
    let accepted = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        accept(
            stream,
            &link.keypair,
            |id| {
                let p = link.take_pending(id)?;
                let secret = p.secret;
                taken = Some(p);
                Some(secret)
            },
            |key| {
                known = match link.auth() {
                    Ok(file) => file.find_key(key).cloned(),
                    Err(e) => {
                        log::error!("auth.toml: {e}");
                        None
                    }
                };
                known.is_some()
            },
        ),
    )
    .await;
    let accepted = match accepted {
        Ok(Ok(accepted)) => accepted,
        Ok(Err(e)) => {
            refused(&e.to_string(), peer, taken);
            return;
        }
        Err(_) => {
            refused("the handshake timed out", peer, taken);
            return;
        }
    };

    let (entry, paired_now) = match (accepted.mode, taken) {
        (AcceptedMode::Pairing(_), Some(p)) => match link.complete(&p, &accepted.peer_key) {
            Ok(entry) => {
                log::info!("Paired '{}' from {peer}", entry.name);
                let _ = p.done.send(LinkPairDone {
                    name: p.name.clone(),
                    paired: true,
                    error: None,
                });
                (entry, true)
            }
            Err(e) => {
                log::warn!("Pairing '{}' from {peer} refused: {e}", p.name);
                let _ = p.done.send(LinkPairDone {
                    name: p.name.clone(),
                    paired: false,
                    error: Some(e),
                });
                return;
            }
        },
        (AcceptedMode::Reconnect, _) => match known {
            Some(entry) => (entry, false),
            None => return,
        },
        (AcceptedMode::Pairing(_), None) => return,
    };

    // Only hello may ride in a handshake: anything else could be a replay.
    if let Some(first) = &accepted.first_message {
        if !is_hello(first) {
            log::warn!(
                "Closed the link connection of '{}' from {peer}: its first message was not hello",
                entry.name
            );
            return;
        }
    }

    let (conn_id, close) = link.register(accepted.peer_key, &entry.name);
    log::info!("Link connection from '{}' ({peer})", entry.name);
    super::conn::handle_link(
        Arc::clone(&daemon),
        accepted.stream,
        accepted.first_message,
        entry,
        paired_now,
        close,
    )
    .await;
    link.unregister(conn_id);
}

fn refused(why: &str, peer: SocketAddr, taken: Option<Pending>) {
    match taken {
        Some(p) => {
            log::warn!("Pairing '{}' from {peer} failed: {why}", p.name);
            let _ = p.done.send(LinkPairDone {
                name: p.name,
                paired: false,
                error: Some(format!("the handshake failed: {why}")),
            });
        }
        None => log::info!("Refused a link connection from {peer}: {why}"),
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
        assert!(s.key_path.ends_with(".config/handlr/link_key"));
        assert!(s.auth_path.ends_with(".config/handlr/auth.toml"));
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
