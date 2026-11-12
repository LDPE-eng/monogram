use crate::domain::NetworkClass;
use gio::prelude::*;
use relm4::gtk::{gio, glib};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

const NM_SERVICE: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_INTERFACE: &str = "org.freedesktop.NetworkManager";
const NM_ACTIVE_INTERFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const NM_DEVICE_INTERFACE: &str = "org.freedesktop.NetworkManager.Device";
const MM_SERVICE: &str = "org.freedesktop.ModemManager1";
const MM_PATH: &str = "/org/freedesktop/ModemManager1";
const MM_3GPP_INTERFACE: &str = "org.freedesktop.ModemManager1.Modem.Modem3gpp";
const DBUS_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const DBUS_OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const ACTIVE_CONNECTION_ACTIVATED: u32 = 2;
const CALL_TIMEOUT_MS: i32 = 3000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Transport {
    Wifi,
    Cellular,
    #[default]
    Unknown,
}

#[derive(Debug, Default)]
struct ActiveConnection {
    connection_type: String,
    device_types: Vec<u32>,
    state: u32,
    vpn: bool,
    default_route: bool,
    primary: bool,
}

impl ActiveConnection {
    fn transport(&self) -> Transport {
        match self.connection_type.as_str() {
            "802-3-ethernet" | "802-11-wireless" => return Transport::Wifi,
            "gsm" | "cdma" => return Transport::Cellular,
            _ => {}
        }
        if self
            .device_types
            .iter()
            .any(|device_type| matches!(device_type, 6 | 8))
        {
            Transport::Cellular
        } else if !self.device_types.is_empty()
            && self
                .device_types
                .iter()
                .all(|device_type| matches!(device_type, 1 | 2))
        {
            Transport::Wifi
        } else {
            Transport::Unknown
        }
    }
}

#[derive(Debug, Default)]
struct NetworkSnapshot {
    connections: Vec<ActiveConnection>,
    metered: u32,
    roaming: bool,
}

fn selected_connection(snapshot: &NetworkSnapshot) -> Option<&ActiveConnection> {
    snapshot
        .connections
        .iter()
        .filter(|connection| connection.state == ACTIVE_CONNECTION_ACTIVATED && !connection.vpn)
        .min_by_key(|connection| {
            (
                !connection.default_route,
                !connection.primary,
                connection.transport() == Transport::Unknown,
            )
        })
}

fn classify(snapshot: &NetworkSnapshot) -> NetworkClass {
    match selected_connection(snapshot).map(ActiveConnection::transport) {
        Some(Transport::Cellular) if snapshot.roaming => NetworkClass::Roaming,
        Some(Transport::Wifi) if matches!(snapshot.metered, 2 | 4) => NetworkClass::Wifi,
        Some(Transport::Cellular | Transport::Wifi | Transport::Unknown) | None => {
            NetworkClass::Mobile
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkAdapter;

impl NetworkAdapter {
    pub fn new() -> Self {
        Self
    }

    pub async fn detect(&self) -> NetworkClass {
        let monitor = gio::NetworkMonitor::default();
        let Ok(bus) = gio::bus_get_future(gio::BusType::System).await else {
            return NetworkClass::Mobile;
        };
        detect_with_bus(&bus, &monitor).await
    }

    pub fn subscribe(&self, callback: impl Fn(NetworkClass) + 'static) -> NetworkSubscription {
        let monitor = gio::NetworkMonitor::default();
        let state = Rc::new(RefCell::new(SubscriptionState {
            context: glib::MainContext::ref_thread_default(),
            monitor: monitor.clone(),
            bus: None,
            bus_subscriptions: Vec::new(),
            callback: Rc::new(callback),
            last_class: None,
            refreshing: false,
            pending: false,
            active: true,
        }));
        let mut handlers = Vec::new();
        handlers.push(monitor.connect_network_changed({
            let state = Rc::downgrade(&state);
            move |_, _| request_refresh(&state)
        }));
        handlers.push(monitor.connect_network_metered_notify({
            let state = Rc::downgrade(&state);
            move |_| request_refresh(&state)
        }));
        handlers.push(monitor.connect_connectivity_notify({
            let state = Rc::downgrade(&state);
            move |_| request_refresh(&state)
        }));
        request_refresh(&Rc::downgrade(&state));
        NetworkSubscription { state, handlers }
    }
}

pub struct NetworkSubscription {
    state: Rc<RefCell<SubscriptionState>>,
    handlers: Vec<glib::SignalHandlerId>,
}

impl Drop for NetworkSubscription {
    fn drop(&mut self) {
        let (monitor, subscriptions) = {
            let mut state = self.state.borrow_mut();
            state.active = false;
            (
                state.monitor.clone(),
                std::mem::take(&mut state.bus_subscriptions),
            )
        };
        for handler in self.handlers.drain(..) {
            monitor.disconnect(handler);
        }
        drop(subscriptions);
    }
}

struct SubscriptionState {
    context: glib::MainContext,
    monitor: gio::NetworkMonitor,
    bus: Option<gio::DBusConnection>,
    bus_subscriptions: Vec<gio::SignalSubscription>,
    callback: Rc<dyn Fn(NetworkClass)>,
    last_class: Option<NetworkClass>,
    refreshing: bool,
    pending: bool,
    active: bool,
}

fn request_refresh(state: &Weak<RefCell<SubscriptionState>>) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let context = {
        let mut state = state.borrow_mut();
        if !state.active {
            return;
        }
        if state.refreshing {
            state.pending = true;
            return;
        }
        state.refreshing = true;
        state.context.clone()
    };
    drop(context.spawn_local(refresh(state)));
}

async fn refresh(state: Rc<RefCell<SubscriptionState>>) {
    let class = match refresh_bus(&state).await {
        Some(bus) => {
            let monitor = state.borrow().monitor.clone();
            detect_with_bus(&bus, &monitor).await
        }
        None => NetworkClass::Mobile,
    };
    let (callback, pending) = {
        let mut state = state.borrow_mut();
        if !state.active {
            return;
        }
        state.refreshing = false;
        let callback = if state.last_class == Some(class) {
            None
        } else {
            state.last_class = Some(class);
            Some(Rc::clone(&state.callback))
        };
        (callback, std::mem::take(&mut state.pending))
    };
    if let Some(callback) = callback {
        callback(class);
    }
    if pending {
        request_refresh(&Rc::downgrade(&state));
    }
}

async fn refresh_bus(state: &Rc<RefCell<SubscriptionState>>) -> Option<gio::DBusConnection> {
    if let Some(bus) = state.borrow().bus.clone() {
        return Some(bus);
    }
    let bus = gio::bus_get_future(gio::BusType::System).await.ok()?;
    let mut current = state.borrow_mut();
    if !current.active {
        return None;
    }
    for service in [NM_SERVICE, MM_SERVICE] {
        let weak = Rc::downgrade(state);
        let subscription = bus.subscribe_to_signal(
            Some(service),
            None,
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_| request_refresh(&weak),
        );
        current.bus_subscriptions.push(subscription);
    }
    current.bus = Some(bus.clone());
    Some(bus)
}

struct DetectionError;

async fn detect_with_bus(bus: &gio::DBusConnection, monitor: &gio::NetworkMonitor) -> NetworkClass {
    let Ok(mut snapshot) = read_snapshot(bus).await else {
        return NetworkClass::Mobile;
    };
    if monitor.is_network_metered() {
        snapshot.metered = 1;
    }
    if selected_connection(&snapshot).map(ActiveConnection::transport) == Some(Transport::Cellular)
    {
        snapshot.roaming = modem_roaming(bus).await;
    }
    classify(&snapshot)
}

async fn read_snapshot(bus: &gio::DBusConnection) -> Result<NetworkSnapshot, DetectionError> {
    let manager = get_all(bus, NM_SERVICE, NM_PATH, NM_INTERFACE).await?;
    let active_paths = object_paths(manager.get("ActiveConnections").ok_or(DetectionError)?)?;
    let primary = property::<String>(&manager, "PrimaryConnection")?;
    let metered = property::<u32>(&manager, "Metered")?;
    let mut connections = Vec::with_capacity(active_paths.len());
    for path in active_paths {
        let properties = get_all(bus, NM_SERVICE, &path, NM_ACTIVE_INTERFACE).await?;
        let state = property::<u32>(&properties, "State")?;
        let vpn = property::<bool>(&properties, "Vpn")?;
        let mut device_types = Vec::new();
        if state == ACTIVE_CONNECTION_ACTIVATED && !vpn {
            for device in object_paths(properties.get("Devices").ok_or(DetectionError)?)? {
                let properties = get_all(bus, NM_SERVICE, &device, NM_DEVICE_INTERFACE).await?;
                device_types.push(property::<u32>(&properties, "DeviceType")?);
            }
        }
        connections.push(ActiveConnection {
            connection_type: property::<String>(&properties, "Type")?,
            device_types,
            state,
            vpn,
            default_route: property::<bool>(&properties, "Default")?
                || property::<bool>(&properties, "Default6")?,
            primary: path == primary,
        });
    }
    Ok(NetworkSnapshot {
        connections,
        metered,
        roaming: false,
    })
}

async fn get_all(
    bus: &gio::DBusConnection,
    service: &str,
    path: &str,
    interface: &str,
) -> Result<HashMap<String, glib::Variant>, DetectionError> {
    let reply = bus
        .call_future(
            Some(service),
            path,
            DBUS_PROPERTIES,
            "GetAll",
            Some(&interface.to_variant()),
            Some(glib::VariantTy::new("(a{sv})").map_err(|_| DetectionError)?),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
        )
        .await
        .map_err(|_| DetectionError)?;
    reply
        .get::<(HashMap<String, glib::Variant>,)>()
        .map(|(properties,)| properties)
        .ok_or(DetectionError)
}

fn property<T: glib::variant::FromVariant>(
    properties: &HashMap<String, glib::Variant>,
    name: &str,
) -> Result<T, DetectionError> {
    properties
        .get(name)
        .and_then(glib::Variant::get)
        .ok_or(DetectionError)
}

fn object_paths(variant: &glib::Variant) -> Result<Vec<String>, DetectionError> {
    if variant.type_().as_str() != "ao" {
        return Err(DetectionError);
    }
    variant
        .iter()
        .map(|path| path.str().map(str::to_owned).ok_or(DetectionError))
        .collect()
}

async fn modem_roaming(bus: &gio::DBusConnection) -> bool {
    let Ok(reply) = bus
        .call_future(
            Some(MM_SERVICE),
            MM_PATH,
            DBUS_OBJECT_MANAGER,
            "GetManagedObjects",
            None,
            Some(glib::VariantTy::new("(a{oa{sa{sv}}})").expect("static variant type")),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
        )
        .await
    else {
        return false;
    };
    roaming_from_managed_objects(&reply.child_value(0))
}

fn roaming_from_managed_objects(objects: &glib::Variant) -> bool {
    if objects.type_().as_str() != "a{oa{sa{sv}}}" {
        return false;
    }
    for object in objects.iter() {
        for interface in object.child_value(1).iter() {
            if interface.child_value(0).str() != Some(MM_3GPP_INTERFACE) {
                continue;
            }
            let Some(properties) = interface
                .child_value(1)
                .get::<HashMap<String, glib::Variant>>()
            else {
                continue;
            };
            if properties
                .get("RegistrationState")
                .and_then(glib::Variant::get::<u32>)
                .is_some_and(|state| matches!(state, 5 | 7 | 9))
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "../../tests/unit/network.rs"]
mod tests;
