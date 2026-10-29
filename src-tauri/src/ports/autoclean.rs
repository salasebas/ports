//! Automatic clean up: stops forgotten dev servers under the policy the user
//! chose in `rules.json`. It is off until the user picks a policy, and pausing
//! or turning it off cancels the stops still waiting.
//!
//! It only considers dev servers whose own process Ports controls, never a
//! container, a database, another user's process or one a supervisor runs.
//! Each stop goes through the same fresh check as a manual clean up, so a
//! server that turned active, was hidden, protected or replaced since the
//! round started is left running. It notifies after each stop, with no
//! warning beforehand, and records every attempt in a size-capped log.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::thread;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use super::control::Target;
use super::rules::{Policy, Scope};
use super::sampler::{Kind, Server, Snapshot, now_ms};
use super::{Located, Monitor, blocking, containers, lock, shares_tree};

/// The log keeps the newest entries within this size.
const LOG_LIMIT: u64 = 64 * 1024;
/// How many entries the settings show.
const SHOWN: usize = 50;

/// The rounds of automatic clean up, and the servers it already tried.
pub struct Runs {
    log: PathBuf,
    running: bool,
    /// Servers it couldn't stop, with the activity it saw, so it tries again
    /// only after one was active and went quiet once more.
    tried: HashMap<String, u64>,
}

impl Runs {
    pub fn new(log: PathBuf) -> Self {
        Self {
            log,
            running: false,
            tried: HashMap::new(),
        }
    }
}

/// Whether the policy lets automatic clean up stop this server.
pub fn eligible(server: &Server, policy: Policy, now: u64) -> bool {
    server.kind == Kind::Dev
        && matches!(server.target, Some(Target::Process(_)))
        && server.supervisor.is_none()
        && !server.hidden
        && !server.protected
        && now.saturating_sub(server.last_active_at) >= policy.idle_minutes * 60_000
        && (policy.scope == Scope::Idle || server.cwd_deleted)
}

/// The servers to stop, once per process tree: each eligible one whose stop
/// would end only eligible servers.
fn candidates(servers: &[Server], policy: Policy, now: u64) -> Vec<Server> {
    let mut chosen: Vec<Server> = Vec::new();
    for server in servers.iter().filter(|s| eligible(s, policy, now)) {
        let Some(target) = &server.target else {
            continue;
        };
        let shares = |other: &Server| shares_tree(target, &server.resources, other);
        if servers
            .iter()
            .filter(|other| shares(other))
            .all(|other| eligible(other, policy, now))
            && !chosen.iter().any(shares)
        {
            chosen.push(server.clone());
        }
    }
    chosen
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Stopped,
    /// The fresh check found it active, hidden, protected or replaced.
    Skipped,
    Failed,
}

/// One attempt. Mirrors `AutoCleanupEntry` in `src/lib/rules.svelte.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    at: u64,
    port: u16,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
    cwd_deleted: bool,
    idle_minutes: u64,
    outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn record(log: &PathBuf, entry: &Entry) {
    let Ok(mut line) = serde_json::to_vec(entry) else {
        return;
    };
    line.push(b'\n');
    let written = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .and_then(|mut file| file.write_all(&line));
    if let Err(error) = written {
        eprintln!("could not record an automatic clean up: {error}");
        return;
    }
    trim(log);
}

/// Cuts the log back to its newest whole lines within `LOG_LIMIT` once it
/// grows past twice that, so trims stay rare.
fn trim(log: &PathBuf) {
    let Ok(text) = fs::read(log) else {
        return;
    };
    if text.len() as u64 <= 2 * LOG_LIMIT {
        return;
    }
    let start = text.len() - LOG_LIMIT as usize;
    let start = text[start..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(text.len(), |i| start + i + 1);
    let _ = fs::write(log, &text[start..]);
}

/// The newest entries first.
fn read(log: &PathBuf) -> Vec<Entry> {
    let text = fs::read_to_string(log).unwrap_or_default();
    let mut entries: Vec<Entry> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    entries.reverse();
    entries.truncate(SHOWN);
    entries
}

impl Monitor {
    /// Starts a round in the background when the policy is on and a server
    /// qualifies; one round at a time.
    pub(super) fn clean_up_if_due(&self, app: &AppHandle, snapshot: &Snapshot) {
        let Some(policy) = lock(&self.rules).rules.auto_cleanup.active() else {
            return;
        };
        let now = now_ms();
        let due = {
            let mut runs = lock(&self.auto);
            if runs.running {
                return;
            }
            runs.tried
                .retain(|id, _| snapshot.servers.iter().any(|server| &server.id == id));
            let due: Vec<_> = candidates(&snapshot.servers, policy, now)
                .into_iter()
                .filter(|server| runs.tried.get(&server.id) != Some(&server.last_active_at))
                .collect();
            if due.is_empty() {
                return;
            }
            runs.running = true;
            due
        };
        let app = app.clone();
        let spawned = thread::Builder::new()
            .name("ports-auto-cleanup".into())
            .spawn(move || {
                let monitor = app.state::<Monitor>();
                for server in due {
                    // Pausing or turning the policy off cancels what's left.
                    let Some(policy) = lock(&monitor.rules).rules.auto_cleanup.active() else {
                        break;
                    };
                    monitor.stop_forgotten(&app, &server, policy);
                }
                lock(&monitor.auto).running = false;
            });
        if let Err(error) = spawned {
            eprintln!("could not clean up automatically: {error}");
            lock(&self.auto).running = false;
        }
    }

    fn stop_forgotten(&self, app: &AppHandle, server: &Server, policy: Policy) {
        let Some(target) = &server.target else {
            return;
        };
        let checked = self.ensure_cleanable(target, |current| eligible(current, policy, now_ms()));
        let result = checked.clone().and_then(|()| {
            let shutdown = self.shutdown(false);
            self.control(
                app,
                target,
                &super::Scope::default(),
                |located| match located {
                    Located::Process(service) => service.stop(shutdown),
                    Located::Container(target) => containers::stop(&self.home, &target, shutdown),
                },
            )
        });
        let (outcome, message) = match (&checked, &result) {
            (Err(error), _) => (Outcome::Skipped, Some(error.clone())),
            (Ok(()), Err(error)) => (Outcome::Failed, Some(error.clone())),
            (Ok(()), Ok(_)) => (Outcome::Stopped, None),
        };
        let runs = {
            let mut runs = lock(&self.auto);
            if outcome != Outcome::Stopped {
                runs.tried.insert(server.id.clone(), server.last_active_at);
            }
            runs.log.clone()
        };
        record(
            &runs,
            &Entry {
                at: now_ms(),
                port: server.port,
                name: server.name.clone(),
                folder: server.folder.clone(),
                cwd_deleted: server.cwd_deleted,
                idle_minutes: now_ms().saturating_sub(server.last_active_at) / 60_000,
                outcome,
                message,
            },
        );
        if outcome == Outcome::Stopped {
            let why = if server.cwd_deleted {
                "Its folder was deleted and it was idle.".to_string()
            } else {
                format!("Idle for {} minutes.", policy.idle_minutes)
            };
            let shown = app
                .notification()
                .builder()
                .title(format!("Stopped {} on :{}", server.name, server.port))
                .body(format!("{why} Automatic clean up is on in Settings."))
                .show();
            if let Err(error) = shown {
                eprintln!("could not announce an automatic clean up: {error}");
            }
        }
    }
}

/// What automatic clean up did, newest first.
#[tauri::command]
pub async fn auto_cleanup_log(app: AppHandle) -> Result<Vec<Entry>, String> {
    blocking(move || Ok(read(&lock(&app.state::<Monitor>().auto).log.clone()))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::alerts::tests::server;
    use crate::ports::control::ProcessTarget;
    use crate::ports::identity::ProcessIdentity;
    use crate::ports::sampler::ProcessUsage;
    use crate::ports::supervisors::{Kind as SupervisorKind, Supervisor};

    const HOUR: u64 = 60 * 60_000;

    fn process(pid: u32) -> ProcessIdentity {
        serde_json::from_value(serde_json::json!({ "pid": pid, "start": "1" })).unwrap()
    }

    /// A dev server Ports controls, quiet for `idle` milliseconds at `HOUR * 10`.
    fn quiet(port: u16, root: u32, idle: u64) -> Server {
        let mut server = server(port, 0);
        server.id = format!("process:{root}:{port}");
        server.last_active_at = HOUR * 10 - idle;
        server.target = Some(Target::Process(ProcessTarget {
            port,
            listener: process(root),
            root: process(root),
            addresses: Vec::new(),
        }));
        server.resources = vec![ProcessUsage {
            process: process(root),
            parent: None,
            name: "node".into(),
            ports: vec![port],
            memory_bytes: 0,
            cpu_percent: 0.0,
        }];
        server
    }

    const IDLE: Policy = Policy {
        scope: Scope::Idle,
        idle_minutes: 60,
    };

    #[test]
    fn stops_only_dev_servers_quiet_for_the_policys_time() {
        let now = HOUR * 10;
        let forgotten = quiet(5173, 1, 2 * HOUR);
        let recent = quiet(3000, 2, HOUR / 2);
        let mut database = quiet(5432, 3, 2 * HOUR);
        database.kind = Kind::Database;
        let mut protected = quiet(8080, 4, 2 * HOUR);
        protected.protected = true;
        let mut hidden = quiet(8081, 5, 2 * HOUR);
        hidden.hidden = true;
        let mut brewed = quiet(6379, 6, 2 * HOUR);
        brewed.supervisor = Some(Supervisor {
            kind: SupervisorKind::Homebrew,
            name: Some("redis".into()),
            supported: true,
        });
        let mut unowned = quiet(9000, 7, 2 * HOUR);
        unowned.target = None;
        let servers = [
            forgotten, recent, database, protected, hidden, brewed, unowned,
        ];
        let ports: Vec<_> = candidates(&servers, IDLE, now)
            .iter()
            .map(|s| s.port)
            .collect();
        assert_eq!(ports, [5173]);
    }

    #[test]
    fn the_deleted_scope_stops_only_servers_whose_folder_is_gone() {
        let now = HOUR * 10;
        let policy = Policy {
            scope: Scope::Deleted,
            ..IDLE
        };
        let kept = quiet(5173, 1, 2 * HOUR);
        let mut removed = quiet(5174, 2, 2 * HOUR);
        removed.cwd_deleted = true;
        let ports: Vec<_> = candidates(&[kept, removed], policy, now)
            .iter()
            .map(|s| s.port)
            .collect();
        assert_eq!(ports, [5174]);
    }

    // Stopping the forgotten port would also end the active one it shares a process with.
    #[test]
    fn leaves_a_tree_running_when_one_of_its_ports_is_not_eligible() {
        let now = HOUR * 10;
        let forgotten = quiet(5173, 1, 2 * HOUR);
        let mut active = quiet(5174, 1, 0);
        active.id = "process:1:5174".into();
        assert!(candidates(&[forgotten.clone(), active], IDLE, now).is_empty());
        let mut sibling = quiet(5174, 1, 2 * HOUR);
        sibling.id = "process:1:5174".into();
        assert_eq!(candidates(&[forgotten, sibling], IDLE, now).len(), 1);
    }

    #[test]
    fn keeps_the_newest_whole_entries_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("auto-cleanup.log");
        let entry = |port| Entry {
            at: 1,
            port,
            name: "web".into(),
            folder: None,
            cwd_deleted: false,
            idle_minutes: 60,
            outcome: Outcome::Stopped,
            message: Some("x".repeat(400)),
        };
        for port in 1..=1000 {
            record(&log, &entry(port));
        }
        assert!(fs::metadata(&log).unwrap().len() <= 2 * LOG_LIMIT);
        let entries = read(&log);
        assert_eq!(entries.len(), SHOWN);
        assert_eq!(entries[0].port, 1000);
    }
}
