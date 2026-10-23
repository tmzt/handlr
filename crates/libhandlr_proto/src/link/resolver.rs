//! A snow resolver whose static key is a [`StaticKey`], so the private key
//! can stay in a key store (an Android Keystore, a Keychain) and snow never
//! holds it. Everything else (ephemeral keys, the cipher, the hash, the
//! random source) is snow's default.
//!
//! snow asks the resolver for two DH objects per handshake, one for the
//! static key and one for the ephemeral, and sets the static one with the
//! bytes given to `Builder::local_private_key`. Each [`KeyDh`] therefore
//! starts as snow's own X25519 and switches to the [`StaticKey`] when it is
//! set with [`STATIC_MARKER`] instead of a key.

use std::sync::Arc;

use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use snow::types::{Cipher, Dh, Hash, Random};

use super::{KEY_LEN, StaticKey};

/// What the handshakes pass to `local_private_key`: not a key, so a
/// [`KeyDh`] set with it uses the [`StaticKey`].
pub(super) const STATIC_MARKER: &[u8] = b"handlr.link/static-key";

pub(super) struct KeyResolver {
    key: Arc<dyn StaticKey>,
}

impl KeyResolver {
    pub(super) fn new<K: StaticKey + Clone>(key: &K) -> Self {
        KeyResolver {
            key: Arc::new(key.clone()),
        }
    }
}

impl CryptoResolver for KeyResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        DefaultResolver.resolve_rng()
    }

    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        Some(Box::new(KeyDh {
            inner: DefaultResolver.resolve_dh(choice)?,
            key: self.key.clone(),
            public: None,
        }))
    }

    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        DefaultResolver.resolve_hash(choice)
    }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        DefaultResolver.resolve_cipher(choice)
    }
}

/// snow's X25519, or the [`StaticKey`] once set with [`STATIC_MARKER`].
struct KeyDh {
    inner: Box<dyn Dh>,
    key: Arc<dyn StaticKey>,
    /// The static key's public key, when this is the static key.
    public: Option<[u8; KEY_LEN]>,
}

impl Dh for KeyDh {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn pub_len(&self) -> usize {
        self.inner.pub_len()
    }

    fn priv_len(&self) -> usize {
        self.inner.priv_len()
    }

    fn set(&mut self, privkey: &[u8]) {
        if privkey == STATIC_MARKER {
            self.public = Some(self.key.public());
        } else {
            self.public = None;
            self.inner.set(privkey);
        }
    }

    fn generate(&mut self, rng: &mut dyn Random) -> Result<(), snow::Error> {
        self.public = None;
        self.inner.generate(rng)
    }

    fn pubkey(&self) -> &[u8] {
        match &self.public {
            Some(public) => public,
            None => self.inner.pubkey(),
        }
    }

    /// Empty for the static key, which this never holds. The handshakes
    /// never ask for it.
    fn privkey(&self) -> &[u8] {
        match self.public {
            Some(_) => &[],
            None => self.inner.privkey(),
        }
    }

    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        if self.public.is_none() {
            return self.inner.dh(pubkey, out);
        }
        // snow passes its whole key buffer, longer than the key.
        let peer: &[u8; KEY_LEN] = pubkey
            .get(..KEY_LEN)
            .and_then(|key| key.try_into().ok())
            .ok_or(snow::Error::Dh)?;
        let shared = self.key.dh(peer).ok_or(snow::Error::Dh)?;
        out.get_mut(..KEY_LEN)
            .ok_or(snow::Error::Dh)?
            .copy_from_slice(&shared);
        Ok(())
    }
}
