use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use super::health::Health;
use super::history::Status;
use super::indicator::TrayMetric;
use super::project::{self, Confidence};
use super::rules::{PortRange, Rules};
use super::sampler::{Kind, Server, now_ms};
use super::supervisors::Supervisor;
use super::{AlertSettings, HealthSettings, Monitor, blocking};

const SCHEMA: u32 = 1;
const REDACTED: &str = "[redacted]";
const SECRET_WORDS: [&str; 8] = [
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "key",
    "auth",
    "credential",
];

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Include {
    commands: bool,
    paths: bool,
    settings: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Export {
    schema: u32,
    app: App,
    platform: Platform,
    exported_at: u64,
    excluded: Vec<&'static str>,
    discovery: Discovery,
    servers: Vec<Row>,
    settings: Option<SavedSettings>,
    rules: Option<RulesSummary>,
}

#[derive(Debug, Serialize)]
struct App {
    name: &'static str,
    version: &'static str,
}

#[derive(Debug, Serialize)]
struct Platform {
    os: &'static str,
    arch: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Discovery {
    complete: bool,
    servers: usize,
    without_process: usize,
    unmeasured: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: String,
    port: u16,
    addresses: Vec<std::net::IpAddr>,
    exposed: bool,
    kind: Kind,
    source: &'static str,
    process: String,
    pid: Option<u32>,
    owner: Option<String>,
    image: Option<String>,
    status: Status,
    health: Health,
    hidden: bool,
    protected: bool,
    framework: Option<&'static str>,
    framework_confidence: Confidence,
    supervisor: Option<Supervisor>,
    memory_bytes: Option<u64>,
    cpu_percent: Option<f64>,
    command: Option<String>,
    folder: Option<String>,
    cwd: Option<String>,
    unavailable: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedSettings {
    alerts: AlertSettings,
    health: HealthSettings,
    tray_metric: TrayMetric,
    stop_grace_seconds: u64,
    idle_after_minutes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RulesSummary {
    port_range: Option<PortRange>,
    shown_kinds: Vec<Kind>,
    include_rules: usize,
    exclude_rules: usize,
    hidden: usize,
    protection: usize,
    snoozes: usize,
    watches: usize,
    auto_cleanup_on: bool,
}

impl From<&Rules> for RulesSummary {
    fn from(rules: &Rules) -> Self {
        Self {
            port_range: rules.port_range,
            shown_kinds: rules.shown_kinds.clone(),
            include_rules: rules.include.len(),
            exclude_rules: rules.exclude.len(),
            hidden: rules.hidden.len(),
            protection: rules.protection.len(),
            snoozes: rules.snoozes.len(),
            watches: rules.watches.len(),
            auto_cleanup_on: rules.auto_cleanup.active().is_some(),
        }
    }
}

fn is_secret_name(name: &str) -> bool {
    let name = name.to_lowercase();
    SECRET_WORDS.iter().any(|word| name.contains(word))
}

fn is_env_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    })
}

fn without_url_credentials(word: &str) -> String {
    let Some((scheme, rest)) = word.split_once("://") else {
        return word.to_string();
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once('@') {
        Some((_, host)) => {
            let path = &rest[authority.len()..];
            format!("{scheme}://{REDACTED}@{host}{path}")
        }
        None => word.to_string(),
    }
}

pub fn redact(command: &str, home: &Path) -> String {
    let mut secret_next = false;
    let words: Vec<String> = command
        .split_whitespace()
        .map(|word| {
            if std::mem::take(&mut secret_next) {
                return REDACTED.to_string();
            }
            if is_env_assignment(word) {
                let name = word.split('=').next().unwrap_or_default();
                return format!("{name}={REDACTED}");
            }
            if let Some((flag, _)) = word.split_once('=')
                && flag.starts_with('-')
                && is_secret_name(flag)
            {
                return format!("{flag}={REDACTED}");
            }
            secret_next = word.starts_with('-') && is_secret_name(word);
            without_url_credentials(word)
        })
        .collect();
    let home = home.to_string_lossy();
    words.join(" ").replace(home.as_ref(), "~")
}

fn row(server: &Server, include: Include, home: &Path) -> Row {
    let measured = !server.resources.is_empty();
    let source = match (&server.container, server.pid) {
        (Some(_), _) => "container",
        (None, Some(_)) => "process",
        (None, None) => "unowned",
    };
    let mut unavailable = Vec::new();
    if server.pid.is_none() {
        unavailable.push("pid");
    }
    if !measured {
        unavailable.extend(["memoryBytes", "cpuPercent"]);
    }
    if server.command.is_empty() {
        unavailable.push("command");
    }
    Row {
        id: server.id.replace(home.to_string_lossy().as_ref(), "~"),
        port: server.port,
        addresses: server.addresses.clone(),
        exposed: server.exposed,
        kind: server.kind,
        source,
        process: server.process.clone(),
        pid: server.pid,
        owner: server.owner.clone(),
        image: server.container.as_ref().map(|c| c.image.clone()),
        status: server.status,
        health: server.health,
        hidden: server.hidden,
        protected: server.protected,
        framework: server.framework,
        framework_confidence: server.attribution.confidence,
        supervisor: server.supervisor.clone(),
        memory_bytes: measured.then_some(server.memory_bytes),
        cpu_percent: measured.then_some(server.cpu_percent),
        command: (include.commands && !server.command.is_empty())
            .then(|| redact(&server.command, home)),
        folder: server.folder.clone().filter(|_| include.paths),
        cwd: server.cwd.clone().filter(|_| include.paths),
        unavailable,
    }
}

fn export(
    servers: &[Server],
    include: Include,
    settings: SavedSettings,
    rules: &Rules,
    home: &Path,
    now: u64,
) -> Export {
    let without_process = servers
        .iter()
        .filter(|server| server.pid.is_none() && server.container.is_none())
        .count();
    let excluded = [
        (!include.commands, "commands"),
        (!include.paths, "paths"),
        (!include.settings, "settings"),
    ];
    Export {
        schema: SCHEMA,
        app: App {
            name: "Ports",
            version: env!("CARGO_PKG_VERSION"),
        },
        platform: Platform {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        exported_at: now,
        excluded: excluded
            .into_iter()
            .filter_map(|(left_out, name)| left_out.then_some(name))
            .collect(),
        discovery: Discovery {
            complete: without_process == 0,
            servers: servers.len(),
            without_process,
            unmeasured: servers.iter().filter(|s| s.resources.is_empty()).count(),
        },
        servers: servers
            .iter()
            .map(|server| row(server, include, home))
            .collect(),
        settings: include.settings.then_some(settings),
        rules: include.settings.then(|| rules.into()),
    }
}

#[tauri::command]
pub async fn preview_diagnostics(app: AppHandle, include: Include) -> Result<String, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        let servers = monitor.servers(&app);
        let (_, rules) = monitor.rules();
        let settings = SavedSettings {
            alerts: monitor.alert_settings(),
            health: monitor.health_settings(),
            tray_metric: monitor.tray_metric(),
            stop_grace_seconds: monitor.stop_grace_secs(),
            idle_after_minutes: monitor.idle_after_ms() / 60_000,
        };
        let home = project::canonical(&app.path().home_dir().map_err(|error| error.to_string())?);
        let export = export(&servers, include, settings, &rules, &home, now_ms());
        serde_json::to_string_pretty(&export).map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn save_diagnostics(app: AppHandle, contents: String) -> Result<Option<String>, String> {
    blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .add_filter("JSON", &["json"])
            .set_file_name("ports-diagnostics.json")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = path.into_path().map_err(|error| error.to_string())?;
        std::fs::write(&path, contents)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::alerts::tests::server;
    use super::*;

    const EVERYTHING: Include = Include {
        commands: true,
        paths: true,
        settings: true,
    };
    const NOTHING_EXTRA: Include = Include {
        commands: false,
        paths: false,
        settings: false,
    };

    fn saved() -> SavedSettings {
        SavedSettings {
            alerts: AlertSettings::default(),
            health: HealthSettings::default(),
            tray_metric: TrayMetric::Off,
            stop_grace_seconds: 3,
            idle_after_minutes: 5,
        }
    }

    fn exported(servers: &[Server], include: Include) -> serde_json::Value {
        let export = export(
            servers,
            include,
            saved(),
            &Rules::default(),
            Path::new("/Users/me"),
            42,
        );
        serde_json::to_value(export).unwrap()
    }

    fn secretive() -> Server {
        let mut server = server(3000, 0);
        server.command = "API_KEY=abc node --token=abc server.js".into();
        server.folder = Some("~/projects/secret-client".into());
        server.cwd = Some("~/projects/secret-client/app".into());
        server
    }

    #[test]
    fn removes_secrets_from_commands() {
        let home = Path::new("/Users/me");
        for (command, expected) in [
            ("vite --port 5173", "vite --port 5173"),
            ("API_KEY=abc vite", "API_KEY=[redacted] vite"),
            ("node --token=abc app", "node --token=[redacted] app"),
            ("node --api-key abc app", "node --api-key [redacted] app"),
            ("node --password abc", "node --password [redacted]"),
            (
                "psql postgres://me:hunter2@localhost:5432/db",
                "psql postgres://[redacted]@localhost:5432/db",
            ),
            ("node /Users/me/app/index.js", "node ~/app/index.js"),
        ] {
            assert_eq!(redact(command, home), expected);
        }
    }

    #[test]
    fn leaves_commands_paths_and_settings_out_unless_asked() {
        let export = exported(&[secretive()], NOTHING_EXTRA);
        let text = export.to_string();
        assert!(!text.contains("abc") && !text.contains("secret-client"));
        assert_eq!(
            export["excluded"],
            serde_json::json!(["commands", "paths", "settings"])
        );
        assert!(export["settings"].is_null() && export["rules"].is_null());
        assert!(export["servers"][0]["command"].is_null());
    }

    #[test]
    fn includes_what_was_asked_for_with_secrets_still_removed() {
        let export = exported(&[secretive()], EVERYTHING);
        let row = &export["servers"][0];
        assert_eq!(
            row["command"],
            "API_KEY=[redacted] node --token=[redacted] server.js"
        );
        assert_eq!(row["folder"], "~/projects/secret-client");
        assert_eq!(export["excluded"], serde_json::json!([]));
        assert_eq!(export["settings"]["health"]["timeoutSeconds"], 3);
        assert_eq!(export["rules"]["shownKinds"][0], "dev");
    }

    #[test]
    fn says_what_it_could_not_find_out() {
        let mut unowned = server(445, 0);
        unowned.pid = None;
        unowned.owner = Some("root".into());
        unowned.command = String::new();
        let mut known = server(3000, 0);
        known.resources = Vec::new();
        let export = exported(&[unowned, known], EVERYTHING);
        assert_eq!(export["discovery"]["complete"], false);
        assert_eq!(export["discovery"]["withoutProcess"], 1);
        let row = &export["servers"][0];
        assert_eq!(row["source"], "unowned");
        assert!(row["pid"].is_null() && row["memoryBytes"].is_null());
        assert_eq!(
            row["unavailable"],
            serde_json::json!(["pid", "memoryBytes", "cpuPercent", "command"])
        );
    }

    #[test]
    fn carries_the_schema_and_app_version() {
        let export = exported(&[], NOTHING_EXTRA);
        assert_eq!(export["schema"], SCHEMA);
        assert_eq!(export["app"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(export["exportedAt"], 42);
        assert_eq!(export["discovery"]["complete"], true);
    }
}
