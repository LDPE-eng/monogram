use std::time::{Duration, Instant};

use relm4::gtk::glib;

use super::*;

fn fix() -> Location {
    Location {
        latitude: 60.0,
        longitude: 30.0,
        accuracy: Some(12.0),
    }
}

#[test]
fn acquisition_accepts_a_fix_and_waits_one_sampling_interval() {
    let mut schedule = LocationSchedule::new();
    let now = Instant::now();
    schedule.start(now);
    assert_eq!(schedule.phase(), Phase::Acquiring);
    assert_eq!(schedule.deadline(), Some(now + ACQUISITION_TIMEOUT));

    let accepted = schedule.advance(now + Duration::from_secs(2), ScheduleInput::Fix(fix()));
    assert_eq!(accepted, Some(fix()));
    assert_eq!(schedule.phase(), Phase::Waiting);
    assert_eq!(
        schedule.deadline(),
        Some(now + Duration::from_secs(2) + SAMPLING_INTERVAL)
    );

    // No new sample is started until the sampling interval elapses.
    let now = now + Duration::from_secs(2) + SAMPLING_INTERVAL;
    assert_eq!(schedule.advance(now, ScheduleInput::Deadline), None);
    assert_eq!(schedule.phase(), Phase::Acquiring);
}

#[test]
fn acquisition_timeout_retries_without_a_fix() {
    let mut schedule = LocationSchedule::new();
    let now = Instant::now();
    schedule.start(now);
    assert!(schedule.acquire_deadline_passed(now + ACQUISITION_TIMEOUT));
    assert!(!schedule.acquire_deadline_passed(now + ACQUISITION_TIMEOUT - Duration::from_secs(1)));

    let accepted = schedule.advance(now + ACQUISITION_TIMEOUT, ScheduleInput::Deadline);
    assert_eq!(accepted, None);
    assert_eq!(schedule.phase(), Phase::Waiting);

    // The next acquisition begins once the sampling interval is reached.
    let next = now + SAMPLING_INTERVAL;
    schedule.advance(next, ScheduleInput::Deadline);
    assert_eq!(schedule.phase(), Phase::Acquiring);
    assert_eq!(schedule.deadline(), Some(next + ACQUISITION_TIMEOUT));
}

#[test]
fn pause_resume_and_stop_gate_the_state_machine() {
    let mut schedule = LocationSchedule::new();
    let now = Instant::now();
    schedule.start(now);

    schedule.pause();
    assert!(schedule.is_paused());
    assert_eq!(schedule.deadline(), None);
    assert_eq!(
        schedule.advance(now + Duration::from_secs(1), ScheduleInput::Fix(fix())),
        None
    );

    let resume_at = now + Duration::from_secs(20);
    schedule.resume(resume_at);
    assert!(!schedule.is_paused());
    assert_eq!(schedule.phase(), Phase::Acquiring);
    assert_eq!(schedule.deadline(), Some(resume_at + ACQUISITION_TIMEOUT));

    schedule.stop();
    assert!(schedule.is_stopped());
    assert_eq!(schedule.deadline(), None);
    assert_eq!(
        schedule.advance(
            resume_at + Duration::from_secs(1),
            ScheduleInput::Fix(fix())
        ),
        None
    );
    assert_eq!(schedule.phase(), Phase::Idle);
}

#[test]
fn failed_acquisition_moves_to_waiting_without_emitting() {
    let mut schedule = LocationSchedule::new();
    let now = Instant::now();
    schedule.start(now);
    assert_eq!(
        schedule.advance(now + Duration::from_secs(3), ScheduleInput::Failed),
        None
    );
    assert_eq!(schedule.phase(), Phase::Waiting);
}

#[test]
fn duplicate_fixes_do_not_end_the_acquisition() {
    let mut schedule = LocationSchedule::new();
    let now = Instant::now();
    schedule.start(now);
    assert_eq!(
        schedule.advance(now + Duration::from_secs(1), ScheduleInput::Fix(fix())),
        Some(fix())
    );
    // Same fix arrives again during a later acquisition: it must be ignored.
    schedule.start(now + Duration::from_secs(20));
    assert_eq!(
        schedule.advance(now + Duration::from_secs(21), ScheduleInput::Fix(fix())),
        None
    );
    assert_eq!(schedule.phase(), Phase::Acquiring);
}

#[test]
fn dbus_errors_map_denials_to_permission_error() {
    let denied = glib::Error::with_domain(
        glib::Quark::from("org.freedesktop.GeoClue2.Error.NotAuthorized"),
        1,
        "Not authorized",
    );
    assert_eq!(classify_dbus_error(denied), LocationError::PermissionDenied);

    let access_denied = glib::Error::with_domain(
        glib::Quark::from("org.freedesktop.DBus.Error.AccessDenied"),
        2,
        "Access denied",
    );
    assert_eq!(
        classify_dbus_error(access_denied),
        LocationError::PermissionDenied
    );

    let other = glib::Error::with_domain(
        glib::Quark::from("org.freedesktop.DBus.Error.UnknownMethod"),
        3,
        "Unknown method",
    );
    assert_eq!(
        classify_dbus_error(other),
        LocationError::ServiceUnavailable
    );
}
