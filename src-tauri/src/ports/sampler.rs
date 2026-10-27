//! Builds snapshots of the listening servers and the machine's memory and CPU.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

use super::containers::{self, Container, ContainerTarget};
use super::control::{self, ProcessTarget, Target};
use super::history::{self, ACTIVE_CPU_PERCENT, History, Status};
use super::identity::ProcessIdentity;
use super::network;
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
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    pub resources: Vec<ProcessUsage>,
    #[serde(skip)]
    pub http: bool,
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
    /// Memory kept growing over the last ten minutes. Part of `status`; kept
    /// apart for the memory alerts.
    #[serde(skip)]
    pub leaking: bool,
    /// False when the last health probe got no answer: the server holds its port but hangs.
    pub responding: bool,
    /// Bound addresses, e.g. `127.0.0.1` for this machine only or `0.0.0.0` for every interface.
    pub addresses: Vec<IpAddr>,
    /// Reachable from other devices on the network.
    pub exposed: bool,
    /// Where another device on the network opens it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lan_url: Option<String>,
    pub protected: bool,
    pub restartable: bool,
    pub started_at: u64,
    pub last_active_at: u64,
    pub memory_bytes: u64,
    pub cpu_percent: f64,
    pub memory_history: Vec<u64>,
    pub cpu_history: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessUsage {
    pub process: ProcessIdentity,
    pub memory_bytes: u64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemUsage {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub revision: u64,
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
    /// Keyed by service and kernel start identity, so replacements start fresh.
    tracked: HashMap<String, Tracked>,
    revision: u64,
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
            revision: 0,
        }
    }

    pub fn sample(&mut self) -> Result<Snapshot, String> {
        let now = now_ms();
        self.system.refresh_memory();
        self.system.refresh_cpu_usage();
        // exec and chdir preserve PID/start identity but change these fields.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::Always)
                .with_exe(UpdateKind::Always)
                .with_user(UpdateKind::Always)
                .without_tasks(),
        );

        let sockets = Sockets::scan()?;
        let containers = containers::published(&self.home);
        Ok(self.sample_with(sockets, containers, now))
    }

    fn sample_with(
        &mut self,
        mut sockets: Sockets,
        containers: Vec<Container>,
        now: u64,
    ) -> Snapshot {
        let table = ProcessTable::new(&self.system);
        let all = sockets.all_pids();
        let published: BTreeSet<_> = containers
            .iter()
            .flat_map(|c| c.bindings.iter().copied())
            .collect();
        let mut jobs: BTreeMap<(u16, ProcessIdentity), (Job, BTreeSet<IpAddr>)> = BTreeMap::new();
        for (socket, listeners) in &sockets.listening {
            for pid in listeners {
                if published.contains(socket)
                    && self
                        .system
                        .process(Pid::from_u32(*pid))
                        .is_some_and(control::is_forwarder)
                {
                    continue;
                }
                let job = processes::scoped_job_for(*pid, listeners, &table, &OsGroups, &all);
                let identity = ProcessIdentity::read(job.listener)
                    .filter(|_| self.system.process(Pid::from_u32(job.listener)).is_some());
                let Some(identity) = identity else {
                    sockets.unowned.entry(*socket).or_insert(table.user(*pid));
                    continue;
                };
                jobs.entry((socket.port(), identity))
                    .or_insert_with(|| (job, BTreeSet::new()))
                    .1
                    .insert(socket.ip());
            }
        }
        let mut servers = Vec::new();
        let mut seen = HashSet::new();
        let mut measured = HashMap::new();
        for ((port, identity), (job, addresses)) in jobs {
            let id = format!("process:{}:{port}", identity.key());
            seen.insert(id.clone());
            let tracked = self.tracked.entry(id.clone()).or_insert_with(|| Tracked {
                last_active_at: now,
                ..Tracked::default()
            });
            let resources = job
                .pids
                .iter()
                .filter_map(|pid| {
                    if !measured.contains_key(pid) {
                        let process = self.system.process(Pid::from_u32(*pid))?;
                        let usage = ProcessUsage {
                            process: ProcessIdentity::read(*pid)?,
                            memory_bytes: memory_of(process),
                            cpu_percent: f64::from(process.cpu_usage())
                                / self.system.cpus().len().max(1) as f64,
                        };
                        measured.insert(*pid, usage);
                    }
                    measured.get(pid).cloned()
                })
                .collect();
            if let Some(server) = describe(
                &self.system,
                &self.home,
                &sockets,
                Service {
                    port,
                    job,
                    identity,
                    id,
                    addresses: addresses.into_iter().collect(),
                    resources,
                },
                tracked,
                now,
            ) {
                servers.push(server);
            }
        }
        if !sockets.unowned.is_empty() {
            let users = Users::new_with_refreshed_list();
            for (socket, uid) in &sockets.unowned {
                if published.contains(socket) {
                    continue;
                }
                let id = format!("unowned:{socket}:{uid:?}");
                seen.insert(id.clone());
                let tracked = self.tracked.entry(id.clone()).or_insert_with(|| Tracked {
                    last_active_at: now,
                    ..Tracked::default()
                });
                let owner = match uid {
                    Some(uid) => users
                        .iter()
                        .find(|user| **user.id() == *uid)
                        .map_or_else(|| uid.to_string(), |user| user.name().to_string()),
                    None => "another user".to_string(),
                };
                let mut server = unowned(socket.port(), owner, tracked.last_active_at);
                server.id = id;
                server.addresses = vec![socket.ip()];
                servers.push(server);
            }
        }
        for container in &containers {
            for port in container
                .bindings
                .iter()
                .map(|b| b.port())
                .collect::<BTreeSet<_>>()
            {
                servers.push(contained(port, container));
            }
        }
        self.tracked.retain(|key, _| seen.contains(key));
        let lan_ip = network::lan_ip();
        for server in &mut servers {
            server.exposed = network::is_exposed(&server.addresses);
            server.lan_url = network::lan_url(server.port, &server.addresses, lan_ip);
        }
        servers.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.id.cmp(&b.id)));
        self.revision = self.revision.saturating_add(1);
        Snapshot {
            revision: self.revision,
            servers,
            system: SystemUsage {
                total_bytes: self.system.total_memory(),
                used_bytes: self.system.used_memory(),
                cpu_percent: f64::from(self.system.global_cpu_usage()),
            },
        }
    }
}

struct Service {
    port: u16,
    job: Job,
    identity: ProcessIdentity,
    id: String,
    addresses: Vec<IpAddr>,
    resources: Vec<ProcessUsage>,
}

fn describe(
    system: &System,
    home: &Path,
    sockets: &Sockets,
    service: Service,
    tracked: &mut Tracked,
    now: u64,
) -> Option<Server> {
    let Service {
        port,
        job,
        identity,
        id,
        addresses,
        resources,
    } = service;
    let listener = system.process(Pid::from_u32(job.listener))?;
    let root = system.process(Pid::from_u32(job.root)).unwrap_or(listener);

    let memory_bytes = resources.iter().map(|r| r.memory_bytes).sum();
    let cpu_percent: f64 = resources.iter().map(|r| r.cpu_percent).sum();
    tracked.last_active_at = tracked.last_active_at.min(now);
    let core_percent = cpu_percent * system.cpus().len() as f64;
    if sockets.has_connections(port, &job.pids) || core_percent >= ACTIVE_CPU_PERCENT {
        tracked.last_active_at = now;
    }
    tracked.history.record(now, memory_bytes, cpu_percent);
    let memory_history = tracked.history.memory();
    let leaking = history::is_leaking(&memory_history, tracked.history.span());
    let status = history::status(leaking, now, tracked.last_active_at);

    let folder = listener
        .cwd()
        .or_else(|| root.cwd())
        .filter(|_| !listener.exe().is_some_and(project::is_app_bundle))
        .and_then(|cwd| project::project_folder(cwd, home));
    let words = project::command_words([listener.cmd()]);
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

    let target = ProcessIdentity::read(job.root).map(|root| {
        Target::Process(ProcessTarget {
            port,
            listener: identity,
            root,
            addresses: addresses.clone(),
        })
    });
    Some(Server {
        id,
        target,
        resources,
        http: project::is_http_command(&words),
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
        leaking,
        responding: true,
        addresses,
        exposed: false,
        lan_url: None,
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
        id: format!("unowned:{port}:{owner}"),
        target: None,
        resources: Vec::new(),
        http: false,
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
        leaking: false,
        responding: true,
        addresses: Vec::new(),
        exposed: false,
        lan_url: None,
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
    let addresses: Vec<_> = container
        .bindings
        .iter()
        .filter(|b| b.port() == port)
        .map(|b| b.ip())
        .collect();
    let target = ContainerTarget {
        port,
        id: container.id.clone(),
        engine: container.socket.clone(),
        addresses: addresses.clone(),
    };
    Server {
        id: format!("{}:{port}", target.key()),
        target: Some(Target::Container(target)),
        resources: Vec::new(),
        http: false,
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
        leaking: false,
        responding: true,
        addresses,
        exposed: false,
        lan_url: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_a_binding_when_its_process_cannot_be_inspected() {
        let home = tempfile::tempdir().unwrap();
        let mut sampler = Sampler::new(home.path().to_path_buf());
        let sockets = Sockets::from_entries([(
            "127.0.0.1:3000".parse().unwrap(),
            u32::MAX,
            listeners::SocketState::Listen,
        )]);
        let snapshot = sampler.sample_with(sockets, Vec::new(), 1000);
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(
            snapshot.servers[0].addresses,
            vec!["127.0.0.1".parse::<IpAddr>().unwrap()]
        );
        assert!(snapshot.servers[0].target.is_none());
        assert!(snapshot.servers[0].pid.is_none());
    }

    #[test]
    fn refreshes_command_and_project_after_exec_and_chdir_without_a_pid_change() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::process::CommandExt;
        use std::process::{Child, Command, Stdio};
        struct Disposable(Child);
        impl Drop for Disposable {
            fn drop(&mut self) {
                unsafe {
                    libc::kill(-(self.0.id() as i32), libc::SIGKILL);
                }
                let _ = self.0.wait();
            }
        }
        let home = tempfile::tempdir().unwrap();
        for name in ["before", "after"] {
            std::fs::create_dir(home.path().join(name)).unwrap();
            std::fs::write(home.path().join(name).join("package.json"), "{}").unwrap();
        }
        let after = home.path().join("after");
        let second = "import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',int(__import__('sys').argv[1])));s.listen();print('after',flush=True);time.sleep(60)";
        let first = r#"import os,socket,signal,sys,time
s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',0));s.listen()
port=s.getsockname()[1]
def replace(*args):
 os.chdir(sys.argv[1]);os.execv(sys.executable,[sys.executable,'-c',sys.argv[2],str(port)])
signal.signal(signal.SIGUSR1,replace)
print(port,flush=True);time.sleep(60)
"#;
        let mut child = Disposable(
            Command::new("python3")
                .args(["-c", first, after.to_str().unwrap(), second])
                .current_dir(home.path().join("before"))
                .stdout(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        let mut output = BufReader::new(child.0.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let port: u16 = line.trim().parse().unwrap();
        let mut sampler = Sampler::new(home.path().to_path_buf());
        let initial = sampler
            .sample()
            .unwrap()
            .servers
            .into_iter()
            .find(|row| row.port == port && row.pid == Some(child.0.id()))
            .unwrap();
        unsafe {
            libc::kill(child.0.id() as i32, libc::SIGUSR1);
        }
        line.clear();
        output.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "after");
        let replaced = sampler
            .sample()
            .unwrap()
            .servers
            .into_iter()
            .find(|row| row.port == port && row.pid == Some(child.0.id()))
            .unwrap();
        assert_eq!(initial.id, replaced.id);
        assert_eq!(initial.name, "before");
        assert_eq!(replaced.name, "after");
        assert_ne!(initial.command, replaced.command);
    }

    #[test]
    fn includes_nat_published_containers_without_host_listeners() {
        let home = tempfile::tempdir().unwrap();
        let mut sampler = Sampler::new(home.path().to_path_buf());
        let container = Container {
            id: "nat-container".into(),
            name: "db".into(),
            image: "postgres".into(),
            started_at: 1,
            socket: "/docker.sock".into(),
            bindings: BTreeSet::from([
                "127.0.0.1:5433".parse().unwrap(),
                "[::1]:5433".parse().unwrap(),
            ]),
        };
        let snapshot = sampler.sample_with(Sockets::default(), vec![container], 1000);
        assert_eq!(snapshot.servers.len(), 1);
        let row = &snapshot.servers[0];
        assert_eq!(row.kind, Kind::Container);
        assert_eq!(row.port, 5433);
        assert_eq!(row.addresses.len(), 2);
        assert!(matches!(row.target, Some(Target::Container(_))));
    }
}
