use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::settings;

pub const DEFAULT: &str = if cfg!(target_os = "macos") {
    "Alt+Super+KeyP"
} else {
    "Control+Alt+KeyP"
};

struct Registered(Mutex<Option<Shortcut>>);

fn registered(app: &AppHandle) -> std::sync::MutexGuard<'_, Option<Shortcut>> {
    app.state::<Registered>()
        .inner()
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

enum Saved {
    Never,
    Removed,
    Chosen(String),
}

fn saved(app: &AppHandle) -> Saved {
    match settings::read::<Option<String>, _>(app, settings::SHORTCUT) {
        None => Saved::Never,
        Some(None) => Saved::Removed,
        Some(Some(text)) => Saved::Chosen(text),
    }
}

pub fn init(app: &tauri::App, toggle: fn(&AppHandle)) -> tauri::Result<()> {
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    toggle(app);
                }
            })
            .build(),
    )?;
    let text = match saved(app.handle()) {
        Saved::Never => Some(DEFAULT.to_string()),
        Saved::Removed => None,
        Saved::Chosen(text) => Some(text),
    };
    let shortcut = text.and_then(|text| parse(&text).ok());
    let shortcut = shortcut.filter(|shortcut| match app.global_shortcut().register(*shortcut) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("could not register the popover shortcut: {error}");
            false
        }
    });
    app.manage(Registered(Mutex::new(shortcut)));
    Ok(())
}

pub fn current(app: &AppHandle) -> Option<String> {
    registered(app).map(Shortcut::into_string)
}

fn parse(text: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = text
        .parse()
        .map_err(|_| format!("{text} isn't a shortcut Ports can use"))?;
    if !shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err("Use Control, Option or Command in the shortcut".to_string());
    }
    Ok(shortcut)
}

pub fn replace(app: &AppHandle, text: Option<&str>) -> Result<(), String> {
    let next = text.map(parse).transpose()?;
    let mut current = registered(app);
    if next == *current {
        return Ok(());
    }
    if let Some(next) = next {
        app.global_shortcut().register(next).map_err(|error| {
            format!("Couldn't use this shortcut; another app may be using it ({error})")
        })?;
    }
    if let Err(error) = settings::write(app, settings::SHORTCUT, next.map(Shortcut::into_string)) {
        if let Some(next) = next {
            let _ = app.global_shortcut().unregister(next);
        }
        return Err(error);
    }
    if let Some(previous) = *current
        && let Err(error) = app.global_shortcut().unregister(previous)
    {
        if let Some(next) = next {
            let _ = app.global_shortcut().unregister(next);
        }
        if let Err(rollback) =
            settings::write(app, settings::SHORTCUT, current.map(Shortcut::into_string))
        {
            eprintln!("could not restore the saved shortcut: {rollback}");
        }
        return Err(format!("Couldn't release the previous shortcut ({error})"));
    }
    *current = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_shortcuts_the_recorder_sends() {
        assert_eq!(
            parse("Control+Shift+Digit1").unwrap().into_string(),
            "shift+control+Digit1"
        );
        assert_eq!(
            parse("Alt+Super+Space").unwrap().into_string(),
            "alt+super+Space"
        );
        assert_eq!(parse(DEFAULT).unwrap(), parse(DEFAULT).unwrap());
    }

    #[test]
    fn refuses_shortcuts_without_a_command_modifier() {
        assert!(parse("Shift+KeyP").is_err());
        assert!(parse("KeyP").is_err());
        assert!(parse("Super+Nope").is_err());
    }
}
