use std::cmp::Ordering;

use gio::prelude::*;
use relm4::gtk::{gio, glib};

const PK_SERVICE: &str = "org.freedesktop.PackageKit";
const PK_PATH: &str = "/org/freedesktop/PackageKit";
const PK_INTERFACE: &str = "org.freedesktop.PackageKit";
const PK_TRANSACTION_INTERFACE: &str = "org.freedesktop.PackageKit.Transaction";
const DBUS_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const CALL_TIMEOUT_MS: i32 = 3000;
const STATUS_FINISHED: u32 = 19;
const STATUS_CANCELLED: u32 = 20;
const INSTALLER_COMMAND: &str = "gnome-software";
const RPM_PACKAGE_NAME: &str = "monogram-desktop";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkgId {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("package manager service is unavailable")]
    ServiceUnavailable,
    #[error("update check timed out")]
    Timeout,
    #[error("package manager returned invalid data")]
    InvalidResponse,
    #[error("no system package installer is available")]
    InstallerUnavailable,
    #[error("failed to open the system installer")]
    InstallerLaunchFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub available_version: Option<String>,
    pub security: bool,
    pub package_count: u32,
}

pub fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Parses a PackageKit package id of the form `name;version;arch;data`.
pub fn parse_package_id(package_id: &str) -> Option<PkgId> {
    let mut parts = package_id.split(';');
    let name = parts.next()?.to_owned();
    let version = parts.next()?.to_owned();
    let arch = parts.next()?.to_owned();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some(PkgId {
        name,
        version,
        arch,
        data: parts.next().unwrap_or_default().to_owned(),
    })
}

/// RPM-style version comparison over a single version string. `~` sorts before
/// every other segment, digit runs are compared numerically, and alphabetic runs
/// compare byte-wise; numeric segments are newer than alphabetic ones.
pub fn rpmvercmp(left: &str, right: &str) -> Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() || right_index < right.len() {
        while left_index < left.len() && is_separator(left[left_index]) {
            left_index += 1;
        }
        while right_index < right.len() && is_separator(right[right_index]) {
            right_index += 1;
        }
        let left_tilde = left_index < left.len() && left[left_index] == b'~';
        let right_tilde = right_index < right.len() && right[right_index] == b'~';
        if left_tilde || right_tilde {
            if left_tilde && right_tilde {
                left_index += 1;
                right_index += 1;
                continue;
            }
            return if left_tilde {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        if left_index >= left.len() || right_index >= right.len() {
            return match (left_index >= left.len(), right_index >= right.len()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => unreachable!(),
            };
        }
        let left_numeric = left[left_index].is_ascii_digit();
        let right_numeric = right[right_index].is_ascii_digit();
        if left_numeric && right_numeric {
            let mut left_end = left_index;
            let mut right_end = right_index;
            while left_end < left.len() && left[left_end].is_ascii_digit() {
                left_end += 1;
            }
            while right_end < right.len() && right[right_end].is_ascii_digit() {
                right_end += 1;
            }
            let mut left_start = left_index;
            let mut right_start = right_index;
            while left_start < left_end && left[left_start] == b'0' {
                left_start += 1;
            }
            while right_start < right_end && right[right_start] == b'0' {
                right_start += 1;
            }
            let left_digits = &left[left_start..left_end];
            let right_digits = &right[right_start..right_end];
            match left_digits.len().cmp(&right_digits.len()) {
                Ordering::Equal => match left_digits.cmp(right_digits) {
                    Ordering::Equal => {}
                    ordering => return ordering,
                },
                ordering => return ordering,
            }
            left_index = left_end;
            right_index = right_end;
        } else if left_numeric {
            return Ordering::Greater;
        } else if right_numeric {
            return Ordering::Less;
        } else {
            let mut left_end = left_index;
            let mut right_end = right_index;
            while left_end < left.len() && left[left_end].is_ascii_alphabetic() {
                left_end += 1;
            }
            while right_end < right.len() && right[right_end].is_ascii_alphabetic() {
                right_end += 1;
            }
            match left[left_index..left_end].cmp(&right[right_index..right_end]) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
            left_index = left_end;
            right_index = right_end;
        }
    }
    Ordering::Equal
}

fn is_separator(byte: u8) -> bool {
    !byte.is_ascii_alphanumeric() && byte != b'~'
}

/// Compares `epoch:version-release` strings. A missing epoch is zero and a
/// missing release is empty, matching the RPM ordering conventions.
pub fn compare_releases(left: &str, right: &str) -> Ordering {
    let (left_epoch, left_rest) = split_epoch(left);
    let (right_epoch, right_rest) = split_epoch(right);
    match left_epoch.cmp(&right_epoch) {
        Ordering::Equal => {}
        ordering => return ordering,
    }
    let (left_version, left_release) = split_release(left_rest);
    let (right_version, right_release) = split_release(right_rest);
    match rpmvercmp(left_version, right_version) {
        Ordering::Equal => rpmvercmp(left_release, right_release),
        ordering => ordering,
    }
}

fn split_epoch(value: &str) -> (u64, &str) {
    match value.split_once(':') {
        Some((epoch, rest)) => (epoch.parse().unwrap_or(0), rest),
        None => (0, value),
    }
}

fn split_release(value: &str) -> (&str, &str) {
    match value.split_once('-') {
        Some((version, release)) => (version, release),
        None => (value, ""),
    }
}

pub fn update_info_from_package_ids(current_version: &str, package_ids: &[String]) -> UpdateInfo {
    let mut available_version: Option<String> = None;
    let mut security = false;
    for package_id in package_ids {
        let Some(parsed) = parse_package_id(package_id) else {
            continue;
        };
        if parsed.name == RPM_PACKAGE_NAME
            && compare_releases(&parsed.version, current_version) == Ordering::Greater
        {
            let newer = available_version.as_ref().is_none_or(|current| {
                compare_releases(&parsed.version, current) == Ordering::Greater
            });
            if newer {
                available_version = Some(parsed.version.clone());
            }
        }
        if parsed.data.to_lowercase().contains("security") {
            security = true;
        }
    }
    UpdateInfo {
        current_version: current_version.to_owned(),
        available_version,
        security,
        package_count: package_ids.len() as u32,
    }
}

/// Blocking PackageKit update check intended for the worker thread. The check is
/// bounded by `CALL_TIMEOUT_MS` and a fixed poll budget; it never installs,
/// removes, or modifies packages.
pub fn check_updates_sync() -> Result<UpdateInfo, UpdateError> {
    let bus = gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>)
        .map_err(|_| UpdateError::ServiceUnavailable)?;
    let package_ids = packagekit_package_ids_sync(&bus)?;
    Ok(update_info_from_package_ids(
        &current_version(),
        &package_ids,
    ))
}

fn packagekit_package_ids_sync(bus: &gio::DBusConnection) -> Result<Vec<String>, UpdateError> {
    let reply = bus
        .call_sync(
            Some(PK_SERVICE),
            PK_PATH,
            PK_INTERFACE,
            "GetUpdates",
            Some(&0_u32.to_variant()),
            Some(glib::VariantTy::new("(o)").map_err(|_| UpdateError::InvalidResponse)?),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&gio::Cancellable>,
        )
        .map_err(|_| UpdateError::ServiceUnavailable)?;
    let (transaction,) = reply
        .get::<(String,)>()
        .ok_or(UpdateError::InvalidResponse)?;

    for _ in 0..10 {
        let status = property_u32(bus, &transaction, "Status")?;
        if status == STATUS_CANCELLED {
            return Err(UpdateError::Timeout);
        }
        if status == STATUS_FINISHED {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let package_ids = property_strings(bus, &transaction, "PackageIDs")?;
    Ok(package_ids)
}

fn property_u32(bus: &gio::DBusConnection, path: &str, name: &str) -> Result<u32, UpdateError> {
    let reply = bus
        .call_sync(
            Some(PK_SERVICE),
            path,
            DBUS_PROPERTIES,
            "Get",
            Some(&(PK_TRANSACTION_INTERFACE, name).to_variant()),
            Some(glib::VariantTy::new("(v)").map_err(|_| UpdateError::InvalidResponse)?),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&gio::Cancellable>,
        )
        .map_err(|_| UpdateError::ServiceUnavailable)?;
    let (value,) = reply
        .get::<(glib::Variant,)>()
        .ok_or(UpdateError::InvalidResponse)?;
    value.get().ok_or(UpdateError::InvalidResponse)
}

fn property_strings(
    bus: &gio::DBusConnection,
    path: &str,
    name: &str,
) -> Result<Vec<String>, UpdateError> {
    let reply = bus
        .call_sync(
            Some(PK_SERVICE),
            path,
            DBUS_PROPERTIES,
            "Get",
            Some(&(PK_TRANSACTION_INTERFACE, name).to_variant()),
            Some(glib::VariantTy::new("(v)").map_err(|_| UpdateError::InvalidResponse)?),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&gio::Cancellable>,
        )
        .map_err(|_| UpdateError::ServiceUnavailable)?;
    let (value,) = reply
        .get::<(glib::Variant,)>()
        .ok_or(UpdateError::InvalidResponse)?;
    value.get().ok_or(UpdateError::InvalidResponse)
}

/// Opens the system package installer. This launches a desktop application and
/// never writes to the application's own install location.
pub fn open_installer() -> Result<(), UpdateError> {
    let application = gio::AppInfo::create_from_commandline(
        INSTALLER_COMMAND,
        None,
        gio::AppInfoCreateFlags::NONE,
    )
    .map_err(|_| UpdateError::InstallerUnavailable)?;
    application
        .launch(&[], None::<&gio::AppLaunchContext>)
        .map_err(|_| UpdateError::InstallerLaunchFailed)
}

#[cfg(test)]
#[path = "../../tests/unit/update.rs"]
mod tests;
