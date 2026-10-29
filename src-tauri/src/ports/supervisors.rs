use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, System};

use super::identity::ProcessIdentity;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Supervisor {
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub controllable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Homebrew {
        formula: String,
        pid: u32,
    },
    Pm2 {
        home: PathBuf,
        daemon: u32,
        pid: u32,
    },
    Supervisord {
        pid: u32,
    },
    /// PM2's daemon itself holds the port for its cluster-mode apps; ending
    /// it would end every PM2 app.
    Pm2Daemon,
}

impl Found {
    pub fn summary(&self) -> Supervisor {
        match self {
            Self::Homebrew { formula, .. } => Supervisor {
                kind: Kind::Homebrew,
                name: Some(formula.clone()),
                controllable: true,
            },
            Self::Pm2 { .. } => Supervisor {
                kind: Kind::Pm2,
                name: None,
                controllable: true,
            },
            Self::Supervisord { .. } => Supervisor {
                kind: Kind::Supervisord,
                name: None,
                controllable: false,
            },
            Self::Pm2Daemon => Supervisor {
                kind: Kind::Pm2,
                name: None,
                controllable: false,
            },
        }
    }
}

fn title(process: &sysinfo::Process) -> Option<String> {
    // A retitled process shows its title as the first argument.
    let title = process.cmd().first()?.to_string_lossy();
    Some(title.trim().to_string())
}

#[derive(Default)]
pub struct LaunchdJobs(Option<Option<HashMap<u32, String>>>);

impl LaunchdJobs {
    fn label(&mut self, pid: u32) -> Option<&str> {
        self.0
            .get_or_insert_with(launchd_jobs)
            .as_ref()?
            .get(&pid)
            .map(String::as_str)
    }

    pub fn failed(&self) -> bool {
        matches!(self.0, Some(None))
    }
}

pub fn find(
    pid: u32,
    system: &System,
    table: &ProcessTable,
    jobs: &mut LaunchdJobs,
) -> Option<Found> {
    if system
        .process(Pid::from_u32(pid))
        .and_then(title)
        .is_some_and(|title| pm2_home(&title).is_some())
    {
        return Some(Found::Pm2Daemon);
    }
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
        if let Some(home) = title(process).and_then(|title| pm2_home(&title)) {
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
#[cfg(any(target_os = "macos", test))]
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

#[cfg(not(target_os = "macos"))]
fn launchd_jobs() -> Option<HashMap<u32, String>> {
    Some(HashMap::new())
}

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
        ids: Vec<u64>,
        pids: Vec<u32>,
        name: String,
    },
}

impl Managed {
    pub fn confirm(found: &Found, system: &System) -> Result<Option<Self>, String> {
        match found {
            Found::Supervisord { .. } => Ok(None),
            Found::Pm2Daemon => Err(
                "PM2's daemon holds this port for its cluster-mode apps, and stopping it would stop every PM2 app. Stop the app with `pm2 stop <name>`."
                    .into(),
            ),
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
                    .iter()
                    .find(|app| app.pid == Some(*pid))
                    .ok_or("PM2 no longer runs this server; refresh before trying again")?;
                let instances = instances_of(&apps, app);
                Ok(Some(Self::Pm2 {
                    pm2,
                    home: home.clone(),
                    daemon: *daemon,
                    ids: instances.iter().map(|app| app.pm_id).collect(),
                    pids: instances.iter().filter_map(|app| app.pid).collect(),
                    name: app.name.clone(),
                }))
            }
        }
    }

    pub fn summary(&self) -> Supervisor {
        match self {
            Self::Homebrew { formula, .. } => Supervisor {
                kind: Kind::Homebrew,
                name: Some(formula.clone()),
                controllable: true,
            },
            Self::Pm2 { name, ids, .. } => Supervisor {
                kind: Kind::Pm2,
                name: Some(match ids.len() {
                    0 | 1 => name.clone(),
                    n => format!("{name} ({n} instances)"),
                }),
                controllable: true,
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
            Self::Pm2 { pm2, home, ids, .. } => {
                let mut command = Command::new(pm2);
                command
                    .arg(action)
                    .args(ids.iter().map(u64::to_string))
                    .env("PM2_HOME", home);
                command
            }
        };
        run(&mut command, CONTROL_TIMEOUT)
            .map(drop)
            .map_err(|error| format!("Can't {action} {}: {error}", self.label()))
    }

    pub fn instances(&self) -> BTreeSet<ProcessIdentity> {
        match self {
            Self::Homebrew { .. } => BTreeSet::new(),
            Self::Pm2 { pids, .. } => pids
                .iter()
                .filter_map(|pid| ProcessIdentity::read(*pid))
                .collect(),
        }
    }

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
    #[serde(default)]
    pm2_env: Pm2Env,
}

#[derive(Debug, Default, Deserialize)]
struct Pm2Env {
    #[serde(default)]
    pm_exec_path: Option<String>,
}

fn instances_of<'a>(apps: &'a [Pm2App], app: &Pm2App) -> Vec<&'a Pm2App> {
    apps.iter()
        .filter(|other| {
            other.name == app.name && other.pm2_env.pm_exec_path == app.pm2_env.pm_exec_path
        })
        .collect()
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
        let cluster = parse_pm2(
            r#"[{"pm_id":0,"name":"api","pid":10,"pm2_env":{"pm_exec_path":"/app/api.js"}},{"pm_id":1,"name":"api","pid":11,"pm2_env":{"pm_exec_path":"/app/api.js"}},{"pm_id":2,"name":"api","pid":12,"pm2_env":{"pm_exec_path":"/other/api.js"}},{"pm_id":3,"name":"web","pid":13,"pm2_env":{"pm_exec_path":"/app/api.js"}}]"#,
        )
        .unwrap();
        let ids: Vec<_> = instances_of(&cluster, &cluster[1])
            .iter()
            .map(|app| app.pm_id)
            .collect();
        assert_eq!(ids, vec![0, 1]);
        assert_eq!(
            (apps[0].pm_id, apps[0].name.as_str(), apps[0].pid),
            (0, "api", Some(4100))
        );
    }

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
        // The child runs bash's title until it execs `sleep`.
        thread::sleep(Duration::from_millis(200));
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
