//! Servers listening on this machine: discovery, live sampling and control.

mod containers;
mod control;
mod history;
mod processes;
mod project;
mod sampler;
mod sockets;

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use sampler::{Sampler, Snapshot};

/// Carries a `Snapshot` to every window. Must match `src/lib/ports/ports-store.svelte.ts`.
const SNAPSHOT_EVENT: &str = "ports://snapshot";
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
    latest: Mutex<Option<Snapshot>>,
    interval_ms: AtomicU64,
    woken: Mutex<bool>,
    wake: Condvar,
    stopping: Mutex<HashSet<u16>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Monitor {
    pub fn new(home: PathBuf) -> Self {
        Self {
            sampler: Mutex::new(Sampler::new(home.clone())),
            home,
            latest: Mutex::new(None),
            interval_ms: AtomicU64::new(DEFAULT_INTERVAL_MS),
            woken: Mutex::new(false),
            wake: Condvar::new(),
            stopping: Mutex::new(HashSet::new()),
        }
    }

    fn sample(&self) -> Snapshot {
        let snapshot = lock(&self.sampler).sample();
        *lock(&self.latest) = Some(snapshot.clone());
        snapshot
    }

    fn publish(&self, app: &AppHandle) -> Snapshot {
        let snapshot = self.sample();
        if let Err(error) = app.emit(SNAPSHOT_EVENT, &snapshot) {
            eprintln!("could not send a snapshot: {error}");
        }
        snapshot
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

    /// Runs one stop or restart per port at a time; a second click while the
    /// first one is still waiting for the server to exit is refused.
    fn control(
        &self,
        app: &AppHandle,
        port: u16,
        work: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if !lock(&self.stopping).insert(port) {
            return Err(format!("Already stopping or restarting :{port}"));
        }
        let result = work();
        lock(&self.stopping).remove(&port);
        self.publish(app);
        result
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
                monitor.publish(&app);
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
        Ok(latest.unwrap_or_else(|| monitor.sample()))
    })
    .await
}

#[tauri::command]
pub async fn refresh(app: AppHandle) -> Result<Snapshot, String> {
    blocking(move || Ok(app.state::<Monitor>().publish(&app))).await
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

/// What the window saw on the port, so Ports never stops something that took its place.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    port: u16,
    pid: Option<u32>,
    container: Option<String>,
}

#[tauri::command]
pub async fn stop_server(app: AppHandle, target: Target) -> Result<(), String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        monitor.control(&app, target.port, || match target {
            Target {
                pid: Some(pid),
                port,
                ..
            } => control::stop(port, pid),
            Target {
                container: Some(id),
                port,
                ..
            } => containers::stop(&monitor.home, port, &id),
            Target { port, .. } => Err(format!("Ports can't stop the server on :{port}")),
        })
    })
    .await
}

#[tauri::command]
pub async fn restart_server(app: AppHandle, target: Target) -> Result<(), String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        monitor.control(&app, target.port, || match target {
            Target {
                pid: Some(pid),
                port,
                ..
            } => control::restart(port, pid),
            Target {
                container: Some(id),
                port,
                ..
            } => containers::restart(&monitor.home, port, &id),
            Target { port, .. } => Err(format!("Ports can't restart the server on :{port}")),
        })
    })
    .await
}
