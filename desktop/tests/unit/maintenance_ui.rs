use std::rc::Rc;

use super::*;
use crate::platform::diagnostics::LogLevel;

fn model() -> Maintenance {
    Maintenance::new(Rc::new(I18n::new("en").unwrap()))
}

#[test]
fn refresh_actions_emit_the_expected_commands() {
    let mut model = model();
    let outputs = model.handle(MaintenanceInput::RefreshLogs);
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        &outputs[0],
        MaintenanceOutput::Command(MaintenanceCommand::Logs { .. })
    ));

    let outputs = model.handle(MaintenanceInput::RefreshStats);
    assert!(matches!(
        &outputs[0],
        MaintenanceOutput::Command(MaintenanceCommand::Stats)
    ));
}

#[test]
fn logs_response_populates_and_clear_response_empties() {
    let mut model = model();
    model.apply_response(MaintenanceResponse::Logs(vec![LogEntry {
        timestamp_seconds: 1,
        level: LogLevel::Info,
        message: "media cache resized".into(),
    }]));
    assert_eq!(model.logs.len(), 1);
    model.apply_response(MaintenanceResponse::Cleared);
    assert!(model.logs.is_empty());
}

#[test]
fn crash_text_joins_rendered_entries() {
    let mut model = model();
    model.logs = vec![
        LogEntry {
            timestamp_seconds: 1,
            level: LogLevel::Info,
            message: "first".into(),
        },
        LogEntry {
            timestamp_seconds: 2,
            level: LogLevel::Error,
            message: "second".into(),
        },
    ];
    let text = model.crash_text();
    assert!(text.contains("first"));
    assert!(text.contains("second"));
}

#[test]
fn text_falls_back_to_key_when_translation_is_missing() {
    let model = model();
    assert_eq!(
        model.text("desktop-maintenance-not-a-key"),
        "desktop-maintenance-not-a-key"
    );
}

#[test]
fn import_sets_a_cancellable_token_until_cancel_or_completion() {
    let mut model = model();
    let outputs = model.handle(MaintenanceInput::ImportTdlib {
        source: "/tmp/source".into(),
    });
    assert!(model.import_token.is_some());
    assert!(matches!(
        &outputs[0],
        MaintenanceOutput::Command(MaintenanceCommand::ImportTdlib { .. })
    ));

    let token = model.import_token.as_ref().expect("token").clone();
    assert!(!token.is_cancelled());
    model.handle(MaintenanceInput::CancelImport);
    assert!(token.is_cancelled());

    model.apply_response(MaintenanceResponse::Imported(ImportSummary {
        files: 3,
        bytes: 9,
        destination: "/tmp/destination".into(),
    }));
    assert!(model.import_token.is_none());
    assert_eq!(model.import_summary.as_ref().unwrap().files, 3);
}
