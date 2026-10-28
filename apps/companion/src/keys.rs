//! Key storage in libmorph's key store and secure store, as libmorph's key
//! store plan (`20261022_06_KEY_STORE_PLAN.md`, section 7) has flankr use
//! them:
//!
//! - this device's Noise static keys are [`NoiseStatic`]s in the key store,
//!   one per paired remote: [`KEY_NAME`] for the pairing made before the
//!   remotes list, `noise.static.<id>` for each later one (plan
//!   20261028_01, decision 5). Its private half never leaves the store: each
//!   Diffie-Hellman borrows it for one call through `key_store::with_key`.
//! - the paired remotes are a TOML blob, [`REMOTES_BLOB`], in the secure
//!   store, in `FileKeyStore`'s `remotes.toml` layout; the single
//!   [`SERVER_BLOB`] of before is migrated into it on first read.
//!
//! On Android these are the Keystore-sealed stores of libmobile, and on
//! macOS the login Keychain. Elsewhere libmorph has neither, and the
//! desktop falls back to `FileKeyStore` (see `lib.rs`); Android never does.

use std::sync::Arc;

use curve25519_dalek::montgomery::MontgomeryPoint;
use flankr_companion::keys::{decode_legacy_server, decode_remotes, encode_remotes, migrated};
use flankr_companion::{KeyStore, Remotes, ServerRecord};
use libflankr_proto::link::StaticKey;
use libmorph::key_store::{self, KeyStoreError, SealToken, SecretKey};
use libmorph::{SecureStore, Vault};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// The key store name of this device's static key.
pub const KEY_NAME: &str = "noise.static";
/// The secure store blob that held the paired server before the remotes
/// list; migrated into [`REMOTES_BLOB`] and deleted (plan 20261028_01,
/// decision 13).
pub const SERVER_BLOB: &str = "server";
/// The secure store blob that holds the paired remotes, in
/// `FileKeyStore`'s `remotes.toml` layout.
pub const REMOTES_BLOB: &str = "remotes";

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
    const KIND: &'static str = "flankr.noise.x25519.v1";

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
/// Diffie-Hellman that borrows the private key named `name` from the store
/// for each call.
pub struct VaultStaticKey {
    vault: Vault,
    name: String,
    public: [u8; 32],
}

impl StaticKey for VaultStaticKey {
    fn public(&self) -> [u8; 32] {
        self.public
    }

    fn dh(&self, peer: &[u8; 32]) -> Option<[u8; 32]> {
        match key_store::with_key::<NoiseStatic, _>(&self.vault, &self.name, |key| key.dh(peer)) {
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
    /// The `remotes` blob; on the first call with none, the `server` blob
    /// migrated into it (plan 20261028_01, decision 13): the list is
    /// written first, and the `server` blob deleted only after.
    fn remotes(&mut self) -> Result<Remotes, String> {
        if let Some(bytes) = self.store.get(REMOTES_BLOB).map_err(|e| e.to_string())? {
            let text = std::str::from_utf8(&bytes).map_err(|e| format!("the remotes blob: {e}"))?;
            let remotes = decode_remotes(text)?;
            // A migration that stopped between its two writes.
            self.store
                .delete(SERVER_BLOB)
                .map_err(|e| format!("the server blob: {e}"))?;
            return Ok(remotes);
        }
        let server = match self.store.get(SERVER_BLOB).map_err(|e| e.to_string())? {
            Some(bytes) => Some(decode_server(&bytes)?),
            None => None,
        };
        let had_server = server.is_some();
        let remotes = migrated(server);
        self.set_remotes(&remotes)?;
        if had_server {
            self.store
                .delete(SERVER_BLOB)
                .map_err(|e| format!("the server blob: {e}"))?;
        } else {
            // A key with no server: a pairing never stored.
            self.delete_static_key(KEY_NAME)?;
        }
        Ok(remotes)
    }

    fn set_remotes(&mut self, remotes: &Remotes) -> Result<(), String> {
        let text = encode_remotes(remotes)?;
        self.store
            .put(REMOTES_BLOB, text.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn static_key(&mut self, key_name: &str) -> Result<Option<Arc<dyn StaticKey>>, String> {
        match key_store::public::<NoiseStatic>(&self.vault, key_name) {
            Ok(public) => Ok(Some(Arc::new(VaultStaticKey {
                vault: self.vault.clone(),
                name: key_name.to_string(),
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

    fn generate_static_key(&mut self, key_name: &str) -> Result<Arc<dyn StaticKey>, String> {
        // No flags: reconnecting in the background works while the phone is
        // locked (the key store plan, section 7).
        let public = key_store::generate::<NoiseStatic>(&self.vault, key_name, 0)
            .map_err(|e| e.to_string())?;
        log::info!("keys: made a new static key in the key store");
        Ok(Arc::new(VaultStaticKey {
            vault: self.vault.clone(),
            name: key_name.to_string(),
            public,
        }))
    }

    fn delete_static_key(&mut self, key_name: &str) -> Result<(), String> {
        key_store::delete(&self.vault, key_name)
            .map(|_| ())
            .map_err(|e| format!("the static key: {e}"))
    }
}

/// The `server` blob of before the remotes list: `FileKeyStore`'s
/// `server.toml` layout, the key in base64.
fn decode_server(bytes: &[u8]) -> Result<ServerRecord, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("the server blob: {e}"))?;
    decode_legacy_server(text).map_err(|e| format!("the server blob: {e}"))
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use libflankr_proto::link::generate_keypair;

    #[test]
    fn the_vault_key_agrees_with_snow() {
        let vault = Vault::memory();
        let public = key_store::generate::<NoiseStatic>(&vault, KEY_NAME, 0).unwrap();
        let ours = VaultStaticKey {
            vault,
            name: KEY_NAME.into(),
            public,
        };
        let peer = generate_keypair().unwrap();
        let shared = ours.dh(&peer.public).expect("a shared secret");
        assert_eq!(Some(shared), peer.dh(&ours.public()));
    }

    #[test]
    fn a_low_order_peer_gives_none() {
        let vault = Vault::memory();
        let public = key_store::generate::<NoiseStatic>(&vault, KEY_NAME, 0).unwrap();
        let ours = VaultStaticKey {
            vault,
            name: KEY_NAME.into(),
            public,
        };
        assert_eq!(ours.dh(&[0u8; 32]), None);
    }

    #[test]
    fn the_server_blob_reads_as_the_remote_it_migrates_to() {
        let blob = b"name = \"m4\"\npublic_key = \"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=\"\naddresses = [\"m4.tail.ts.net:7347\"]\nsession_id = \"companion:phone\"\n";
        let record = decode_server(blob).unwrap();
        assert_eq!(record.name, "m4");
        assert_eq!(record.public_key, [7u8; 32]);
        assert_eq!(record.addresses, ["m4.tail.ts.net:7347"]);
        assert_eq!(record.session_id.as_deref(), Some("companion:phone"));
        assert_eq!(record.key_name, KEY_NAME);
        assert_eq!(record.paired_at, None);
        assert!(decode_server(b"name = \"m4\"\npublic_key = \"AAAA\"\n").is_err());
    }

    #[test]
    fn each_remote_has_its_own_vault_key() {
        let vault = Vault::memory();
        let a = key_store::generate::<NoiseStatic>(&vault, KEY_NAME, 0).unwrap();
        let b =
            key_store::generate::<NoiseStatic>(&vault, "noise.static.0011223344556677", 0).unwrap();
        assert_ne!(a, b);
        key_store::delete(&vault, KEY_NAME).unwrap();
        // Deleting again is not an error.
        assert!(!key_store::delete(&vault, KEY_NAME).unwrap());
        assert!(key_store::public::<NoiseStatic>(&vault, "noise.static.0011223344556677").is_ok());
    }
}
