//! The processes that make up a server: what Ports measures and what Stop ends.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use sysinfo::System;

use super::project;

#[derive(Debug, Default)]
pub struct ProcessTable {
    parents: HashMap<u32, u32>,
    children: HashMap<u32, Vec<u32>>,
    programs: HashMap<u32, PathBuf>,
    users: HashMap<u32, u32>,
}

impl ProcessTable {
    pub fn new(system: &System) -> Self {
        let mut table = Self::from_parents(
            system
                .processes()
                .iter()
                .filter_map(|(pid, p)| Some((pid.as_u32(), p.parent()?.as_u32()))),
        );
        table.programs = system
            .processes()
            .iter()
            .filter_map(|(pid, p)| Some((pid.as_u32(), p.exe()?.to_path_buf())))
            .collect();
        table.users = system
            .processes()
            .iter()
            .filter_map(|(pid, p)| Some((pid.as_u32(), **p.user_id()?)))
            .collect();
        table
    }

    pub fn from_parents(pairs: impl IntoIterator<Item = (u32, u32)>) -> Self {
        let mut table = Self::default();
        for (pid, parent) in pairs {
            table.parents.insert(pid, parent);
            table.children.entry(parent).or_default().push(pid);
        }
        table
    }

    pub fn parent(&self, pid: u32) -> Option<u32> {
        self.parents.get(&pid).copied()
    }

    /// Parent, grandparent and so on, nearest first.
    pub fn ancestors(&self, pid: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut current = pid;
        while let Some(parent) = self.parent(current) {
            if parent == current || chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            current = parent;
        }
        chain
    }

    /// `pid` and everything it spawned as the same user. A root daemon such
    /// as `sshd` spawns whole user sessions, which aren't part of it; and the
    /// init process spawned everything.
    pub fn descendants(&self, pid: u32) -> BTreeSet<u32> {
        let mut found = BTreeSet::from([pid]);
        if pid <= 1 {
            return found;
        }
        let user = self.users.get(&pid);
        let mut pending = vec![pid];
        while let Some(next) = pending.pop() {
            for child in self.children.get(&next).into_iter().flatten() {
                if self.users.get(child) == user && found.insert(*child) {
                    pending.push(*child);
                }
            }
        }
        found
    }

    pub fn user(&self, pid: u32) -> Option<u32> {
        self.users.get(&pid).copied()
    }
}

const SHELLS: [&str; 8] = ["sh", "bash", "zsh", "dash", "fish", "ksh", "tcsh", "csh"];

/// What started a service and may start it again when it exits, such as
/// nodemon or turbo: the nearest ancestor of `root` past any shell wrapper.
/// Only this user's own programs count, never a terminal or shell session,
/// an app, Ports or what started it, or the system.
pub fn watcher(root: u32, table: &ProcessTable, groups: &impl Groups) -> Option<u32> {
    let user = table.user(root)?;
    let own = groups.own_pid();
    let ports: Vec<u32> = std::iter::once(own).chain(table.ancestors(own)).collect();
    for ancestor in table.ancestors(root) {
        let program = table.programs.get(&ancestor)?;
        if ancestor <= 1
            || table.user(ancestor) != Some(user)
            || groups.sid(ancestor) == Some(ancestor)
            || ports.contains(&ancestor)
            || project::is_app_bundle(program)
        {
            return None;
        }
        let shell = program
            .file_name()
            .is_some_and(|name| SHELLS.contains(&name.to_string_lossy().as_ref()));
        if !shell {
            return Some(ancestor);
        }
    }
    None
}

/// Process group and session lookups, injectable so the ownership rules can be tested.
pub trait Groups {
    fn pgid(&self, pid: u32) -> Option<u32>;
    fn sid(&self, pid: u32) -> Option<u32>;
    /// Ports' own process group.
    fn own_pgid(&self) -> u32;
    fn own_pid(&self) -> u32;
}

pub struct OsGroups;

impl Groups for OsGroups {
    fn pgid(&self, pid: u32) -> Option<u32> {
        let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
        u32::try_from(pgid).ok()
    }

    fn sid(&self, pid: u32) -> Option<u32> {
        let sid = unsafe { libc::getsid(pid as libc::pid_t) };
        u32::try_from(sid).ok()
    }

    fn own_pgid(&self) -> u32 {
        unsafe { libc::getpgrp() as u32 }
    }

    fn own_pid(&self) -> u32 {
        std::process::id()
    }
}

#[derive(Debug, PartialEq)]
pub struct Job {
    /// The process holding the port.
    pub listener: u32,
    /// What the user launched, e.g. `npm run dev` above the `node` that listens.
    pub root: u32,
    /// Tracked through shutdown so newly spawned group members go too.
    pub group: Option<u32>,
    pub pids: BTreeSet<u32>,
}

/// The server behind the PIDs listening on one port: the first listener's
/// descendants (its workers among them), plus the listener's process group when that group is a job
/// started from a shell (such as `npm run dev`) or by Ports, never the group
/// of a terminal, of an app like an editor, or of Ports itself.
#[cfg(test)]
pub fn job(listeners: &BTreeSet<u32>, table: &ProcessTable, groups: &impl Groups) -> Job {
    scoped_job(listeners, table, groups, listeners)
}

#[cfg(test)]
pub fn scoped_job(
    listeners: &BTreeSet<u32>,
    table: &ProcessTable,
    groups: &impl Groups,
    all_listeners: &BTreeSet<u32>,
) -> Job {
    // Workers sharing the port (SO_REUSEPORT) hang off the first one.
    let first = listeners
        .iter()
        .copied()
        .find(|pid| table.parent(*pid).is_none_or(|p| !listeners.contains(&p)))
        .or_else(|| listeners.first().copied())
        .unwrap_or_default();
    scoped_job_for(first, listeners, table, groups, all_listeners)
}

pub fn scoped_job_for(
    first: u32,
    co_listeners: &BTreeSet<u32>,
    table: &ProcessTable,
    groups: &impl Groups,
    all_listeners: &BTreeSet<u32>,
) -> Job {
    let descendants = table.descendants(first);
    let listener = master(
        first,
        table,
        groups,
        co_listeners,
        all_listeners,
        &descendants,
    );

    let descendants = table.descendants(listener);
    let group = owned_group(listener, table, groups).filter(|group| {
        let members = table.descendants(*group);
        !all_listeners
            .iter()
            .any(|pid| members.contains(pid) && !descendants.contains(pid))
    });
    // Unrelated processes can listen on one port too, e.g. on 127.0.0.1 and
    // ::1; they aren't part of this server and must survive its stop.
    let mut pids = table.descendants(listener);
    if let Some(group) = group {
        pids.extend(
            table
                .descendants(group)
                .into_iter()
                .filter(|pid| groups.pgid(*pid) == Some(group)),
        );
    }
    Job {
        listener,
        root: group.unwrap_or(listener),
        group,
        pids,
    }
}

/// Pre-forking servers (gunicorn, puma, php-fpm) share one socket between a
/// master and its workers, and Linux reports whichever process it finds
/// last. The master is the nearest ancestor still running the same program
/// in the same process group.
fn master(
    listener: u32,
    table: &ProcessTable,
    groups: &impl Groups,
    co_listeners: &BTreeSet<u32>,
    all_listeners: &BTreeSet<u32>,
    service: &BTreeSet<u32>,
) -> u32 {
    let (Some(program), Some(pgid)) = (table.programs.get(&listener), groups.pgid(listener)) else {
        return listener;
    };
    table
        .ancestors(listener)
        .into_iter()
        .take_while(|p| table.programs.get(p) == Some(program) && groups.pgid(*p) == Some(pgid))
        .take_while(|parent| {
            let descendants = table.descendants(*parent);
            !all_listeners.iter().any(|pid| {
                descendants.contains(pid) && !service.contains(pid) && !co_listeners.contains(pid)
            })
        })
        .last()
        .unwrap_or(listener)
}

fn owned_group(listener: u32, table: &ProcessTable, groups: &impl Groups) -> Option<u32> {
    let pgid = groups.pgid(listener)?;
    if pgid <= 1 || pgid == groups.own_pgid() {
        return None;
    }
    if pgid == listener {
        return Some(pgid);
    }
    // Shells give each job its own group, led by a child of the shell, which
    // leads the terminal's session. Apps started by launchd or a desktop
    // session also lead their group, but their parent isn't a session leader:
    // taking such a group would quit the whole app for one helper's port.
    let started_by_shell_or_ports = table.parent(pgid).is_some_and(|parent| {
        parent > 1 && (groups.sid(parent) == Some(parent) || parent == groups.own_pid())
    });
    let leads_job = table.ancestors(listener).contains(&pgid)
        && groups.sid(pgid) != Some(pgid)
        && started_by_shell_or_ports;
    leads_job.then_some(pgid)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeGroups {
        pgids: HashMap<u32, u32>,
        session_leaders: Vec<u32>,
        own: u32,
    }

    impl Groups for FakeGroups {
        fn pgid(&self, pid: u32) -> Option<u32> {
            self.pgids.get(&pid).copied()
        }
        fn sid(&self, pid: u32) -> Option<u32> {
            self.session_leaders
                .contains(&pid)
                .then_some(pid)
                .or(Some(0))
        }
        fn own_pgid(&self) -> u32 {
            self.own
        }
        fn own_pid(&self) -> u32 {
            self.own
        }
    }

    const TERMINAL: u32 = 50;
    const SHELL: u32 = 100;
    const NPM: u32 = 200;
    const SH: u32 = 201;
    const NODE: u32 = 202;
    const ESBUILD: u32 = 203;

    /// Terminal → zsh → `npm run dev` → sh → node (listens) → esbuild.
    fn npm_run_dev() -> ProcessTable {
        ProcessTable::from_parents([
            (SHELL, TERMINAL),
            (NPM, SHELL),
            (SH, NPM),
            (NODE, SH),
            (ESBUILD, NODE),
        ])
    }

    #[test]
    fn a_watcher_is_the_nearest_own_program_above_the_shell_wrappers() {
        // Editor task: zsh (session) → nodemon → sh -c → node (listens).
        const NODEMON: u32 = 300;
        let mut table = ProcessTable::from_parents([(NODEMON, SHELL), (SH, NODEMON), (NODE, SH)]);
        table.programs = HashMap::from([
            (SHELL, PathBuf::from("/bin/zsh")),
            (NODEMON, PathBuf::from("/usr/local/bin/node")),
            (SH, PathBuf::from("/bin/sh")),
            (NODE, PathBuf::from("/usr/local/bin/node")),
        ]);
        table.users = [SHELL, NODEMON, SH, NODE].map(|pid| (pid, 501)).into();
        let groups = FakeGroups {
            pgids: HashMap::new(),
            session_leaders: vec![SHELL],
            own: 999,
        };
        assert_eq!(watcher(NODE, &table, &groups), Some(NODEMON));
        // A shell job's own leader sits right under the session: no watcher.
        assert_eq!(watcher(NODEMON, &table, &groups), None);
        // Nor an app, another user's process, or what started Ports.
        let app = FakeGroups {
            own: NODEMON,
            ..groups
        };
        assert_eq!(watcher(NODE, &table, &app), None);
        table.programs.insert(
            NODEMON,
            PathBuf::from("/Applications/Code.app/Contents/MacOS/Code Helper"),
        );
        assert_eq!(watcher(NODE, &table, &FakeGroups { own: 999, ..app }), None);
        table
            .programs
            .insert(NODEMON, PathBuf::from("/usr/local/bin/node"));
        table.users.insert(NODEMON, 0);
        let groups = FakeGroups {
            pgids: HashMap::new(),
            session_leaders: vec![SHELL],
            own: 999,
        };
        assert_eq!(watcher(NODE, &table, &groups), None);
    }

    #[test]
    fn sibling_workers_sharing_one_binding_normalize_to_their_master() {
        let mut table = ProcessTable::from_parents([(200, SHELL), (201, 200), (202, 200)]);
        for pid in [200, 201, 202] {
            table.programs.insert(pid, PathBuf::from("/python"));
        }
        let groups = FakeGroups {
            pgids: HashMap::from([(SHELL, SHELL), (200, 200), (201, 200), (202, 200)]),
            session_leaders: vec![SHELL],
            own: 999,
        };
        let co = BTreeSet::from([201, 202]);
        for pid in &co {
            let actual = scoped_job_for(*pid, &co, &table, &groups, &co);
            assert_eq!(actual.listener, 200);
            assert_eq!(actual.root, 200);
            assert_eq!(actual.pids, BTreeSet::from([200, 201, 202]));
        }
    }

    #[test]
    fn a_shell_job_is_the_whole_process_group() {
        let groups = FakeGroups {
            pgids: HashMap::from([
                (SHELL, SHELL),
                (NPM, NPM),
                (SH, NPM),
                (NODE, NPM),
                (ESBUILD, NPM),
            ]),
            session_leaders: vec![SHELL],
            own: 1,
        };
        let job = job(&BTreeSet::from([NODE]), &npm_run_dev(), &groups);
        assert_eq!(
            job,
            Job {
                listener: NODE,
                root: NPM,
                group: Some(NPM),
                pids: BTreeSet::from([NPM, SH, NODE, ESBUILD]),
            }
        );
    }

    #[test]
    fn never_takes_the_group_of_a_shell_without_job_control() {
        let groups = FakeGroups {
            pgids: HashMap::from([(SHELL, SHELL), (NPM, SHELL), (SH, SHELL), (NODE, SHELL)]),
            session_leaders: vec![SHELL],
            own: 1,
        };
        let job = job(&BTreeSet::from([NODE]), &npm_run_dev(), &groups);
        assert_eq!(job.group, None);
        assert_eq!(job.root, NODE);
        assert_eq!(job.pids, BTreeSet::from([NODE, ESBUILD]));
    }

    #[test]
    fn never_takes_the_group_ports_runs_in() {
        let groups = FakeGroups {
            pgids: HashMap::from([(NPM, NPM), (SH, NPM), (NODE, NPM)]),
            session_leaders: vec![],
            own: NPM,
        };
        let job = job(&BTreeSet::from([NODE]), &npm_run_dev(), &groups);
        assert_eq!(job.group, None);
        assert_eq!(job.pids, BTreeSet::from([NODE, ESBUILD]));
    }

    #[test]
    fn never_takes_the_group_of_an_app_for_its_helper() {
        // launchd → Cursor → Cursor Helper → extension host (listens), all in Cursor's group.
        const LAUNCHD: u32 = 1;
        const APP: u32 = 300;
        const HELPER: u32 = 301;
        const EXTENSION_HOST: u32 = 302;
        let table =
            ProcessTable::from_parents([(APP, LAUNCHD), (HELPER, APP), (EXTENSION_HOST, HELPER)]);
        let groups = FakeGroups {
            pgids: HashMap::from([(APP, APP), (HELPER, APP), (EXTENSION_HOST, APP)]),
            session_leaders: vec![],
            own: 999,
        };
        let job = job(&BTreeSet::from([EXTENSION_HOST]), &table, &groups);
        assert_eq!(job.group, None);
        assert_eq!(job.pids, BTreeSet::from([EXTENSION_HOST]));
    }

    #[test]
    fn a_pre_forked_worker_leads_back_to_its_master() {
        // Linux names a worker as the socket's owner; the master forked it.
        const MASTER: u32 = 10;
        let mut table = ProcessTable::from_parents([(MASTER, SHELL), (11, MASTER), (12, MASTER)]);
        table.programs = [MASTER, 11, 12]
            .map(|pid| (pid, PathBuf::from("/usr/bin/python3")))
            .into();
        let groups = FakeGroups {
            pgids: HashMap::from([(SHELL, SHELL), (MASTER, MASTER), (11, MASTER), (12, MASTER)]),
            session_leaders: vec![SHELL],
            own: 1,
        };
        let job = job(&BTreeSet::from([12]), &table, &groups);
        assert_eq!(job.listener, MASTER);
        assert_eq!(job.pids, BTreeSet::from([MASTER, 11, 12]));
    }

    #[test]
    fn a_daemon_of_another_user_leaves_out_the_sessions_it_spawned() {
        const SSHD: u32 = 400;
        const SESSION: u32 = 401;
        const USER_SHELL: u32 = 402;
        let mut table =
            ProcessTable::from_parents([(SSHD, 1), (SESSION, SSHD), (USER_SHELL, SESSION)]);
        table.users = HashMap::from([(1, 0), (SSHD, 0), (SESSION, 0), (USER_SHELL, 501)]);
        let groups = FakeGroups {
            pgids: HashMap::from([(SSHD, SSHD)]),
            session_leaders: vec![SSHD],
            own: 999,
        };
        assert_eq!(
            job(&BTreeSet::from([SSHD]), &table, &groups).pids,
            BTreeSet::from([SSHD, SESSION])
        );
        assert_eq!(
            job(&BTreeSet::from([1]), &table, &groups).pids,
            BTreeSet::from([1])
        );
    }

    #[test]
    fn workers_sharing_a_port_belong_to_the_first_listener() {
        let table = ProcessTable::from_parents([(11, 10), (12, 10)]);
        let groups = FakeGroups {
            pgids: HashMap::from([(10, 10), (11, 10), (12, 10)]),
            session_leaders: vec![],
            own: 1,
        };
        let job = job(&BTreeSet::from([10, 11, 12]), &table, &groups);
        assert_eq!(job.listener, 10);
        assert_eq!(job.pids, BTreeSet::from([10, 11, 12]));
    }

    #[test]
    fn leaves_out_an_unrelated_process_on_the_same_port() {
        // Node on [::1]:3000 next to Python on 127.0.0.1:3000.
        let table = ProcessTable::from_parents([(20, 5), (21, 20), (30, 6)]);
        let groups = FakeGroups {
            pgids: HashMap::from([(20, 20), (21, 20), (30, 30)]),
            session_leaders: vec![],
            own: 1,
        };
        let job = job(&BTreeSet::from([20, 30]), &table, &groups);
        assert_eq!(job.listener, 20);
        assert_eq!(job.pids, BTreeSet::from([20, 21]));
    }

    #[test]
    fn does_not_take_a_task_runner_that_owns_another_listening_service() {
        let table = ProcessTable::from_parents([(200, SHELL), (201, 200), (202, 201), (210, 200)]);
        let groups = FakeGroups {
            pgids: HashMap::from([
                (SHELL, SHELL),
                (200, 200),
                (201, 200),
                (202, 200),
                (210, 200),
            ]),
            session_leaders: vec![SHELL],
            own: 999,
        };
        let job = scoped_job(
            &BTreeSet::from([202]),
            &table,
            &groups,
            &BTreeSet::from([202, 210]),
        );
        assert_eq!(job.root, 202);
        assert_eq!(job.group, None);
        assert_eq!(job.pids, BTreeSet::from([202]));
    }
}
