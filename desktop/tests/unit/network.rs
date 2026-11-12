use super::*;

fn connection(connection_type: &str) -> ActiveConnection {
    ActiveConnection {
        connection_type: connection_type.into(),
        state: ACTIVE_CONNECTION_ACTIVATED,
        default_route: true,
        ..Default::default()
    }
}

fn snapshot(connection: ActiveConnection) -> NetworkSnapshot {
    NetworkSnapshot {
        connections: vec![connection],
        metered: 2,
        roaming: false,
    }
}

fn managed_objects(registration_state: u32) -> glib::Variant {
    let text = format!(
        "{{'/org/freedesktop/ModemManager1/Modem/0': {{'org.freedesktop.ModemManager1.Modem.Modem3gpp': {{'RegistrationState': <uint32 {registration_state}>}}}}}}"
    );
    glib::Variant::parse(Some(glib::VariantTy::new("a{oa{sa{sv}}}").unwrap()), &text).unwrap()
}

#[test]
fn ethernet_and_wifi_are_wifi_only_when_explicitly_unmetered() {
    for connection_type in ["802-3-ethernet", "802-11-wireless"] {
        let mut state = snapshot(connection(connection_type));
        assert_eq!(classify(&state), NetworkClass::Wifi);
        state.metered = 4;
        assert_eq!(classify(&state), NetworkClass::Wifi);
    }
}

#[test]
fn metered_unknown_and_invalid_states_use_mobile_policy() {
    for metered in [0, 1, 3, 5, u32::MAX] {
        let mut state = snapshot(connection("802-11-wireless"));
        state.metered = metered;
        assert_eq!(classify(&state), NetworkClass::Mobile);
    }
}

#[test]
fn cellular_uses_roaming_only_with_confirmation() {
    for connection_type in ["gsm", "cdma"] {
        let mut state = snapshot(connection(connection_type));
        assert_eq!(classify(&state), NetworkClass::Mobile);
        state.roaming = true;
        assert_eq!(classify(&state), NetworkClass::Roaming);
    }
    let mut wifi = snapshot(connection("802-11-wireless"));
    wifi.roaming = true;
    assert_eq!(classify(&wifi), NetworkClass::Wifi);
}

#[test]
fn vpn_does_not_replace_the_underlying_default_connection() {
    let mut wifi = connection("802-11-wireless");
    wifi.primary = false;
    let mut vpn = connection("vpn");
    vpn.vpn = true;
    vpn.primary = true;
    let mut cellular = connection("gsm");
    cellular.default_route = false;
    let state = NetworkSnapshot {
        connections: vec![vpn, cellular, wifi],
        metered: 2,
        roaming: true,
    };
    assert_eq!(classify(&state), NetworkClass::Wifi);

    let state = NetworkSnapshot {
        connections: vec![connection("vpn")],
        metered: 2,
        roaming: false,
    };
    assert_eq!(classify(&state), NetworkClass::Mobile);
}

#[test]
fn default_route_wins_over_other_activated_connections() {
    let mut cellular = connection("gsm");
    cellular.default_route = false;
    cellular.primary = true;
    let wifi = connection("802-3-ethernet");
    let state = NetworkSnapshot {
        connections: vec![cellular, wifi],
        metered: 2,
        roaming: false,
    };
    assert_eq!(classify(&state), NetworkClass::Wifi);
}

#[test]
fn device_type_is_a_fallback_and_unknown_transport_is_conservative() {
    let mut wifi = connection("generic");
    wifi.device_types = vec![1];
    assert_eq!(classify(&snapshot(wifi)), NetworkClass::Wifi);

    let mut cellular = connection("generic");
    cellular.device_types = vec![8];
    assert_eq!(classify(&snapshot(cellular)), NetworkClass::Mobile);

    let mut unknown = connection("wireguard");
    unknown.device_types = vec![14];
    assert_eq!(classify(&snapshot(unknown)), NetworkClass::Mobile);
}

#[test]
fn missing_activated_or_unflagged_vpn_connections_are_conservative() {
    assert_eq!(
        classify(&NetworkSnapshot {
            connections: Vec::new(),
            metered: 2,
            roaming: false,
        }),
        NetworkClass::Mobile
    );
    let mut activating = connection("802-11-wireless");
    activating.state = 1;
    assert_eq!(classify(&snapshot(activating)), NetworkClass::Mobile);

    let mut unflagged_vpn = connection("vpn");
    unflagged_vpn.vpn = true;
    assert_eq!(classify(&snapshot(unflagged_vpn)), NetworkClass::Mobile);
}

#[test]
fn modem_manager_roaming_states_are_mapped_without_a_bus() {
    for state in [5, 7, 9] {
        assert!(roaming_from_managed_objects(&managed_objects(state)));
    }
    for state in [0, 1, 2, 3, 4, 6, 8, 10, u32::MAX] {
        assert!(!roaming_from_managed_objects(&managed_objects(state)));
    }
    assert!(!roaming_from_managed_objects(
        &Vec::<String>::new().to_variant()
    ));
}
