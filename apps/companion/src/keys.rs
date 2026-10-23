//! Key storage in libmorph's key store and secure store, as libmorph's key
//! store plan (`20261022_06_KEY_STORE_PLAN.md`, section 7) has handlr use
//! them:
//!
//! - this device's Noise static key is a [`NoiseStatic`] in the key store,
//!   under [`KEY_NAME`]. Its private half never leaves the store: each
//!   Diffie-Hellman borrows it for one call through `key_store::with_key`.
//! - the paired server is a TOML blob, [`SERVER_BLOB`], in the secure
//!   store, in `FileKeyStore`'s `server.toml` layout.
//!
//! On Android these are the Keystore-sealed stores of libmobile, and on
//! macOS the login Keychain. Elsewhere libmorph has neither, and the
//! desktop falls back to `FileKeyStore` (see `lib.rs`); Android never does.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use curve25519_dalek::montgomery::MontgomeryPoint;
use handlr_companion::{KeyStore, ServerRecord};
use libhandlr_proto::link::StaticKey;
use libmorph::key_store::{self, KeyStoreError, SealToken, SecretKey};
use libmorph::{SecureStore, Vault};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// The key store name of this device's static key.
pub const KEY_NAME: &str = "noise.static";
/// The secure store blob that holds the paired server.
pub const SERVER_BLOB: &str = "server";

/// An X25519 private key, kept behind a heap pointer so that moves do not
/// leave copies, and cleared on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct NoiseStatic(Box<[u8; 32]>);

impl NoiseStatic {
    /// X25519 of this key with `peer`, or `None` for an all-zero result (a
    /// low-order peer key).
    fn dh(&self, peer: &[u8; 32]) -> Option<[u8; 32]> {
        let shared = MontgomeryPoint(*peer).mul_clamped(*self.0).to_bytes();
        (shared != [0u8; 32]).then_some(shared)
    }
}

impl SecretKey for NoiseStatic {
    type Public = [u8; 32];
    const KIND: &'static str = "handlr.noise.x25519.v1";

    fn generate(_: &SealToken) -> Self {
        let mut key = Box::new([0u8; 32]);
        getrandom::getrandom(&mut key[..]).expect("the OS random source");
        Self(key)
    }

    fn seal(&self, _: &SealToken) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.0.to_vec())
    }

    fn unseal(_kind: &str, bytes: &[u8], _: &SealToken) -> Result<Self, String> {
        if bytes.len() != 32 {
            return Err(format!("a Noise key is 32 bytes, not {}", bytes.len()));
        }
        let mut key = Box::new([0u8; 32]);
        key.copy_from_slice(bytes);
        Ok(Self(key))
    }

    fn public(&self) -> [u8; 32] {
        MontgomeryPoint::mul_base_clamped(*self.0).to_bytes()
    }
}

/// The static key the link's handshakes use: the stored public value, and a
/// Diffie-Hellman that borrows the private key from the store for each call.
pub struct VaultStaticKey {
    vault: Vault,
    public: [u8; 32],
}

impl StaticKey for VaultStaticKey {
    fn public(&self) -> [u8; 32] {
        self.public
    }

    fn dh(&self, peer: &[u8; 32]) -> Option<[u8; 32]> {
        match key_store::with_key::<NoiseStatic, _>(&self.vault, KEY_NAME, |key| key.dh(peer)) {
            Ok(shared) => shared,
            Err(e) => {
                log::error!("keys: {e}");
                None
            }
        }
    }
}

/// [`KeyStore`] on libmorph's key store and secure store.
pub struct VaultKeyStore {
    vault: Vault,
    store: SecureStore,
}

impl VaultKeyStore {
    pub fn new(vault: Vault, store: SecureStore) -> Self {
        Self { vault, store }
    }

    /// Where the secure store keeps its blobs, for the log.
    pub fn describe(&self) -> String {
        match self.store.protection() {
            Ok(protection) => format!("{:?}, {protection:?}", self.vault),
            Err(e) => format!("{:?}, protection unknown: {e}", self.vault),
        }
    }
}

impl KeyStore for VaultKeyStore {
    fn static_key(&mut self) -> Result<Option<Arc<dyn StaticKey>>, String> {
        match key_store::public::<NoiseStatic>(&self.vault, KEY_NAME) {
            Ok(public) => Ok(Some(Arc::new(VaultStaticKey {
                vault: self.vault.clone(),
                public,
            }))),
            Err(KeyStoreError::NotFound) => Ok(None),
            // A key that cannot be read means pairing again, which makes a
            // new one in its place.
            Err(e @ (KeyStoreError::Corrupt(_) | KeyStoreError::WrongKind(_))) => {
                log::warn!("keys: the stored key is unusable ({e}); pair again");
                Ok(None)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn generate_static_key(&mut self) -> Result<Arc<dyn StaticKey>, String> {
        // No flags: reconnecting in the background works while the phone is
        // locked (the key store plan, section 7).
        let public = key_store::generate::<NoiseStatic>(&self.vault, KEY_NAME, 0)
            .map_err(|e| e.to_string())?;
        log::info!("keys: made a new static key in the key store");
        Ok(Arc::new(VaultStaticKey {
            vault: self.vault.clone(),
            public,
        }))
    }

    fn server(&mut self) -> Result<Option<ServerRecord>, String> {
        match self.store.get(SERVER_BLOB).map_err(|e| e.to_string())? {
            Some(bytes) => decode_server(&bytes).map(Some),
            None => Ok(None),
        }
    }

    fn set_server(&mut self, server: ServerRecord) -> Result<(), String> {
        let bytes = encode_server(server)?;
        self.store
            .put(SERVER_BLOB, &bytes)
            .map_err(|e| e.to_string())
    }
}

/// The server blob: `FileKeyStore`'s `server.toml`, the key in base64.
#[derive(Serialize, Deserialize)]
struct ServerFile {
    name: String,
    public_key: String,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

fn encode_server(server: ServerRecord) -> Result<Vec<u8>, String> {
    let file = ServerFile {
        name: server.name,
        public_key: STANDARD.encode(server.public_key),
        addresses: server.addresses,
        session_id: server.session_id,
    };
    toml::to_string(&file)
        .map(String::into_bytes)
        .map_err(|e| e.to_string())
}

fn decode_server(bytes: &[u8]) -> Result<ServerRecord, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("the server blob: {e}"))?;
    let file: ServerFile = toml::from_str(text).map_err(|e| format!("the server blob: {e}"))?;
    let key = STANDARD
        .decode(file.public_key.trim())
        .map_err(|e| format!("the server's public key: {e}"))?;
    let public_key = key
        .try_into()
        .map_err(|k: Vec<u8>| format!("the server's public key is {} bytes, not 32", k.len()))?;
    Ok(ServerRecord {
        name: file.name,
        public_key,
        addresses: file.addresses,
        session_id: file.session_id,
    })
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use libhandlr_proto::link::generate_keypair;

    #[test]
    fn the_vault_key_agrees_with_snow() {
        let vault = Vault::memory();
        let public = key_store::generate::<NoiseStatic>(&vault, KEY_NAME, 0).unwrap();
        let ours = VaultStaticKey { vault, public };
        let peer = generate_keypair().unwrap();
        let shared = ours.dh(&peer.public).expect("a shared secret");
        assert_eq!(Some(shared), peer.dh(&ours.public()));
    }

    #[test]
    fn a_low_order_peer_gives_none() {
        let vault = Vault::memory();
        let public = key_store::generate::<NoiseStatic>(&vault, KEY_NAME, 0).unwrap();
        let ours = VaultStaticKey { vault, public };
        assert_eq!(ours.dh(&[0u8; 32]), None);
    }

    #[test]
    fn the_server_blob_round_trips() {
        let record = ServerRecord {
            name: "m4".into(),
            public_key: [7u8; 32],
            addresses: vec!["m4.tail.ts.net:7347".into()],
            session_id: Some("companion:phone".into()),
        };
        let bytes = encode_server(record.clone()).unwrap();
        assert_eq!(decode_server(&bytes).unwrap(), record);
        assert!(decode_server(b"name = \"m4\"\npublic_key = \"AAAA\"\n").is_err());
    }
}
