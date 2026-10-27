//! Opens a server's project folder in the user's editor, terminal or file manager.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Editor {
    Cursor,
    Vscode,
    Zed,
}

impl Editor {
    /// macOS launches apps by name; Linux runs the editor's command line launcher.
    fn program(&self) -> &'static str {
        #[cfg(target_os = "macos")]
        let program = match self {
            Editor::Cursor => "Cursor",
            Editor::Vscode => "Visual Studio Code",
            Editor::Zed => "Zed",
        };
        #[cfg(not(target_os = "macos"))]
        let program = match self {
            Editor::Cursor => "cursor",
            Editor::Vscode => "code",
            Editor::Zed => "zed",
        };
        program
    }
}

fn resolve(app: &AppHandle, folder: &str) -> Result<PathBuf, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    existing_dir(&home, folder)
}

fn existing_dir(home: &Path, folder: &str) -> Result<PathBuf, String> {
    let path = match folder.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(folder),
    };
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("{folder} doesn't exist"))
    }
}

/// `open -a` reports a missing app through its exit status, so wait for it.
#[cfg(target_os = "macos")]
fn open_with(program: &str, path: &PathBuf) -> Result<(), String> {
    let status = Command::new("open")
        .args(["-a", program])
        .arg(path)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} isn't installed"))
    }
}

#[tauri::command]
pub async fn open_in_editor(app: AppHandle, folder: String, editor: Editor) -> Result<(), String> {
    let path = resolve(&app, &folder)?;
    #[cfg(target_os = "macos")]
    return open_with(editor.program(), &path);
    #[cfg(not(target_os = "macos"))]
    Command::new(editor.program())
        .arg(&path)
        .spawn()
        .map(drop)
        .map_err(|_| format!("{} isn't installed", editor.program()))
}

#[tauri::command]
pub async fn open_in_terminal(app: AppHandle, folder: String) -> Result<(), String> {
    let path = resolve(&app, &folder)?;
    #[cfg(target_os = "macos")]
    return open_with("Terminal", &path);
    // Debian and Ubuntu point this alternative at the default terminal.
    #[cfg(not(target_os = "macos"))]
    Command::new("x-terminal-emulator")
        .current_dir(&path)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reveal_folder(app: AppHandle, folder: String) -> Result<(), String> {
    let path = resolve(&app, &folder)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_a_home_relative_folder() {
        let home = std::env::temp_dir();
        std::fs::create_dir_all(home.join("ports-test-project")).unwrap();
        assert_eq!(
            existing_dir(&home, "~/ports-test-project"),
            Ok(home.join("ports-test-project"))
        );
    }

    #[test]
    fn keeps_an_absolute_folder() {
        let dir = std::env::temp_dir();
        assert_eq!(
            existing_dir(Path::new("/nowhere"), dir.to_str().unwrap()),
            Ok(dir)
        );
    }

    #[test]
    fn reports_a_missing_folder_as_written_by_the_user() {
        assert_eq!(
            existing_dir(&std::env::temp_dir(), "~/ports-test-missing"),
            Err("~/ports-test-missing doesn't exist".to_string())
        );
    }
}
