use std::path::PathBuf;

use super::*;
use crate::platform::{diagnostics, tdlib, update};

#[derive(Debug, Clone)]
pub enum MaintenanceCommand {
    Logs {
        level: Option<diagnostics::LogLevel>,
        query: String,
    },
    Stats,
    ClearLogs,
    ExportLogs,
    CrashReport,
    CheckUpdates,
    OpenInstaller,
    ImportTdlib {
        source: PathBuf,
        token: tdlib::ImportToken,
    },
}

#[derive(Debug, Clone)]
pub enum MaintenanceResponse {
    Logs(Vec<diagnostics::LogEntry>),
    Stats(diagnostics::DebugStats),
    Cleared,
    Exported(PathBuf),
    CrashReport(String),
    Update(update::UpdateInfo),
    Installed,
    Imported(tdlib::ImportSummary),
}

impl Runtime {
    pub(super) fn execute_maintenance(
        &mut self,
        command: MaintenanceCommand,
    ) -> Result<MaintenanceResponse, ClientError> {
        match command {
            MaintenanceCommand::Logs { level, query } => {
                let logs = diagnostics::LogStore::global().search(&query, level);
                Ok(MaintenanceResponse::Logs(logs))
            }
            MaintenanceCommand::Stats => Ok(MaintenanceResponse::Stats(
                diagnostics::collect_debug_stats(),
            )),
            MaintenanceCommand::ClearLogs => {
                diagnostics::LogStore::global().clear();
                Ok(MaintenanceResponse::Cleared)
            }
            MaintenanceCommand::ExportLogs => {
                let directory = crate::platform::AppPaths::discover()
                    .map_err(|_| ClientError::Storage)?
                    .data
                    .join("logs");
                let path =
                    diagnostics::export_logs(&directory).map_err(|_| ClientError::Storage)?;
                diagnostics::LogStore::global().record(
                    diagnostics::LogLevel::Info,
                    &format!("logs exported to {}", path.display()),
                );
                Ok(MaintenanceResponse::Exported(path))
            }
            MaintenanceCommand::CrashReport => Ok(MaintenanceResponse::CrashReport(
                diagnostics::LogStore::global().export(),
            )),
            MaintenanceCommand::CheckUpdates => {
                let info = update::check_updates_sync().map_err(|_| ClientError::Network)?;
                diagnostics::LogStore::global()
                    .record(diagnostics::LogLevel::Info, "update check finished");
                Ok(MaintenanceResponse::Update(info))
            }
            MaintenanceCommand::OpenInstaller => {
                update::open_installer().map_err(|_| ClientError::Network)?;
                Ok(MaintenanceResponse::Installed)
            }
            MaintenanceCommand::ImportTdlib { source, token } => {
                let key = crate::platform::session_key().map_err(|_| ClientError::SessionKey)?;
                let destination_available = match std::fs::symlink_metadata(&self.session_path) {
                    Ok(metadata) => metadata.is_file() && metadata.len() == 0,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                    Err(_) => false,
                };
                if destination_available {
                    self.client = None;
                    self.phone = None;
                }
                let summary = tdlib::import(&source, &self.session_path, &key, &token)
                    .map_err(import_error)?;
                diagnostics::LogStore::global()
                    .record(diagnostics::LogLevel::Info, "imported TDLib session");
                Ok(MaintenanceResponse::Imported(summary))
            }
        }
    }
}

fn import_error(error: tdlib::ImportError) -> ClientError {
    match error {
        tdlib::ImportError::Cancelled => ClientError::Cancelled,
        tdlib::ImportError::TooLarge(_)
        | tdlib::ImportError::FileTooLarge(_)
        | tdlib::ImportError::TooManyEntries
        | tdlib::ImportError::TooDeep => ClientError::UnsupportedContent,
        tdlib::ImportError::InvalidSource
        | tdlib::ImportError::DestinationExists
        | tdlib::ImportError::AuthKeyNotFound => ClientError::InvalidRequest,
        tdlib::ImportError::InvalidKey => ClientError::SessionKey,
        tdlib::ImportError::Encryption
        | tdlib::ImportError::VerificationFailed
        | tdlib::ImportError::Io(_) => ClientError::Storage,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/maintenance.rs"]
mod tests;
