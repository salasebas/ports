use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;

use super::blocking;
use super::identity::ProcessIdentity;
use super::project;
use super::sampler::now_ms;
use crate::ports::Monitor;

const STORE: &str = "projects.json";
const KEY: &str = "projects";
const PROJECTS_EVENT: &str = "ports://projects";
const LOG_LIMIT: usize = 64 * 1024;
const TRIM_EVERY: Duration = Duration::from_secs(1);
const REAP_EVERY: Duration = Duration::from_millis(50);
const COMMAND_LIMIT: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Recovery {
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Started,
    Exited,
    Failed,
    Conflict,
    MissingFolder,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictOwner {
    pub port: u16,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResult {
    pub at: u64,
    pub outcome: Outcome,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<ConflictOwner>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedProject {
    pub id: String,
    pub folder: String,
    pub command: String,
    pub favorites: Vec<String>,
    pub expected_port: u16,
    pub expected_address: String,
    pub recovery: Recovery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_result: Option<LaunchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptView {
    pub name: String,
    pub body: String,
    pub run_command: String,
    pub favorite: bool,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: SavedProject,
    pub manager: PackageManager,
    pub scripts: Vec<ScriptView>,
    pub folder_missing: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLog {
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub id: String,
    pub port: u16,
    pub name: String,
    pub pid: Option<u32>,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortState {
    Free,
    Conflict(ConflictOwner),
    AlreadyServing { server_id: String },
}

pub struct Projects {
    log_dir: PathBuf,
    gate: Mutex<()>,
}

impl Projects {
    pub fn open(app: &AppHandle) -> Self {
        let log_dir = app
            .path()
            .app_log_dir()
            .or_else(|_| app.path().app_data_dir())
            .unwrap_or_else(|_| std::env::temp_dir().join("ports"))
            .join("runs");
        let _ = fs::create_dir_all(&log_dir);
        Self {
            log_dir,
            gate: Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    folder: String,
    command: String,
    expected_port: u16,
    #[serde(default)]
    expected_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingRequest {
    id: String,
    expected_port: u16,
    expected_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteRequest {
    id: String,
    script: String,
    favorite: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRequest {
    id: String,
    command: String,
}

pub fn package_manager(folder: &Path) -> PackageManager {
    if folder.join("bun.lock").exists() || folder.join("bun.lockb").exists() {
        PackageManager::Bun
    } else if folder.join("pnpm-lock.yaml").exists() {
        PackageManager::Pnpm
    } else if folder.join("yarn.lock").exists() {
        PackageManager::Yarn
    } else {
        PackageManager::Npm
    }
}

pub fn run_command(manager: PackageManager, script: &str) -> Option<String> {
    if !is_script_name(script) {
        return None;
    }
    Some(match manager {
        PackageManager::Bun => format!("bun run {script}"),
        PackageManager::Pnpm => format!("pnpm run {script}"),
        PackageManager::Yarn => format!("yarn {script}"),
        PackageManager::Npm => format!("npm run {script}"),
    })
}

pub fn package_scripts(folder: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(folder.join("package.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(|scripts| scripts.as_object()) else {
        return Vec::new();
    };
    let mut found: Vec<_> = scripts
        .iter()
        .filter_map(|(name, body)| {
            if !is_script_name(name) {
                return None;
            }
            Some((name.clone(), body.as_str()?.to_string()))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

fn is_script_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("..")
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'@' | b'/')
        })
}

pub fn classify_port(folder: &str, port: u16, listeners: &[Listener]) -> PortState {
    match listeners.iter().find(|listener| listener.port == port) {
        Some(listener) if listener.folder.as_deref() == Some(folder) => PortState::AlreadyServing {
            server_id: listener.id.clone(),
        },
        Some(listener) => PortState::Conflict(ConflictOwner {
            port: listener.port,
            name: listener.name.clone(),
            pid: listener.pid,
            server_id: Some(listener.id.clone()),
        }),
        None => PortState::Free,
    }
}

pub fn exit_result(code: Option<i32>, stopped: bool, now: u64) -> LaunchResult {
    if stopped {
        return LaunchResult {
            at: now,
            outcome: Outcome::Stopped,
            message: "Stopped. Ports will not start it again.".into(),
            exit_code: code,
            owner: None,
        };
    }
    let (outcome, message) = match code {
        Some(0) => (Outcome::Exited, "The project exited.".into()),
        Some(code) => (
            Outcome::Failed,
            format!("The project exited with status {code}."),
        ),
        None => (
            Outcome::Failed,
            "The project stopped without an exit status.".into(),
        ),
    };
    LaunchResult {
        at: now,
        outcome,
        message,
        exit_code: code,
        owner: None,
    }
}

pub fn stable_id(canonical: &Path) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in canonical.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn alive(process: &ProcessIdentity) -> bool {
    process.pid > 1 && process.is_current()
}

fn stop_run(process: &ProcessIdentity) {
    if !alive(process) {
        return;
    }
    let group = -(process.pid as libc::pid_t);
    unsafe { libc::kill(group, libc::SIGTERM) };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if !alive(process) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if alive(process) {
        unsafe { libc::kill(group, libc::SIGKILL) };
    }
}

fn trim_log(path: &Path) {
    let Ok(mut file) = fs::OpenOptions::new().read(true).append(true).open(path) else {
        return;
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return;
    };
    let limit = LOG_LIMIT as u64;
    if len <= 2 * limit || file.seek(SeekFrom::Start(len - limit)).is_err() {
        return;
    }
    let mut tail = Vec::with_capacity(LOG_LIMIT);
    if Read::by_ref(&mut file)
        .take(limit)
        .read_to_end(&mut tail)
        .is_err()
    {
        return;
    }
    if file.set_len(0).is_ok() {
        let _ = file.write_all(&tail);
    }
}

pub fn spawn_logged(
    folder: &Path,
    command: &str,
    log_path: &Path,
    on_exit: impl FnOnce(Option<i32>) + Send + 'static,
) -> Result<ProcessIdentity, String> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|log| log.set_len(0).map(|()| log))
        .map_err(|error| format!("Couldn't open the project's log: {error}"))?;
    let errors = log.try_clone().map_err(|error| error.to_string())?;
    let mut process = Command::new("sh");
    process
        .arg("-c")
        .arg(command)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errors));
    // Safety: `setsid` is async-signal-safe, and the closure runs in the
    // forked child before exec.
    unsafe {
        use std::os::unix::process::CommandExt;
        process.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = process.spawn().map_err(|error| {
        format!("Couldn't start the project: {error}. Check that the command can run from a shell.")
    })?;
    let pid = child.id();
    // Until this thread reaps it, the child exists (at least as a zombie), so
    // its start time can be read even if it already exited.
    let identity = ProcessIdentity::read(pid).ok_or("Couldn't read the started process.")?;
    let log_path = log_path.to_path_buf();
    std::thread::spawn(move || {
        // Reaped promptly: until then an exited leader is a zombie that still
        // looks alive, and Stop would escalate to SIGKILL.
        let mut trimmed = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if trimmed.elapsed() >= TRIM_EVERY {
                        trim_log(&log_path);
                        trimmed = std::time::Instant::now();
                    }
                    std::thread::sleep(REAP_EVERY);
                }
                Err(_) => break None,
            }
        };
        trim_log(&log_path);
        on_exit(status.and_then(|status| status.code()));
    });
    Ok(identity)
}

fn home_of(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .home_dir()
        .map(|path| project::canonical(&path))
        .map_err(|error| error.to_string())
}

fn resolve_folder(home: &Path, folder: &str) -> PathBuf {
    let path = match folder.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(folder),
    };
    project::canonical(&path)
}

fn display_folder(home: &Path, canonical: &Path) -> String {
    project::display_path(canonical, home)
}

fn load(app: &AppHandle) -> Vec<SavedProject> {
    let Ok(store) = app.store(STORE) else {
        return Vec::new();
    };
    store
        .get(KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, projects: &[SavedProject]) -> Result<(), String> {
    let store = app.store(STORE).map_err(|error| error.to_string())?;
    let previous = store.get(KEY);
    store.set(
        KEY,
        serde_json::to_value(projects).map_err(|error| error.to_string())?,
    );
    if let Err(error) = store.save() {
        match previous {
            Some(value) => store.set(KEY, value),
            None => {
                store.delete(KEY);
            }
        }
        return Err(format!("Could not save projects: {error}"));
    }
    Ok(())
}

fn listeners_of(app: &AppHandle) -> Vec<Listener> {
    app.state::<Monitor>()
        .servers(app)
        .into_iter()
        .map(|server| Listener {
            id: server.id,
            port: server.port,
            name: server.name,
            pid: server.pid,
            folder: server.folder,
        })
        .collect()
}

fn view_of(home: &Path, project: &SavedProject, listeners: &[Listener]) -> ProjectView {
    let path = resolve_folder(home, &project.folder);
    let missing = !path.is_dir();
    let manager = package_manager(&path);
    let scripts = package_scripts(&path);
    let mut shown: Vec<ScriptView> = scripts
        .iter()
        .filter_map(|(name, body)| {
            Some(ScriptView {
                name: name.clone(),
                body: body.clone(),
                run_command: run_command(manager, name)?,
                favorite: project.favorites.iter().any(|favorite| favorite == name),
                missing: false,
            })
        })
        .collect();
    for favorite in &project.favorites {
        if !shown.iter().any(|script| script.name == *favorite) {
            shown.push(ScriptView {
                name: favorite.clone(),
                body: String::new(),
                run_command: favorite.clone(),
                favorite: true,
                missing: true,
            });
        }
    }
    let matched = listeners.iter().find(|listener| {
        listener.port == project.expected_port
            && listener.folder.as_deref() == Some(project.folder.as_str())
    });
    let running = matched.is_some() || project.process.as_ref().is_some_and(alive);
    ProjectView {
        project: project.clone(),
        manager,
        scripts: shown,
        folder_missing: missing,
        running,
        matched_server_id: matched.map(|listener| listener.id.clone()),
    }
}

fn require<'a>(projects: &'a mut [SavedProject], id: &str) -> Result<&'a mut SavedProject, String> {
    projects
        .iter_mut()
        .find(|project| project.id == id)
        .ok_or_else(|| "That saved project is no longer on this Mac.".to_string())
}

fn check_command(command: &str) -> Result<(), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("The project needs a command to start.".into());
    }
    if command.len() > COMMAND_LIMIT {
        return Err("That command is too long.".into());
    }
    Ok(())
}

fn check_binding(port: u16, address: &str) -> Result<String, String> {
    if port == 0 {
        return Err("Choose the port the project should listen on.".into());
    }
    let address = address.trim();
    if address.is_empty() || address.len() > 64 || address.chars().any(char::is_whitespace) {
        return Err("The expected address should look like 127.0.0.1.".into());
    }
    Ok(address.to_string())
}

fn reconcile(projects: &mut [SavedProject], now: u64) -> bool {
    let mut changed = false;
    for project in projects {
        if project
            .process
            .as_ref()
            .is_some_and(|process| !alive(process))
        {
            let stopped = project
                .last_result
                .as_ref()
                .is_some_and(|result| result.outcome == Outcome::Stopped);
            project.process = None;
            if !stopped {
                project.last_result = Some(exit_result(None, false, now));
            }
            changed = true;
        }
    }
    changed
}

#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectView>, String> {
    blocking(move || {
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let home = home_of(&app)?;
        let mut saved = load(&app);
        if reconcile(&mut saved, now_ms()) {
            save(&app, &saved)?;
        }
        for project in &saved {
            trim_log(&projects.log_dir.join(format!("{}.log", project.id)));
        }
        let listeners = listeners_of(&app);
        Ok(saved
            .iter()
            .map(|project| view_of(&home, project, &listeners))
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn save_project(app: AppHandle, request: SaveRequest) -> Result<ProjectView, String> {
    blocking(move || {
        check_command(&request.command)?;
        let address = check_binding(
            request.expected_port,
            request.expected_address.as_deref().unwrap_or("127.0.0.1"),
        )?;
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let home = home_of(&app)?;
        let path = resolve_folder(&home, &request.folder);
        if !path.is_dir() {
            return Err(format!(
                "The project folder {} is missing.",
                request.folder.trim()
            ));
        }
        let folder = display_folder(&home, &path);
        let id = stable_id(&path);
        let mut saved = load(&app);
        if let Some(existing) = saved.iter_mut().find(|project| project.id == id) {
            existing.folder = folder;
            existing.command = request.command.trim().to_string();
            existing.expected_port = request.expected_port;
            existing.expected_address = address;
            existing.recovery = Recovery::Manual;
        } else {
            saved.push(SavedProject {
                id,
                folder,
                command: request.command.trim().to_string(),
                favorites: Vec::new(),
                expected_port: request.expected_port,
                expected_address: address,
                recovery: Recovery::Manual,
                last_result: None,
                process: None,
            });
        }
        save(&app, &saved)?;
        let project = saved
            .iter()
            .find(|project| project.id == stable_id(&path))
            .unwrap();
        Ok(view_of(&home, project, &listeners_of(&app)))
    })
    .await
}

#[tauri::command]
pub async fn forget_project(app: AppHandle, id: String) -> Result<(), String> {
    blocking(move || {
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let mut saved = load(&app);
        let before = saved.len();
        saved.retain(|project| project.id != id);
        if saved.len() == before {
            return Err("That saved project is no longer on this Mac.".into());
        }
        save(&app, &saved)?;
        let _ = fs::remove_file(projects.log_dir.join(format!("{id}.log")));
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn set_favorite(app: AppHandle, request: FavoriteRequest) -> Result<ProjectView, String> {
    blocking(move || {
        if !is_script_name(&request.script) {
            return Err("That script name can't be saved.".into());
        }
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let home = home_of(&app)?;
        let mut saved = load(&app);
        {
            let project = require(&mut saved, &request.id)?;
            if request.favorite {
                if !project
                    .favorites
                    .iter()
                    .any(|favorite| favorite == &request.script)
                {
                    project.favorites.push(request.script);
                }
            } else {
                project
                    .favorites
                    .retain(|favorite| favorite != &request.script);
            }
        }
        save(&app, &saved)?;
        let project = saved
            .iter()
            .find(|project| project.id == request.id)
            .unwrap();
        Ok(view_of(&home, project, &listeners_of(&app)))
    })
    .await
}

#[tauri::command]
pub async fn set_project_command(
    app: AppHandle,
    request: CommandRequest,
) -> Result<ProjectView, String> {
    blocking(move || {
        check_command(&request.command)?;
        update_project(&app, &request.id, |project| {
            project.command = request.command.trim().to_string();
            Ok(())
        })
    })
    .await
}

#[tauri::command]
pub async fn set_project_binding(
    app: AppHandle,
    request: BindingRequest,
) -> Result<ProjectView, String> {
    blocking(move || {
        let address = check_binding(request.expected_port, &request.expected_address)?;
        update_project(&app, &request.id, |project| {
            project.expected_port = request.expected_port;
            project.expected_address = address;
            Ok(())
        })
    })
    .await
}

fn update_project(
    app: &AppHandle,
    id: &str,
    change: impl FnOnce(&mut SavedProject) -> Result<(), String>,
) -> Result<ProjectView, String> {
    let projects = app.state::<Projects>();
    let _guard = projects.lock();
    let home = home_of(app)?;
    let mut saved = load(app);
    change(require(&mut saved, id)?)?;
    save(app, &saved)?;
    let project = saved.iter().find(|project| project.id == id).unwrap();
    Ok(view_of(&home, project, &listeners_of(app)))
}

#[tauri::command]
pub async fn start_project(app: AppHandle, id: String) -> Result<ProjectView, String> {
    blocking(move || {
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let home = home_of(&app)?;
        let mut saved = load(&app);
        let path = {
            let project = require(&mut saved, &id)?;
            resolve_folder(&home, &project.folder)
        };
        let folder_label = {
            let project = saved.iter().find(|project| project.id == id).unwrap();
            project.folder.clone()
        };
        if !path.is_dir() {
            let project = require(&mut saved, &id)?;
            project.last_result = Some(LaunchResult {
                at: now_ms(),
                outcome: Outcome::MissingFolder,
                message: format!("The project folder {folder_label} is missing."),
                exit_code: None,
                owner: None,
            });
            save(&app, &saved)?;
            return Err(format!("The project folder {folder_label} is missing."));
        }
        let listeners = listeners_of(&app);
        let (command, port, process) = {
            let project = saved.iter().find(|project| project.id == id).unwrap();
            (
                project.command.clone(),
                project.expected_port,
                project.process.clone(),
            )
        };
        if process.as_ref().is_some_and(alive) {
            return Err("Ports already started this project. Stop it before starting it again.".into());
        }
        match classify_port(&folder_label, port, &listeners) {
            PortState::AlreadyServing { .. } => {
                return Err(format!(
                    "This project is already listening on :{port}. Restart stays on that server; Start is for when it is stopped."
                ));
            }
            PortState::Conflict(owner) => {
                let message = match owner.pid {
                    Some(pid) => format!(
                        "Port {port} is already used by {} (PID {pid}). Ports left it running.",
                        owner.name
                    ),
                    None => format!(
                        "Port {port} is already used by {}. Ports left it running.",
                        owner.name
                    ),
                };
                let project = require(&mut saved, &id)?;
                project.last_result = Some(LaunchResult {
                    at: now_ms(),
                    outcome: Outcome::Conflict,
                    message: message.clone(),
                    exit_code: None,
                    owner: Some(owner),
                });
                save(&app, &saved)?;
                return Err(message);
            }
            PortState::Free => {}
        }
        let log_path = projects.log_dir.join(format!("{id}.log"));
        let app_for_exit = app.clone();
        let id_for_exit = id.clone();
        let process = spawn_logged(&path, &command, &log_path, move |code| {
            finish_run(&app_for_exit, &id_for_exit, code);
        })?;
        let project = require(&mut saved, &id)?;
        project.process = Some(process);
        project.last_result = Some(LaunchResult {
            at: now_ms(),
            outcome: Outcome::Started,
            message: format!("Started in {folder_label}."),
            exit_code: None,
            owner: None,
        });
        save(&app, &saved)?;
        let project = saved.iter().find(|project| project.id == id).unwrap();
        Ok(view_of(&home, project, &listeners))
    })
    .await
}

fn finish_run(app: &AppHandle, id: &str, code: Option<i32>) {
    let projects = app.state::<Projects>();
    let _guard = projects.lock();
    let mut saved = load(app);
    let Some(project) = saved.iter_mut().find(|project| project.id == id) else {
        return;
    };
    let stopped = project
        .last_result
        .as_ref()
        .is_some_and(|result| result.outcome == Outcome::Stopped);
    project.process = None;
    if !stopped {
        project.last_result = Some(exit_result(code, false, now_ms()));
    }
    let _ = save(app, &saved);
    if let Err(error) = app.emit(PROJECTS_EVENT, ()) {
        eprintln!("could not announce a finished run: {error}");
    }
}

#[tauri::command]
pub async fn stop_project(app: AppHandle, id: String) -> Result<ProjectView, String> {
    blocking(move || {
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let home = home_of(&app)?;
        let mut saved = load(&app);
        let process = require(&mut saved, &id)?.process.clone();
        {
            let project = require(&mut saved, &id)?;
            project.last_result = Some(exit_result(None, true, now_ms()));
        }
        save(&app, &saved)?;
        drop(_guard);
        if let Some(process) = process {
            stop_run(&process);
        }
        let projects = app.state::<Projects>();
        let _guard = projects.lock();
        let mut saved = load(&app);
        if let Some(project) = saved.iter_mut().find(|project| project.id == id) {
            project.process = None;
            project.last_result = Some(exit_result(None, true, now_ms()));
        }
        save(&app, &saved)?;
        let project = require(&mut saved, &id)?;
        Ok(view_of(&home, project, &listeners_of(&app)))
    })
    .await
}

#[tauri::command]
pub async fn project_log(app: AppHandle, id: String) -> Result<ProjectLog, String> {
    blocking(move || {
        let projects = app.state::<Projects>();
        let path = projects.log_dir.join(format!("{id}.log"));
        let Ok(bytes) = fs::read(&path) else {
            return Ok(ProjectLog {
                text: String::new(),
                truncated: false,
            });
        };
        let truncated = bytes.len() >= LOG_LIMIT;
        let bytes = if truncated {
            &bytes[bytes.len() - LOG_LIMIT..]
        } else {
            &bytes[..]
        };
        Ok(ProjectLog {
            text: String::from_utf8_lossy(bytes).into_owned(),
            truncated,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_listen_to_the_projects_event() {
        let windows = include_str!("../../../src/lib/ports/ports-store.svelte.ts");
        assert!(windows.contains(&format!("= \"{PROJECTS_EVENT}\";")));
    }

    #[test]
    fn discovers_package_scripts_and_the_lockfile_manager() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("bun.lock"), "").unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"dev":"vite","build":"vite build","bad name":"no","../x":"no"}}"#,
        )
        .unwrap();
        assert_eq!(package_manager(dir.path()), PackageManager::Bun);
        assert_eq!(
            package_scripts(dir.path()),
            vec![
                ("build".into(), "vite build".into()),
                ("dev".into(), "vite".into()),
            ]
        );
        assert_eq!(
            run_command(PackageManager::Bun, "dev").as_deref(),
            Some("bun run dev")
        );
        assert!(run_command(PackageManager::Npm, "bad name").is_none());
    }

    #[test]
    fn a_conflict_names_the_owner_and_does_not_treat_it_as_free() {
        let listeners = vec![Listener {
            id: "process:1:3000".into(),
            port: 3000,
            name: "web".into(),
            pid: Some(42),
            folder: Some("~/other".into()),
        }];
        match classify_port("~/shop", 3000, &listeners) {
            PortState::Conflict(owner) => {
                assert_eq!(owner.name, "web");
                assert_eq!(owner.pid, Some(42));
                assert_eq!(owner.server_id.as_deref(), Some("process:1:3000"));
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert!(matches!(
            classify_port("~/other", 3000, &listeners),
            PortState::AlreadyServing { .. }
        ));
        assert!(matches!(
            classify_port("~/shop", 5173, &listeners),
            PortState::Free
        ));
    }

    #[test]
    fn an_exit_is_recorded_with_its_outcome() {
        let failed = exit_result(Some(1), false, 10);
        assert_eq!(failed.outcome, Outcome::Failed);
        assert_eq!(failed.exit_code, Some(1));

        let stopped = exit_result(None, true, 10);
        assert_eq!(stopped.outcome, Outcome::Stopped);
    }

    #[test]
    fn favorites_round_trip_through_the_project_record() {
        let project = SavedProject {
            id: "abc".into(),
            folder: "~/shop".into(),
            command: "bun run dev".into(),
            favorites: vec!["dev".into(), "test".into()],
            expected_port: 5173,
            expected_address: "127.0.0.1".into(),
            recovery: Recovery::Manual,
            last_result: None,
            process: None,
        };
        let json = serde_json::to_string(&project).unwrap();
        let back: SavedProject = serde_json::from_str(&json).unwrap();
        assert_eq!(back.favorites, project.favorites);
        assert_eq!(back.recovery, Recovery::Manual);
    }

    #[test]
    fn the_log_keeps_only_the_newest_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("run.log");
        let mut text = vec![b'a'; 2 * LOG_LIMIT];
        text.extend_from_slice(b"tail");
        fs::write(&log, &text).unwrap();
        trim_log(&log);
        let kept = fs::read(&log).unwrap();
        assert_eq!(kept.len(), LOG_LIMIT);
        assert!(kept.ends_with(b"tail"));

        let within = vec![b'b'; LOG_LIMIT + 4];
        fs::write(&log, &within).unwrap();
        trim_log(&log);
        assert_eq!(fs::read(&log).unwrap(), within);
    }

    // A run that wrote to a pipe Ports reads would hit EPIPE, and Node exit,
    // once Ports quits. The log file has to be the run's own output.
    #[test]
    fn a_detached_run_writes_to_its_log_and_records_its_exit_without_spawning_again() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("run.log");
        let (sender, receiver) = std::sync::mpsc::channel();
        let process = spawn_logged(
            dir.path(),
            "echo hello-from-ports; [ -p /dev/stdout ] && echo stdout-is-a-pipe; true",
            &log,
            move |code| {
                let _ = sender.send(code);
            },
        )
        .unwrap();
        assert!(process.pid > 1);
        let code = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the run should exit");
        assert_eq!(code, Some(0));
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("hello-from-ports"));
        assert!(!text.contains("stdout-is-a-pipe"));
    }

    #[test]
    fn a_long_run_is_capped_and_can_be_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("run.log");
        let (sender, receiver) = std::sync::mpsc::channel();
        let process = spawn_logged(
            dir.path(),
            "dd if=/dev/zero bs=1024 count=160 status=none; echo done; sleep 30",
            &log,
            move |code| {
                let _ = sender.send(code);
            },
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut trimmed = false;
        while std::time::Instant::now() < deadline {
            let text = fs::read(&log).unwrap_or_default();
            if text.len() == LOG_LIMIT && text.ends_with(b"done\n") {
                trimmed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(trimmed, "the log should be trimmed to its newest bytes");
        assert!(alive(&process));
        stop_run(&process);
        let _ = receiver.recv_timeout(Duration::from_secs(3));
        assert!(!alive(&process));
    }

    #[test]
    fn a_reused_pid_is_not_the_run_and_is_not_stopped() {
        use std::os::unix::process::CommandExt;
        let mut other = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let reused = ProcessIdentity {
            pid: other.id(),
            start: "0:0".into(),
        };
        assert!(!alive(&reused));
        stop_run(&reused);
        assert!(
            other.try_wait().unwrap().is_none(),
            "the other process should keep running"
        );
        let _ = other.kill();
        let _ = other.wait();
    }
}
