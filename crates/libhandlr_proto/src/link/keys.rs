//! Static keys and pairing secrets, from snow's default resolver.

use std::fmt;

use snow::params::DHChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};

use super::{LinkError, KEY_LEN, RECONNECT_PATTERN};

/// An X25519 static key pair. `Debug` does not print the private key.
#[derive(Clone, PartialEq, Eq)]
pub struct StaticKeypair {
    pub private: [u8; KEY_LEN],
    pub public: [u8; KEY_LEN],
}

impl StaticKeypair {
    /// The key pair for a stored private key.
    pub fn from_private(private: [u8; KEY_LEN]) -> Result<Self, LinkError> {
        Ok(StaticKeypair {
            public: public_key(&private)?,
            private,
        })
    }
}

impl fmt::Debug for StaticKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticKeypair")
            .field("private", &"<redacted>")
            .field("public", &self.public)
            .finish()
    }
}

/// A new random X25519 static key pair.
pub fn generate_keypair() -> Result<StaticKeypair, LinkError> {
    let params = RECONNECT_PATTERN.parse()?;
    let kp = snow::Builder::new(params).generate_keypair()?;
    Ok(StaticKeypair {
        private: to_key(&kp.private)?,
        public: to_key(&kp.public)?,
    })
}

/// The X25519 public key of a private key.
pub fn public_key(private: &[u8; KEY_LEN]) -> Result<[u8; KEY_LEN], LinkError> {
    let mut dh = DefaultResolver
        .resolve_dh(&DHChoice::Curve25519)
        .ok_or(snow::Error::Init(snow::error::InitStage::GetDhImpl))?;
    dh.set(private);
    to_key(dh.pubkey())
}

/// A new random 32-byte pairing secret.
pub fn generate_secret() -> Result<[u8; KEY_LEN], LinkError> {
    let mut rng = DefaultResolver
        .resolve_rng()
        .ok_or(snow::Error::Init(snow::error::InitStage::GetRngImpl))?;
    let mut secret = [0u8; KEY_LEN];
    rng.try_fill_bytes(&mut secret)?;
    Ok(secret)
}

pub(super) fn to_key(bytes: &[u8]) -> Result<[u8; KEY_LEN], LinkError> {
    bytes
        .try_into()
        .map_err(|_| LinkError::from(snow::Error::Input))
}
