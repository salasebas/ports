//! Identity-checked control of one logical service and its bindings.

use super::containers::ContainerTarget;
use super::identity::ProcessIdentity;
use super::processes::{self, Groups, Job, OsGroups, ProcessTable};
use super::sampler::can_relaunch;
use super::sockets::Sockets;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::net::IpAddr;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind, Users,
};

const TERM_GRACE: Duration = Duration::from_secs(3);
const KILL_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Reserve every affected process, since a parent and child listener can
/// have different roots while their destructive scopes still overlap.
pub fn scope(target: &ProcessTarget) -> Result<Vec<String>, String> {
    let mut keys = vec![format!("process:{}", target.root.key())];
    if let Some((job, _)) = locate(target)? {
        keys.extend(
            job.pids
                .into_iter()
                .filter_map(ProcessIdentity::read)
                .map(|id| format!("process:{}", id.key())),
        );
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

pub fn stop(target: &ProcessTarget) -> Result<(), String> {
    let Some((job, system)) = locate(target)? else {
        return Ok(());
    };
    end(&job, &system, target)?;
    ensure_free(target)
}

pub fn restart(target: &ProcessTarget) -> Result<(), String> {
    let Some((job, mut system)) = locate(target)? else {
        return Err(format!("Nothing listens on :{} anymore", target.port));
    };
    let launch = Launch::capture(job.root, &mut system)?;
    end(&job, &system, target)?;
    ensure_free(target)?;
    let child = launch.spawn()?;
    wait_until_listening(target, child, &launch.name, LISTEN_TIMEOUT)
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
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_user(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .without_tasks(),
    );
    system
}

fn locate(target: &ProcessTarget) -> Result<Option<(Job, System)>, String> {
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
    let system = process_table();
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
            let own = std::process::id();
            if std::iter::once(own)
                .chain(table.ancestors(own))
                .any(|pid| job.pids.contains(&pid))
            {
                return Err("Ports can't stop the process that started it".into());
            }
            return Ok(Some((job, system)));
        }
    }
    Err(format!(
        "The owner of :{} changed or cannot be inspected; refresh before trying again",
        target.port
    ))
}

fn end(job: &Job, system: &System, selected: &ProcessTarget) -> Result<(), String> {
    if !selected.root.is_current() || !selected.listener.is_current() {
        return Err("The selected service changed before shutdown".into());
    }
    if job.group.is_none() {
        return end_shared_group(job, system, selected);
    }
    let leader = selected.root.clone();
    let session = OsGroups.sid(job.root);
    let uid = system
        .process(Pid::from_u32(job.root))
        .and_then(Process::user_id)
        .map(|uid| **uid);
    let mut targets: BTreeSet<_> = job
        .pids
        .iter()
        .filter_map(|pid| {
            if *pid == selected.root.pid {
                Some(selected.root.clone())
            } else if *pid == selected.listener.pid {
                Some(selected.listener.clone())
            } else {
                ProcessIdentity::read(*pid)
            }
        })
        .collect();
    for (signal, grace) in [(libc::SIGTERM, TERM_GRACE), (libc::SIGKILL, KILL_GRACE)] {
        let deadline = Instant::now() + grace;
        let mut sent = HashSet::new();
        let mut empty_rounds = 0;
        loop {
            let current = process_table();
            let table = ProcessTable::new(&current);
            let leader_now = ProcessIdentity::read(leader.pid);
            let group_not_reused = leader_now.as_ref().is_none_or(|id| id == &leader);
            let descendants = if leader_now.as_ref() == Some(&leader) {
                table.descendants(leader.pid)
            } else {
                BTreeSet::new()
            };
            for (pid, process) in current.processes() {
                let pid = pid.as_u32();
                let in_group = group_not_reused
                    && job
                        .group
                        .is_some_and(|group| OsGroups.pgid(pid) == Some(group))
                    && OsGroups.sid(pid) == session
                    && process.user_id().map(|uid| **uid) == uid;
                if (in_group || descendants.contains(&pid))
                    && process.status() != ProcessStatus::Zombie
                    && let Some(identity) = ProcessIdentity::read(pid)
                {
                    targets.insert(identity);
                }
            }
            let living = alive(&targets);
            if living.is_empty() {
                // A parent may fork after this round's process scan and exit
                // before alive() runs. Scan again after observing its exit.
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

/// A shared group cannot be swept after an orphan forks. Freeze this tree
/// before force-stopping it, so shutdown handlers cannot create untracked
/// children and unrelated group members never receive a signal.
fn end_shared_group(job: &Job, system: &System, selected: &ProcessTarget) -> Result<(), String> {
    struct Frozen(BTreeSet<ProcessIdentity>);
    impl Drop for Frozen {
        fn drop(&mut self) {
            for target in &self.0 {
                if target.is_current() {
                    unsafe {
                        libc::kill(target.pid as libc::pid_t, libc::SIGCONT);
                    }
                }
            }
        }
    }
    let mut frozen = Frozen(BTreeSet::new());
    let root = selected.root.clone();
    let deadline = Instant::now() + TERM_GRACE;
    let mut pending: BTreeSet<_> = job
        .pids
        .iter()
        .filter_map(|pid| ProcessIdentity::read(*pid))
        .collect();
    pending.retain(|id| id.pid != selected.root.pid && id.pid != selected.listener.pid);
    pending.insert(root.clone());
    pending.insert(selected.listener.clone());
    loop {
        // Stop parents first; a stopped parent cannot fork or reparent its children.
        let mut ordered: Vec<_> = pending.iter().collect();
        ordered.sort_by_key(|identity| if identity.pid == root.pid { 0 } else { 1 });
        for target in ordered {
            if !frozen.0.contains(target) && target.is_current() {
                send(
                    unsafe { libc::kill(target.pid as libc::pid_t, libc::SIGSTOP) },
                    target.pid,
                    system,
                )?;
                frozen.0.insert(target.clone());
            }
        }
        let current = process_table();
        let table = ProcessTable::new(&current);
        let all_stopped = frozen.0.iter().filter(|id| id.is_current()).all(|id| {
            current
                .process(Pid::from_u32(id.pid))
                .is_none_or(|process| {
                    matches!(
                        process.status(),
                        ProcessStatus::Stop | ProcessStatus::Zombie
                    )
                })
        });
        let discovered: BTreeSet<_> = frozen
            .0
            .iter()
            .filter(|id| id.is_current())
            .flat_map(|id| table.descendants(id.pid))
            .filter_map(ProcessIdentity::read)
            .collect();
        let added = !discovered.is_subset(&frozen.0);
        pending.extend(discovered);
        if !added && all_stopped {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Could not safely freeze the service tree".into());
        }
        thread::sleep(POLL);
    }
    for target in &frozen.0 {
        if target.is_current() {
            send(
                unsafe { libc::kill(target.pid as libc::pid_t, libc::SIGKILL) },
                target.pid,
                system,
            )?;
        }
    }
    let deadline = Instant::now() + KILL_GRACE;
    while !alive(&frozen.0).is_empty() {
        if Instant::now() >= deadline {
            return Err("The service tree did not exit".into());
        }
        thread::sleep(POLL);
    }
    Ok(())
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

/// Everything needed to start a server the way it was started before.
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
        // Prefer the exact binary; fall back to a PATH lookup if it was replaced since.
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
            // Its own group, so it outlives Ports and Stop can end it as a whole.
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
            if sockets.listening.iter().any(|(socket, owners)| {
                socket.port() == target.port
                    && target.addresses.iter().any(|ip| {
                        *ip == socket.ip()
                            || ip.is_ipv4() == socket.ip().is_ipv4()
                                && (ip.is_unspecified() || socket.ip().is_unspecified())
                    })
                    && !owners.is_disjoint(&descendants)
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

    /// Listens on the port in argv[1], spawns a child, and prints the child's
    /// PID once ready. With "ignore-term" it shrugs off SIGTERM.
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

    /// Starts the server in its own process group, as a shell job would be.
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
        let (job, system) = locate(&selected).unwrap().unwrap();
        let mut stale = selected.clone();
        stale.root.start.push_str("-reused");
        let result = end(&job, &system, &stale);
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
        let keys = scope(&selected).unwrap();
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
        let mut sampler = super::super::sampler::Sampler::new(dir.path().to_path_buf());
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

    #[test]
    fn shutdown_cannot_leave_a_newly_forked_child_in_private_or_shared_groups() {
        for private in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("forked");
            let code = r#"import os,signal,socket,time,sys
s=socket.socket();s.bind(('127.0.0.1',0));s.listen()
def term(*args):
 p=os.fork()
 if p==0:
  s.close()
  with open(sys.argv[1],'w') as f:f.write(str(os.getpid()))
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
