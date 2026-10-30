use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sysinfo::{Pid, Process, ProcessRefreshKind, System, UpdateKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub executable: String,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    Cursor,
    Claude,
    Codex,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum Evidence {
    Ancestor { name: String },
    Environment { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Resume {
    Available { label: String, path: String },
    Missing { message: String },
    Unsupported { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub agent: Agent,
    pub label: String,
    pub evidence: Vec<Evidence>,
    pub resume: Resume,
}

const ENV_KEYS: [&str; 4] = [
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CURSOR_AGENT",
    "CODEX_THREAD_ID",
];

const IGNORE_EXECUTABLES: [&str; 5] = ["launchd", "init", "systemd", "kernel_task", "ports"];

pub fn observe(system: &System, pid: u32) -> Option<Observation> {
    let process = system.process(Pid::from_u32(pid))?;
    let executable = executable_name(process);
    if executable.is_empty() {
        return None;
    }
    let env = allowlisted_env(process);
    Some(Observation { executable, env })
}

fn executable_name(process: &Process) -> String {
    process
        .exe()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| process.name().to_string_lossy().into_owned())
}

fn allowlisted_env(process: &Process) -> Vec<(String, String)> {
    process
        .environ()
        .iter()
        .filter_map(|entry| {
            let entry = entry.to_str()?;
            let (key, value) = entry.split_once('=')?;
            if ENV_KEYS.contains(&key) && !value.is_empty() {
                Some((key.to_string(), value.to_string()))
            } else {
                None
            }
        })
        .collect()
}

pub fn refresh(system: &mut System, pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let pids: Vec<Pid> = pids.iter().copied().map(Pid::from_u32).collect();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&pids),
        false,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_environ(UpdateKind::Always)
            .without_tasks(),
    );
}

pub fn attribute(chain: &[Observation], home: &Path) -> Option<Session> {
    let mut agent: Option<Agent> = None;
    let mut evidence = Vec::new();
    let mut claude_id = None;
    let mut codex_id = None;
    let mut cursor_agent = false;

    for observation in chain {
        let name = observation.executable.as_str();
        if IGNORE_EXECUTABLES
            .iter()
            .any(|ignored| name.eq_ignore_ascii_case(ignored))
        {
            continue;
        }
        if let Some(found) = agent_from_executable(name) {
            push_agent(&mut agent, found);
            evidence.push(Evidence::Ancestor {
                name: name.to_string(),
            });
        }
        for (key, value) in &observation.env {
            if let Some(found) = agent_from_env(key) {
                push_agent(&mut agent, found);
                evidence.push(Evidence::Environment { name: key.clone() });
                if key == "CURSOR_AGENT" {
                    cursor_agent = true;
                }
            }
            if key == "CLAUDE_CODE_SESSION_ID" {
                claude_id = session_token(value);
            }
            if key == "CODEX_THREAD_ID" {
                codex_id = session_token(value);
            }
        }
    }

    let agent = agent?;
    evidence.dedup();
    let label = match agent {
        Agent::Cursor if cursor_agent => "Cursor agent",
        Agent::Cursor => "Cursor",
        Agent::Claude => "Claude Code",
        Agent::Codex => "Codex",
        Agent::Terminal => "Terminal",
    };
    let resume = resume_for(agent, home, claude_id, codex_id);
    Some(Session {
        agent,
        label: label.to_string(),
        evidence,
        resume,
    })
}

fn push_agent(current: &mut Option<Agent>, found: Agent) {
    match (*current, found) {
        (None, found) => *current = Some(found),
        (Some(Agent::Terminal), found) if found != Agent::Terminal => *current = Some(found),
        _ => {}
    }
}

fn agent_from_executable(name: &str) -> Option<Agent> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "claude" => Some(Agent::Claude),
        "codex" => Some(Agent::Codex),
        "cursor" => Some(Agent::Cursor),
        "terminal" | "iterm2" | "warp" | "ghostty" | "wezterm" | "wezterm-gui" | "kitty"
        | "alacritty" => Some(Agent::Terminal),
        _ if name.starts_with("cursor helper") => Some(Agent::Cursor),
        _ => None,
    }
}

fn agent_from_env(key: &str) -> Option<Agent> {
    match key {
        "CLAUDECODE" | "CLAUDE_CODE_SESSION_ID" => Some(Agent::Claude),
        "CURSOR_AGENT" => Some(Agent::Cursor),
        "CODEX_THREAD_ID" => Some(Agent::Codex),
        _ => None,
    }
}

fn session_token(value: &str) -> Option<String> {
    let token = value.trim();
    if (8..=128).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        Some(token.to_string())
    } else {
        None
    }
}

fn resume_for(
    agent: Agent,
    home: &Path,
    claude_id: Option<String>,
    codex_id: Option<String>,
) -> Resume {
    match agent {
        Agent::Claude => session_file(
            home.join(".claude"),
            claude_id.as_deref(),
            "Reveal Claude session",
            "This Claude session file is no longer on disk.",
            "Ports can't resume this Claude session.",
        ),
        Agent::Codex => session_file(
            home.join(".codex"),
            codex_id.as_deref(),
            "Reveal Codex session",
            "This Codex session file is no longer on disk.",
            "Ports can't resume this Codex session.",
        ),
        Agent::Cursor => Resume::Unsupported {
            message: "Ports can't resume a Cursor session.".into(),
        },
        Agent::Terminal => Resume::Unsupported {
            message: "A terminal session has no resume link.".into(),
        },
    }
}

fn session_file(
    root: PathBuf,
    id: Option<&str>,
    label: &str,
    missing: &str,
    unsupported: &str,
) -> Resume {
    let Some(id) = id else {
        return Resume::Unsupported {
            message: unsupported.into(),
        };
    };
    let file_name = format!("{id}.jsonl");
    match find_named(&root, &file_name) {
        Some(path) => Resume::Available {
            label: label.into(),
            path: path.display().to_string(),
        },
        None => Resume::Missing {
            message: missing.into(),
        },
    }
}

const MAX_VISITS: usize = 2_000;
const MAX_DEPTH: usize = 5;

fn find_named(root: &Path, file_name: &str) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut visits = 0_usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visits += 1;
            if visits > MAX_VISITS {
                return None;
            }
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|name| name == file_name && path.is_file())
            {
                return Some(path);
            }
            if path.is_dir() {
                stack.push((path, depth + 1));
            }
        }
    }
    None
}

pub fn revealable(home: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        project_canonical(path)
    } else {
        return Err("This session link is no longer available.".into());
    };
    let allowed = [home.join(".claude"), home.join(".codex")]
        .into_iter()
        .map(|dir| project_canonical(&dir))
        .any(|dir| path.starts_with(&dir));
    if allowed && path.is_file() {
        Ok(path)
    } else {
        Err("This session link is no longer available.".into())
    }
}

fn project_canonical(path: &Path) -> PathBuf {
    super::project::canonical(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn refreshing_session_environment_does_not_enumerate_threads() {
        let pid = Pid::from_u32(std::process::id());
        let mut system = System::new();
        refresh(&mut system, &[pid.as_u32()]);
        assert!(system.process(pid).unwrap().tasks().is_none());
        assert!(observe(&system, pid.as_u32()).is_some());
    }

    fn obs(executable: &str, env: &[(&str, &str)]) -> Observation {
        Observation {
            executable: executable.into(),
            env: env
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        }
    }

    #[test]
    fn attributes_claude_from_the_ancestor_executable_not_from_an_argument() {
        let home = Path::new("/tmp");
        let session = attribute(&[obs("node", &[]), obs("claude", &[])], home).unwrap();
        assert_eq!(session.agent, Agent::Claude);
        assert_eq!(
            session.evidence,
            vec![Evidence::Ancestor {
                name: "claude".into()
            }]
        );
        assert!(matches!(session.resume, Resume::Unsupported { .. }));

        assert!(attribute(&[obs("node", &[])], home).is_none());
    }

    #[test]
    fn an_allowlisted_variable_names_the_agent_and_a_terminal_does_not_override_it() {
        let home = Path::new("/tmp");
        let session = attribute(
            &[obs("node", &[("CLAUDECODE", "1")]), obs("Terminal", &[])],
            home,
        )
        .unwrap();
        assert_eq!(session.agent, Agent::Claude);
        assert!(session.evidence.iter().any(|item| matches!(
            item,
            Evidence::Environment { name } if name == "CLAUDECODE"
        )));
    }

    #[test]
    fn cursor_agent_is_named_only_from_its_environment() {
        let home = Path::new("/tmp");
        let session = attribute(
            &[obs("Cursor Helper (Plugin)", &[("CURSOR_AGENT", "1")])],
            home,
        )
        .unwrap();
        assert_eq!(session.label, "Cursor agent");
        assert!(matches!(session.resume, Resume::Unsupported { .. }));
    }

    #[test]
    fn a_missing_session_file_is_a_fallback_and_a_present_one_can_be_revealed() {
        let home = tempfile::TempDir::new().unwrap();
        let id = "0123456789abcdef";
        let missing = attribute(
            &[obs("claude", &[("CLAUDE_CODE_SESSION_ID", id)])],
            home.path(),
        )
        .unwrap();
        assert!(matches!(missing.resume, Resume::Missing { .. }));

        let path = home.path().join(".claude/projects/demo");
        fs::create_dir_all(&path).unwrap();
        let file = path.join(format!("{id}.jsonl"));
        fs::write(&file, "{}\n").unwrap();
        let found = attribute(
            &[obs("claude", &[("CLAUDE_CODE_SESSION_ID", id)])],
            home.path(),
        )
        .unwrap();
        match found.resume {
            Resume::Available {
                path: found_path, ..
            } => {
                assert_eq!(
                    revealable(home.path(), Path::new(&found_path)).unwrap(),
                    super::project_canonical(&file)
                );
            }
            other => panic!("expected a revealable session, got {other:?}"),
        }
        let outside = home.path().join("notes.txt");
        fs::write(&outside, "nope").unwrap();
        assert!(revealable(home.path(), &outside).is_err());
    }

    #[test]
    fn ignores_environment_variables_that_are_not_allowlisted() {
        let home = Path::new("/tmp");
        assert!(attribute(&[obs("node", &[("PATH", "/usr/bin/claude")])], home).is_none());
    }
}
