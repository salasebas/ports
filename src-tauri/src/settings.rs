use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

use crate::ports::{AlertSettings, Monitor};

const STORE: &str = "settings.json";
pub const SHORTCUT: &str = "shortcut";
const ALERTS: &str = "alerts";
const STOP_GRACE: &str = "stopGraceSeconds";
const IDLE_AFTER: &str = "idleAfterMinutes";
const IDLE_AFTER_MINUTES: [u64; 6] = [5, 10, 15, 30, 60, 120];
const SETTINGS_EVENT: &str = "ports://settings";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    revision: u64,
    shortcut: Option<String>,
    alerts: AlertSettings,
    stop_grace_seconds: u64,
    idle_after_minutes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDefaults {
    shortcut: Option<String>,
    alerts: AlertSettings,
    stop_grace_seconds: u64,
    idle_after_minutes: u64,
}

#[tauri::command]
pub fn get_settings_defaults() -> SettingsDefaults {
    SettingsDefaults {
        #[cfg(desktop)]
        shortcut: Some(crate::shortcut::DEFAULT.to_string()),
        #[cfg(not(desktop))]
        shortcut: None,
        alerts: AlertSettings::default(),
        stop_grace_seconds: crate::ports::DEFAULT_GRACE_SECS,
        idle_after_minutes: crate::ports::DEFAULT_IDLE_AFTER_MS / 60_000,
    }
}

pub fn read<T: DeserializeOwned, R: Runtime>(app: &AppHandle<R>, key: &str) -> Option<T> {
    let value = app.store(STORE).ok()?.get(key)?;
    serde_json::from_value(value).ok()
}

pub fn write<R: Runtime>(
    app: &AppHandle<R>,
    key: &str,
    value: impl Serialize,
) -> Result<(), String> {
    let store = app.store(STORE).map_err(|e| e.to_string())?;
    persist(&store, key, value)
}

fn persist<R: Runtime>(
    store: &tauri_plugin_store::Store<R>,
    key: &str,
    value: impl Serialize,
) -> Result<(), String> {
    let previous = store.get(key);
    store.set(key, serde_json::to_value(value).map_err(|e| e.to_string())?);
    if let Err(error) = store.save() {
        match previous {
            Some(value) => store.set(key, value),
            None => {
                store.delete(key);
            }
        }
        return Err(format!("Could not save {key}: {error}"));
    }
    Ok(())
}

#[derive(Default)]
pub struct Changes(Mutex<u64>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlertPatch {
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    threshold_bytes: Option<Option<u64>>,
}
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<u64>>, D::Error> {
    Option::<u64>::deserialize(d).map(Some)
}

pub fn alerts(app: &AppHandle) -> AlertSettings {
    read(app, ALERTS).unwrap_or_default()
}

pub fn stop_grace_seconds(app: &AppHandle) -> u64 {
    read(app, STOP_GRACE).unwrap_or(crate::ports::DEFAULT_GRACE_SECS)
}

pub fn idle_after_ms(app: &AppHandle) -> u64 {
    read::<u64, _>(app, IDLE_AFTER)
        .filter(|minutes| IDLE_AFTER_MINUTES.contains(minutes))
        .map_or(crate::ports::DEFAULT_IDLE_AFTER_MS, |minutes| {
            minutes * 60_000
        })
}

fn current(app: &AppHandle, revision: u64) -> Settings {
    #[cfg(desktop)]
    let shortcut = crate::shortcut::current(app);
    #[cfg(not(desktop))]
    let shortcut = None;
    Settings {
        revision,
        shortcut,
        alerts: app.state::<Monitor>().alert_settings(),
        stop_grace_seconds: app.state::<Monitor>().stop_grace_secs(),
        idle_after_minutes: app.state::<Monitor>().idle_after_ms() / 60_000,
    }
}

fn broadcast(app: &AppHandle, revision: u64) -> Settings {
    let settings = current(app, revision);
    if let Err(error) = app.emit(SETTINGS_EVENT, &settings) {
        eprintln!("could not send the settings: {error}");
    }
    settings
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    let state = app.state::<Changes>();
    let revision = state.0.lock().unwrap_or_else(|e| e.into_inner());
    current(&app, *revision)
}

#[tauri::command]
pub fn set_alerts(app: AppHandle, alerts: AlertPatch) -> Result<Settings, String> {
    let state = app.state::<Changes>();
    let mut revision = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let monitor = app.state::<Monitor>();
    let mut next = monitor.alert_settings();
    if let Some(enabled) = alerts.enabled {
        next.enabled = enabled;
    }
    if let Some(threshold) = alerts.threshold_bytes {
        next.threshold_bytes = threshold;
    }
    write(&app, ALERTS, next)?;
    monitor.set_alert_settings(next);
    *revision += 1;
    Ok(broadcast(&app, *revision))
}

#[tauri::command]
pub fn set_stop_grace(app: AppHandle, seconds: u64) -> Result<Settings, String> {
    let state = app.state::<Changes>();
    let mut revision = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let monitor = app.state::<Monitor>();
    let seconds = seconds.clamp(
        *crate::ports::GRACE_SECS.start(),
        *crate::ports::GRACE_SECS.end(),
    );
    write(&app, STOP_GRACE, seconds)?;
    monitor.set_stop_grace_secs(seconds);
    *revision += 1;
    Ok(broadcast(&app, *revision))
}

#[tauri::command]
pub fn set_idle_after(app: AppHandle, minutes: u64) -> Result<Settings, String> {
    if !IDLE_AFTER_MINUTES.contains(&minutes) {
        return Err(format!(
            "{minutes} minutes is not an idle time Ports offers"
        ));
    }
    let state = app.state::<Changes>();
    let mut revision = state.0.lock().unwrap_or_else(|e| e.into_inner());
    write(&app, IDLE_AFTER, minutes)?;
    let monitor = app.state::<Monitor>();
    monitor.set_idle_after_ms(minutes * 60_000);
    monitor.sample_now();
    *revision += 1;
    Ok(broadcast(&app, *revision))
}

#[tauri::command]
pub fn set_shortcut(app: AppHandle, shortcut: Option<String>) -> Result<Settings, String> {
    let state = app.state::<Changes>();
    let mut revision = state.0.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(desktop)]
    crate::shortcut::replace(&app, shortcut.as_deref())?;
    #[cfg(not(desktop))]
    let _ = shortcut;
    *revision += 1;
    Ok(broadcast(&app, *revision))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_windows_listen_to_the_settings_event() {
        let windows = include_str!("../../src/lib/settings.svelte.ts");
        assert!(windows.contains(&format!("= \"{SETTINGS_EVENT}\";")));
    }

    #[test]
    fn returns_save_failure_and_restores_the_previous_cached_value() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("data");
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("settings.json");
        let store = app
            .store_builder(&path)
            .disable_auto_save()
            .build()
            .unwrap();
        persist(&store, "enabled", true).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::fs::write(&parent, "blocks the settings directory").unwrap();
        assert!(
            persist(&store, "enabled", false)
                .unwrap_err()
                .contains("Could not save")
        );
        assert_eq!(store.get("enabled"), Some(serde_json::json!(true)));
    }

    #[test]
    fn distinguishes_omitted_threshold_from_explicit_null() {
        let omitted: AlertPatch = serde_json::from_str(r#"{"enabled":false}"#).unwrap();
        assert_eq!(omitted.threshold_bytes, None);
        let cleared: AlertPatch = serde_json::from_str(r#"{"thresholdBytes":null}"#).unwrap();
        assert_eq!(cleared.threshold_bytes, Some(None));
    }
}
