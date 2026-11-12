use std::{
    fs::{self, DirBuilder, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
};

mod deep_links;
pub mod diagnostics;
mod location;
mod notifications;
pub(crate) mod passkeys;
mod session;
pub mod tdlib;
pub mod update;

pub use deep_links::{DeepLink, LaunchAllowlist, external_action_allowed, parse};
pub use location::{
    ACQUISITION_TIMEOUT, GeoClueSampler, Location, LocationError, SAMPLING_INTERVAL,
};
pub use notifications::{
    NotificationAction, NotificationAdapter, NotificationCapabilities, NotificationContent,
    NotificationLabels, NotificationTarget,
};
pub use session::{
    ApiCredentials, KeyringSecretStore, ProxyCredentials, SecretStore, SessionKeyBackend,
    api_credentials, api_credentials_with, clear_session_secret, delete_api_credentials,
    delete_api_credentials_with, delete_proxy_credentials, delete_proxy_credentials_with,
    delete_session_key_with_backend, load_api_credentials, proxy_credentials,
    proxy_credentials_with, session_key, session_key_with_backend, store_api_credentials,
    store_api_credentials_with, store_proxy_credentials, store_proxy_credentials_with,
};

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("platform filesystem or locking operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("Secret Service is unavailable; start a compatible KWallet or GNOME Keyring service")]
    SecretServiceUnavailable,
    #[error("Secret Service is locked or access was denied; unlock the wallet and retry")]
    SecretServiceLocked,
    #[error("multiple session keys found in Secret Service; resolve the duplicate entries")]
    AmbiguousSessionKey,
    #[error("stored session key is invalid; it was not replaced")]
    InvalidSessionKey,
    #[error("session key could not be verified after saving to Secret Service")]
    SessionKeyVerificationFailed,
    #[error("the operating system random source is unavailable")]
    RandomUnavailable,
    #[error("the notification server is unavailable or returned invalid capabilities")]
    NotificationServerUnavailable,
    #[error("notification delivery requires a registered primary GApplication")]
    ApplicationNotRegistered,
    #[error("notification content or action data is invalid")]
    InvalidNotificationContent,
    #[error("stored API credentials are invalid; they were not replaced")]
    InvalidApiCredentials,
    #[error("stored proxy credentials are invalid; they were not replaced")]
    InvalidProxyCredentials,
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub database: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub runtime: PathBuf,
}

impl AppPaths {
    pub fn discover() -> io::Result<Self> {
        Self::from_xdg(&xdg::BaseDirectories::with_prefix("monogram"))
    }

    fn from_xdg(dirs: &xdg::BaseDirectories) -> io::Result<Self> {
        let required = |path: Option<PathBuf>| {
            private_directory(path.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "XDG home directory is unavailable")
            })?)
        };
        let config = required(dirs.get_config_home())?;
        let data = required(dirs.get_data_home())?;
        let cache = required(dirs.get_cache_home())?;
        let state = required(dirs.get_state_home())?;
        let runtime = private_directory(dirs.get_runtime_file("")?)?;
        Ok(Self {
            database: data.join("monogram.sqlite3"),
            config,
            data,
            cache,
            state,
            runtime,
        })
    }
}

fn runtime_directory() -> io::Result<PathBuf> {
    private_directory(xdg::BaseDirectories::with_prefix("monogram").get_runtime_file("")?)
}

fn private_directory(path: PathBuf) -> io::Result<PathBuf> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "application directory must be a real private directory",
        ));
    }
    fs::set_permissions(&path, Permissions::from_mode(0o700))?;
    Ok(path)
}

#[cfg(test)]
#[path = "../../tests/unit/platform.rs"]
mod tests;
