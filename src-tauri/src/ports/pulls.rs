//! The open pull request for a worktree branch, and a remote preview URL.
//!
//! GitHub is the first host, through the `gh` command the user already logged
//! in. The result is cached in memory for a few minutes and is never written
//! to disk. A failure (no `gh`, no login, offline, a fork Ports can't match,
//! several open pull requests) is a result the window can show, not a guess.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use super::place;
use super::project;

const TTL: Duration = Duration::from_secs(5 * 60);
const GH_LIMIT: &str = "100";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pull {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub provider: String,
    /// Always a remote URL. Never the server's local address.
    pub url: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Lookup {
    Found {
        pull: Pull,
        #[serde(skip_serializing_if = "Option::is_none")]
        preview: Option<Preview>,
    },
    None {
        message: String,
    },
    Ambiguous {
        message: String,
        pulls: Vec<Pull>,
    },
    Unauthenticated {
        message: String,
    },
    Offline {
        message: String,
    },
    Unavailable {
        message: String,
    },
    Unsupported {
        message: String,
    },
}

impl Lookup {
    fn cacheable(&self) -> bool {
        matches!(
            self,
            Self::Found { .. } | Self::None { .. } | Self::Ambiguous { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckLink {
    pub name: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedPull {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head: String,
    pub head_owner: Option<String>,
    pub checks: Vec<CheckLink>,
}

#[derive(Debug)]
pub struct GhOutput {
    /// `None` when the command did not finish (timeout) or could not reach GitHub.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub missing: bool,
}

#[derive(Default)]
pub struct PullCache {
    entries: Mutex<HashMap<String, (Instant, Lookup)>>,
}

impl PullCache {
    fn get(&self, key: &str) -> Option<Lookup> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (at, value) = entries.get(key)?;
        if at.elapsed() < TTL {
            Some(value.clone())
        } else {
            None
        }
    }

    fn insert(&self, key: String, value: Lookup) {
        if !value.cacheable() {
            return;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.insert(key, (Instant::now(), value));
    }
}

pub fn parse_github_url(name: &str, url: &str) -> Option<Remote> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(Remote {
        name: name.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

pub fn github_remotes(config: &str) -> Vec<Remote> {
    let mut current = None;
    let mut remotes = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            current = section
                .strip_prefix("remote ")
                .map(|name| name.trim().trim_matches('"').to_string());
            continue;
        }
        let Some(name) = current.as_deref() else {
            continue;
        };
        let Some(url) = line
            .strip_prefix("url =")
            .or_else(|| line.strip_prefix("url="))
        else {
            continue;
        };
        if let Some(remote) = parse_github_url(name, url.trim()) {
            remotes.push(remote);
        }
    }
    remotes
}

/// `upstream` when the checkout has one, otherwise `origin`.
pub fn lookup_remote(remotes: &[Remote]) -> Option<&Remote> {
    remotes
        .iter()
        .find(|remote| remote.name == "upstream")
        .or_else(|| remotes.iter().find(|remote| remote.name == "origin"))
        .or_else(|| remotes.first())
}

pub fn interpret(
    branch: &str,
    origin_owner: Option<&str>,
    repo_owner: &str,
    repo_name: &str,
    output: &GhOutput,
    local_port: Option<u16>,
) -> Lookup {
    if output.missing {
        return Lookup::Unavailable {
            message: "GitHub CLI (gh) is not installed.".into(),
        };
    }
    if output.status.is_none() {
        return Lookup::Offline {
            message: "Couldn't reach GitHub. The local server is unchanged.".into(),
        };
    }
    let stderr = output.stderr.to_ascii_lowercase();
    let status = output.status.unwrap_or(1);
    if status == 4
        || stderr.contains("auth login")
        || stderr.contains("authentication")
        || stderr.contains("not logged in")
    {
        return Lookup::Unauthenticated {
            message: "GitHub CLI is not logged in. Run gh auth login, then open this server again."
                .into(),
        };
    }
    if status != 0 {
        if stderr.contains("could not resolve")
            || stderr.contains("network")
            || stderr.contains("timed out")
            || stderr.contains("connection")
        {
            return Lookup::Offline {
                message: "Couldn't reach GitHub. The local server is unchanged.".into(),
            };
        }
        let detail = output.stderr.lines().next().unwrap_or("gh failed");
        return Lookup::Unavailable {
            message: format!("Couldn't look up the pull request: {detail}"),
        };
    }
    let pulls = match parse_pulls(&output.stdout) {
        Ok(pulls) => pulls,
        Err(error) => {
            return Lookup::Unavailable {
                message: format!("Couldn't read the pull request list: {error}"),
            };
        }
    };
    let capped = pulls.len() >= GH_LIMIT.parse::<usize>().unwrap_or(100);
    select(
        branch,
        origin_owner,
        repo_owner,
        repo_name,
        &pulls,
        capped,
        local_port,
    )
}

pub fn select(
    branch: &str,
    origin_owner: Option<&str>,
    repo_owner: &str,
    repo_name: &str,
    pulls: &[ListedPull],
    capped: bool,
    local_port: Option<u16>,
) -> Lookup {
    let matches: Vec<_> = pulls
        .iter()
        .filter(|pull| {
            pull.head == branch
                && head_belongs(pull.head_owner.as_deref(), origin_owner, repo_owner)
        })
        .collect();
    match matches.as_slice() {
        [] if capped => Lookup::Ambiguous {
            message: format!(
                "Too many open pull requests on {repo_owner}/{repo_name} to match {branch}."
            ),
            pulls: Vec::new(),
        },
        [] => {
            let message = match origin_owner {
                Some(owner) if owner != repo_owner => format!(
                    "No open pull request from {owner}:{branch} on {repo_owner}/{repo_name}."
                ),
                _ => format!("No open pull request for {branch}."),
            };
            Lookup::None { message }
        }
        [pull] => Lookup::Found {
            preview: preview_from(&pull.checks, local_port),
            pull: Pull {
                number: pull.number,
                title: pull.title.clone(),
                url: pull.url.clone(),
                branch: pull.head.clone(),
            },
        },
        many => Lookup::Ambiguous {
            message: format!(
                "{branch} matches {} open pull requests. Ports didn't pick one.",
                many.len()
            ),
            pulls: many
                .iter()
                .map(|pull| Pull {
                    number: pull.number,
                    title: pull.title.clone(),
                    url: pull.url.clone(),
                    branch: pull.head.clone(),
                })
                .collect(),
        },
    }
}

fn head_belongs(head_owner: Option<&str>, origin_owner: Option<&str>, repo_owner: &str) -> bool {
    match head_owner {
        None => true,
        Some(owner) => owner == repo_owner || origin_owner == Some(owner),
    }
}

pub fn preview_from(checks: &[CheckLink], local_port: Option<u16>) -> Option<Preview> {
    let mut ranked: Vec<_> = checks
        .iter()
        .filter(|check| is_remote_preview(&check.url, local_port))
        .collect();
    ranked.sort_by_key(|check| preview_rank(&check.url, check.name.as_deref()));
    let check = ranked.first()?;
    let (provider, label) = preview_label(&check.url, check.name.as_deref());
    Some(Preview {
        provider,
        url: check.url.clone(),
        label,
    })
}

fn preview_rank(url: &str, name: Option<&str>) -> u8 {
    let host = url.to_ascii_lowercase();
    let name = name.unwrap_or("").to_ascii_lowercase();
    if host.contains("vercel") || name.contains("vercel") {
        0
    } else if host.contains("netlify") || name.contains("netlify") {
        1
    } else {
        2
    }
}

fn preview_label(url: &str, name: Option<&str>) -> (String, String) {
    let host = url.to_ascii_lowercase();
    let name_lower = name.unwrap_or("").to_ascii_lowercase();
    if host.contains("vercel") || name_lower.contains("vercel") {
        ("Vercel".into(), "Vercel preview".into())
    } else if host.contains("netlify") || name_lower.contains("netlify") {
        ("Netlify".into(), "Netlify preview".into())
    } else {
        let provider = name.unwrap_or("Preview").to_string();
        let label = format!("{provider} preview");
        (provider, label)
    }
}

/// A preview is a remote https URL. Local hosts and this server's own port are not previews.
pub fn is_remote_preview(url: &str, local_port: Option<u16>) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let (authority, _) = rest.split_once('/').unwrap_or((rest, ""));
    let hostport = authority.split('@').next_back().unwrap_or(authority);
    let (host, port) = if let Some(host) = hostport.strip_prefix('[') {
        let Some((host, rest)) = host.split_once(']') else {
            return false;
        };
        (
            host,
            rest.strip_prefix(':').and_then(|port| port.parse().ok()),
        )
    } else if let Some((host, port)) = hostport.rsplit_once(':') {
        (host, port.parse().ok())
    } else {
        (hostport, None)
    };
    if host.is_empty()
        || host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host.ends_with(".local")
        || host == "github.com"
        || host.ends_with(".github.com")
    {
        return false;
    }
    local_port.is_none_or(|expected| port != Some(expected))
}

fn parse_pulls(stdout: &str) -> Result<Vec<ListedPull>, String> {
    let value: Value = serde_json::from_str(stdout).map_err(|error| error.to_string())?;
    let array = value.as_array().ok_or("expected a list")?;
    Ok(array.iter().filter_map(parse_pull).collect())
}

fn parse_pull(value: &Value) -> Option<ListedPull> {
    let checks = value
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .map(|checks| checks.iter().filter_map(parse_check).collect())
        .unwrap_or_default();
    Some(ListedPull {
        number: value.get("number")?.as_u64()?,
        title: value.get("title")?.as_str()?.to_string(),
        url: value.get("url")?.as_str()?.to_string(),
        head: value.get("headRefName")?.as_str()?.to_string(),
        head_owner: value.get("headRepositoryOwner").and_then(owner_login),
        checks,
    })
}

fn parse_check(value: &Value) -> Option<CheckLink> {
    let url = value
        .get("detailsUrl")
        .or_else(|| value.get("targetUrl"))
        .and_then(Value::as_str)?;
    Some(CheckLink {
        name: value
            .get("name")
            .or_else(|| value.get("context"))
            .and_then(Value::as_str)
            .map(str::to_string),
        url: url.to_string(),
    })
}

fn owner_login(value: &Value) -> Option<String> {
    value
        .as_str()
        .or_else(|| value.get("login")?.as_str())
        .map(str::to_string)
}

fn run_gh(repo: &str) -> GhOutput {
    let mut command = Command::new("gh");
    command
        .args([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            GH_LIMIT,
            "--json",
            "number,title,url,headRefName,headRepositoryOwner,statusCheckRollup",
        ])
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return GhOutput {
                status: None,
                stdout: String::new(),
                stderr: String::new(),
                missing: true,
            };
        }
        Err(error) => {
            return GhOutput {
                status: None,
                stdout: String::new(),
                stderr: error.to_string(),
                missing: false,
            };
        }
    };
    let pid = child.id();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    match receiver.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(output)) => GhOutput {
            status: Some(output.status.code().unwrap_or(1)),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            missing: false,
        },
        _ => {
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
            GhOutput {
                status: None,
                stdout: String::new(),
                stderr: "timed out".into(),
                missing: false,
            }
        }
    }
}

fn folder_path(app: &AppHandle, folder: &str) -> Result<std::path::PathBuf, String> {
    let home = app.path().home_dir().map_err(|error| error.to_string())?;
    let home = project::canonical(&home);
    let path = match folder.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => std::path::PathBuf::from(folder),
    };
    Ok(project::canonical(&path))
}

#[tauri::command]
pub fn pull_request(
    app: AppHandle,
    cache: State<'_, PullCache>,
    folder: String,
    port: Option<u16>,
) -> Lookup {
    let Ok(path) = folder_path(&app, &folder) else {
        return Lookup::Unsupported {
            message: "The project folder is missing.".into(),
        };
    };
    if !path.exists() {
        return Lookup::Unsupported {
            message: format!("The project folder {folder} is missing."),
        };
    }
    let Ok(home) = app.path().home_dir() else {
        return Lookup::Unsupported {
            message: "Ports couldn't find the home folder.".into(),
        };
    };
    let home = project::canonical(&home);
    let Some(checkout) = place::checkout(&path, &home) else {
        return Lookup::Unsupported {
            message: "This folder is not a Git checkout.".into(),
        };
    };
    let Some(branch) = checkout.branch.clone() else {
        return Lookup::Unsupported {
            message: "This checkout is not on a branch.".into(),
        };
    };
    let config_path = checkout.repository_id.join("config");
    let Ok(config) = std::fs::read_to_string(&config_path) else {
        return Lookup::Unsupported {
            message: "Ports couldn't read the Git remotes.".into(),
        };
    };
    let remotes = github_remotes(&config);
    let Some(remote) = lookup_remote(&remotes) else {
        return Lookup::Unsupported {
            message: "This project has no GitHub remote.".into(),
        };
    };
    let origin_owner = remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .map(|remote| remote.owner.clone());
    let repo = format!("{}/{}", remote.owner, remote.repo);
    let key = format!("{repo}@{branch}");
    if let Some(cached) = cache.get(&key) {
        return cached;
    }
    let output = run_gh(&repo);
    let lookup = interpret(
        &branch,
        origin_owner.as_deref(),
        &remote.owner,
        &remote.repo,
        &output,
        port,
    );
    cache.insert(key, lookup.clone());
    lookup
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pull(number: u64, head: &str, owner: &str) -> ListedPull {
        ListedPull {
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/acme/shop/pull/{number}"),
            head: head.into(),
            head_owner: Some(owner.into()),
            checks: vec![CheckLink {
                name: Some("Vercel".into()),
                url: "https://shop-git-feat.vercel.app".into(),
            }],
        }
    }

    #[test]
    fn reads_github_remotes_and_prefers_upstream() {
        let config = "\
[remote \"origin\"]\n\
\turl = git@github.com:me/shop.git\n\
[remote \"upstream\"]\n\
\turl = https://github.com/acme/shop.git\n\
[remote \"local\"]\n\
\turl = /tmp/shop.git\n";
        let remotes = github_remotes(config);
        assert_eq!(remotes.len(), 2);
        let chosen = lookup_remote(&remotes).unwrap();
        assert_eq!(chosen.name, "upstream");
        assert_eq!(chosen.owner, "acme");
        assert!(parse_github_url("origin", "https://gitlab.com/acme/shop.git").is_none());
    }

    #[test]
    fn one_matching_pull_request_includes_a_remote_preview() {
        match select(
            "feat",
            Some("me"),
            "acme",
            "shop",
            &[pull(7, "feat", "me")],
            false,
            Some(5173),
        ) {
            Lookup::Found { pull, preview } => {
                assert_eq!(pull.number, 7);
                assert_eq!(pull.url, "https://github.com/acme/shop/pull/7");
                let preview = preview.unwrap();
                assert_eq!(preview.label, "Vercel preview");
                assert_ne!(preview.url, "http://127.0.0.1:5173");
            }
            other => panic!("expected one pull request, got {other:?}"),
        }
    }

    #[test]
    fn a_fork_without_a_pull_request_and_two_matches_stay_unresolved() {
        match select("feat", Some("me"), "acme", "shop", &[], false, None) {
            Lookup::None { message } => {
                assert!(message.contains("me:feat"), "{message}");
                assert!(message.contains("acme/shop"), "{message}");
            }
            other => panic!("expected a fork fallback, got {other:?}"),
        }
        match select(
            "feat",
            Some("me"),
            "acme",
            "shop",
            &[pull(1, "feat", "me"), pull(2, "feat", "me")],
            false,
            None,
        ) {
            Lookup::Ambiguous { pulls, .. } => assert_eq!(pulls.len(), 2),
            other => panic!("expected an ambiguous list, got {other:?}"),
        }
    }

    #[test]
    fn a_local_url_is_not_a_preview_and_auth_or_offline_are_explicit() {
        assert!(!is_remote_preview("http://localhost:3000", None));
        assert!(!is_remote_preview(
            "https://127.0.0.1:5173/path",
            Some(5173)
        ));
        assert!(is_remote_preview("https://shop.vercel.app", Some(5173)));
        let checks = [
            CheckLink {
                name: Some("local".into()),
                url: "http://127.0.0.1:5173".into(),
            },
            CheckLink {
                name: Some("Netlify".into()),
                url: "https://feat--shop.netlify.app".into(),
            },
        ];
        let preview = preview_from(&checks, Some(5173)).unwrap();
        assert_eq!(preview.provider, "Netlify");
        assert!(preview.label.contains("preview"));

        let offline = interpret(
            "main",
            None,
            "acme",
            "shop",
            &GhOutput {
                status: None,
                stdout: String::new(),
                stderr: "timed out".into(),
                missing: false,
            },
            None,
        );
        assert!(matches!(offline, Lookup::Offline { .. }));
        let auth = interpret(
            "main",
            None,
            "acme",
            "shop",
            &GhOutput {
                status: Some(4),
                stdout: String::new(),
                stderr: "run `gh auth login`".into(),
                missing: false,
            },
            None,
        );
        assert!(matches!(auth, Lookup::Unauthenticated { .. }));
        let missing = interpret(
            "main",
            None,
            "acme",
            "shop",
            &GhOutput {
                status: None,
                stdout: String::new(),
                stderr: String::new(),
                missing: true,
            },
            None,
        );
        assert!(matches!(missing, Lookup::Unavailable { .. }));
    }
}
