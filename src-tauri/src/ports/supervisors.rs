//! Supervisors that start a service again as soon as it exits. Ports stops
//! and restarts the services they manage through their own commands, since
//! a signal would only make them start the service again.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, System};

use super::processes::ProcessTable;

const HOMEBREW_LABEL: &str = "homebrew.mxcl.";
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Homebrew,
    Pm2,
    Supervisord,
}

/// What manages a server, as the snapshot reports it. Mirrors `Supervisor`
/// in `src/lib/ports/types.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Supervisor {
    pub kind: Kind,
    /// The Homebrew formula, or the PM2 app once PM2 confirmed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Ports stops and restarts it through the supervisor; otherwise only
    /// signals reach it, and the supervisor may start it again.
    pub supported: bool,
}

/// A supervisor found from the process tree, before its own tooling confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// A launchd job of this user labelled `homebrew.mxcl.<formula>`.
    Homebrew {
        formula: String,
        pid: u32,
    },
    /// A child of the PM2 daemon whose home is `home`.
    Pm2 {
        home: PathBuf,
        daemon: u32,
        pid: u32,
    },
    Supervisord {
        pid: u32,
    },
}

impl Found {
    pub fn summary(&self) -> Supervisor {
        match self {
            Self::Homebrew { formula, .. } => Supervisor {
                kind: Kind::Homebrew,
                name: Some(formula.clone()),
                supported: true,
            },
            Self::Pm2 { .. } => Supervisor {
                kind: Kind::Pm2,
                name: None,
                supported: true,
            },
            Self::Supervisord { .. } => Supervisor {
                kind: Kind::Supervisord,
                name: None,
                supported: false,
            },
        }
    }
}

/// The launchd jobs of this user that have a process, by PID; read once
/// per sample and only when a server's ancestry reaches launchd.
#[derive(Default)]
pub struct LaunchdJobs(Option<HashMap<u32, String>>);

impl LaunchdJobs {
    fn label(&mut self, pid: u32) -> Option<&str> {
        self.0
            .get_or_insert_with(|| launchd_jobs().unwrap_or_default())
            .get(&pid)
            .map(String::as_str)
    }
}

/// The nearest supervisor above `pid`: PM2's daemon, supervisord, or launchd
/// running a Homebrew service. Watchers such as nodemon are parents, not supervisors.
pub fn find(
    pid: u32,
    system: &System,
    table: &ProcessTable,
    jobs: &mut LaunchdJobs,
) -> Option<Found> {
    let chain: Vec<u32> = std::iter::once(pid).chain(table.ancestors(pid)).collect();
    for pair in chain.windows(2) {
        let [child, ancestor] = [pair[0], pair[1]];
        if ancestor == 1 {
            // Homebrew runs command-line programs as this user, never an app.
            let process = system.process(Pid::from_u32(child))?;
            let own = process
                .user_id()
                .is_some_and(|uid| **uid == unsafe { libc::getuid() });
            if !own || process.exe().is_none_or(super::project::is_app_bundle) {
                return None;
            }
            let formula = homebrew_formula(jobs.label(child)?)?;
            return Some(Found::Homebrew {
                formula,
                pid: child,
            });
        }
        let process = system.process(Pid::from_u32(ancestor))?;
        // A retitled process shows its title as the first argument.
        let title = process.cmd().first().map(|arg| arg.to_string_lossy());
        if let Some(home) = title.as_deref().and_then(|title| pm2_home(title.trim())) {
            return Some(Found::Pm2 {
                home,
                daemon: ancestor,
                pid: child,
            });
        }
        let named = |name: &str| name == "supervisord";
        if named(&process.name().to_string_lossy())
            || process.cmd().iter().take(2).any(|arg| {
                Path::new(arg)
                    .file_name()
                    .is_some_and(|name| named(&name.to_string_lossy()))
            })
        {
            return Some(Found::Supervisord { pid: child });
        }
    }
    None
}

/// `postgresql@16` for `homebrew.mxcl.postgresql@16`, if it is a name brew accepts.
fn homebrew_formula(label: &str) -> Option<String> {
    let formula = label.strip_prefix(HOMEBREW_LABEL)?;
    let valid = !formula.is_empty()
        && !formula.starts_with('-')
        && formula
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c));
    valid.then(|| formula.to_string())
}

/// PM2 titles its daemon `PM2 v5.3.0: God Daemon (/Users/me/.pm2)`.
fn pm2_home(title: &str) -> Option<PathBuf> {
    let rest = title.strip_prefix("PM2 v")?;
    let (_, home) = rest.split_once(": God Daemon (")?;
    let home = home.strip_suffix(')')?;
    Path::new(home).is_absolute().then(|| PathBuf::from(home))
}

/// `launchctl list` prints `PID\tStatus\tLabel`, with `-` for jobs that aren't running.
fn parse_launchctl(output: &str) -> HashMap<u32, String> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let pid = fields.next()?.parse().ok()?;
            let label = fields.nth(1)?;
            Some((pid, label.to_string()))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn launchd_jobs() -> Option<HashMap<u32, String>> {
    let output = run(Command::new("/bin/launchctl").arg("list"), LIST_TIMEOUT).ok()?;
    Some(parse_launchctl(&output))
}

/// Homebrew on Linux runs services through systemd, which Ports doesn't control yet.
#[cfg(not(target_os = "macos"))]
fn launchd_jobs() -> Option<HashMap<u32, String>> {
    None
}

/// A supervised service, confirmed by the supervisor itself just now.
#[derive(Debug, Clone)]
pub enum Managed {
    Homebrew {
        brew: PathBuf,
        formula: String,
    },
    Pm2 {
        pm2: PathBuf,
        home: PathBuf,
        daemon: u32,
        id: u64,
        name: String,
    },
}

impl Managed {
    /// Asks the supervisor whether it really runs `found`, so the same
    /// executable on another port, or a name alone, can't be mistaken for it.
    pub fn confirm(found: &Found, system: &System) -> Result<Option<Self>, String> {
        match found {
            Found::Supervisord { .. } => Ok(None),
            Found::Homebrew { formula, pid } => {
                let brew = homebrew_command().ok_or_else(|| {
                    format!(
                        "Homebrew runs {formula}, but Ports can't find brew. Stop it with `brew services stop {formula}`."
                    )
                })?;
                let label = format!("{HOMEBREW_LABEL}{formula}");
                let running = launchd_jobs()
                    .and_then(|jobs| jobs.get(pid).cloned())
                    .is_some_and(|current| current == label);
                if !running {
                    return Err(format!(
                        "Homebrew no longer runs {formula} in that process; refresh before trying again"
                    ));
                }
                Ok(Some(Self::Homebrew {
                    brew,
                    formula: formula.clone(),
                }))
            }
            Found::Pm2 { home, daemon, pid } => {
                let pm2 = pm2_command(system, *daemon).ok_or(
                    "PM2 runs this server, but Ports can't find the pm2 command. Stop it with `pm2 stop <name>`.",
                )?;
                let apps = pm2_apps(&pm2, home)?;
                let app = apps
                    .into_iter()
                    .find(|app| app.pid == Some(*pid))
                    .ok_or("PM2 no longer runs this server; refresh before trying again")?;
                Ok(Some(Self::Pm2 {
                    pm2,
                    home: home.clone(),
                    daemon: *daemon,
                    id: app.pm_id,
                    name: app.name,
                }))
            }
        }
    }

    pub fn summary(&self) -> Supervisor {
        match self {
            Self::Homebrew { formula, .. } => Supervisor {
                kind: Kind::Homebrew,
                name: Some(formula.clone()),
                supported: true,
            },
            Self::Pm2 { name, .. } => Supervisor {
                kind: Kind::Pm2,
                name: Some(name.clone()),
                supported: true,
            },
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Homebrew { formula, .. } => format!("{formula} (Homebrew)"),
            Self::Pm2 { name, .. } => format!("{name} (PM2)"),
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        self.control("stop")
    }

    pub fn restart(&self) -> Result<(), String> {
        self.control("restart")
    }

    fn control(&self, action: &str) -> Result<(), String> {
        let mut command = match self {
            Self::Homebrew { brew, formula } => {
                let mut command = Command::new(brew);
                command
                    .args(["services", action, formula])
                    .env("HOMEBREW_NO_AUTO_UPDATE", "1")
                    .env("HOMEBREW_NO_ENV_HINTS", "1");
                command
            }
            Self::Pm2 { pm2, home, id, .. } => {
                let mut command = Command::new(pm2);
                command
                    .args([action, &id.to_string()])
                    .env("PM2_HOME", home);
                command
            }
        };
        run(&mut command, CONTROL_TIMEOUT)
            .map(drop)
            .map_err(|error| format!("Can't {action} {}: {error}", self.label()))
    }

    /// The process the supervisor runs for this service now, which a restart replaces.
    pub fn anchor(&self) -> Option<u32> {
        match self {
            Self::Homebrew { formula, .. } => {
                let label = format!("{HOMEBREW_LABEL}{formula}");
                launchd_jobs()?
                    .into_iter()
                    .find_map(|(pid, current)| (current == label).then_some(pid))
            }
            Self::Pm2 { daemon, .. } => Some(*daemon),
        }
    }
}

fn homebrew_command() -> Option<PathBuf> {
    [
        "/opt/homebrew/bin/brew",
        "/usr/local/bin/brew",
        "/home/linuxbrew/.linuxbrew/bin/brew",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

/// The `pm2` installed next to the Node.js that runs the daemon, which a
/// version manager keeps off the PATH that apps opened from the Dock get.
fn pm2_command(system: &System, daemon: u32) -> Option<PathBuf> {
    let beside_node = system
        .process(Pid::from_u32(daemon))
        .and_then(|process| process.exe())
        .and_then(Path::parent)
        .map(|bin| bin.join("pm2"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    beside_node
        .into_iter()
        .chain(["/opt/homebrew/bin/pm2", "/usr/local/bin/pm2"].map(PathBuf::from))
        .chain(std::env::split_paths(&path).map(|dir| dir.join("pm2")))
        .find(|path| path.is_file())
}

#[derive(Debug, Deserialize)]
struct Pm2App {
    pm_id: u64,
    name: String,
    #[serde(default)]
    pid: Option<u32>,
}

fn pm2_apps(pm2: &Path, home: &Path) -> Result<Vec<Pm2App>, String> {
    let output = run(
        Command::new(pm2).arg("jlist").env("PM2_HOME", home),
        LIST_TIMEOUT,
    )
    .map_err(|error| format!("Can't ask PM2 about its apps: {error}"))?;
    parse_pm2(&output)
}

/// `pm2 jlist` may print warnings before its JSON array.
fn parse_pm2(output: &str) -> Result<Vec<Pm2App>, String> {
    let json = output
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("[{") || *line == "[]")
        .ok_or("PM2 didn't list its apps")?;
    serde_json::from_str(json).map_err(|error| format!("Can't read PM2's apps: {error}"))
}

/// Runs a supervisor command and returns its output, or the last line it
/// printed when it fails. A command that hangs is killed after `timeout`.
fn run(command: &mut Command, timeout: Duration) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no output")?;
    let mut stderr = child.stderr.take().ok_or("no output")?;
    let out = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("it didn't finish within {}s", timeout.as_secs()));
        }
        thread::sleep(Duration::from_millis(50));
    };
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if status.success() {
        return Ok(out);
    }
    let last = |text: &str| {
        text.lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(str::to_string)
    };
    let reason = last(&err)
        .or_else(|| last(&out))
        .unwrap_or_else(|| status.to_string());
    Err(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_running_launchd_jobs_and_only_valid_homebrew_formulas() {
        let jobs = parse_launchctl(
            "PID\tStatus\tLabel\n-\t0\thomebrew.mxcl.redis\n812\t0\thomebrew.mxcl.postgresql@16\n90\t-9\tcom.apple.Finder\n",
        );
        assert_eq!(
            jobs,
            HashMap::from([
                (812, "homebrew.mxcl.postgresql@16".to_string()),
                (90, "com.apple.Finder".to_string()),
            ])
        );
        assert_eq!(
            homebrew_formula("homebrew.mxcl.postgresql@16").as_deref(),
            Some("postgresql@16")
        );
        assert_eq!(homebrew_formula("com.apple.Finder"), None);
        assert_eq!(homebrew_formula("homebrew.mxcl.--all"), None);
        assert_eq!(homebrew_formula("homebrew.mxcl.a b"), None);
    }

    #[test]
    fn reads_the_pm2_daemon_title_and_its_apps() {
        assert_eq!(
            pm2_home("PM2 v5.3.0: God Daemon (/Users/me/.pm2)"),
            Some(PathBuf::from("/Users/me/.pm2"))
        );
        assert_eq!(pm2_home("node server.js"), None);
        assert_eq!(pm2_home("PM2 v5.3.0: God Daemon (relative)"), None);
        let apps = parse_pm2(
            "[PM2] Spawning PM2 daemon\n[{\"pm_id\":0,\"name\":\"api\",\"pid\":4100,\"pm2_env\":{}},{\"pm_id\":1,\"name\":\"worker\",\"pid\":0}]\n",
        )
        .unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(
            (apps[0].pm_id, apps[0].name.as_str(), apps[0].pid),
            (0, "api", Some(4100))
        );
    }

    /// Starts a process titled `title`, like PM2 titles its daemon, with a
    /// child standing for the app it runs.
    fn titled(title: &str) -> (std::process::Child, u32) {
        use std::io::BufRead;
        let mut parent = Command::new("bash")
            .args([
                "-c",
                &format!("exec -a '{title}' bash -c 'sleep 30 & echo $!; wait'"),
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(parent.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        (parent, line.trim().parse().unwrap())
    }

    fn found(pid: u32) -> Option<Found> {
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_user(sysinfo::UpdateKind::Always),
        );
        find(
            pid,
            &system,
            &ProcessTable::new(&system),
            &mut LaunchdJobs::default(),
        )
    }

    #[test]
    fn finds_pm2_and_supervisord_from_the_process_tree() {
        let (mut daemon, app) = titled("PM2 v5.3.0: God Daemon (/tmp/pm2-home)");
        let pm2 = found(app);
        let (mut supervisord, program) = titled("/usr/local/bin/supervisord");
        let other = found(program);
        for child in [&mut daemon, &mut supervisord] {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert_eq!(
            pm2,
            Some(Found::Pm2 {
                home: PathBuf::from("/tmp/pm2-home"),
                daemon: daemon.id(),
                pid: app,
            })
        );
        assert_eq!(other, Some(Found::Supervisord { pid: program }));
        assert_eq!(found(std::process::id()), None);
    }

    #[test]
    fn reports_the_last_line_of_a_failing_command_and_kills_a_hanging_one() {
        let error = run(
            Command::new("sh").args([
                "-c",
                "echo working; echo 'Error: no such formula' >&2; exit 1",
            ]),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert_eq!(error, "Error: no such formula");
        let error = run(Command::new("sleep").arg("5"), Duration::from_millis(200)).unwrap_err();
        assert!(error.contains("didn't finish"), "{error}");
    }
}
