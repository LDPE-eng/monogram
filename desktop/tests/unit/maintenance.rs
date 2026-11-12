use super::*;

#[test]
fn import_errors_map_to_stable_client_errors() {
    assert_eq!(
        import_error(tdlib::ImportError::Cancelled),
        ClientError::Cancelled
    );
    assert_eq!(
        import_error(tdlib::ImportError::TooLarge(1)),
        ClientError::UnsupportedContent
    );
    assert_eq!(
        import_error(tdlib::ImportError::FileTooLarge(1)),
        ClientError::UnsupportedContent
    );
    assert_eq!(
        import_error(tdlib::ImportError::TooManyEntries),
        ClientError::UnsupportedContent
    );
    assert_eq!(
        import_error(tdlib::ImportError::AuthKeyNotFound),
        ClientError::InvalidRequest
    );
    assert_eq!(
        import_error(tdlib::ImportError::InvalidSource),
        ClientError::InvalidRequest
    );
    assert_eq!(
        import_error(tdlib::ImportError::InvalidKey),
        ClientError::SessionKey
    );
    assert_eq!(
        import_error(tdlib::ImportError::VerificationFailed),
        ClientError::Storage
    );
    assert_eq!(
        import_error(tdlib::ImportError::Io(std::io::Error::from(
            std::io::ErrorKind::Other
        ))),
        ClientError::Storage
    );
}

#[test]
fn maintenance_commands_are_cloneable_for_the_worker_channel() {
    let command = MaintenanceCommand::Logs {
        level: Some(diagnostics::LogLevel::Warn),
        query: "media".to_owned(),
    };
    let cloned = command.clone();
    match cloned {
        MaintenanceCommand::Logs { level, query } => {
            assert_eq!(level, Some(diagnostics::LogLevel::Warn));
            assert_eq!(query, "media");
        }
        _ => panic!("expected logs command"),
    }
}
