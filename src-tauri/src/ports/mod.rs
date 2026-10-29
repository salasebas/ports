mod alerts;
mod containers;
mod control;
mod health;
mod history;
mod icons;
mod identity;
mod network;
mod pages;
mod place;
mod processes;
mod project;
pub mod pulls;
mod sampler;
pub mod saved;
mod session;
mod sockets;
mod supervisors;

pub use alerts::AlertSettings;
use containers::ContainerTarget;
use control::{Preview, Shutdown, Target};
pub use icons::SCHEME as ICON_SCHEME;
use identity::ProcessIdentity;

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::http::Response;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

use alerts::Alerts;
use icons::Icons;
use pages::Pages;
use sampler::{Sampler, Snapshot, now_ms};

const SNAPSHOT_EVENT: &str = "ports://snapshot";
pub const DETAILS_EVENT: &str = "ports://details";
const DEFAULT_INTERVAL_MS: u64 = 2_000;
const NEVER: u64 = 0;
const INTERVALS_MS: RangeInclusive<u64> = 500..=60_000;
const HIDDEN_INTERVAL_MS: u64 = 30_000;

enum Located {
    Process(Box<control::Service>),
    Container(ContainerTarget),
}

pub struct Monitor {
    home: PathBuf,
    sampler: Mutex<Sampler>,
    publication: Mutex<()>,
    latest: Mutex<Option<Snapshot>>,
    interval_ms: AtomicU64,
    woken: Mutex<bool>,
    wake: Condvar,
    stopping: Mutex<HashSet<String>>,
    health: Mutex<Health>,
    pages: Mutex<Pages>,
    icons: Arc<Icons>,
    alerts: Mutex<Alerts>,
    alert_settings: Mutex<AlertSettings>,
    stop_grace_secs: AtomicU64,
}

#[derive(Default)]
struct Health {
    unresponsive: HashSet<health::Key>,
    probing: bool,
    last_round_at: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Monitor {
    pub fn new(home: PathBuf, alert_settings: AlertSettings, stop_grace_secs: u64) -> Self {
        let icons = Arc::new(Icons::default());
        Self {
            sampler: Mutex::new(Sampler::new(home.clone(), icons.clone())),
            publication: Mutex::new(()),
            home,
            latest: Mutex::new(None),
            interval_ms: AtomicU64::new(DEFAULT_INTERVAL_MS),
            woken: Mutex::new(false),
            wake: Condvar::new(),
            stopping: Mutex::new(HashSet::new()),
            health: Mutex::new(Health::default()),
            pages: Mutex::new(Pages::default()),
            icons,
            alerts: Mutex::new(Alerts::default()),
            alert_settings: Mutex::new(alert_settings),
            stop_grace_secs: AtomicU64::new(clamp_grace(stop_grace_secs)),
        }
    }

    fn sample(&self) -> Result<Snapshot, String> {
        let mut snapshot = lock(&self.sampler).sample()?;
        lock(&self.pages).apply(&mut snapshot.servers);
        health::apply(&lock(&self.health).unresponsive, &mut snapshot.servers);
        Ok(snapshot)
    }

    fn publish(&self, app: &AppHandle) -> Result<Snapshot, String> {
        let _publication = lock(&self.publication);
        let snapshot = self.sample()?;
        *lock(&self.latest) = Some(snapshot.clone());
        if let Err(error) = app.emit(SNAPSHOT_EVENT, &snapshot) {
            eprintln!("could not send a snapshot: {error}");
        }
        self.alert(app, &snapshot);
        self.probe_if_due(app, &snapshot);
        self.check_pages_if_due(app, &snapshot);
        Ok(snapshot)
    }

    pub fn alert_settings(&self) -> AlertSettings {
        *lock(&self.alert_settings)
    }

    pub fn set_alert_settings(&self, settings: AlertSettings) {
        *lock(&self.alert_settings) = settings;
    }

    pub fn stop_grace_secs(&self) -> u64 {
        self.stop_grace_secs.load(Ordering::Relaxed)
    }

    pub fn set_stop_grace_secs(&self, secs: u64) -> u64 {
        let secs = clamp_grace(secs);
        self.stop_grace_secs.store(secs, Ordering::Relaxed);
        secs
    }

    pub(crate) fn latest_servers(&self) -> Option<Vec<sampler::Server>> {
        lock(&self.latest)
            .as_ref()
            .map(|snapshot| snapshot.servers.clone())
    }

    pub(crate) fn servers(&self, app: &AppHandle) -> Vec<sampler::Server> {
        if self.latest_servers().is_none() {
            let _ = self.publish(app);
        }
        self.latest_servers().unwrap_or_default()
    }

    fn shutdown(&self, force: bool) -> Shutdown {
        Shutdown {
            grace: Duration::from_secs(self.stop_grace_secs()),
            force,
        }
    }

    fn alert(&self, app: &AppHandle, snapshot: &Snapshot) {
        let settings = self.alert_settings();
        let due = lock(&self.alerts).due(settings, &snapshot.servers, now_ms());
        for alert in due {
            let mut hash = DefaultHasher::new();
            alert.server_id.hash(&mut hash);
            let shown = app
                .notification()
                .builder()
                .id(hash.finish() as i32)
                .extra("serverId", &alert.server_id)
                .title(alert.title)
                .body(alert.body)
                .show();
            if let Err(error) = shown {
                eprintln!("could not show a memory alert: {error}");
            }
        }
    }

    fn probe_if_due(&self, app: &AppHandle, snapshot: &Snapshot) {
        let now = now_ms();
        {
            let mut health = lock(&self.health);
            if health.probing || now.saturating_sub(health.last_round_at) < health::PROBE_EVERY_MS {
                return;
            }
            health.probing = true;
            health.last_round_at = now;
        }
        let targets: Vec<_> = snapshot.servers.iter().filter_map(health::target).collect();
        let app = app.clone();
        let spawned = thread::Builder::new()
            .name("ports-health".into())
            .spawn(move || {
                let unresponsive = health::unresponsive(&targets);
                let monitor = app.state::<Monitor>();
                let changed = {
                    let mut health = lock(&monitor.health);
                    health.probing = false;
                    std::mem::replace(&mut health.unresponsive, unresponsive) != health.unresponsive
                };
                if changed {
                    monitor.sample_now();
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not probe the servers: {error}");
            lock(&self.health).probing = false;
        }
    }

    fn check_pages_if_due(&self, app: &AppHandle, snapshot: &Snapshot) {
        let targets = snapshot.servers.iter().filter_map(pages::target).collect();
        let Some(due) = lock(&self.pages).due(targets, now_ms()) else {
            return;
        };
        let app = app.clone();
        let spawned = thread::Builder::new()
            .name("ports-pages".into())
            .spawn(move || {
                let monitor = app.state::<Monitor>();
                let results = pages::check_all(&due, &monitor.icons);
                if lock(&monitor.pages).finish(results, now_ms()) {
                    monitor.sample_now();
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not check the servers' pages: {error}");
            lock(&self.pages).cancel();
        }
    }

    fn sample_now(&self) {
        *lock(&self.woken) = true;
        self.wake.notify_one();
    }

    pub fn window_shown(&self) {
        if self.interval_ms.load(Ordering::Relaxed) != NEVER {
            self.sample_now();
        }
    }

    fn wait(&self, visible: bool) {
        let guard = lock(&self.woken);
        let mut woken = match self.interval_ms.load(Ordering::Relaxed) {
            NEVER => self
                .wake
                .wait_while(guard, |woken| !*woken)
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ms => {
                let ms = if visible {
                    ms
                } else {
                    ms.max(HIDDEN_INTERVAL_MS)
                };
                self.wake
                    .wait_timeout_while(guard, Duration::from_millis(ms), |woken| !*woken)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0
            }
        };
        *woken = false;
    }

    fn control(
        &self,
        app: &AppHandle,
        target: &Target,
        scope: &Scope,
        work: impl FnOnce(Located) -> Result<(), String>,
    ) -> Result<Snapshot, String> {
        let located = locate(target, scope.parent.as_ref())?;
        if let (Located::Process(service), Some(confirmed)) = (&located, &scope.confirmed) {
            service.ensure_within(confirmed)?;
        }
        let keys = match &located {
            Located::Process(service) => service.keys(),
            Located::Container(_) => vec![target.key()],
        };
        {
            let mut stopping = lock(&self.stopping);
            if keys.iter().any(|key| stopping.contains(key)) {
                return Err("Already stopping or restarting part of this service".into());
            }
            stopping.extend(keys.iter().cloned());
        }
        let result = work(located);
        let snapshot = self.publish(app);
        lock(&self.stopping).retain(|key| !keys.contains(key));
        result?;
        snapshot.or_else(|error| self.latest_after_failed_sample(error))
    }

    fn latest_after_failed_sample(&self, error: String) -> Result<Snapshot, String> {
        eprintln!("Could not sample ports: {error}");
        self.sample_now();
        lock(&self.latest).clone().ok_or(error)
    }
}

pub const GRACE_SECS: RangeInclusive<u64> = 1..=60;
pub const DEFAULT_GRACE_SECS: u64 = control::DEFAULT_GRACE.as_secs();

fn clamp_grace(secs: u64) -> u64 {
    secs.clamp(*GRACE_SECS.start(), *GRACE_SECS.end())
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    #[serde(default)]
    parent: Option<ProcessIdentity>,
    #[serde(default)]
    confirmed: Option<Vec<ProcessIdentity>>,
}

fn locate(target: &Target, parent: Option<&ProcessIdentity>) -> Result<Located, String> {
    match target {
        Target::Process(target) => Ok(Located::Process(Box::new(control::Service::locate(
            target, parent,
        )?))),
        Target::Container(_) if parent.is_some() => {
            Err("A container has no parent process to stop".into())
        }
        Target::Container(container) => Ok(Located::Container(container.clone())),
    }
}

pub fn start(app: &AppHandle) -> std::io::Result<()> {
    let app = app.clone();
    thread::Builder::new()
        .name("ports-monitor".into())
        .spawn(move || {
            let monitor = app.state::<Monitor>();
            loop {
                let visible = app
                    .webview_windows()
                    .values()
                    .any(|window| window.is_visible().unwrap_or(false));
                if let Err(error) = monitor.publish(&app) {
                    eprintln!("Could not sample ports: {error}");
                }
                monitor.wait(visible);
            }
        })
        .map(drop)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

pub fn serve_icon(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    app.state::<Monitor>().icons.respond(path)
}

#[tauri::command]
pub fn reveal_session(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let home = project::canonical(&app.path().home_dir().map_err(|error| error.to_string())?);
    let path = session::revealable(&home, std::path::Path::new(&path))?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn snapshot(app: AppHandle) -> Result<Snapshot, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        let latest = lock(&monitor.latest).clone();
        match latest {
            Some(snapshot) => Ok(snapshot),
            None => monitor.publish(&app),
        }
    })
    .await
}

#[tauri::command]
pub async fn refresh(app: AppHandle) -> Result<Snapshot, String> {
    blocking(move || app.state::<Monitor>().publish(&app)).await
}

pub fn open_alerts_on_click(app: &AppHandle, show: impl Fn(&AppHandle) + Send + Sync + 'static) {
    let handle = app.clone();
    let registered = app.notification().on_action(move |performed| {
        if performed.action_id() != "tap" {
            return;
        }
        let Some(server_id) = performed
            .notification()
            .and_then(|notification| notification.extra().get("serverId"))
            .and_then(serde_json::Value::as_str)
        else {
            return;
        };
        show(&handle);
        if let Err(error) = handle.emit(DETAILS_EVENT, server_id) {
            eprintln!("could not open server details: {error}");
        }
    });
    if let Err(error) = registered {
        eprintln!("could not follow clicks on alerts: {error}");
    }
}

#[tauri::command]
pub fn set_refresh_interval(monitor: State<'_, Monitor>, ms: u64) {
    let ms = match ms {
        NEVER => NEVER,
        ms => ms.clamp(*INTERVALS_MS.start(), *INTERVALS_MS.end()),
    };
    if monitor.interval_ms.swap(ms, Ordering::Relaxed) != ms {
        monitor.sample_now();
    }
}

#[tauri::command]
pub async fn preview_control(
    target: Target,
    parent: Option<ProcessIdentity>,
) -> Result<Preview, String> {
    blocking(move || match locate(&target, parent.as_ref())? {
        Located::Process(service) => Ok(service.preview()),
        Located::Container(_) => Ok(Preview::default()),
    })
    .await
}

#[tauri::command]
pub async fn stop_server(
    app: AppHandle,
    target: Target,
    only_if_idle: Option<bool>,
    allowed_ids: Option<Vec<String>>,
    force: Option<bool>,
    scope: Option<Scope>,
) -> Result<Snapshot, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        if only_if_idle.unwrap_or(false) {
            ensure_idle(&monitor, &target, allowed_ids.as_deref())?;
        }
        let shutdown = monitor.shutdown(force.unwrap_or(false));
        monitor.control(
            &app,
            &target,
            &scope.unwrap_or_default(),
            |located| match located {
                Located::Process(service) => service.stop(shutdown),
                Located::Container(target) => containers::stop(&monitor.home, &target, shutdown),
            },
        )
    })
    .await
}

fn ensure_idle(
    monitor: &Monitor,
    target: &Target,
    allowed_ids: Option<&[String]>,
) -> Result<(), String> {
    let fresh = monitor.sample()?;
    let selected = fresh
        .servers
        .iter()
        .find(|server| {
            server.target.as_ref().is_some_and(|current| {
                current.key() == target.key() && current.port() == target.port()
            })
        })
        .ok_or("The idle server changed; refresh before cleaning up")?;
    let affected = fresh.servers.iter().filter(|other| {
        other
            .target
            .as_ref()
            .is_some_and(|current| current.key() == target.key())
            || other.resources.iter().any(|resource| {
                selected
                    .resources
                    .iter()
                    .any(|ours| ours.process == resource.process)
            })
    });
    // A supervisor keeps its services running on purpose; Homebrew's stop
    // would also keep one from starting at login.
    if affected.into_iter().any(|server| {
        server.status != history::Status::Idle
            || server.supervisor.is_some()
            || !allowed_ids.is_some_and(|ids| ids.contains(&server.id))
    }) {
        return Err(
            "The service is active or its affected ports changed; it was left running".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn restart_server(
    app: AppHandle,
    target: Target,
    scope: Option<Scope>,
) -> Result<Snapshot, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        let shutdown = monitor.shutdown(false);
        let scope = scope.unwrap_or_default();
        if scope.parent.is_some() {
            return Err("Ports restarts a server, not its parent".into());
        }
        monitor.control(&app, &target, &scope, |located| match located {
            Located::Process(service) => service.restart(shutdown),
            Located::Container(target) => containers::restart(&monitor.home, &target, shutdown),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_listen_to_the_snapshot_and_details_events() {
        let store = include_str!("../../../src/lib/ports/ports-store.svelte.ts");
        assert!(store.contains(&format!("= \"{SNAPSHOT_EVENT}\";")));
        let layout = include_str!("../../../src/routes/+layout.svelte");
        assert!(layout.contains(&format!("= \"{DETAILS_EVENT}\";")));
    }
}
