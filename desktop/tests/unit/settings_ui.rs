use super::*;

const MIB: u64 = 1024 * 1024;

fn model() -> Settings {
    Settings {
        i18n: Rc::new(I18n::new("en").unwrap()),
        theme: 0,
        locale: 0,
        page: SettingsPage::Overview,
        logout_enabled: false,
        confirming_logout: false,
        media_settings: Some(MediaSettings {
            cache_limit: CACHE_LIMITS[2],
            network: NetworkClass::Wifi,
            auto_download: AutoDownloadSettings::default(),
            autoplay: NetworkPolicy::default(),
            preload: NetworkPolicy::default(),
        }),
        usage: Some(MediaUsage {
            complete_bytes: 10 * MIB,
            partial_bytes: 2 * MIB,
            pinned_entries: 0,
            cache_limit: CACHE_LIMITS[2],
            by_category: vec![
                crate::domain::MediaCategoryUsage {
                    category: MediaCategory::Photo,
                    complete_bytes: 6 * MIB,
                    partial_bytes: 0,
                },
                crate::domain::MediaCategoryUsage {
                    category: MediaCategory::Video,
                    complete_bytes: 2 * MIB,
                    partial_bytes: 2 * MIB,
                },
                crate::domain::MediaCategoryUsage {
                    category: MediaCategory::Document,
                    complete_bytes: 2 * MIB,
                    partial_bytes: 0,
                },
            ],
            by_chat: vec![
                crate::domain::MediaChatUsage {
                    chat_id: 42,
                    complete_bytes: 6 * MIB,
                    partial_bytes: 2 * MIB,
                },
                crate::domain::MediaChatUsage {
                    chat_id: 7,
                    complete_bytes: 4 * MIB,
                    partial_bytes: 0,
                },
            ],
        }),
        media_loading: false,
        media_error: None,
        selected_network: NetworkClass::Wifi,
        confirming_clear: None,
        connectivity: None,
        appearance: None,
        maintenance: None,
        appearance_state: AppearanceState::default(),
        appearance_backend: AppearancePreferences::default(),
        appearance_restored: false,
        preference_versions: BTreeMap::new(),
        notification_settings: NotificationSettings::default(),
        local_notification_policy: LocalNotificationPolicy::default(),
        proxy_ids: Vec::new(),
        restoring_connectivity: 0,
        rendering: Rc::new(Cell::new(false)),
    }
}

#[test]
fn byte_labels_use_binary_capacities_without_losing_precision_at_unit_boundaries() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(1024), "1 KB");
    assert_eq!(format_bytes(500 * MIB), "500 MB");
    assert_eq!(format_bytes(1024 * MIB), "1 GB");
    assert_eq!(format_bytes(10 * 1024 * MIB), "10 GB");
    assert_eq!(format_bytes(1536 * MIB), "1.5 GB");
    assert_eq!(format_bytes(1024_u64.pow(4)), "1 TB");
}

#[test]
fn auto_download_edits_only_the_selected_network_and_category() {
    let original = AutoDownloadSettings::default();
    let rule = AutoDownloadRule {
        enabled: true,
        max_bytes: DOWNLOAD_SIZE_STEPS[6],
    };
    let updated =
        with_auto_download_rule(original, NetworkClass::Mobile, MediaCategory::Video, rule)
            .unwrap();
    assert_eq!(updated.mobile.video, rule);
    assert_eq!(updated.wifi, original.wifi);
    assert_eq!(updated.roaming, original.roaming);
    assert_eq!(updated.mobile.photo, original.mobile.photo);
    assert_eq!(updated.mobile.document, original.mobile.document);
    assert_eq!(
        with_auto_download_rule(original, NetworkClass::Wifi, MediaCategory::Avatar, rule,),
        None
    );
}

#[test]
fn every_download_size_step_is_valid_for_core_policies() {
    let original = AutoDownloadSettings::default();
    for network in NETWORK_CLASSES {
        for category in DOWNLOAD_CATEGORIES {
            for max_bytes in DOWNLOAD_SIZE_STEPS {
                let updated = with_auto_download_rule(
                    original,
                    network,
                    category,
                    AutoDownloadRule {
                        enabled: true,
                        max_bytes,
                    },
                )
                .unwrap();
                updated.validate().unwrap();
                assert_eq!(
                    updated.policy(network).rule(category).unwrap().max_bytes,
                    max_bytes
                );
            }
        }
    }
}

#[test]
fn custom_core_size_is_preserved_and_can_be_selected_without_rounding() {
    let steps = size_steps(16 * MIB);
    let index = steps.iter().position(|bytes| *bytes == 16 * MIB).unwrap();
    assert_eq!(steps[index - 1], 15 * MIB);
    assert_eq!(steps[index + 1], 25 * MIB);

    let mut page = model();
    page.media_settings
        .as_mut()
        .unwrap()
        .auto_download
        .wifi
        .photo = AutoDownloadRule {
        enabled: true,
        max_bytes: 16 * MIB,
    };
    assert!(
        page.handle(SettingsInput::DownloadLimit(0, index as u32))
            .is_empty()
    );
    let output = page.handle(SettingsInput::DownloadLimit(0, index as u32 + 1));
    let SettingsOutput::Command(WorkerCommand::SetAutoDownload(updated)) = &output[0] else {
        panic!()
    };
    assert_eq!(updated.wifi.photo.max_bytes, 25 * MIB);
}

#[test]
fn refresh_emits_real_worker_requests_and_busy_blocks_duplicates() {
    let mut page = model();
    let outputs = page.handle(SettingsInput::RefreshMedia);
    assert_eq!(outputs.len(), 2);
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::MediaUsage)
    ));
    assert!(matches!(
        outputs[1],
        SettingsOutput::Command(WorkerCommand::GetMediaSettings)
    ));
    assert!(page.media_loading);
    assert!(page.handle(SettingsInput::RefreshMedia).is_empty());
    assert!(page.handle(SettingsInput::CacheLimit(0)).is_empty());
}

#[test]
fn loading_and_error_keep_last_usage_and_settings_visible() {
    let mut page = model();
    let settings = page.media_settings;
    let usage = page.usage.clone();
    page.handle(SettingsInput::SetMediaLoading(true));
    page.handle(SettingsInput::SetMediaError(Some("storage failed".into())));
    assert!(!page.media_loading);
    assert_eq!(page.media_error.as_deref(), Some("storage failed"));
    assert_eq!(page.media_settings, settings);
    assert_eq!(page.usage, usage);
    assert_eq!(page.total_usage(), 12 * MIB);
}

#[test]
fn cache_limit_waits_for_worker_snapshot_instead_of_fabricating_success() {
    let mut page = model();
    let before = page.media_settings;
    let outputs = page.handle(SettingsInput::CacheLimit(0));
    let SettingsOutput::Command(WorkerCommand::SetCacheLimit { limit }) = &outputs[0] else {
        panic!()
    };
    assert_eq!(*limit, CACHE_LIMITS[0]);
    assert!(matches!(
        outputs[1],
        SettingsOutput::Command(WorkerCommand::GetMediaSettings)
    ));
    assert_eq!(page.media_settings, before);
    assert!(page.media_loading);
}

#[test]
fn unlimited_cache_uses_the_core_enum_not_a_sentinel_byte_count() {
    let mut page = model();
    let outputs = page.handle(SettingsInput::CacheLimit(5));
    let SettingsOutput::Command(WorkerCommand::SetCacheLimit { limit }) = &outputs[0] else {
        panic!()
    };
    assert_eq!(*limit, CacheLimit::Unlimited);
    assert_eq!(page.media_settings.unwrap().cache_limit, CACHE_LIMITS[2]);
}

#[test]
fn clear_all_requires_confirmation_and_cancel_never_emits_a_command() {
    let mut page = model();
    assert!(page.handle(SettingsInput::ConfirmClear).is_empty());
    assert!(page.handle(SettingsInput::RequestClear).is_empty());
    assert_eq!(page.confirming_clear, Some(ClearTarget::All));
    assert!(page.handle(SettingsInput::CancelClear).is_empty());
    assert_eq!(page.confirming_clear, None);
    page.handle(SettingsInput::RequestClear);
    let outputs = page.handle(SettingsInput::ConfirmClear);
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::ClearMedia(ClearMediaScope::All))
    ));
    assert!(page.usage.is_some());
}

#[test]
fn category_clear_uses_current_bytes_and_requires_its_own_confirmation() {
    let mut page = model();
    assert_eq!(page.category_usage(MediaCategory::Video), 4 * MIB);
    assert!(
        page.handle(SettingsInput::RequestClearCategory(MediaCategory::Avatar))
            .is_empty()
    );
    assert_eq!(page.confirming_clear, None);
    page.handle(SettingsInput::RequestClearCategory(MediaCategory::Video));
    assert_eq!(
        page.confirming_clear,
        Some(ClearTarget::Category(MediaCategory::Video))
    );
    assert_eq!(
        page.clear_bytes(ClearTarget::Category(MediaCategory::Video)),
        4 * MIB
    );
    let outputs = page.handle(SettingsInput::ConfirmClear);
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::ClearMedia(ClearMediaScope::Category(
            MediaCategory::Video
        )))
    ));
}

#[test]
fn chat_clear_validates_identity_and_rejects_unscoped_or_stale_rows() {
    let mut page = model();
    page.handle(SettingsInput::RequestClearChat(42));
    assert_eq!(page.confirming_clear, Some(ClearTarget::Chat(42)));
    assert_eq!(page.clear_bytes(ClearTarget::Chat(42)), 8 * MIB);
    let outputs = page.handle(SettingsInput::ConfirmClear);
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::ClearMedia(ClearMediaScope::Chat(42)))
    ));

    let mut page = model();
    page.usage
        .as_mut()
        .unwrap()
        .by_chat
        .push(crate::domain::MediaChatUsage {
            chat_id: 0,
            complete_bytes: MIB,
            partial_bytes: 0,
        });
    assert!(page.handle(SettingsInput::RequestClearChat(0)).is_empty());
    page.confirming_clear = Some(ClearTarget::Chat(0));
    assert!(page.handle(SettingsInput::ConfirmClear).is_empty());

    page.handle(SettingsInput::RequestClearChat(42));
    page.handle(SettingsInput::SetMediaUsage(MediaUsage::default()));
    assert_eq!(page.confirming_clear, None);
    assert!(page.handle(SettingsInput::ConfirmClear).is_empty());
}

#[test]
fn selecting_a_preset_does_not_change_the_devices_active_network() {
    let mut page = model();
    let outputs = page.handle(SettingsInput::SelectNetwork(1));
    assert!(outputs.is_empty());
    assert_eq!(page.selected_network, NetworkClass::Mobile);
    assert_eq!(page.media_settings.unwrap().network, NetworkClass::Wifi);

    let outputs = page.handle(SettingsInput::DownloadEnabled(1, true));
    let SettingsOutput::Command(WorkerCommand::SetAutoDownload(updated)) = &outputs[0] else {
        panic!()
    };
    assert!(updated.mobile.video.enabled);
    assert!(!updated.wifi.video.enabled);
    assert!(!updated.roaming.video.enabled);
}

#[test]
fn autoplay_and_preload_update_only_the_selected_network_policy() {
    let mut page = model();
    page.media_settings.as_mut().unwrap().autoplay.wifi = true;
    page.handle(SettingsInput::SelectNetwork(1));
    let before = page.media_settings;
    let outputs = page.handle(SettingsInput::Autoplay(true));
    let SettingsOutput::Command(WorkerCommand::SetAutoplay(updated)) = &outputs[0] else {
        panic!()
    };
    assert!(updated.wifi && updated.mobile && !updated.roaming);
    assert_eq!(page.media_settings, before);
    assert!(page.handle(SettingsInput::Preload(true)).is_empty());

    page.handle(SettingsInput::SetMediaLoading(false));
    let outputs = page.handle(SettingsInput::Preload(true));
    let SettingsOutput::Command(WorkerCommand::SetPreload(updated)) = &outputs[0] else {
        panic!()
    };
    assert!(!updated.wifi && updated.mobile && !updated.roaming);
    assert_eq!(page.media_settings, before);
}

#[test]
fn network_policy_noops_do_not_emit_worker_commands() {
    let mut page = model();
    assert!(page.handle(SettingsInput::Autoplay(false)).is_empty());
    assert!(page.handle(SettingsInput::Preload(false)).is_empty());
    assert!(!page.media_loading);
}

#[test]
fn navigation_opens_real_pages_and_back_returns_to_overview() {
    let mut page = model();
    let outputs = page.handle(SettingsInput::Open(SettingsPage::Data));
    assert_eq!(page.page, SettingsPage::Data);
    assert_eq!(outputs.len(), 2);
    assert!(page.handle(SettingsInput::Back).is_empty());
    assert_eq!(page.page, SettingsPage::Overview);
    assert!(matches!(
        page.handle(SettingsInput::Back)[0],
        SettingsOutput::Back
    ));

    let outputs = page.handle(SettingsInput::Open(SettingsPage::Connectivity));
    assert_eq!(page.page, SettingsPage::Connectivity);
    assert_eq!(outputs.len(), 4);
    assert!(outputs.iter().all(|output| matches!(
        output,
        SettingsOutput::Command(WorkerCommand::Connectivity(_))
    )));

    let outputs = page.handle(SettingsInput::Open(SettingsPage::Appearance));
    assert_eq!(page.page, SettingsPage::Appearance);
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::GetPreferences)
    ));

    let outputs = page.handle(SettingsInput::Open(SettingsPage::Maintenance));
    assert_eq!(page.page, SettingsPage::Maintenance);
    assert_eq!(outputs.len(), 2);
    assert!(outputs.iter().all(|output| matches!(
        output,
        SettingsOutput::Command(WorkerCommand::Maintenance(_))
    )));
}

#[test]
fn appearance_restore_preserves_backend_wallpaper_and_versions_followup_writes() {
    let mut page = model();
    let document = PreferenceDocument {
        version: APP_PREFERENCES_VERSION,
        payload_json: serde_json::json!({
            "source": { "kind": "local", "path": "/tmp/wall.png" },
            "dim": true,
            "blur": true,
            "intensity": 20,
            "rotation": 90,
            "hash": 7,
            "ui": {
                "theme": "dark",
                "locale": "ru",
                "accent": "Ocean"
            }
        })
        .to_string(),
    };
    page.handle(SettingsInput::SetPreferences(AppPreferences {
        documents: vec![(PreferenceNamespace::Appearance, document)],
    }));
    assert_eq!(page.i18n.locale(), "ru");
    assert_eq!(
        page.appearance_state.theme,
        crate::appearance::ThemePreference::Dark
    );
    assert_eq!(
        page.appearance_state.accent,
        crate::appearance::AccentPreset::Ocean
    );
    assert_eq!(page.appearance_state.wallpaper.mode, WallpaperMode::Image);
    assert_eq!(page.appearance_backend.hash, 7);

    let outputs = page.handle(SettingsInput::AppearanceOutput(AppearanceOutput::Command(
        AppearanceCommand::SetAccent(crate::appearance::AccentPreset::Sakura),
    )));
    let SettingsOutput::Command(WorkerCommand::SetPreference {
        namespace,
        expected_version,
        document,
    }) = &outputs[0]
    else {
        panic!()
    };
    assert_eq!(*namespace, PreferenceNamespace::Appearance);
    assert_eq!(*expected_version, Some(APP_PREFERENCES_VERSION));
    let payload: serde_json::Value = serde_json::from_str(&document.payload_json).unwrap();
    assert_eq!(payload["source"]["path"], "/tmp/wall.png");
    assert_eq!(payload["hash"], 7);
    assert_eq!(payload["ui"]["accent"], "Sakura");
    assert_eq!(payload["ui"]["theme"], "dark");
}

#[test]
fn theme_and_locale_keep_legacy_outputs_and_persist_appearance_document() {
    let mut page = model();
    let outputs = page.handle(SettingsInput::AppearanceOutput(AppearanceOutput::Theme(
        crate::appearance::ThemePreference::Dark,
    )));
    assert!(matches!(
        outputs[0],
        SettingsOutput::ChangeTheme(ThemePreference::Dark)
    ));
    let SettingsOutput::Command(WorkerCommand::SetPreference { document, .. }) = &outputs[1] else {
        panic!()
    };
    let payload: serde_json::Value = serde_json::from_str(&document.payload_json).unwrap();
    assert_eq!(payload["ui"]["theme"], "dark");

    let outputs = page.handle(SettingsInput::Locale(1));
    assert!(matches!(outputs[0], SettingsOutput::ChangeLocale(_)));
    let SettingsOutput::Command(WorkerCommand::SetPreference { document, .. }) = &outputs[1] else {
        panic!()
    };
    let payload: serde_json::Value = serde_json::from_str(&document.payload_json).unwrap();
    assert_eq!(payload["ui"]["locale"], "ru");
}

#[test]
fn local_import_waits_for_backend_without_persisting_an_unvalidated_path() {
    let mut page = model();
    let original = page.appearance_state.wallpaper.clone();
    let outputs = page.handle(SettingsInput::AppearanceOutput(
        AppearanceOutput::ImportLocalWallpaper {
            path: "/tmp/original.png".into(),
        },
    ));
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::Appearance(
            crate::client::appearance::AppearanceCommand::ImportLocalWallpaper { .. }
        ))
    ));
    assert_eq!(page.appearance_state.wallpaper, original);
}

#[test]
fn telegram_wallpaper_persists_source_metadata_and_saved_catalog_hash() {
    let mut page = model();
    let config = WallpaperConfig {
        mode: WallpaperMode::Image,
        path: Some("/cache/telegram-7.png".into()),
        telegram: Some(crate::appearance::TelegramWallpaper {
            id: 7,
            access_hash: 70,
            slug: "cached".into(),
        }),
        pattern: true,
        colors: vec![0x102030, 0x405060],
        dim: 0.65,
        blur: true,
        intensity: -30,
        rotation: 45,
    };
    let outputs = page.handle(SettingsInput::AppearanceOutput(AppearanceOutput::Command(
        AppearanceCommand::SetWallpaper(config.clone()),
    )));
    assert_eq!(outputs.len(), 2);
    let SettingsOutput::Command(WorkerCommand::SetPreference { document, .. }) = &outputs[0] else {
        panic!()
    };
    let payload_json = document.payload_json.clone();
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
    assert_eq!(payload["source"]["kind"], "telegram");
    assert_eq!(payload["source"]["id"], 7);
    assert_eq!(payload["ui"]["wallpaper"]["telegram"]["slug"], "cached");
    assert_eq!(
        payload["ui"]["wallpaper"]["dim"].as_f64().unwrap(),
        config.dim as f64
    );
    assert_eq!(payload["ui"]["wallpaper"]["intensity"], -30);
    assert!(matches!(outputs[1], SettingsOutput::Wallpaper(_)));

    let mut restored = model();
    restored.handle(SettingsInput::SetPreferences(AppPreferences {
        documents: vec![(
            PreferenceNamespace::Appearance,
            PreferenceDocument {
                version: APP_PREFERENCES_VERSION,
                payload_json,
            },
        )],
    }));
    assert_eq!(restored.appearance_state.wallpaper, config);

    restored.appearance_backend.hash = 77;
    let outputs = restored.handle(SettingsInput::AppearanceOutput(
        AppearanceOutput::BrowseTelegramWallpapers,
    ));
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::Appearance(
            crate::client::appearance::AppearanceCommand::GetWallpapers { hash: 77 }
        ))
    ));
}

#[test]
fn connectivity_outputs_use_real_worker_commands_and_notification_preferences() {
    let mut page = model();
    let proxy = |id| crate::connectivity::ProxyRecord {
        id,
        kind: UiProxyKind::Socks5,
        host: "localhost".into(),
        port: 1080,
        username: String::new(),
        has_password: false,
        has_secret: false,
        active: false,
        checked: false,
        latency_ms: None,
    };
    page.handle(SettingsInput::Connectivity(ConnectivityInput::SetProxies(
        vec![proxy(1), proxy(2)],
    )));
    let outputs = page.handle(SettingsInput::ConnectivityOutput(
        ConnectivityOutput::ProxyProbeAll,
    ));
    let SettingsOutput::Command(WorkerCommand::Connectivity(ConnectivityCommand::ProbeProxies {
        ids,
    })) = &outputs[0]
    else {
        panic!()
    };
    assert_eq!(ids, &[1, 2]);

    let outputs = page.handle(SettingsInput::ConnectivityOutput(
        ConnectivityOutput::NotificationSave(NotificationSettings {
            private: false,
            groups: true,
            channels: false,
            mentions: false,
        }),
    ));
    assert_eq!(outputs.len(), 4);
    let SettingsOutput::Command(WorkerCommand::SetPreference {
        namespace,
        document,
        ..
    }) = &outputs[3]
    else {
        panic!()
    };
    assert_eq!(*namespace, PreferenceNamespace::Notifications);
    let payload: serde_json::Value = serde_json::from_str(&document.payload_json).unwrap();
    assert_eq!(payload["private"], false);
    assert_eq!(payload["mentions"], false);
}

#[test]
fn preference_conflict_reloads_and_maintenance_uses_its_backend() {
    let mut page = model();
    page.preference_versions
        .insert(PreferenceNamespace::Appearance, 1);
    let outputs = page.handle(SettingsInput::PreferenceChanged {
        namespace: PreferenceNamespace::Appearance,
        changed: false,
    });
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::GetPreferences)
    ));
    assert!(
        !page
            .preference_versions
            .contains_key(&PreferenceNamespace::Appearance)
    );

    let outputs = page.handle(SettingsInput::MaintenanceOutput(
        MaintenanceOutput::Command(MaintenanceCommand::Stats),
    ));
    assert!(matches!(
        outputs[0],
        SettingsOutput::Command(WorkerCommand::Maintenance(MaintenanceCommand::Stats))
    ));
}

#[test]
fn logout_still_requires_confirmation() {
    let mut page = model();
    assert!(page.handle(SettingsInput::ConfirmLogout).is_empty());
    page.handle(SettingsInput::SetLogoutEnabled(true));
    page.handle(SettingsInput::RequestLogout);
    assert!(matches!(
        page.handle(SettingsInput::ConfirmLogout)[0],
        SettingsOutput::Logout
    ));
}

#[test]
fn media_labels_exist_in_every_supported_locale() {
    for locale in SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "feature-settings-settings_data",
            "feature-settings-settings_cache",
            "feature-settings-settings_cache_sub",
            "feature-settings-settings_cache_size",
            "feature-settings-settings_cache_limit",
            "feature-settings-settings_cache_limit_500",
            "feature-settings-settings_cache_limit_1",
            "feature-settings-settings_cache_limit_2",
            "feature-settings-settings_cache_limit_5",
            "feature-settings-settings_cache_limit_10",
            "feature-settings-settings_cache_unlimited",
            "feature-settings-settings_clear_cache",
            "feature-settings-settings_clear_cache_action",
            "feature-settings-settings_clear_cache_confirm",
            "feature-settings-settings_storage_chats",
            "feature-settings-settings_storage_clear_chat",
            "feature-settings-settings_storage_empty",
            "feature-settings-settings_storage_files",
            "feature-settings-settings_storage_other",
            "feature-settings-settings_storage_photos",
            "feature-settings-settings_storage_videos",
            "feature-settings-settings_show_avatars",
            "feature-settings-settings_autodownload",
            "feature-settings-settings_autodownload_wifi",
            "feature-settings-settings_autodownload_mobile",
            "feature-settings-settings_autodownload_roaming",
            "feature-settings-settings_autodownload_photos",
            "feature-settings-settings_autodownload_videos",
            "feature-settings-settings_autodownload_files",
            "feature-settings-settings_autodownload_limit",
            "feature-settings-settings_autoplay",
            "feature-settings-settings_autoplay_videos_sub",
            "feature-settings-settings_autodownload_preload_video",
            "feature-settings-settings_autodownload_preload_video_sub",
            "feature-settings-settings_refresh",
            "desktop-storage",
            "desktop-retry",
        ] {
            let value =
                if key.ends_with("_size") || key.ends_with("_confirm") || key.ends_with("_limit") {
                    i18n.text(key, &[("arg1", "1 MB")]).unwrap()
                } else {
                    i18n.text(key, &[]).unwrap()
                };
            assert!(!value.is_empty(), "{locale}: {key}");
        }
    }
}
