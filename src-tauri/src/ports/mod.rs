//! Servers listening on this machine: discovery, live sampling and control.

mod alerts;
mod containers;
mod control;
mod health;
mod history;
mod identity;
mod network;
mod processes;
mod project;
mod sampler;
mod sockets;

pub use alerts::AlertSettings;
use control::Target;

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

use alerts::Alerts;
use sampler::{Sampler, Snapshot, now_ms};

/// Carries a `Snapshot` to every window. Must match `src/lib/ports/ports-store.svelte.ts`.
const SNAPSHOT_EVENT: &str = "ports://snapshot";
/// Asks the main window to show a server's details, with its port. Must match `src/routes/+page.svelte`.
pub const DETAILS_EVENT: &str = "ports://details";
const DEFAULT_INTERVAL_MS: u64 = 2_000;
/// Samples only on request: the refresh button, a stop or a restart.
const NEVER: u64 = 0;
const INTERVALS_MS: RangeInclusive<u64> = 500..=60_000;
/// While every window is hidden, sampling slows down instead of stopping, so
/// idle times and memory leaks stay accurate for the next time Ports opens.
const HIDDEN_INTERVAL_MS: u64 = 30_000;

/// Samples the servers on an interval and keeps the latest snapshot, so
/// every window shows the same data and history outlives a closed popover.
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
    alerts: Mutex<Alerts>,
    alert_settings: Mutex<AlertSettings>,
}

/// What the latest health probes found, and when the next round is due.
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
    pub fn new(home: PathBuf, alert_settings: AlertSettings) -> Self {
        Self {
            sampler: Mutex::new(Sampler::new(home.clone())),
            publication: Mutex::new(()),
            home,
            latest: Mutex::new(None),
            interval_ms: AtomicU64::new(DEFAULT_INTERVAL_MS),
            woken: Mutex::new(false),
            wake: Condvar::new(),
            stopping: Mutex::new(HashSet::new()),
            health: Mutex::new(Health::default()),
            alerts: Mutex::new(Alerts::default()),
            alert_settings: Mutex::new(alert_settings),
        }
    }

    fn sample(&self) -> Result<Snapshot, String> {
        let mut snapshot = lock(&self.sampler).sample()?;
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
        Ok(snapshot)
    }

    pub fn alert_settings(&self) -> AlertSettings {
        *lock(&self.alert_settings)
    }

    pub fn set_alert_settings(&self, settings: AlertSettings) {
        *lock(&self.alert_settings) = settings;
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
                // A click reports the id, which leads back to the server.
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

    /// Starts a round of health probes in the background once the last one
    /// is old enough, and samples again when a server stops or resumes answering.
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
                // Also with refreshing off: the round belongs to the sample that started it.
                if changed {
                    monitor.wake();
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not probe the servers: {error}");
            lock(&self.health).probing = false;
        }
    }

    fn wake(&self) {
        *lock(&self.woken) = true;
        self.wake.notify_one();
    }

    /// Samples now instead of at the end of the interval, so a window that
    /// appears shows fresh data; unless the user turned refreshing off.
    pub fn window_shown(&self) {
        if self.interval_ms.load(Ordering::Relaxed) != NEVER {
            self.wake();
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

    /// Runs one stop or restart per affected process at a time; a second click while the
    /// first one is still waiting for the server to exit is refused.
    fn control(
        &self,
        app: &AppHandle,
        target: &Target,
        work: impl FnOnce() -> Result<(), String>,
    ) -> Result<Snapshot, String> {
        let keys = match target {
            Target::Process(target) => control::scope(target)?,
            Target::Container(_) => vec![target.key()],
        };
        {
            let mut stopping = lock(&self.stopping);
            if keys.iter().any(|key| stopping.contains(key)) {
                return Err("Already stopping or restarting part of this service".into());
            }
            stopping.extend(keys.iter().cloned());
        }
        let result = work();
        let snapshot = self.publish(app);
        lock(&self.stopping).retain(|key| !keys.contains(key));
        result?;
        snapshot
    }
}

/// Samples in the background at the chosen interval while any window is
/// visible, and every 30 seconds while all are hidden, to save battery.
/// The first sample runs right away, whatever the interval.
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

/// Shows the server's details when the user clicks its memory alert.
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
        monitor.wake();
    }
}

#[tauri::command]
pub async fn stop_server(
    app: AppHandle,
    target: Target,
    only_if_idle: Option<bool>,
    allowed_ids: Option<Vec<String>>,
) -> Result<Snapshot, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        monitor.control(&app, &target, || {
            if only_if_idle.unwrap_or(false) {
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
                if affected.into_iter().any(|server| {
                    server.status != history::Status::Idle
                        || !allowed_ids
                            .as_ref()
                            .is_some_and(|ids| ids.contains(&server.id))
                }) {
                    return Err(
                        "The service is active or its affected ports changed; it was left running"
                            .into(),
                    );
                }
            }
            match &target {
                Target::Process(target) => control::stop(target),
                Target::Container(target) => containers::stop(&monitor.home, target),
            }
        })
    })
    .await
}

#[tauri::command]
pub async fn restart_server(app: AppHandle, target: Target) -> Result<Snapshot, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        monitor.control(&app, &target, || match &target {
            Target::Process(target) => control::restart(target),
            Target::Container(target) => containers::restart(&monitor.home, target),
        })
    })
    .await
}
