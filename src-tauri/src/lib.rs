use tauri::{
    AppHandle, Manager, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_positioner::{Position, WindowExt};

mod folders;

const MAIN_WINDOW: &str = "main";
const TRAY_WINDOW: &str = "tray";

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(TRAY_WINDOW) {
        let _ = window.hide();
    }
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_tray_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(TRAY_WINDOW) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    // Linux trays emit no click events, so there is no tray position to anchor to.
    if window
        .move_window_constrained(Position::TrayBottomCenter)
        .is_err()
    {
        let _ = window.move_window(Position::TopRight);
    }
    let _ = window.show();
    let _ = window.set_focus();
}

#[tauri::command]
fn open_main_window(app: AppHandle) {
    show_main(&app);
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// ⌥⌘P on macOS, Ctrl+Alt+P elsewhere, toggles the popover like a tray click.
#[cfg(desktop)]
fn register_popover_shortcut(app: &tauri::App) -> tauri::Result<()> {
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    let modifiers = if cfg!(target_os = "macos") {
        Modifiers::SUPER | Modifiers::ALT
    } else {
        Modifiers::CONTROL | Modifiers::ALT
    };
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    toggle_tray_window(app);
                }
            })
            .build(),
    )?;
    // Another app may own the combination; Ports still works from the tray.
    if let Err(error) = app
        .global_shortcut()
        .register(Shortcut::new(Some(modifiers), Code::KeyP))
    {
        eprintln!("could not register the popover shortcut: {error}");
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            #[cfg(desktop)]
            register_popover_shortcut(app)?;

            let quick_view =
                MenuItem::with_id(app, "quick-view", "Quick View", true, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Open Ports", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Ports", true, Some("CmdOrCtrl+Q"))?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&quick_view, &open, &separator, &quit])?;

            TrayIconBuilder::with_id("ports")
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true)
                .tooltip("Ports")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quick-view" => toggle_tray_window(app),
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_tray_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| match (window.label(), event) {
            (TRAY_WINDOW, WindowEvent::Focused(false)) => {
                let _ = window.hide();
            }
            (MAIN_WINDOW, WindowEvent::CloseRequested { api, .. }) => {
                // Keep running in the menu bar; quitting happens from the tray.
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            open_main_window,
            quit_app,
            folders::open_in_editor,
            folders::open_in_terminal,
            folders::reveal_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            show_main(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
