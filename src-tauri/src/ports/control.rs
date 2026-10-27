//! Stopping servers and starting them again.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{
    Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind, Users,
};

use super::processes::{self, Groups, Job, OsGroups, ProcessTable};
use super::sampler::can_relaunch;
use super::sockets::Sockets;

const TERM_GRACE: Duration = Duration::from_secs(3);
const KILL_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(100);
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);

/// A process as it was when Ports decided to end it. A different start time
/// later means the PID now belongs to another process, which is left alone.
#[derive(Debug, Clone, Copy)]
struct Target {
    pid: u32,
    started: u64,
}

/// Ends the server on `port`: SIGTERM to its process group and tree, SIGKILL
/// to whatever is left after a grace period, then checks the port is free.
pub fn stop(port: u16, pid: u32) -> Result<(), String> {
    let Some((job, system)) = locate(port, pid)? else {
        return Ok(());
    };
    end(&job, &system)?;
    ensure_free(port)
}

/// Stops the server on `port` and launches what started it again, with the
/// same command line, folder and environment, detached from Ports.
pub fn restart(port: u16, pid: u32) -> Result<(), String> {
    let Some((job, mut system)) = locate(port, pid)? else {
        return Err(format!("Nothing listens on :{port} anymore"));
    };
    let launch = Launch::capture(job.root, &mut system)?;
    end(&job, &system)?;
    ensure_free(port)?;
    let child = launch.spawn()?;
    wait_until_listening(port, child, &launch.name)
}

/// The job behind `pid` on `port`, or `None` when the port is already free.
/// Processes that hold the published ports of every container of a runtime.
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

fn locate(port: u16, pid: u32) -> Result<Option<(Job, System)>, String> {
    let sockets = Sockets::scan();
    let Some(listeners) = sockets.listening.get(&port) else {
        return Ok(None);
    };
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_user(UpdateKind::Always)
            .without_tasks(),
    );
    if !listeners.contains(&pid) {
        let owner = listeners.first().copied().unwrap_or_default();
        return Err(format!(":{port} is now used by {}", label(&system, owner)));
    }
    let table = ProcessTable::new(&system);
    let job = processes::job(listeners, &table, &OsGroups);
    if let Some(name) = system
        .process(Pid::from_u32(job.listener))
        .map(|p| p.name().to_string_lossy().into_owned())
        .filter(|name| FORWARDERS.contains(&name.as_str()))
    {
        return Err(format!(
            "{name} forwards :{port} for a container runtime; stopping it would stop every container. Stop the container instead."
        ));
    }
    if table.user(job.listener) != Some(unsafe { libc::getuid() }) {
        return Err(not_permitted(&system, job.listener));
    }
    let own = std::process::id();
    let lineage = std::iter::once(own).chain(table.ancestors(own));
    if lineage.into_iter().any(|p| job.pids.contains(&p)) {
        return Err("Ports can't stop the process that started it".to_string());
    }
    Ok(Some((job, system)))
}

fn end(job: &Job, system: &System) -> Result<(), String> {
    let targets: Vec<Target> = job
        .pids
        .iter()
        .filter_map(|pid| {
            let process = system.process(Pid::from_u32(*pid))?;
            Some(Target {
                pid: *pid,
                started: process.start_time(),
            })
        })
        .collect();

    for (signal, grace) in [(libc::SIGTERM, TERM_GRACE), (libc::SIGKILL, KILL_GRACE)] {
        signal_all(job.group, &targets, signal, system)?;
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if alive(&targets).is_empty() {
                return Ok(());
            }
            thread::sleep(POLL);
        }
    }
    match alive(&targets).first() {
        Some(survivor) => Err(format!("{} didn't exit", label(system, survivor.pid))),
        None => Ok(()),
    }
}

fn signal_all(
    group: Option<u32>,
    targets: &[Target],
    signal: libc::c_int,
    system: &System,
) -> Result<(), String> {
    let living = alive(targets);
    // Signalling the group also reaches processes forked after the scan, but
    // only while a known member is still in it: an empty group's ID can be reused.
    if let Some(group) = group
        && living.iter().any(|t| OsGroups.pgid(t.pid) == Some(group))
    {
        send(
            unsafe { libc::killpg(group as libc::pid_t, signal) },
            group,
            system,
        )?;
    }
    for target in living {
        send(
            unsafe { libc::kill(target.pid as libc::pid_t, signal) },
            target.pid,
            system,
        )?;
    }
    Ok(())
}

fn send(result: libc::c_int, pid: u32, system: &System) -> Result<(), String> {
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        // Exited between the check and the signal.
        Some(libc::ESRCH) => Ok(()),
        Some(libc::EPERM) => Err(not_permitted(system, pid)),
        _ => Err(format!("Can't stop {}: {error}", label(system, pid))),
    }
}

/// The targets still running as the same processes. Zombies have exited and
/// only wait for their parent to collect them.
fn alive(targets: &[Target]) -> Vec<Target> {
    let pids: Vec<Pid> = targets.iter().map(|t| Pid::from_u32(t.pid)).collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    targets
        .iter()
        .filter(|t| {
            system
                .process(Pid::from_u32(t.pid))
                .is_some_and(|p| p.start_time() == t.started && p.status() != ProcessStatus::Zombie)
        })
        .copied()
        .collect()
}

fn ensure_free(port: u16) -> Result<(), String> {
    let sockets = Sockets::scan();
    let Some(holder) = sockets.listening.get(&port).and_then(|p| p.first()) else {
        return Ok(());
    };
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(*holder)]), true);
    Err(format!(
        ":{port} is still in use by {}",
        label(&system, *holder)
    ))
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

fn wait_until_listening(port: u16, mut child: Child, name: &str) -> Result<(), String> {
    let deadline = Instant::now() + LISTEN_TIMEOUT;
    let result = loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "{name} exited ({status}) before listening on :{port}"
            ));
        }
        if Sockets::scan().listening.contains_key(&port) {
            break Ok(());
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "{name} started again but isn't listening on :{port} yet"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    };
    // Collect the exit status whenever it stops, so it doesn't linger as a zombie.
    thread::spawn(move || child.wait());
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
    fn stopping_frees_the_port_and_ends_the_children() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, worker) = start(port, dir.path(), &[]);

        stop(port, server.id()).unwrap();

        assert!(!Sockets::scan().listening.contains_key(&port));
        assert!(!running(worker));
        server.wait().unwrap();
    }

    #[test]
    fn stopping_kills_a_server_that_ignores_sigterm() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, _) = start(port, dir.path(), &["ignore-term"]);

        stop(port, server.id()).unwrap();

        assert!(!Sockets::scan().listening.contains_key(&port));
        assert!(server.wait().unwrap().code().is_none());
    }

    #[test]
    fn refuses_to_stop_a_pid_that_no_longer_holds_the_port() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, worker) = start(port, dir.path(), &[]);

        let error = stop(port, worker).unwrap_err();

        assert!(
            error.contains(&format!(":{port} is now used by")),
            "{error}"
        );
        stop(port, server.id()).unwrap();
        server.wait().unwrap();
    }

    #[test]
    fn restarting_brings_the_server_back_on_the_same_port_and_folder() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let (mut server, _) = start(port, dir.path(), &[]);

        restart(port, server.id()).unwrap();
        server.wait().unwrap();

        let listeners = Sockets::scan().listening.remove(&port).unwrap();
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
        stop(port, new_pid).unwrap();
    }
}
