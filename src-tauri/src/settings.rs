//! Settings the backend needs before any window loads, kept in
//! `settings.json` in the app's data folder. Preferences only the windows
//! use stay in the webview's storage.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

use crate::ports::{AlertSettings, Monitor};

const STORE: &str = "settings.json";
pub const SHORTCUT: &str = "shortcut";
const ALERTS: &str = "alerts";
/// Carries `Settings` to every window when one changes. Must match `src/lib/settings.svelte.ts`.
const SETTINGS_EVENT: &str = "ports://settings";

/// Mirrors `Settings` in `src/lib/settings.svelte.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The accelerator that shows the popover, e.g. `alt+super+KeyP`; `None` when there's none.
    shortcut: Option<String>,
    alerts: AlertSettings,
}

/// The stored value of `key`, or `None` when it was never set or can't be read.
pub fn read<T: DeserializeOwned, R: Runtime>(app: &AppHandle<R>, key: &str) -> Option<T> {
    let value = app.store(STORE).ok()?.get(key)?;
    serde_json::from_value(value).ok()
}

pub fn write<R: Runtime>(app: &AppHandle<R>, key: &str, value: impl Serialize) {
    let saved = app
        .store(STORE)
        .map_err(|e| e.to_string())
        .and_then(|store| {
            let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
            store.set(key, value);
            store.save().map_err(|e| e.to_string())
        });
    if let Err(error) = saved {
        eprintln!("could not save the {key} setting: {error}");
    }
}

pub fn alerts(app: &AppHandle) -> AlertSettings {
    read(app, ALERTS).unwrap_or_default()
}

fn current(app: &AppHandle) -> Settings {
    #[cfg(desktop)]
    let shortcut = crate::shortcut::current(app);
    #[cfg(not(desktop))]
    let shortcut = None;
    Settings {
        shortcut,
        alerts: app.state::<Monitor>().alert_settings(),
    }
}

/// Tells every window, so a change in one reaches the other.
fn broadcast(app: &AppHandle) -> Settings {
    let settings = current(app);
    if let Err(error) = app.emit(SETTINGS_EVENT, &settings) {
        eprintln!("could not send the settings: {error}");
    }
    settings
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    current(&app)
}

#[tauri::command]
pub fn set_alerts(app: AppHandle, alerts: AlertSettings) -> Settings {
    app.state::<Monitor>().set_alert_settings(alerts);
    write(&app, ALERTS, alerts);
    broadcast(&app)
}

/// Replaces the shortcut that shows the popover; `None` removes it. A
/// shortcut that can't be registered leaves the current one in place.
#[tauri::command]
pub fn set_shortcut(app: AppHandle, shortcut: Option<String>) -> Result<Settings, String> {
    #[cfg(desktop)]
    crate::shortcut::replace(&app, shortcut.as_deref())?;
    #[cfg(not(desktop))]
    let _ = shortcut;
    Ok(broadcast(&app))
}
