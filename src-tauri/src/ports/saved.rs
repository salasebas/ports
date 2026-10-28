//! Saved projects and the runs Ports starts for them.
//!
//! Projects, favorite scripts, the expected binding, the manual recovery policy
//! and the last launch result live in `projects.json` through tauri-plugin-store
//! (the app data directory). Logs of those runs are capped text files in the
//! app log directory. Ports stores no environment and no secrets: a launch
//! inherits the app's own environment.
//!
//! Script discovery reads `package.json` and chooses npm, pnpm, yarn or bun
//! from the lockfile. A custom command is stored as the user wrote it.
//! Recovery is manual, and it is not Restart: when a run Ports started exits,
//! the result is recorded and nothing starts it again.
//!
//! A run is detached (`setsid`, and SIGPIPE ignored) so it keeps going after
//! Ports quits. The pid is stored so the next time Ports opens it can see
//! whether that process is still alive.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use super::project;
use super::sampler::now_ms;
use crate::ports::Monitor;

const STORE: &str = "projects.json";
const KEY: &str = "projects";
const LOG_LIMIT: usize = 64 * 1024;
const COMMAND_LIMIT: usize = 4_096;

/// What starts a saved project again after it exits. Only manual recovery is
/// offered: Ports never restarts a saved project on its own.
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
    /// Display path, the same form as a server's `folder`.
    pub folder: String,
    pub command: String,
    pub favorites: Vec<String>,
    pub expected_port: u16,
    pub expected_address: String,
    pub recovery: Recovery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_result: Option<LaunchResult>,
    /// The process Ports started, when it may still be running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
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
    /// The script body from `package.json`.
    pub command: String,
    /// The command Ports runs for this script.
    pub run: String,
    pub favorite: bool,
    /// A favorite whose script is no longer in `package.json`.
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
    /// A listener matches the saved folder and port, or the process Ports started is alive.
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLog {
    pub text: String,
    /// True when older output was dropped to keep the file within its cap.
    pub truncated: bool,
}

/// A listener Ports already knows about, used to refuse a conflicting start.
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
    /// Something else already listens on the expected port. Do not stop it.
    Conflict(ConflictOwner),
    /// The saved project is the listener. This is not a new launch.
    AlreadyServing {
        server_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitPlan {
    pub result: LaunchResult,
    /// Manual recovery never continues into another launch.
    pub relaunch: bool,
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

/// A port already taken by another server is a conflict. The same folder on
/// that port is the saved project itself, already serving.
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

pub fn plan_exit(code: Option<i32>, stopped: bool, now: u64) -> ExitPlan {
    if stopped {
        return ExitPlan {
            result: LaunchResult {
                at: now,
                outcome: Outcome::Stopped,
                message: "Stopped. Ports will not start it again.".into(),
                exit_code: code,
                owner: None,
            },
            relaunch: false,
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
    ExitPlan {
        result: LaunchResult {
            at: now,
            outcome,
            message,
            exit_code: code,
            owner: None,
        },
        // Recovery is manual for every saved project.
        relaunch: false,
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

#[derive(Debug)]
struct LogBuf {
    data: Vec<u8>,
    truncated: bool,
}

impl LogBuf {
    fn push(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
        if self.data.len() > LOG_LIMIT {
            let drop = self.data.len() - LOG_LIMIT;
            self.data.drain(..drop);
            self.truncated = true;
        }
    }
}

fn alive(pid: u32) -> bool {
    pid > 1 && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
}

fn stop_pid(pid: u32) {
    if !alive(pid) {
        return;
    }
    unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if !alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
}

struct SharedLog {
    buf: Mutex<LogBuf>,
    path: PathBuf,
}

impl SharedLog {
    fn push(&self, chunk: &[u8]) {
        let mut buf = self
            .buf
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        buf.push(chunk);
        let _ = fs::write(&self.path, &buf.data);
    }
}

/// Starts `command` in `folder`, detached from Ports, and writes a capped log.
/// The returned pid is the leader of the new session. `on_exit` runs once,
/// with the exit code, and must not start the command again.
pub fn spawn_logged(
    folder: &Path,
    command: &str,
    log_path: &Path,
    on_exit: impl FnOnce(Option<i32>) + Send + 'static,
) -> Result<u32, String> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let _ = fs::write(log_path, b"");
    let shared = Arc::new(SharedLog {
        buf: Mutex::new(LogBuf {
            data: Vec::new(),
            truncated: false,
        }),
        path: log_path.to_path_buf(),
    });
    let mut process = Command::new("sh");
    process
        .arg("-c")
        .arg(command)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Safety: `setsid` and `signal` are async-signal-safe, and the closure
    // runs in the forked child before exec.
    unsafe {
        use std::os::unix::process::CommandExt;
        process.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            Ok(())
        });
    }
    let mut child = process.spawn().map_err(|error| {
        format!("Couldn't start the project: {error}. Check that the command can run from a shell.")
    })?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_log = Arc::clone(&shared);
    let err_log = Arc::clone(&shared);
    let stdout_thread = std::thread::spawn(move || copy_pipe(stdout, &out_log));
    let stderr_thread = std::thread::spawn(move || copy_pipe(stderr, &err_log));
    std::thread::spawn(move || {
        let status = child.wait().ok();
        let _ = stdout_thread.join();
        let _ = stderr_thread.join();
        on_exit(status.and_then(|status| status.code()));
    });
    Ok(pid)
}

fn copy_pipe(pipe: Option<impl Read>, log: &SharedLog) {
    let Some(mut pipe) = pipe else {
        return;
    };
    let mut chunk = [0_u8; 4096];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => log.push(&chunk[..n]),
        }
    }
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
                command: body.clone(),
                run: run_command(manager, name)?,
                favorite: project.favorites.iter().any(|favorite| favorite == name),
                missing: false,
            })
        })
        .collect();
    for favorite in &project.favorites {
        if !shown.iter().any(|script| script.name == *favorite) {
            shown.push(ScriptView {
                name: favorite.clone(),
                command: String::new(),
                run: favorite.clone(),
                favorite: true,
                missing: true,
            });
        }
    }
    let matched = listeners.iter().find(|listener| {
        listener.port == project.expected_port
            && listener.folder.as_deref() == Some(project.folder.as_str())
    });
    let running = matched.is_some() || project.pid.is_some_and(alive);
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

fn reconcile(projects: &mut [SavedProject], now: u64) {
    for project in projects {
        if project.pid.is_some_and(|pid| !alive(pid)) {
            let stopped = project
                .last_result
                .as_ref()
                .is_some_and(|result| result.outcome == Outcome::Stopped);
            project.pid = None;
            if !stopped {
                project.last_result = Some(plan_exit(None, false, now).result);
            }
        }
    }
}

#[tauri::command]
pub fn list_projects(app: AppHandle) -> Result<Vec<ProjectView>, String> {
    let projects = app.state::<Projects>();
    let _guard = projects.lock();
    let home = home_of(&app)?;
    let mut saved = load(&app);
    reconcile(&mut saved, now_ms());
    save(&app, &saved)?;
    let listeners = listeners_of(&app);
    Ok(saved
        .iter()
        .map(|project| view_of(&home, project, &listeners))
        .collect())
}

#[tauri::command]
pub fn save_project(app: AppHandle, request: SaveRequest) -> Result<ProjectView, String> {
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
            pid: None,
        });
    }
    save(&app, &saved)?;
    let project = saved
        .iter()
        .find(|project| project.id == stable_id(&path))
        .unwrap();
    Ok(view_of(&home, project, &listeners_of(&app)))
}

#[tauri::command]
pub fn forget_project(app: AppHandle, id: String) -> Result<(), String> {
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
}

#[tauri::command]
pub fn set_favorite(app: AppHandle, request: FavoriteRequest) -> Result<ProjectView, String> {
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
}

#[tauri::command]
pub fn set_project_command(app: AppHandle, request: CommandRequest) -> Result<ProjectView, String> {
    check_command(&request.command)?;
    update_project(&app, &request.id, |project| {
        project.command = request.command.trim().to_string();
        Ok(())
    })
}

#[tauri::command]
pub fn set_project_binding(app: AppHandle, request: BindingRequest) -> Result<ProjectView, String> {
    let address = check_binding(request.expected_port, &request.expected_address)?;
    update_project(&app, &request.id, |project| {
        project.expected_port = request.expected_port;
        project.expected_address = address;
        Ok(())
    })
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
pub fn start_project(app: AppHandle, id: String) -> Result<ProjectView, String> {
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
    let (command, port, pid) = {
        let project = saved.iter().find(|project| project.id == id).unwrap();
        (project.command.clone(), project.expected_port, project.pid)
    };
    if pid.is_some_and(alive) {
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
    let pid = spawn_logged(&path, &command, &log_path, move |code| {
        finish_run(&app_for_exit, &id_for_exit, code);
    })?;
    let project = require(&mut saved, &id)?;
    project.pid = Some(pid);
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
    let plan = plan_exit(code, stopped, now_ms());
    // `relaunch` is false for manual recovery. A finished run is only recorded.
    project.pid = None;
    if !stopped && !plan.relaunch {
        project.last_result = Some(plan.result);
    }
    let _ = save(app, &saved);
}

#[tauri::command]
pub fn stop_project(app: AppHandle, id: String) -> Result<ProjectView, String> {
    let projects = app.state::<Projects>();
    let _guard = projects.lock();
    let home = home_of(&app)?;
    let mut saved = load(&app);
    let pid = require(&mut saved, &id)?.pid;
    {
        let project = require(&mut saved, &id)?;
        project.last_result = Some(plan_exit(None, true, now_ms()).result);
    }
    save(&app, &saved)?;
    drop(_guard);
    if let Some(pid) = pid {
        stop_pid(pid);
    }
    let projects = app.state::<Projects>();
    let _guard = projects.lock();
    let mut saved = load(&app);
    if let Some(project) = saved.iter_mut().find(|project| project.id == id) {
        project.pid = None;
        project.last_result = Some(plan_exit(None, true, now_ms()).result);
    }
    save(&app, &saved)?;
    let project = require(&mut saved, &id)?;
    Ok(view_of(&home, project, &listeners_of(&app)))
}

#[tauri::command]
pub fn project_log(app: AppHandle, id: String) -> Result<ProjectLog, String> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_failed_run_is_not_started_again() {
        let failed = plan_exit(Some(1), false, 10);
        assert!(!failed.relaunch);
        assert_eq!(failed.result.outcome, Outcome::Failed);
        assert_eq!(failed.result.exit_code, Some(1));

        let stopped = plan_exit(None, true, 10);
        assert!(!stopped.relaunch);
        assert_eq!(stopped.result.outcome, Outcome::Stopped);
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
            pid: None,
        };
        let json = serde_json::to_string(&project).unwrap();
        let back: SavedProject = serde_json::from_str(&json).unwrap();
        assert_eq!(back.favorites, project.favorites);
        assert_eq!(back.recovery, Recovery::Manual);
    }

    #[test]
    fn the_log_keeps_only_the_newest_bytes() {
        let mut log = LogBuf {
            data: Vec::new(),
            truncated: false,
        };
        log.push(&vec![b'a'; LOG_LIMIT]);
        log.push(b"tail");
        assert_eq!(log.data.len(), LOG_LIMIT);
        assert!(log.truncated);
        assert!(log.data.ends_with(b"tail"));
    }

    #[test]
    fn a_detached_run_records_its_output_and_exit_without_spawning_again() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("run.log");
        let (sender, receiver) = std::sync::mpsc::channel();
        let pid = spawn_logged(dir.path(), "echo hello-from-ports", &log, move |code| {
            let _ = sender.send(code);
        })
        .unwrap();
        assert!(pid > 1);
        let code = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the run should exit");
        assert_eq!(code, Some(0));
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("hello-from-ports"));
        assert!(text.len() <= LOG_LIMIT);
        let plan = plan_exit(code, false, 1);
        assert!(!plan.relaunch);
    }

    #[test]
    fn a_long_run_is_capped_and_can_be_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("run.log");
        let (sender, receiver) = std::sync::mpsc::channel();
        let pid = spawn_logged(
            dir.path(),
            "dd if=/dev/zero bs=1024 count=80 status=none; echo done",
            &log,
            move |code| {
                let _ = sender.send(code);
            },
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if fs::metadata(&log).is_ok_and(|meta| meta.len() as usize >= LOG_LIMIT) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(fs::metadata(&log).unwrap().len() as usize <= LOG_LIMIT);
        stop_pid(pid);
        let _ = receiver.recv_timeout(Duration::from_secs(3));
        assert!(!alive(pid));
    }
}
