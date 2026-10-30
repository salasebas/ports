use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    AppHandle, Emitter, Manager, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_positioner::{Position, WindowExt};

mod child_process;
mod folders;
mod ports;
mod settings;
#[cfg(desktop)]
mod shortcut;

const MAIN_WINDOW: &str = "main";
const TRAY_WINDOW: &str = "tray";
const OPEN_SETTINGS_EVENT: &str = "ports://open-settings";

#[derive(Default)]
struct TrayWindow {
    creating: Mutex<()>,
    opening: AtomicBool,
}

fn show_main<R: tauri::Runtime>(app: &AppHandle<R>) {
    app.state::<TrayWindow>()
        .opening
        .store(false, Ordering::Relaxed);
    if let Some(window) = app.get_webview_window(TRAY_WINDOW) {
        let _ = window.hide();
    }
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn tray_window<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::WebviewWindow<R>> {
    if let Some(window) = app.get_webview_window(TRAY_WINDOW) {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == TRAY_WINDOW)
        .ok_or(tauri::Error::WindowNotFound)?;
    tauri::WebviewWindowBuilder::from_config(app, config)?.build()
}

fn toggle_tray_window(app: &AppHandle) {
    let state = app.state::<TrayWindow>();
    if let Some(window) = app.get_webview_window(TRAY_WINDOW)
        && window.is_visible().unwrap_or(false)
    {
        state.opening.store(false, Ordering::Relaxed);
        let _ = window.hide();
        return;
    }
    if state.opening.fetch_xor(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<TrayWindow>();
        let _toggle = state
            .creating
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.opening.load(Ordering::Relaxed) {
            return;
        }
        let window = match tray_window(&app) {
            Ok(window) => window,
            Err(error) => {
                state.opening.store(false, Ordering::Relaxed);
                eprintln!("could not open the popover: {error}");
                return;
            }
        };
        if let Err(error) = app.run_on_main_thread(move || {
            if !window
                .state::<TrayWindow>()
                .opening
                .swap(false, Ordering::Relaxed)
            {
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
        }) {
            state.opening.store(false, Ordering::Relaxed);
            eprintln!("could not show the popover: {error}");
        }
    });
}

#[tauri::command]
fn open_main_window(app: AppHandle) {
    show_main(&app);
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_main(&app);
    if let Err(error) = app.emit_to(MAIN_WINDOW, OPEN_SETTINGS_EVENT, ()) {
        eprintln!("could not open the settings: {error}");
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
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
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .register_uri_scheme_protocol(ports::ICON_SCHEME, |context, request| {
            ports::serve_icon(context.app_handle(), request.uri().path())
        })
        .setup(|app| {
            app.manage(TrayWindow::default());
            #[cfg(desktop)]
            shortcut::init(app, toggle_tray_window)?;
            app.manage(settings::Changes::default());
            app.manage(ports::saved::Projects::open(app.handle()));
            app.manage(ports::pulls::PullCache::default());

            let log_dir = app
                .path()
                .app_log_dir()
                .or_else(|_| app.path().app_data_dir())?;
            if let Err(error) = std::fs::create_dir_all(&log_dir) {
                eprintln!("could not create the log folder: {error}");
            }
            app.manage(ports::Monitor::new(
                app.path().home_dir()?,
                settings::alerts(app.handle()),
                settings::stop_grace_seconds(app.handle()),
                settings::idle_after_ms(app.handle()),
                ports::rules::load(app.handle()),
                log_dir.join("auto-cleanup.log"),
            ));
            ports::start(app.handle())?;
            ports::open_alerts_on_click(app.handle(), show_main);

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
            (TRAY_WINDOW, WindowEvent::Focused(false)) if window.is_visible().unwrap_or(false) => {
                window
                    .state::<TrayWindow>()
                    .opening
                    .store(false, Ordering::Relaxed);
                let _ = window.hide();
            }
            (_, WindowEvent::Focused(true)) => window.state::<ports::Monitor>().window_shown(),
            (MAIN_WINDOW, WindowEvent::CloseRequested { api, .. }) => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            open_main_window,
            open_settings,
            quit_app,
            folders::open_in_editor,
            folders::open_in_terminal,
            folders::reveal_folder,
            ports::snapshot,
            ports::refresh,
            ports::set_refresh_interval,
            ports::preview_control,
            ports::stop_server,
            ports::restart_server,
            ports::saved::list_projects,
            ports::saved::save_project,
            ports::saved::forget_project,
            ports::saved::set_favorite,
            ports::saved::set_project_command,
            ports::saved::set_project_binding,
            ports::saved::start_project,
            ports::saved::stop_project,
            ports::saved::project_log,
            ports::pulls::pull_request,
            ports::reveal_session,
            ports::rules::get_rules,
            ports::rules::update_rules,
            ports::autoclean::auto_cleanup_log,
            settings::get_settings,
            settings::get_settings_defaults,
            settings::set_alerts,
            settings::set_stop_grace,
            settings::set_idle_after,
            settings::set_shortcut,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_main_cancels_a_pending_popover_without_a_window() {
        let app = tauri::test::mock_builder()
            .manage(TrayWindow::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.state::<TrayWindow>()
            .opening
            .store(true, Ordering::Relaxed);
        show_main(app.handle());
        assert!(!app.state::<TrayWindow>().opening.load(Ordering::Relaxed));
    }

    #[test]
    fn the_tray_webview_is_loaded_only_when_requested() {
        let app = tauri::test::mock_builder()
            .setup(|app| {
                assert!(app.get_webview_window(MAIN_WINDOW).is_some());
                assert!(app.get_webview_window(TRAY_WINDOW).is_none());
                let window = tray_window(app.handle())?;
                assert_eq!(window.url()?.path(), "/tray");
                let reused = tray_window(app.handle())?;
                assert_eq!(window.label(), reused.label());
                assert_eq!(app.webview_windows().len(), 2);
                Ok(())
            })
            .build(tauri::generate_context!())
            .unwrap();
        app.run_return(|app, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                app.get_webview_window(TRAY_WINDOW)
                    .unwrap()
                    .destroy()
                    .unwrap();
                app.get_webview_window(MAIN_WINDOW)
                    .unwrap()
                    .destroy()
                    .unwrap();
            }
        });
    }

    #[test]
    fn the_main_window_listens_to_the_open_settings_event() {
        let layout = include_str!("../../src/routes/+layout.svelte");
        assert!(layout.contains(&format!("= \"{OPEN_SETTINGS_EVENT}\";")));
    }
}
