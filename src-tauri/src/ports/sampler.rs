//! Builds snapshots of the listening servers and the machine's memory and CPU.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

use super::containers::{self, Container};
use super::history::{self, ACTIVE_CPU_PERCENT, History, Status};
use super::processes::{self, Job, OsGroups, ProcessTable};
use super::project;
use super::sockets::Sockets;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Dev,
    Database,
    Container,
    /// Desktop apps and system services.
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContainerRef {
    pub id: String,
    pub image: String,
}

/// Mirrors `Server` in `src/lib/ports/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub port: u16,
    /// Missing when the socket belongs to another user's process Ports can't see (Linux).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Set for a port a container publishes; Ports stops it through Docker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerRef>,
    pub process: String,
    pub command: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<&'static str>,
    pub kind: Kind,
    pub status: Status,
    pub protected: bool,
    pub restartable: bool,
    pub started_at: u64,
    pub last_active_at: u64,
    pub memory_bytes: u64,
    pub cpu_percent: f64,
    pub memory_history: Vec<u64>,
    pub cpu_history: Vec<f64>,
}

/// Mirrors `SystemUsage` in `src/lib/ports/types.ts`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemUsage {
    pub total_bytes: u64,
    /// The servers of each kind, counting a process that holds several ports once.
    pub kinds: BTreeMap<Kind, Usage>,
    /// Everything else in use, outside every server.
    pub other_apps_bytes: u64,
    pub other_apps_cpu_percent: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub memory_bytes: u64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub servers: Vec<Server>,
    pub system: SystemUsage,
}

/// What Ports remembers about a server between samples.
#[derive(Default)]
struct Tracked {
    history: History,
    last_active_at: u64,
}

pub struct Sampler {
    system: System,
    home: PathBuf,
    /// Keyed by port and listening PID, so a new process on the port starts fresh.
    tracked: HashMap<(u16, u32), Tracked>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Sampler {
    pub fn new(home: PathBuf) -> Self {
        Self {
            system: System::new(),
            home,
            tracked: HashMap::new(),
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let now = now_ms();
        self.system.refresh_memory();
        self.system.refresh_cpu_usage();
        // Command lines, folders and owners rarely change, so they're read once per process.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );

        let sockets = Sockets::scan();
        let containers = containers::published(&self.home);
        let table = ProcessTable::new(&self.system);
        let mut servers = Vec::new();
        let mut seen = Vec::new();
        let mut kinds: BTreeMap<Kind, BTreeSet<u32>> = BTreeMap::new();
        let mut claimed: BTreeSet<u32> = BTreeSet::new();
        for (port, listeners) in &sockets.listening {
            // The listener is the runtime's port forwarder, shared by every container.
            if let Some(container) = containers.get(port) {
                servers.push(contained(*port, container));
                continue;
            }
            let job = processes::job(listeners, &table, &OsGroups);
            let key = (*port, job.listener);
            seen.push(key);
            let tracked = self.tracked.entry(key).or_insert_with(|| Tracked {
                last_active_at: now,
                ..Tracked::default()
            });
            if let Some(server) = describe(
                &self.system,
                &self.home,
                &sockets,
                *port,
                &job,
                tracked,
                now,
            ) {
                // Each process counts toward one kind only, so the kinds add up.
                let unclaimed = job.pids.iter().filter(|pid| !claimed.contains(*pid));
                kinds.entry(server.kind).or_default().extend(unclaimed);
                claimed.extend(&job.pids);
                servers.push(server);
            }
        }
        if !sockets.unowned.is_empty() {
            let users = Users::new_with_refreshed_list();
            for (port, uid) in &sockets.unowned {
                if let Some(container) = containers.get(port) {
                    servers.push(contained(*port, container));
                    continue;
                }
                let key = (*port, 0);
                seen.push(key);
                let first_seen = self
                    .tracked
                    .entry(key)
                    .or_insert_with(|| Tracked {
                        last_active_at: now,
                        ..Tracked::default()
                    })
                    .last_active_at;
                let owner = match uid {
                    Some(uid) => users
                        .iter()
                        .find(|user| **user.id() == *uid)
                        .map_or_else(|| uid.to_string(), |user| user.name().to_string()),
                    None => "another user".to_string(),
                };
                servers.push(unowned(*port, owner, first_seen));
            }
            servers.sort_by_key(|server| server.port);
        }
        self.tracked.retain(|key, _| seen.contains(key));

        let all: BTreeSet<u32> = kinds.values().flatten().copied().collect();
        let (servers_bytes, servers_cpu_percent) = usage(&self.system, &all);
        Snapshot {
            servers,
            system: SystemUsage {
                total_bytes: self.system.total_memory(),
                kinds: kinds
                    .iter()
                    .map(|(kind, pids)| {
                        let (memory_bytes, cpu_percent) = usage(&self.system, pids);
                        let usage = Usage {
                            memory_bytes,
                            cpu_percent,
                        };
                        (*kind, usage)
                    })
                    .collect(),
                other_apps_bytes: self.system.used_memory().saturating_sub(servers_bytes),
                other_apps_cpu_percent: (f64::from(self.system.global_cpu_usage())
                    - servers_cpu_percent)
                    .max(0.0),
            },
        }
    }
}

fn describe(
    system: &System,
    home: &Path,
    sockets: &Sockets,
    port: u16,
    job: &Job,
    tracked: &mut Tracked,
    now: u64,
) -> Option<Server> {
    let listener = system.process(Pid::from_u32(job.listener))?;
    let root = system.process(Pid::from_u32(job.root)).unwrap_or(listener);

    let (memory_bytes, cpu_percent) = usage(system, &job.pids);
    let core_percent = cpu_percent * system.cpus().len() as f64;
    if sockets.has_connections(port, &job.pids) || core_percent >= ACTIVE_CPU_PERCENT {
        tracked.last_active_at = now;
    }
    tracked.history.record(now, memory_bytes, cpu_percent);
    let memory_history = tracked.history.memory();
    let status = history::status(
        history::is_leaking(&memory_history, tracked.history.span()),
        now,
        tracked.last_active_at,
    );

    let folder = listener
        .cwd()
        .or_else(|| root.cwd())
        .filter(|_| !listener.exe().is_some_and(project::is_app_bundle))
        .and_then(|cwd| project::project_folder(cwd, home));
    let words = project::command_words([listener.cmd(), root.cmd()]);
    let framework = project::framework(&words, folder.as_deref());
    let process = listener.name().to_string_lossy().into_owned();
    let kind = match (framework, &folder) {
        (Some(name), _) if project::is_database(name) => Kind::Database,
        (_, Some(_)) => Kind::Dev,
        _ => Kind::Other,
    };
    let command = match project::display_command(root.cmd()) {
        command if command.is_empty() => process.clone(),
        command => command,
    };

    Some(Server {
        port,
        pid: Some(job.listener),
        owner: None,
        container: None,
        name: folder
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| process.clone(), |n| n.to_string_lossy().into_owned()),
        branch: folder.as_deref().and_then(|f| project::git_branch(f, home)),
        folder: folder.as_deref().map(|f| project::display_path(f, home)),
        framework,
        protected: matches!(kind, Kind::Database),
        restartable: can_relaunch(root, system),
        kind,
        status,
        process,
        command,
        started_at: root.start_time() * 1000,
        last_active_at: tracked.last_active_at,
        memory_bytes,
        cpu_percent,
        memory_history,
        cpu_history: tracked.history.cpu(),
    })
}

/// A listening port whose process belongs to another user and can't be
/// inspected: only the port, the owner and what usually listens there are known.
fn unowned(port: u16, owner: String, first_seen: u64) -> Server {
    let (name, framework) = project::well_known(port).unwrap_or((&owner, None));
    let kind = match framework {
        Some(name) if project::is_database(name) => Kind::Database,
        _ => Kind::Other,
    };
    Server {
        port,
        pid: None,
        name: name.to_string(),
        process: name.to_string(),
        command: String::new(),
        owner: Some(owner),
        container: None,
        folder: None,
        branch: None,
        framework,
        protected: matches!(kind, Kind::Database),
        restartable: false,
        kind,
        status: Status::Running,
        started_at: first_seen,
        last_active_at: first_seen,
        memory_bytes: 0,
        cpu_percent: 0.0,
        memory_history: Vec::new(),
        cpu_history: Vec::new(),
    }
}

/// A port a container publishes. The runtime's forwarder holds the socket, so
/// the container's own memory and CPU aren't known here.
fn contained(port: u16, container: &Container) -> Server {
    let image = containers::image_name(&container.image);
    let framework = project::image_framework(image);
    Server {
        port,
        pid: None,
        owner: None,
        container: Some(ContainerRef {
            id: container.id.clone(),
            image: container.image.clone(),
        }),
        name: container.name.clone(),
        process: image.to_string(),
        command: container.image.clone(),
        folder: None,
        branch: None,
        framework,
        protected: framework.is_some_and(project::is_database),
        restartable: true,
        kind: Kind::Container,
        status: Status::Running,
        started_at: container.started_at,
        last_active_at: container.started_at,
        memory_bytes: 0,
        cpu_percent: 0.0,
        memory_history: Vec::new(),
        cpu_history: Vec::new(),
    }
}

/// A launch can be reproduced when it belongs to this user, was started by
/// something other than a service manager (which restarts its own services),
/// and still shows its real command line: servers such as nginx, puma or
/// Next.js rewrite theirs into a title like `nginx: master process`.
pub fn can_relaunch(root: &Process, system: &System) -> bool {
    const SERVICE_MANAGERS: [&str; 3] = ["launchd", "systemd", "init"];
    let own_uid = unsafe { libc::getuid() };
    let is_own = root.user_id().is_some_and(|uid| **uid == own_uid);
    let from_service_manager = root.parent().is_none_or(|parent| {
        parent.as_u32() <= 1
            || system
                .process(parent)
                .is_some_and(|p| SERVICE_MANAGERS.contains(&p.name().to_string_lossy().as_ref()))
    });
    let titled = match root.cmd() {
        [] => true,
        [only] => only.to_string_lossy().contains(' '),
        [first, ..] => first.to_string_lossy().contains(": "),
    };
    is_own && !from_service_manager && !titled && root.cwd().is_some()
}

/// Memory and share of the whole machine's CPU used by `pids` together.
fn usage(system: &System, pids: &BTreeSet<u32>) -> (u64, f64) {
    let (memory, core_percent) = pids
        .iter()
        .filter_map(|pid| system.process(Pid::from_u32(*pid)))
        .fold((0, 0.0), |(memory, cpu), p| {
            (memory + memory_of(p), cpu + f64::from(p.cpu_usage()))
        });
    (memory, core_percent / system.cpus().len().max(1) as f64)
}

/// Physical footprint on macOS, which is what Activity Monitor shows; RSS
/// elsewhere, which is what `top` shows. Falls back to RSS when the
/// footprint isn't readable (processes of other users).
fn memory_of(process: &Process) -> u64 {
    #[cfg(target_os = "macos")]
    if let Some(footprint) = phys_footprint(process.pid().as_u32()) {
        return footprint;
    }
    process.memory()
}

#[cfg(target_os = "macos")]
fn phys_footprint(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    let result = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    (result == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}
