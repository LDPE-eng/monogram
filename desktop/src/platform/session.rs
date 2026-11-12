use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};
use rand::{TryRng, rngs::SysRng};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{PlatformError, runtime_directory};
use crate::domain::{ApiConfig, ClientError};

const SERVICE: &str = "io.github.monogram.desktop";
const USER: &str = "session-encryption-key-v1";

pub trait SessionKeyBackend {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, PlatformError>;
    fn write(&self, encoded_key: &[u8]) -> Result<(), PlatformError>;
    fn delete(&self) -> Result<(), PlatformError> {
        Ok(())
    }
}

struct SecretServiceBackend(keyring::Entry);

impl SessionKeyBackend for SecretServiceBackend {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, PlatformError> {
        match self.0.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn write(&self, encoded_key: &[u8]) -> Result<(), PlatformError> {
        let password =
            std::str::from_utf8(encoded_key).map_err(|_| PlatformError::InvalidSessionKey)?;
        self.0.set_password(password).map_err(keyring_error)
    }

    fn delete(&self) -> Result<(), PlatformError> {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}

pub(super) fn keyring_error(error: keyring::Error) -> PlatformError {
    match error {
        keyring::Error::NoStorageAccess(_) => PlatformError::SecretServiceLocked,
        keyring::Error::Ambiguous(_) => PlatformError::AmbiguousSessionKey,
        keyring::Error::BadEncoding(mut bytes) | keyring::Error::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            PlatformError::InvalidSessionKey
        }
        _ => PlatformError::SecretServiceUnavailable,
    }
}

/// Blocking: call on a worker and wrap the returned array in `Zeroizing` immediately.
pub fn session_key() -> Result<[u8; 32], PlatformError> {
    let lock = runtime_directory()?.join("session-key.lock");
    let backend = SecretServiceBackend(keyring::Entry::new(SERVICE, USER).map_err(keyring_error)?);
    session_key_with_backend(&backend, &lock)
}

/// All processes using this backend must share the same private lock path.
pub fn session_key_with_backend(
    backend: &impl SessionKeyBackend,
    lock_path: &Path,
) -> Result<[u8; 32], PlatformError> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    lock.lock()?;
    if let Some(encoded) = backend.read()? {
        return decode_key(&encoded).map(|key| *key);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    SysRng
        .try_fill_bytes(key.as_mut())
        .map_err(|_| PlatformError::RandomUnavailable)?;
    let encoded = encode_key(&key);
    backend.write(&encoded)?;
    let saved = backend
        .read()?
        .ok_or(PlatformError::SessionKeyVerificationFailed)?;
    let saved = decode_key(&saved)?;
    if *saved != *key {
        return Err(PlatformError::SessionKeyVerificationFailed);
    }
    Ok(*key)
}

/// Delete the stored session encryption key through the supplied backend.
pub fn delete_session_key_with_backend(
    backend: &impl SessionKeyBackend,
) -> Result<(), PlatformError> {
    backend.delete()
}

/// Blocking: delete API credentials and the session encryption key after logout.
pub fn clear_session_secret() -> Result<(), ClientError> {
    delete_api_credentials().map_err(|_| ClientError::SessionKey)?;
    let backend = SecretServiceBackend(
        keyring::Entry::new(SERVICE, USER)
            .map_err(keyring_error)
            .map_err(|_| ClientError::SessionKey)?,
    );
    delete_session_key_with_backend(&backend).map_err(|_| ClientError::SessionKey)
}

/// Blocking: load stored API credentials for startup configuration.
pub fn load_api_credentials() -> Result<ApiConfig, ClientError> {
    match api_credentials().map_err(|_| ClientError::SessionKey)? {
        Some(credentials) => ApiConfig::new(credentials.api_id, credentials.api_hash.to_string()),
        None => Err(ClientError::NotConfigured),
    }
}

fn encode_key(key: &[u8; 32]) -> Zeroizing<Vec<u8>> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = Zeroizing::new(Vec::with_capacity(64));
    for byte in key {
        encoded.push(HEX[usize::from(byte >> 4)]);
        encoded.push(HEX[usize::from(byte & 15)]);
    }
    encoded
}

fn decode_key(encoded: &[u8]) -> Result<Zeroizing<[u8; 32]>, PlatformError> {
    if encoded.len() != 64 {
        return Err(PlatformError::InvalidSessionKey);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (byte, pair) in key.iter_mut().zip(encoded.as_chunks::<2>().0) {
        let digit = |value: u8| match value {
            b'0'..=b'9' => Ok(value - b'0'),
            b'a'..=b'f' => Ok(value - b'a' + 10),
            b'A'..=b'F' => Ok(value - b'A' + 10),
            _ => Err(PlatformError::InvalidSessionKey),
        };
        *byte = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Ok(key)
}

/// A Secret Service-backed store shared by API and proxy credential helpers.
///
/// Each `name` maps to a dedicated keyring entry under [`SERVICE`]. Reads,
/// writes, and deletes are blocking; callers should use a worker and never
/// fall back to plaintext storage when this returns an error.
pub trait SecretStore {
    fn read(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, PlatformError>;
    fn write(&self, name: &str, secret: &[u8]) -> Result<(), PlatformError>;
    fn delete(&self, name: &str) -> Result<(), PlatformError>;
}

pub struct KeyringSecretStore;

impl KeyringSecretStore {
    fn entry(name: &str) -> Result<keyring::Entry, PlatformError> {
        keyring::Entry::new(SERVICE, &secret_user(name)).map_err(keyring_error)
    }
}

fn secret_user(name: &str) -> String {
    match name {
        "api-id" => "api-id-v1".to_owned(),
        "api-hash" => "api-hash-v1".to_owned(),
        "proxy" => "proxy-credentials-v1".to_owned(),
        _ => name.to_owned(),
    }
}

impl SecretStore for KeyringSecretStore {
    fn read(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, PlatformError> {
        match Self::entry(name)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn write(&self, name: &str, secret: &[u8]) -> Result<(), PlatformError> {
        Self::entry(name)?.set_secret(secret).map_err(keyring_error)
    }

    fn delete(&self, name: &str) -> Result<(), PlatformError> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ApiCredentials {
    pub api_id: i32,
    pub api_hash: Zeroizing<String>,
}

impl std::fmt::Debug for ApiCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiCredentials")
            .field("api_id", &self.api_id)
            .finish_non_exhaustive()
    }
}

/// Blocking: call on a worker and wrap the hash in `Zeroizing` immediately.
pub fn api_credentials() -> Result<Option<ApiCredentials>, PlatformError> {
    api_credentials_with(&KeyringSecretStore)
}

pub fn api_credentials_with(
    store: &impl SecretStore,
) -> Result<Option<ApiCredentials>, PlatformError> {
    let Some(id) = store.read("api-id")? else {
        return Ok(None);
    };
    let Some(hash) = store.read("api-hash")? else {
        return Ok(None);
    };
    let id = decode_api_id(id.as_slice())?;
    let hash = decode_api_hash(hash.as_slice())?;
    Ok(Some(ApiCredentials {
        api_id: id,
        api_hash: Zeroizing::new(hash),
    }))
}

/// Blocking: call on a worker; validate the hash before touching the keyring.
pub fn store_api_credentials(api_id: i32, api_hash: &str) -> Result<(), PlatformError> {
    store_api_credentials_with(&KeyringSecretStore, api_id, api_hash)
}

pub fn store_api_credentials_with(
    store: &impl SecretStore,
    api_id: i32,
    api_hash: &str,
) -> Result<(), PlatformError> {
    if api_id <= 0 {
        return Err(PlatformError::InvalidApiCredentials);
    }
    let hash = Zeroizing::new(api_hash.as_bytes().to_vec());
    validate_api_hash(hash.as_slice())?;
    store.write("api-id", api_id.to_string().as_bytes())?;
    store.write("api-hash", hash.as_slice())
}

/// Blocking: delete both API credential entries; missing entries are ignored.
pub fn delete_api_credentials() -> Result<(), PlatformError> {
    delete_api_credentials_with(&KeyringSecretStore)
}

pub fn delete_api_credentials_with(store: &impl SecretStore) -> Result<(), PlatformError> {
    store.delete("api-id")?;
    store.delete("api-hash")
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProxyCredentials {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Zeroizing<String>,
}

impl ProxyCredentials {
    pub fn new(host: String, port: u16, username: String, password: String) -> Self {
        Self {
            host,
            port,
            username,
            password: Zeroizing::new(password),
        }
    }

    fn validate(&self) -> Result<(), PlatformError> {
        if self.host.trim().is_empty() || self.port == 0 {
            return Err(PlatformError::InvalidProxyCredentials);
        }
        Ok(())
    }
}

impl std::fmt::Debug for ProxyCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyCredentials")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
struct ProxyWire {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl From<&ProxyCredentials> for ProxyWire {
    fn from(value: &ProxyCredentials) -> Self {
        Self {
            host: value.host.clone(),
            port: value.port,
            username: value.username.clone(),
            password: value.password.to_string(),
        }
    }
}

impl TryFrom<ProxyWire> for ProxyCredentials {
    type Error = PlatformError;

    fn try_from(value: ProxyWire) -> Result<Self, Self::Error> {
        let credentials = Self::new(value.host, value.port, value.username, value.password);
        credentials.validate()?;
        Ok(credentials)
    }
}

/// Blocking: call on a worker; Secret Service stores the base64-encoded blob.
pub fn store_proxy_credentials(credentials: &ProxyCredentials) -> Result<(), PlatformError> {
    store_proxy_credentials_with(&KeyringSecretStore, credentials)
}

pub fn store_proxy_credentials_with(
    store: &impl SecretStore,
    credentials: &ProxyCredentials,
) -> Result<(), PlatformError> {
    credentials.validate()?;
    let wire = ProxyWire::from(credentials);
    let encoded = STANDARD
        .encode(serde_json::to_vec(&wire).map_err(|_| PlatformError::InvalidProxyCredentials)?);
    store.write("proxy", encoded.as_bytes())
}

/// Blocking: call on a worker; returns `None` when no proxy is configured.
pub fn proxy_credentials() -> Result<Option<ProxyCredentials>, PlatformError> {
    proxy_credentials_with(&KeyringSecretStore)
}

pub fn proxy_credentials_with(
    store: &impl SecretStore,
) -> Result<Option<ProxyCredentials>, PlatformError> {
    let Some(encoded) = store.read("proxy")? else {
        return Ok(None);
    };
    let encoded = std::str::from_utf8(encoded.as_slice())
        .map_err(|_| PlatformError::InvalidProxyCredentials)?;
    let json = STANDARD
        .decode(encoded)
        .map_err(|_| PlatformError::InvalidProxyCredentials)?;
    let wire: ProxyWire =
        serde_json::from_slice(&json).map_err(|_| PlatformError::InvalidProxyCredentials)?;
    ProxyCredentials::try_from(wire).map(Some)
}

/// Blocking: delete the stored proxy credentials; missing entries are ignored.
pub fn delete_proxy_credentials() -> Result<(), PlatformError> {
    delete_proxy_credentials_with(&KeyringSecretStore)
}

pub fn delete_proxy_credentials_with(store: &impl SecretStore) -> Result<(), PlatformError> {
    store.delete("proxy")
}

fn validate_api_hash(hash: &[u8]) -> Result<(), PlatformError> {
    if hash.len() != 32 || !hash.iter().all(u8::is_ascii_hexdigit) {
        return Err(PlatformError::InvalidApiCredentials);
    }
    Ok(())
}

fn decode_api_id(encoded: &[u8]) -> Result<i32, PlatformError> {
    let text = std::str::from_utf8(encoded).map_err(|_| PlatformError::InvalidApiCredentials)?;
    let id = text
        .parse::<i32>()
        .map_err(|_| PlatformError::InvalidApiCredentials)?;
    if id <= 0 {
        return Err(PlatformError::InvalidApiCredentials);
    }
    Ok(id)
}

fn decode_api_hash(encoded: &[u8]) -> Result<String, PlatformError> {
    let hash = std::str::from_utf8(encoded).map_err(|_| PlatformError::InvalidApiCredentials)?;
    if !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlatformError::InvalidApiCredentials);
    }
    Ok(hash.to_owned())
}
