use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gio::prelude::*;
use relm4::gtk::{gio, glib};

const GEOCLUE_SERVICE: &str = "org.freedesktop.GeoClue2";
const GEOCLUE_MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const GEOCLUE_MANAGER_INTERFACE: &str = "org.freedesktop.GeoClue2.Manager";
const GEOCLUE_CLIENT_INTERFACE: &str = "org.freedesktop.GeoClue2.Client";
const GEOCLUE_LOCATION_INTERFACE: &str = "org.freedesktop.GeoClue2.Location";
const GEOCLUE_LOCATION_PROPERTY: &str = "org.freedesktop.GeoClue2.Client.Location";
const DBUS_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const CALL_TIMEOUT_MS: i32 = 9_000;
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Foreground-only sampling cadence. A location acquisition is attempted every
/// `SAMPLING_INTERVAL`; each attempt is abandoned after `ACQUISITION_TIMEOUT`.
pub const SAMPLING_INTERVAL: Duration = Duration::from_secs(15);
pub const ACQUISITION_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LocationError {
    #[error("GeoClue is unavailable or returned no usable location")]
    ServiceUnavailable,
    #[error("location access was denied; enable location permission in system settings")]
    PermissionDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Acquiring,
    Waiting,
}

#[derive(Debug)]
struct LocationSchedule {
    phase: Phase,
    paused: bool,
    stopped: bool,
    acquire_started: Option<Instant>,
    next_sample: Option<Instant>,
    seen: Option<Location>,
}

impl LocationSchedule {
    fn new() -> Self {
        Self {
            phase: Phase::Idle,
            paused: false,
            stopped: false,
            acquire_started: None,
            next_sample: None,
            seen: None,
        }
    }

    fn start(&mut self, now: Instant) {
        if self.stopped {
            return;
        }
        self.phase = Phase::Acquiring;
        self.acquire_started = Some(now);
        self.next_sample = Some(now + SAMPLING_INTERVAL);
    }

    fn pause(&mut self) {
        self.paused = true;
    }

    fn resume(&mut self, now: Instant) {
        if self.stopped || !self.paused {
            return;
        }
        self.paused = false;
        self.phase = Phase::Acquiring;
        self.acquire_started = Some(now);
        self.next_sample = Some(now + SAMPLING_INTERVAL);
    }

    fn stop(&mut self) {
        self.stopped = true;
        self.phase = Phase::Idle;
    }

    fn phase(&self) -> Phase {
        self.phase
    }

    fn is_stopped(&self) -> bool {
        self.stopped
    }

    fn is_paused(&self) -> bool {
        self.paused
    }

    #[allow(dead_code)]
    fn deadline(&self) -> Option<Instant> {
        if self.paused || self.stopped {
            return None;
        }
        match self.phase {
            Phase::Idle => None,
            Phase::Acquiring => Some(self.acquire_started? + ACQUISITION_TIMEOUT),
            Phase::Waiting => self.next_sample,
        }
    }

    fn acquire_deadline_passed(&self, now: Instant) -> bool {
        self.phase == Phase::Acquiring
            && self
                .acquire_started
                .is_some_and(|started| now >= started + ACQUISITION_TIMEOUT)
    }

    /// Advances the state machine and returns an accepted location when an
    /// acquisition completes with a fix.
    fn advance(&mut self, now: Instant, input: ScheduleInput) -> Option<Location> {
        if self.stopped || self.paused {
            return None;
        }
        match self.phase {
            Phase::Idle => {
                self.start(now);
                None
            }
            Phase::Acquiring => match input {
                ScheduleInput::Fix(location) => {
                    if self.seen == Some(location) {
                        return None;
                    }
                    self.seen = Some(location);
                    self.phase = Phase::Waiting;
                    self.next_sample = Some(now + SAMPLING_INTERVAL);
                    Some(location)
                }
                ScheduleInput::Failed | ScheduleInput::Deadline => {
                    self.phase = Phase::Waiting;
                    self.next_sample = Some(
                        self.next_sample
                            .unwrap_or(now + SAMPLING_INTERVAL)
                            .max(now + Duration::from_secs(1)),
                    );
                    None
                }
            },
            Phase::Waiting => {
                if now >= self.next_sample.unwrap_or(now) {
                    self.start(now);
                }
                None
            }
        }
    }
}

enum ScheduleInput {
    Fix(Location),
    Failed,
    Deadline,
}

struct Shared {
    schedule: LocationSchedule,
    error: Option<LocationError>,
}

pub struct GeoClueSampler {
    shared: Rc<RefCell<Shared>>,
}

impl GeoClueSampler {
    pub fn new(context: glib::MainContext, on_location: impl Fn(Location) + 'static) -> Self {
        let shared = Rc::new(RefCell::new(Shared {
            schedule: LocationSchedule::new(),
            error: None,
        }));
        let loop_shared = Rc::clone(&shared);
        context.spawn_local(geo_clue_loop(loop_shared, on_location));
        Self { shared }
    }

    pub fn start(&self) {
        let now = Instant::now();
        self.shared.borrow_mut().schedule.start(now);
    }

    pub fn pause(&self) {
        self.shared.borrow_mut().schedule.pause();
    }

    pub fn resume(&self) {
        let now = Instant::now();
        self.shared.borrow_mut().schedule.resume(now);
    }

    pub fn stop(&self) {
        self.shared.borrow_mut().schedule.stop();
    }

    /// The terminal sampling error, if GeoClue denied access or disappeared.
    pub fn error(&self) -> Option<LocationError> {
        self.shared.borrow().error
    }
}

async fn geo_clue_loop(shared: Rc<RefCell<Shared>>, on_location: impl Fn(Location)) {
    let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
        shared.borrow_mut().error = Some(LocationError::ServiceUnavailable);
        return;
    };
    let client = match GeoClueClient::connect(&bus).await {
        Ok(client) => client,
        Err(error) => {
            shared.borrow_mut().error = Some(error);
            return;
        }
    };
    loop {
        glib::timeout_future(POLL_INTERVAL).await;
        let now = Instant::now();
        let mut deny = false;
        let should_acquire = {
            let shared = shared.borrow();
            let schedule = &shared.schedule;
            !schedule.is_stopped()
                && !schedule.is_paused()
                && schedule.phase() == Phase::Acquiring
                && !schedule.acquire_deadline_passed(now)
        };
        let input = if should_acquire {
            match client.current(&bus).await {
                Ok(Some(location)) => ScheduleInput::Fix(location),
                Ok(None) => ScheduleInput::Failed,
                Err(LocationError::PermissionDenied) => {
                    deny = true;
                    ScheduleInput::Failed
                }
                Err(_) => ScheduleInput::Failed,
            }
        } else {
            ScheduleInput::Deadline
        };
        let event = shared.borrow_mut().schedule.advance(now, input);
        if deny {
            shared.borrow_mut().error = Some(LocationError::PermissionDenied);
            shared.borrow_mut().schedule.stop();
        }
        if let Some(location) = event {
            on_location(location);
        }
        if shared.borrow().schedule.is_stopped() {
            break;
        }
    }
    let _ = client.disconnect(&bus).await;
}

struct GeoClueClient {
    object_path: String,
}

impl GeoClueClient {
    async fn connect(bus: &gio::DBusConnection) -> Result<Self, LocationError> {
        let object_path = manager_get_client(bus).await?;
        call(
            bus,
            Some(GEOCLUE_SERVICE),
            &object_path,
            GEOCLUE_CLIENT_INTERFACE,
            "Start",
            None,
            Some("()"),
        )
        .await?;
        Ok(Self { object_path })
    }

    async fn current(&self, bus: &gio::DBusConnection) -> Result<Option<Location>, LocationError> {
        let reply = call(
            bus,
            Some(GEOCLUE_SERVICE),
            &self.object_path,
            DBUS_PROPERTIES,
            "Get",
            Some(&(GEOCLUE_LOCATION_PROPERTY, "Location").to_variant()),
            Some("(v)"),
        )
        .await?;
        let path = reply
            .get::<(glib::Variant,)>()
            .and_then(|(value,)| value.get::<String>())
            .ok_or(LocationError::ServiceUnavailable)?;
        if path == "/" {
            return Ok(None);
        }
        location_from_path(bus, &path).await.map(Some)
    }

    async fn disconnect(&self, bus: &gio::DBusConnection) -> Result<(), LocationError> {
        call(
            bus,
            Some(GEOCLUE_SERVICE),
            &self.object_path,
            GEOCLUE_CLIENT_INTERFACE,
            "Stop",
            None,
            Some("()"),
        )
        .await
        .map(|_| ())
    }
}

async fn manager_get_client(bus: &gio::DBusConnection) -> Result<String, LocationError> {
    let reply = call(
        bus,
        Some(GEOCLUE_SERVICE),
        GEOCLUE_MANAGER_PATH,
        GEOCLUE_MANAGER_INTERFACE,
        "GetClient",
        None,
        Some("(o)"),
    )
    .await?;
    reply
        .get::<(String,)>()
        .map(|(path,)| path)
        .ok_or(LocationError::ServiceUnavailable)
}

async fn location_from_path(
    bus: &gio::DBusConnection,
    path: &str,
) -> Result<Location, LocationError> {
    let reply = call(
        bus,
        Some(GEOCLUE_SERVICE),
        path,
        DBUS_PROPERTIES,
        "GetAll",
        Some(&GEOCLUE_LOCATION_INTERFACE.to_variant()),
        Some("(a{sv})"),
    )
    .await?;
    let properties = reply
        .get::<(HashMap<String, glib::Variant>,)>()
        .map(|(properties,)| properties)
        .ok_or(LocationError::ServiceUnavailable)?;
    let latitude = property(&properties, "Latitude").ok_or(LocationError::ServiceUnavailable)?;
    let longitude = property(&properties, "Longitude").ok_or(LocationError::ServiceUnavailable)?;
    Ok(Location {
        latitude,
        longitude,
        accuracy: property(&properties, "Accuracy"),
    })
}

async fn call(
    bus: &gio::DBusConnection,
    service: Option<&str>,
    path: &str,
    interface: &str,
    method: &str,
    parameters: Option<&glib::Variant>,
    reply_type: Option<&str>,
) -> Result<glib::Variant, LocationError> {
    bus.call_future(
        service,
        path,
        interface,
        method,
        parameters,
        reply_type.and_then(|ty| glib::VariantTy::new(ty).ok()),
        gio::DBusCallFlags::NONE,
        CALL_TIMEOUT_MS,
    )
    .await
    .map_err(classify_dbus_error)
}

fn classify_dbus_error(error: glib::Error) -> LocationError {
    let domain = error.domain().as_str().as_str();
    let message = error.message();
    if domain.contains("NotAuthorized")
        || domain.contains("AccessDenied")
        || message.to_ascii_lowercase().contains("not authorized")
        || message.to_ascii_lowercase().contains("permission")
    {
        LocationError::PermissionDenied
    } else {
        LocationError::ServiceUnavailable
    }
}

fn property<T: glib::variant::FromVariant>(
    properties: &HashMap<String, glib::Variant>,
    name: &str,
) -> Option<T> {
    properties.get(name).and_then(glib::Variant::get)
}

#[cfg(test)]
#[path = "../../tests/unit/location.rs"]
mod tests;
