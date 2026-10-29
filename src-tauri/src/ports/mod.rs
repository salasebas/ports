//! Servers listening on this machine: discovery, live sampling and control.

mod alerts;
pub mod autoclean;
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
pub mod rules;
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

/// A control target resolved once: the process tree it ends, or a container.
enum Located {
    Process(Box<control::Service>),
    Container(ContainerTarget),
}

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
    pages: Mutex<Pages>,
    icons: Arc<Icons>,
    alerts: Mutex<Alerts>,
    alert_settings: Mutex<AlertSettings>,
    stop_grace_secs: AtomicU64,
    idle_after_ms: AtomicU64,
    rules: Mutex<RulesState>,
    auto: Mutex<autoclean::Runs>,
}

/// The rules every sample applies, and how many times they changed.
struct RulesState {
    revision: u64,
    rules: rules::Rules,
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
    pub fn new(
        home: PathBuf,
        alert_settings: AlertSettings,
        stop_grace_secs: u64,
        idle_after_ms: u64,
        rules: rules::Rules,
        auto_log: PathBuf,
    ) -> Self {
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
            idle_after_ms: AtomicU64::new(idle_after_ms),
            rules: Mutex::new(RulesState { revision: 0, rules }),
            auto: Mutex::new(autoclean::Runs::new(auto_log)),
        }
    }

    fn sample(&self) -> Result<Snapshot, String> {
        let mut snapshot = {
            let mut sampler = lock(&self.sampler);
            sampler.idle_after_ms = self.idle_after_ms.load(Ordering::Relaxed);
            sampler.sample()?
        };
        lock(&self.pages).apply(&mut snapshot.servers);
        health::apply(&lock(&self.health).unresponsive, &mut snapshot.servers);
        lock(&self.rules).rules.apply(&mut snapshot.servers);
        Ok(snapshot)
    }

    /// The rules and their revision.
    pub fn rules(&self) -> (u64, rules::Rules) {
        let state = lock(&self.rules);
        (state.revision, state.rules.clone())
    }

    pub fn idle_after_ms(&self) -> u64 {
        self.idle_after_ms.load(Ordering::Relaxed)
    }

    /// Takes effect with the next sample.
    pub fn set_idle_after_ms(&self, ms: u64) {
        self.idle_after_ms.store(ms, Ordering::Relaxed);
    }

    fn publish(&self, app: &AppHandle) -> Result<Snapshot, String> {
        let _publication = lock(&self.publication);
        let snapshot = self.sample()?;
        *lock(&self.latest) = Some(snapshot.clone());
        if let Err(error) = app.emit(SNAPSHOT_EVENT, &snapshot) {
            eprintln!("could not send a snapshot: {error}");
        }
        self.alert(app, &snapshot);
        self.clean_up_if_due(app, &snapshot);
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

    /// The servers from the last sample, if one has finished.
    pub(crate) fn latest_servers(&self) -> Option<Vec<sampler::Server>> {
        lock(&self.latest)
            .as_ref()
            .map(|snapshot| snapshot.servers.clone())
    }

    /// Samples once when Ports has not published anything yet, then returns the servers.
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

    /// Checks in the background which new servers serve a page, and samples
    /// again when one's page, title or icon changed.
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
                // Also with refreshing off: the round belongs to the sample that started it.
                if lock(&monitor.pages).finish(results, now_ms()) {
                    monitor.wake();
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not check the servers' pages: {error}");
            lock(&self.pages).cancel();
        }
    }

    /// Samples again now, whatever the refresh interval.
    pub(crate) fn wake(&self) {
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

    /// Runs one control per affected process at a time; a second click while the
    /// first one is still waiting for the server to exit is refused.
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
        // The control worked; a failed sample right after it must not report it as failed.
        // The monitor samples again right away to publish the change.
        snapshot.or_else(|error| {
            eprintln!("Could not sample ports: {error}");
            self.wake();
            lock(&self.latest).clone().ok_or(error)
        })
    }

    /// Clean up, manual or automatic, stops a service only if a fresh sample
    /// shows that every server the stop would end may go: none a supervisor
    /// runs, the rules hide or the user protects, and each one `may_stop`
    /// accepts. A sibling the user left out or hid keeps the whole tree running.
    fn ensure_cleanable(
        &self,
        target: &Target,
        may_stop: impl Fn(&sampler::Server) -> bool,
    ) -> Result<(), String> {
        let fresh = self.sample()?;
        let selected = fresh
            .servers
            .iter()
            .find(|server| {
                server.target.as_ref().is_some_and(|current| {
                    current.key() == target.key() && current.port() == target.port()
                })
            })
            .ok_or("The server changed; refresh before cleaning up")?;
        let mut affected = fresh
            .servers
            .iter()
            .filter(|other| shares_tree(target, &selected.resources, other));
        // A supervisor keeps its services running on purpose; Homebrew's stop
        // would also keep one from starting at login.
        if affected.any(|server| {
            server.supervisor.is_some() || server.hidden || server.protected || !may_stop(server)
        }) {
            return Err(
                "The service is active or its affected ports changed; it was left running".into(),
            );
        }
        Ok(())
    }
}

/// Whether stopping `target`, whose tree holds `resources`, also ends `other`:
/// the same root, or a process the two share.
fn shares_tree(
    target: &Target,
    resources: &[sampler::ProcessUsage],
    other: &sampler::Server,
) -> bool {
    other
        .target
        .as_ref()
        .is_some_and(|current| current.key() == target.key())
        || other
            .resources
            .iter()
            .any(|theirs| resources.iter().any(|ours| ours.process == theirs.process))
}

/// Bounds of the shutdown grace period, in seconds.
pub const GRACE_SECS: RangeInclusive<u64> = 1..=60;
pub const DEFAULT_GRACE_SECS: u64 = control::DEFAULT_GRACE.as_secs();
pub const DEFAULT_IDLE_AFTER_MS: u64 = history::DEFAULT_IDLE_AFTER_MS;

fn clamp_grace(secs: u64) -> u64 {
    secs.clamp(*GRACE_SECS.start(), *GRACE_SECS.end())
}

/// Which processes a control reaches, as the windows ask for it.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    /// Also the process that started the service, with everything it runs.
    #[serde(default)]
    parent: Option<ProcessIdentity>,
    /// The processes of the preview the user confirmed; the control refuses
    /// to reach any other.
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

/// Answers the windows' `ports-icon://` requests with a project's or page's icon.
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

/// What a control would end, from the same lookup the control repeats
/// before it acts: the processes, the ports they hold and their supervisor.
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
    clean_up: Option<bool>,
    allowed_ids: Option<Vec<String>>,
    force: Option<bool>,
    scope: Option<Scope>,
) -> Result<Snapshot, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        // Sample before locating, so the stop ends what the check saw.
        if clean_up.unwrap_or(false) {
            let allowed = allowed_ids.unwrap_or_default();
            monitor.ensure_cleanable(&target, |server| {
                (server.status == history::Status::Idle || server.cwd_deleted)
                    && allowed.contains(&server.id)
            })?;
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
