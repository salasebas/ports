use super::containers::ContainerTarget;
use super::identity::ProcessIdentity;
use super::processes::{self, Groups, Job, OsGroups, ProcessTable};
use super::sampler::can_relaunch;
use super::sockets::Sockets;
use super::supervisors::{self, LaunchdJobs, Managed, Supervisor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::net::IpAddr;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind, Users,
};

pub const DEFAULT_GRACE: Duration = Duration::from_secs(3);
const KILL_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(100);
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
const RESPAWN_CHECK: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessTarget {
    pub port: u16,
    pub listener: ProcessIdentity,
    pub root: ProcessIdentity,
    pub addresses: Vec<IpAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    Process(ProcessTarget),
    Container(ContainerTarget),
}

impl Target {
    pub fn key(&self) -> String {
        match self {
            Self::Process(t) => format!("process:{}", t.root.key()),
            Self::Container(t) => t.key(),
        }
    }
    pub fn port(&self) -> u16 {
        match self {
            Self::Process(t) => t.port,
            Self::Container(t) => t.port,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Shutdown {
    pub grace: Duration,
    pub force: bool,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self {
            grace: DEFAULT_GRACE,
            force: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Affected {
    pub process: ProcessIdentity,
    pub name: String,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub processes: Vec<Affected>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<Supervisor>,
}

pub struct Service {
    target: ProcessTarget,
    job: Option<(Job, System)>,
    ports: BTreeMap<u32, BTreeSet<u16>>,
    managed: Option<Managed>,
    unsupported: Option<Supervisor>,
    identities: BTreeSet<ProcessIdentity>,
}

impl Service {
    pub fn locate(
        target: &ProcessTarget,
        parent: Option<&ProcessIdentity>,
    ) -> Result<Self, String> {
        let Some((job, system, sockets)) = locate(target)? else {
            return Ok(Self {
                target: target.clone(),
                job: None,
                ports: BTreeMap::new(),
                managed: None,
                unsupported: None,
                identities: BTreeSet::new(),
            });
        };
        let table = ProcessTable::new(&system);
        let mut launchd = LaunchdJobs::default();
        let found = supervisors::find(job.listener, &system, &table, &mut launchd);
        if found.is_none() && launchd.failed() {
            return Err(
                "Ports couldn't ask launchd whether it restarts this server; try again".into(),
            );
        }
        let (target, job) = match parent {
            Some(parent) => {
                if found.is_some()
                    || processes::watcher(job.root, &table, &OsGroups) != Some(parent.pid)
                    || !parent.is_current()
                {
                    return Err(
                        "The parent of this server changed; refresh before trying again".into(),
                    );
                }
                let job = Job {
                    listener: job.listener,
                    root: parent.pid,
                    group: None,
                    pids: table.descendants(parent.pid),
                };
                ensure_not_ports(&job, &table)?;
                let target = ProcessTarget {
                    root: parent.clone(),
                    ..target.clone()
                };
                (target, job)
            }
            None => (target.clone(), job),
        };
        let (managed, unsupported) = match &found {
            Some(found) if parent.is_none() => match Managed::confirm(found, &system)? {
                Some(managed) => (Some(managed), None),
                None => (None, Some(found.summary())),
            },
            _ => (None, None),
        };
        let job = if managed.is_none() {
            with_group_members(job, &system)
        } else {
            job
        };
        let mut ports: BTreeMap<u32, BTreeSet<u16>> = BTreeMap::new();
        for (socket, owners) in &sockets.listening {
            for pid in owners.iter().filter(|pid| job.pids.contains(pid)) {
                ports.entry(*pid).or_default().insert(socket.port());
            }
        }
        let identities = job
            .pids
            .iter()
            .filter_map(|pid| ProcessIdentity::read(*pid))
            .collect();
        Ok(Self {
            target,
            job: Some((job, system)),
            ports,
            managed,
            unsupported,
            identities,
        })
    }

    pub fn keys(&self) -> Vec<String> {
        let mut keys = vec![format!("process:{}", self.target.root.key())];
        keys.extend(
            self.identities
                .iter()
                .map(|id| format!("process:{}", id.key())),
        );
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn preview(&self) -> Preview {
        let processes = self
            .job
            .iter()
            .flat_map(|(_, system)| {
                self.identities.iter().filter_map(|identity| {
                    let process = system.process(Pid::from_u32(identity.pid))?;
                    Some(Affected {
                        process: identity.clone(),
                        name: process.name().to_string_lossy().into_owned(),
                        ports: self
                            .ports
                            .get(&identity.pid)
                            .into_iter()
                            .flatten()
                            .copied()
                            .collect(),
                    })
                })
            })
            .collect();
        Preview {
            processes,
            supervisor: self
                .managed
                .as_ref()
                .map(Managed::summary)
                .or_else(|| self.unsupported.clone()),
        }
    }

    pub fn ensure_within(&self, confirmed: &[ProcessIdentity]) -> Result<(), String> {
        if self.identities.iter().all(|id| confirmed.contains(id)) {
            Ok(())
        } else {
            Err("What this would stop changed since you confirmed it; review it again".into())
        }
    }

    pub fn stop(self, shutdown: Shutdown) -> Result<(), String> {
        let Some((job, system)) = self.job else {
            return Ok(());
        };
        if let Some(managed) = &self.managed {
            if shutdown.force {
                return Err(format!(
                    "{} is managed by its supervisor, which would start it again; use Stop instead",
                    managed.label()
                ));
            }
            managed.stop()?;
            return wait_until_free(&self.target, shutdown.grace + KILL_GRACE + KILL_GRACE);
        }
        end(&job, &system, &self.target, &self.identities, shutdown)?;
        ensure_free(&self.target)?;
        if let Some(supervisor) = &self.unsupported {
            thread::sleep(RESPAWN_CHECK);
            if Sockets::scan()?.occupied(self.target.port, &self.target.addresses) {
                return Err(format!(
                    "{0} started it again on :{1}; stop it through {0} to keep it stopped",
                    supervisor_name(supervisor),
                    self.target.port
                ));
            }
        }
        Ok(())
    }

    pub fn restart(self, shutdown: Shutdown) -> Result<(), String> {
        let Some((job, mut system)) = self.job else {
            return Err(format!("Nothing listens on :{} anymore", self.target.port));
        };
        if let Some(managed) = &self.managed {
            let before = self
                .identities
                .union(&managed.instances())
                .cloned()
                .collect();
            managed.restart()?;
            return wait_until_supervised(&self.target, managed, &before, LISTEN_TIMEOUT);
        }
        if let Some(supervisor) = &self.unsupported {
            return Err(format!(
                "{} runs this server and Ports can't restart it through {0}",
                supervisor_name(supervisor)
            ));
        }
        let launch = Launch::capture(job.root, &mut system)?;
        end(&job, &system, &self.target, &self.identities, shutdown)?;
        ensure_free(&self.target)?;
        let child = launch.spawn()?;
        wait_until_listening(&self.target, child, &launch.name, LISTEN_TIMEOUT)
    }
}

fn with_group_members(mut job: Job, system: &System) -> Job {
    let Some(group) = job.group else {
        return job;
    };
    let session = OsGroups.sid(job.root);
    let uid = system
        .process(Pid::from_u32(job.root))
        .and_then(Process::user_id)
        .map(|uid| **uid);
    job.pids.extend(
        system
            .processes()
            .iter()
            .filter(|(pid, process)| {
                let pid = pid.as_u32();
                OsGroups.pgid(pid) == Some(group)
                    && OsGroups.sid(pid) == session
                    && process.user_id().map(|uid| **uid) == uid
                    && process.status() != ProcessStatus::Zombie
            })
            .map(|(pid, _)| pid.as_u32()),
    );
    job
}

fn supervisor_name(supervisor: &Supervisor) -> &'static str {
    match supervisor.kind {
        supervisors::Kind::Homebrew => "Homebrew",
        supervisors::Kind::Pm2 => "PM2",
        supervisors::Kind::Supervisord => "supervisord",
    }
}

#[cfg(test)]
fn stop(target: &ProcessTarget) -> Result<(), String> {
    Service::locate(target, None)?.stop(Shutdown::default())
}

#[cfg(test)]
fn restart(target: &ProcessTarget) -> Result<(), String> {
    Service::locate(target, None)?.restart(Shutdown::default())
}

const FORWARDERS: [&str; 8] = [
    "com.docker.backend",
    "com.docker.vpnkit",
    "vpnkit-bridge",
    "OrbStack Helper",
    "docker-proxy",
    "rootlessport",
    "gvproxy",
    "limactl",
];

pub fn is_forwarder(process: &Process) -> bool {
    FORWARDERS.contains(&process.name().to_string_lossy().as_ref())
        || process
            .exe()
            .and_then(|exe| exe.file_name())
            .is_some_and(|name| FORWARDERS.contains(&name.to_string_lossy().as_ref()))
}

fn process_table() -> System {
    refreshed(ProcessRefreshKind::nothing())
}

fn inspection_table() -> System {
    refreshed(ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always))
}

fn refreshed(kind: ProcessRefreshKind) -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        kind.with_user(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .without_tasks(),
    );
    system
}

fn locate(target: &ProcessTarget) -> Result<Option<(Job, System, Sockets)>, String> {
    if target.addresses.is_empty() {
        return Err("The selected server has no known binding".into());
    }
    let sockets = Sockets::scan()?;
    if !sockets.occupied(target.port, &target.addresses) {
        return Ok(None);
    }
    if !target.listener.is_current() || !target.root.is_current() {
        return Err(format!(
            "The server on :{} changed; refresh before trying again",
            target.port
        ));
    }
    let system = inspection_table();
    let table = ProcessTable::new(&system);
    let all = sockets.all_pids();
    for (socket, listeners) in &sockets.listening {
        if socket.port() != target.port || !target.addresses.contains(&socket.ip()) {
            continue;
        }
        for pid in listeners {
            let job = processes::scoped_job_for(*pid, listeners, &table, &OsGroups, &all);
            if job.listener != target.listener.pid || job.root != target.root.pid {
                continue;
            }
            let Some(process) = system.process(Pid::from_u32(job.listener)) else {
                continue;
            };
            if is_forwarder(process) {
                return Err(
                    "This is a shared container forwarder; stop the container instead".into(),
                );
            }
            if table.user(job.listener) != Some(unsafe { libc::getuid() }) {
                return Err(not_permitted(&system, job.listener));
            }
            ensure_not_ports(&job, &table)?;
            return Ok(Some((job, system, sockets)));
        }
    }
    Err(format!(
        "The owner of :{} changed or cannot be inspected; refresh before trying again",
        target.port
    ))
}

fn ensure_not_ports(job: &Job, table: &ProcessTable) -> Result<(), String> {
    let own = std::process::id();
    if std::iter::once(own)
        .chain(table.ancestors(own))
        .any(|pid| job.pids.contains(&pid))
    {
        return Err("Ports can't stop the process that started it".into());
    }
    Ok(())
}

fn end(
    job: &Job,
    system: &System,
    selected: &ProcessTarget,
    located: &BTreeSet<ProcessIdentity>,
    shutdown: Shutdown,
) -> Result<(), String> {
    if !selected.root.is_current() || !selected.listener.is_current() {
        return Err("The selected service changed before shutdown".into());
    }
    let leader = selected.root.clone();
    let session = OsGroups.sid(job.root);
    let uid = system
        .process(Pid::from_u32(job.root))
        .and_then(Process::user_id)
        .map(|uid| **uid);
    let mut targets = located.clone();
    let shared_group = job
        .group
        .is_none()
        .then(|| OsGroups.pgid(selected.listener.pid))
        .flatten()
        .filter(|group| *group > 1);
    let earlier: BTreeSet<_> = shared_group
        .map(|group| {
            system
                .processes()
                .keys()
                .map(|pid| pid.as_u32())
                .filter(|pid| OsGroups.pgid(*pid) == Some(group))
                .filter_map(ProcessIdentity::read)
                .collect()
        })
        .unwrap_or_default();
    let mut programs: HashSet<_> = job
        .pids
        .iter()
        .filter_map(|pid| system.process(Pid::from_u32(*pid))?.exe())
        .map(Path::to_path_buf)
        .collect();
    let rounds: &[_] = if shutdown.force {
        &[(libc::SIGKILL, KILL_GRACE)]
    } else {
        &[(libc::SIGTERM, shutdown.grace), (libc::SIGKILL, KILL_GRACE)]
    };
    for &(signal, grace) in rounds {
        let deadline = Instant::now() + grace;
        let mut sent = HashSet::new();
        let mut empty_rounds = 0;
        loop {
            let current = process_table();
            let table = ProcessTable::new(&current);
            let leader_now = ProcessIdentity::read(leader.pid);
            let group_not_reused = leader_now.as_ref().is_none_or(|id| id == &leader);
            let descendants: BTreeSet<_> = targets
                .iter()
                .filter(|target| target.is_current())
                .flat_map(|target| table.descendants(target.pid))
                .collect();
            for (pid, process) in current.processes() {
                let pid = pid.as_u32();
                let in_group = group_not_reused
                    && job
                        .group
                        .is_some_and(|group| OsGroups.pgid(pid) == Some(group))
                    && OsGroups.sid(pid) == session
                    && process.user_id().map(|uid| **uid) == uid;
                let escaped = shared_group.is_some_and(|group| {
                    process.exe().is_some_and(|exe| programs.contains(exe))
                        && OsGroups.pgid(pid) == Some(group)
                        && process
                            .parent()
                            .is_none_or(|parent| OsGroups.pgid(parent.as_u32()) != Some(group))
                        && OsGroups.sid(pid) == session
                        && process.user_id().map(|uid| **uid) == uid
                        && ProcessIdentity::read(pid).is_some_and(|id| !earlier.contains(&id))
                });
                if (in_group || escaped || descendants.contains(&pid))
                    && process.status() != ProcessStatus::Zombie
                    && let Some(identity) = ProcessIdentity::read(pid)
                {
                    targets.insert(identity);
                    programs.extend(process.exe().map(Path::to_path_buf));
                }
            }
            let living = alive(&targets);
            if living.is_empty() {
                empty_rounds += 1;
                if empty_rounds >= 2 {
                    return Ok(());
                }
                thread::sleep(POLL);
                continue;
            }
            empty_rounds = 0;
            for target in living {
                if sent.insert(target.clone()) && target.is_current() {
                    send(
                        unsafe { libc::kill(target.pid as libc::pid_t, signal) },
                        target.pid,
                        system,
                    )?;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(POLL);
        }
    }
    match alive(&targets).first() {
        Some(survivor) => Err(format!("{} didn't exit", label(system, survivor.pid))),
        None => Ok(()),
    }
}

fn send(result: libc::c_int, pid: u32, system: &System) -> Result<(), String> {
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        Some(libc::EPERM) => Err(not_permitted(system, pid)),
        _ => Err(format!("Can't stop {}: {error}", label(system, pid))),
    }
}

fn alive(targets: &BTreeSet<ProcessIdentity>) -> Vec<ProcessIdentity> {
    let pids: Vec<_> = targets.iter().map(|t| Pid::from_u32(t.pid)).collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    targets
        .iter()
        .filter(|t| {
            t.is_current()
                && system
                    .process(Pid::from_u32(t.pid))
                    .is_some_and(|p| p.status() != ProcessStatus::Zombie)
        })
        .cloned()
        .collect()
}

fn ensure_free(target: &ProcessTarget) -> Result<(), String> {
    if Sockets::scan()?.occupied(target.port, &target.addresses) {
        Err(format!("The binding on :{} is still in use", target.port))
    } else {
        Ok(())
    }
}

fn wait_until_free(target: &ProcessTarget, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match ensure_free(target) {
            Err(_) if Instant::now() < deadline => thread::sleep(POLL),
            result => return result,
        }
    }
}

fn wait_until_supervised(
    target: &ProcessTarget,
    managed: &Managed,
    before: &BTreeSet<ProcessIdentity>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(anchor) = managed.anchor() {
            let sockets = Sockets::scan()?;
            let system = process_table();
            let descendants = ProcessTable::new(&system).descendants(anchor);
            let owners: Vec<_> = sockets
                .listening
                .iter()
                .filter(|(socket, _)| socket.port() == target.port)
                .flat_map(|(_, owners)| owners)
                .filter_map(|pid| ProcessIdentity::read(*pid))
                .collect();
            let fresh = owners
                .iter()
                .any(|id| descendants.contains(&id.pid) && !before.contains(id));
            if fresh && owners.iter().all(|id| !before.contains(id)) {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{} started again but isn't listening on :{} yet",
                managed.label(),
                target.port
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn not_permitted(system: &System, pid: u32) -> String {
    format!(
        "Can't stop {}: it belongs to {}. Stop it from a terminal with sudo.",
        label(system, pid),
        owner(system, pid).unwrap_or_else(|| "another user".to_string())
    )
}

fn label(system: &System, pid: u32) -> String {
    match system.process(Pid::from_u32(pid)) {
        Some(process) => format!("{} (PID {pid})", process.name().to_string_lossy()),
        None => format!("PID {pid}"),
    }
}

fn owner(system: &System, pid: u32) -> Option<String> {
    let uid = system.process(Pid::from_u32(pid))?.user_id()?;
    let users = Users::new_with_refreshed_list();
    users
        .get_user_by_id(uid)
        .map(|user| user.name().to_string())
}

struct Launch {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    env: Vec<(OsString, OsString)>,
    name: String,
}

impl Launch {
    fn capture(root: u32, system: &mut System) -> Result<Self, String> {
        let pid = Pid::from_u32(root);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::Always)
                .with_exe(UpdateKind::Always)
                .with_environ(UpdateKind::Always)
                .with_user(UpdateKind::Always)
                .without_tasks(),
        );
        let process = system.process(pid).ok_or("The server exited")?;
        let name = label(system, root);
        let cannot = || format!("Can't restart {name}: Ports can't see how it was started");
        if !can_relaunch(process, system) || process.environ().is_empty() {
            return Err(cannot());
        }
        let args = process.cmd().to_vec();
        let program = match process.exe() {
            Some(exe) if exe.exists() => exe.to_path_buf(),
            _ => PathBuf::from(args.first().ok_or_else(cannot)?),
        };
        let env = process
            .environ()
            .iter()
            .filter_map(|entry| {
                let entry = entry.to_str()?;
                let (key, value) = entry.split_once('=')?;
                Some((key.into(), value.into()))
            })
            .collect();
        Ok(Self {
            program,
            args,
            cwd: process.cwd().ok_or_else(cannot)?.to_path_buf(),
            env,
            name,
        })
    }

    fn spawn(&self) -> Result<Child, String> {
        let (arg0, rest) = self
            .args
            .split_first()
            .ok_or_else(|| format!("Can't restart {}: empty command line", self.name))?;
        Command::new(&self.program)
            .arg0(arg0)
            .args(rest)
            .current_dir(&self.cwd)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|error| format!("Can't start {} again: {error}", self.name))
    }
}

fn wait_until_listening(
    target: &ProcessTarget,
    mut child: Child,
    name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let launched = ProcessIdentity::read(child.id());
    let result = (|| loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Err(format!(
                "{name} exited ({status}) before listening on :{}",
                target.port
            ));
        }
        let sockets = Sockets::scan()?;
        if let Some(launched) = &launched
            && launched.is_current()
        {
            let system = process_table();
            let descendants = ProcessTable::new(&system).descendants(launched.pid);
            // The relaunch may resolve `localhost` to another family, so any
            // binding on the port counts once the launched tree owns it.
            if sockets.listening.iter().any(|(socket, owners)| {
                socket.port() == target.port && !owners.is_disjoint(&descendants)
            }) {
                break Ok(());
            }
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "{name} started again but isn't listening on :{} yet",
                target.port
            ));
        }
        thread::sleep(POLL);
    })();
    crate::child_process::reap(child);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    const SERVER: &str = r#"
import signal, socket, subprocess, sys, time
if "ignore-term" in sys.argv:
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))
s.listen()
child = subprocess.Popen(["sleep", "60"])
print(child.pid, flush=True)
time.sleep(60)
"#;

    fn target(port: u16, pid: u32) -> ProcessTarget {
        let sockets = Sockets::scan().unwrap();
        let system = process_table();
        let table = ProcessTable::new(&system);
        let job = processes::scoped_job(
            &BTreeSet::from([pid]),
            &table,
            &OsGroups,
            &sockets.all_pids(),
        );
        ProcessTarget {
            port,
            listener: ProcessIdentity::read(job.listener).unwrap(),
            root: ProcessIdentity::read(job.root).unwrap(),
            addresses: vec!["127.0.0.1".parse().unwrap()],
        }
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn start(port: u16, cwd: &std::path::Path, extra: &[&str]) -> (Child, u32) {
        std::fs::write(cwd.join("server.py"), SERVER).unwrap();
        let mut child = Command::new("python3")
            .arg("server.py")
            .arg(port.to_string())
            .args(extra)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        (child, line.trim().parse().unwrap())
    }

    fn running(pid: u32) -> bool {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
        system
            .process(Pid::from_u32(pid))
            .is_some_and(|p| p.status() != ProcessStatus::Zombie)
    }

    #[test]
    fn shutdown_rechecks_the_selected_identity_after_locating_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let (mut server, _) = start(port, dir.path(), &[]);
        let selected = target(port, server.id());
        let (job, system, _) = locate(&selected).unwrap().unwrap();
        let located = job
            .pids
            .iter()
            .filter_map(|pid| ProcessIdentity::read(*pid))
            .collect();
        let mut stale = selected.clone();
        stale.root.start.push_str("-reused");
        let result = end(&job, &system, &stale, &located, Shutdown::default());
        let survived = running(server.id());
        stop(&selected).unwrap();
        server.wait().unwrap();
        assert!(result.unwrap_err().contains("changed"));
        assert!(survived);
    }

    #[test]
    fn reserves_the_children_that_a_service_control_can_end() {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let (mut server, worker) = start(port, dir.path(), &[]);
        let selected = target(port, server.id());
        let keys = Service::locate(&selected, None).unwrap().keys();
        let worker_key = format!("process:{}", ProcessIdentity::read(worker).unwrap().key());
        stop(&selected).unwrap();
        server.wait().unwrap();
        assert!(keys.contains(&worker_key));
    }

    #[test]
    fn sampled_prefork_master_can_be_stopped_when_only_its_workers_own_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let code = r#"import os,socket,time
s=socket.socket();s.bind(('127.0.0.1',0));s.listen()
port=s.getsockname()[1]
workers=[]
for _ in range(2):
 p=os.fork()
 if p==0:
  time.sleep(60);os._exit(0)
 workers.append(p)
s.close()
print(port,*workers,flush=True)
time.sleep(60)
"#;
        let mut server = Command::new("python3")
            .args(["-c", code])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let values: Vec<u32> = line
            .split_whitespace()
            .map(|p| p.parse().unwrap())
            .collect();
        let mut sampler =
            super::super::sampler::Sampler::new(dir.path().to_path_buf(), Default::default());
        let snapshot = sampler.sample().unwrap();
        let selected = snapshot
            .servers
            .iter()
            .find(|s| s.port == values[0] as u16 && s.pid == Some(server.id()));
        let result = match selected.and_then(|s| s.target.as_ref()) {
            Some(Target::Process(target)) => stop(target),
            _ => Err("Sampler did not identify the prefork master".into()),
        };
        let children_alive = values[1..].iter().any(|pid| running(*pid));
        unsafe {
            libc::kill(-(server.id() as i32), libc::SIGKILL);
        }
        server.wait().unwrap();
        result.unwrap();
        assert!(!children_alive);
    }

    static SHARED_GROUP: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn shutdown_cannot_leave_a_child_forked_by_a_term_handler_in_private_or_shared_groups() {
        let _shared = SHARED_GROUP.lock().unwrap_or_else(|e| e.into_inner());
        for private in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("forked");
            let code = r#"import os,signal,socket,time,sys
s=socket.socket();s.bind(('127.0.0.1',0));s.listen()
def term(*args):
 p=os.fork()
 if p==0:
  s.close()
  with open(sys.argv[1]+'.tmp','w') as f:f.write(str(os.getpid()))
  os.rename(sys.argv[1]+'.tmp',sys.argv[1])
  time.sleep(60);os._exit(0)
 os._exit(0)
signal.signal(signal.SIGTERM,term)
print(s.getsockname()[1],flush=True)
time.sleep(60)
"#;
            let mut command = Command::new("python3");
            command
                .args(["-c", code, path.to_str().unwrap()])
                .stdout(Stdio::piped());
            if private {
                command.process_group(0);
            }
            let mut server = command.spawn().unwrap();
            let mut line = String::new();
            BufReader::new(server.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            let port = line.trim().parse().unwrap();
            let result = stop(&target(port, server.id()));
            thread::sleep(Duration::from_millis(300));
            let forked = std::fs::read_to_string(path)
                .ok()
                .map(|pid| pid.parse::<u32>().unwrap());
            let child_alive = forked.is_some_and(running);
            if let Some(pid) = forked {
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
            let _ = server.kill();
            server.wait().unwrap();
            result.unwrap();
            assert!(!child_alive, "private group: {private}");
        }
    }

    #[test]
    fn a_server_in_a_shared_group_still_gets_to_shut_down_gracefully() {
        let _shared = SHARED_GROUP.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terminated");
        let code = r#"import signal,socket,sys,time
s=socket.socket();s.bind(('127.0.0.1',0));s.listen()
def term(*args):
 open(sys.argv[1],'w').close()
 sys.exit(0)
signal.signal(signal.SIGTERM,term)
print(s.getsockname()[1],flush=True)
time.sleep(60)
"#;
        let mut server = Command::new("python3")
            .args(["-c", code, path.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line.trim().parse().unwrap();
        let result = stop(&target(port, server.id()));
        let _ = server.kill();
        server.wait().unwrap();
        result.unwrap();
        assert!(path.exists(), "the server never received SIGTERM");
    }

    fn listening_within(port: u16, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !Sockets::scan().unwrap().listeners(port).is_empty() {
                return true;
            }
            thread::sleep(POLL);
        }
        false
    }

    #[test]
    fn stopping_the_parent_keeps_a_watcher_from_starting_the_server_again() {
        let _shared = SHARED_GROUP.lock().unwrap_or_else(|e| e.into_inner());
        let port = free_port();
        let server = "import socket,sys,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',int(sys.argv[1])));s.listen();time.sleep(60)";
        let code = r#"$|=1;my($server,$port)=@ARGV;
while(1){my $pid=fork();if(!$pid){exec('python3','-c',$server,$port)}print "$pid\n";waitpid($pid,0);select(undef,undef,undef,0.1)}"#;
        let mut watcher = Command::new("perl")
            .args(["-e", code, server, &port.to_string()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(watcher.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let server: u32 = line.trim().parse().unwrap();
        assert!(listening_within(port, Duration::from_secs(5)));
        let selected = target(port, server);
        let parent = ProcessIdentity::read(watcher.id()).unwrap();

        let result = Service::locate(&selected, Some(&parent)).map(|service| {
            let preview = service.preview();
            (preview, service.stop(Shutdown::default()))
        });
        thread::sleep(Duration::from_millis(500));
        let started_again = !Sockets::scan().unwrap().listeners(port).is_empty();
        let _ = watcher.kill();
        let _ = watcher.wait();

        let (preview, stopped) = result.unwrap();
        stopped.unwrap();
        let pids: Vec<_> = preview.processes.iter().map(|p| p.process.pid).collect();
        assert!(
            pids.contains(&watcher.id()) && pids.contains(&server),
            "{pids:?}"
        );
        let listener = preview
            .processes
            .iter()
            .find(|p| p.process.pid == server)
            .unwrap();
        assert_eq!(listener.ports, vec![port]);
        assert!(!started_again);
    }

    #[test]
    fn a_control_goes_through_pm2_for_an_app_of_its_daemon() {
        let port = free_port();
        let server = format!(
            "import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',{port}));s.listen();print(1,flush=True);time.sleep(30)"
        );
        let mut daemon = Command::new("bash")
            .args([
                "-c",
                &format!(
                    "exec -a 'PM2 v5.3.0: God Daemon (/nonexistent/.pm2)' bash -c \"python3 -c \\\"{server}\\\" & wait\""
                ),
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let listener = *Sockets::scan().unwrap().listeners(port).first().unwrap();
        let result =
            Service::locate(&target(port, listener), None).map(|service| service.preview());
        unsafe {
            libc::kill(listener as i32, libc::SIGKILL);
        }
        let _ = daemon.kill();
        let _ = daemon.wait();
        let error = result.unwrap_err();
        assert!(
            error.contains("can't find the pm2 command") || error.contains("Can't ask PM2"),
            "{error}"
        );
    }

    #[test]
    fn a_control_refuses_pm2s_own_daemon_since_it_runs_every_app() {
        let port = free_port();
        let mut daemon = Command::new("bash")
            .args([
                "-c",
                &format!(
                    "exec -a 'PM2 v5.3.0: God Daemon (/nonexistent/.pm2)' nc -l 127.0.0.1 {port}"
                ),
            ])
            .spawn()
            .unwrap();
        let listening = listening_within(port, Duration::from_secs(5));
        let result = Service::locate(&target(port, daemon.id()), None).map(|_| ());
        let _ = daemon.kill();
        let _ = daemon.wait();
        assert!(listening);
        let error = result.unwrap_err();
        assert!(error.contains("would stop every PM2 app"), "{error}");
    }

    #[test]
    fn force_kill_ends_the_server_without_its_shutdown_handler() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terminated");
        let code = r#"import signal,socket,sys,time
s=socket.socket();s.bind(('127.0.0.1',0));s.listen()
def term(*args):
 open(sys.argv[1],'w').close()
 sys.exit(0)
signal.signal(signal.SIGTERM,term)
print(s.getsockname()[1],flush=True)
time.sleep(60)
"#;
        let mut server = Command::new("python3")
            .args(["-c", code, path.to_str().unwrap()])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line.trim().parse().unwrap();
        let force = Shutdown {
            force: true,
            ..Shutdown::default()
        };
        let result = Service::locate(&target(port, server.id()), None).and_then(|s| s.stop(force));
        let status = server.wait().unwrap();
        result.unwrap();
        assert!(
            !path.exists(),
            "Force Kill let the server shut down gracefully"
        );
        assert!(status.code().is_none());
    }

    #[test]
    fn stop_waits_the_configured_grace_before_killing() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, _) = start(port, dir.path(), &["ignore-term"]);
        let grace = Shutdown {
            grace: Duration::from_secs(1),
            force: false,
        };
        let started = Instant::now();
        let result = Service::locate(&target(port, server.id()), None).and_then(|s| s.stop(grace));
        let elapsed = started.elapsed();
        server.wait().unwrap();
        result.unwrap();
        assert!(elapsed >= Duration::from_secs(1), "{elapsed:?}");
        assert!(elapsed < DEFAULT_GRACE, "{elapsed:?}");
    }

    #[test]
    fn a_confirmed_preview_cannot_authorize_a_process_it_did_not_list() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, worker) = start(port, dir.path(), &[]);
        let selected = target(port, server.id());
        let service = Service::locate(&selected, None).unwrap();
        let listed: Vec<_> = service
            .preview()
            .processes
            .into_iter()
            .map(|p| p.process)
            .collect();
        let without_worker: Vec<_> = listed
            .iter()
            .filter(|id| id.pid != worker)
            .cloned()
            .collect();

        let refused = service.ensure_within(&without_worker);
        let replaced: Vec<_> = listed
            .iter()
            .map(|id| ProcessIdentity {
                start: if id.pid == worker {
                    format!("{}-old", id.start)
                } else {
                    id.start.clone()
                },
                ..id.clone()
            })
            .collect();
        let refused_replacement = service.ensure_within(&replaced);
        let accepted = service.ensure_within(&listed);
        let worker_alive = running(worker);
        stop(&selected).unwrap();
        server.wait().unwrap();

        assert!(refused.unwrap_err().contains("changed"));
        assert!(refused_replacement.is_err());
        accepted.unwrap();
        assert!(worker_alive);
    }

    /// Needs a macOS login session, so it runs only on request: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn a_homebrew_service_is_found_through_launchd_and_never_reported_stopped_by_mistake() {
        let label = "homebrew.mxcl.ports-test-service";
        let port = free_port();
        let server = format!(
            "import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',{port}));s.listen();time.sleep(600)"
        );
        // Homebrew's own, since Xcode's python3 runs from inside Xcode.app.
        let python = ["/opt/homebrew/bin/python3", "/usr/local/bin/python3"]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .expect("a Homebrew python3");
        let submitted = Command::new("/bin/launchctl")
            .args(["submit", "-l", label, "--", python, "-c", &server])
            .status()
            .unwrap();
        assert!(submitted.success());
        let listening = listening_within(port, Duration::from_secs(10));
        let mut sampler =
            super::super::sampler::Sampler::new(std::env::temp_dir(), Default::default());
        let row = sampler
            .sample()
            .unwrap()
            .servers
            .into_iter()
            .find(|row| row.port == port);
        let located = row.as_ref().and_then(|row| match &row.target {
            Some(Target::Process(target)) => Some(Service::locate(target, None)),
            _ => None,
        });
        let preview = located
            .as_ref()
            .map(|service| service.as_ref().map(Service::preview).map_err(Clone::clone));
        let stopped = located.map(|service| service.and_then(|s| s.stop(Shutdown::default())));
        let still_listening = !Sockets::scan().unwrap().listeners(port).is_empty();
        let _ = Command::new("/bin/launchctl")
            .args(["remove", label])
            .status();

        assert!(listening);
        let row = row.unwrap();
        let supervisor = row.supervisor.unwrap();
        assert_eq!(supervisor.name.as_deref(), Some("ports-test-service"));
        assert!(row.parent.is_none());
        let preview = preview.unwrap().unwrap();
        assert_eq!(
            preview.supervisor.unwrap().kind,
            supervisors::Kind::Homebrew
        );
        let error = stopped.unwrap().unwrap_err();
        assert!(
            error.contains("Can't stop ports-test-service (Homebrew)"),
            "{error}"
        );
        assert!(still_listening);
    }

    #[test]
    fn restart_readiness_does_not_accept_an_unrelated_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let identity = ProcessIdentity::read(std::process::id()).unwrap();
        let target = ProcessTarget {
            port,
            listener: identity.clone(),
            root: identity,
            addresses: vec!["127.0.0.1".parse().unwrap()],
        };
        let child = Command::new("sleep").arg("1").spawn().unwrap();
        let error =
            wait_until_listening(&target, child, "sleep", Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("isn't listening"), "{error}");
    }

    #[test]
    fn stopping_frees_the_port_and_ends_the_children() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, worker) = start(port, dir.path(), &[]);

        stop(&target(port, server.id())).unwrap();

        assert!(Sockets::scan().unwrap().listeners(port).is_empty());
        assert!(!running(worker));
        server.wait().unwrap();
    }

    #[test]
    fn stopping_kills_a_server_that_ignores_sigterm() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, _) = start(port, dir.path(), &["ignore-term"]);

        stop(&target(port, server.id())).unwrap();

        assert!(Sockets::scan().unwrap().listeners(port).is_empty());
        assert!(server.wait().unwrap().code().is_none());
    }

    #[test]
    fn refuses_to_stop_a_stale_process_start_identity() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, _worker) = start(port, dir.path(), &[]);

        let mut stale = target(port, server.id());
        stale.listener.start.push_str("-old");
        let error = stop(&stale).unwrap_err();

        assert!(error.contains("changed"), "{error}");
        stop(&target(port, server.id())).unwrap();
        server.wait().unwrap();
    }

    #[test]
    fn restarting_brings_the_server_back_on_the_same_port_and_folder() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, _) = start(port, dir.path(), &[]);

        restart(&target(port, server.id())).unwrap();
        server.wait().unwrap();

        let listeners = Sockets::scan().unwrap().listeners(port);
        let new_pid = *listeners.first().unwrap();
        assert_ne!(new_pid, server.id());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(new_pid)]),
            true,
            ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
        );
        let cwd = system
            .process(Pid::from_u32(new_pid))
            .unwrap()
            .cwd()
            .unwrap();
        assert_eq!(
            cwd.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
        stop(&target(port, new_pid)).unwrap();
    }
}
