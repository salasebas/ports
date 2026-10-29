use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::sampler::{Kind, Server};

const REPEAT_AFTER_MS: u64 = 60 * 60_000;
const MB: f64 = 1024.0 * 1024.0;
const GB: f64 = 1024.0 * MB;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertSettings {
    pub enabled: bool,
    #[serde(default)]
    pub threshold_bytes: Option<u64>,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_bytes: Some(2 * 1024 * 1024 * 1024),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Alert {
    pub server_id: String,
    pub port: u16,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
pub struct Alerts {
    sent: HashMap<String, u64>,
}

impl Alerts {
    pub fn due(&mut self, settings: AlertSettings, servers: &[Server], now: u64) -> Vec<Alert> {
        self.sent
            .retain(|_, at| now.saturating_sub(*at) < REPEAT_AFTER_MS);
        servers
            .iter()
            .filter(|server| matches!(server.kind, Kind::Dev | Kind::Database))
            .filter_map(|server| {
                server.pid?;
                let key = server.id.clone();
                let body = if settings.enabled && server.leaking {
                    let first = server.memory_history.first().copied().unwrap_or_default();
                    let growth = server.memory_bytes.saturating_sub(first);
                    format!("Grew by {} in 10 minutes and keeps growing.", bytes(growth))
                } else {
                    let threshold = settings
                        .threshold_bytes
                        .filter(|threshold| server.memory_bytes >= *threshold)?;
                    format!(
                        "Uses {}, above your {} alert.",
                        bytes(server.memory_bytes),
                        bytes(threshold)
                    )
                };
                if self.sent.contains_key(&key) {
                    return None;
                }
                self.sent.insert(key, now);
                Some(Alert {
                    server_id: server.id.clone(),
                    port: server.port,
                    title: format!("{} on :{}", server.name, server.port),
                    body,
                })
            })
            .collect()
    }
}

fn bytes(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else {
        format!("{:.0} MB", bytes / MB)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::history::Status;
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;
    const MINUTE: u64 = 60_000;

    pub(crate) fn server(port: u16, memory_bytes: u64) -> Server {
        Server {
            id: format!("test:{port}"),
            target: None,
            resources: Vec::new(),
            http: true,
            port,
            pid: Some(u32::from(port)),
            owner: None,
            container: None,
            process: "node".into(),
            command: "vite".into(),
            name: "web".into(),
            folder: None,
            cwd: None,
            project_name: None,
            branch: None,
            worktree: None,
            worktree_path: None,
            repository: None,
            repository_name: None,
            repository_id: None,
            session: None,
            framework: None,
            framework_version: None,
            attribution: Default::default(),
            web: Default::default(),
            icon: None,
            kind: Kind::Dev,
            status: Status::Running,
            leaking: false,
            responding: true,
            addresses: Vec::new(),
            exposed: false,
            lan_host: None,
            protected: false,
            restartable: true,
            supervisor: None,
            parent: None,
            started_at: 0,
            last_active_at: 0,
            memory_bytes,
            cpu_percent: 0.0,
            memory_history: vec![memory_bytes],
            cpu_history: vec![0.0],
        }
    }

    const MIB: u64 = 1024 * 1024;

    fn leaking(port: u16) -> Server {
        let mut server = server(port, 900 * MIB);
        server.leaking = true;
        server.memory_history = vec![300 * MIB, 900 * MIB];
        server
    }

    #[test]
    fn alerts_once_an_hour_about_memory_that_keeps_growing() {
        let mut alerts = Alerts::default();
        let settings = AlertSettings::default();
        let due = alerts.due(settings, &[leaking(3000)], 0);
        assert_eq!(
            due,
            vec![Alert {
                server_id: "test:3000".into(),
                port: 3000,
                title: "web on :3000".into(),
                body: "Grew by 600 MB in 10 minutes and keeps growing.".into(),
            }]
        );
        for minute in [1, 2, 30] {
            assert!(
                alerts
                    .due(settings, &[leaking(3000)], minute * MINUTE)
                    .is_empty()
            );
        }
        assert_eq!(alerts.due(settings, &[leaking(3000)], 62 * MINUTE).len(), 1);
    }

    #[test]
    fn alerts_once_when_a_server_grows_past_the_threshold() {
        let mut alerts = Alerts::default();
        let settings = AlertSettings::default();
        let sent: usize = [(0, GIB), (1, 3 * GIB), (2, 3 * GIB), (30, 4 * GIB)]
            .into_iter()
            .map(|(minute, memory)| {
                alerts
                    .due(settings, &[server(5173, memory)], minute * MINUTE)
                    .len()
            })
            .sum();
        assert_eq!(sent, 1);
    }

    #[test]
    fn ignores_high_memory_once_the_threshold_is_off() {
        let mut alerts = Alerts::default();
        let leaks_only = AlertSettings {
            threshold_bytes: None,
            ..AlertSettings::default()
        };
        assert!(
            alerts
                .due(leaks_only, &[server(5173, 8 * GIB)], 0)
                .is_empty()
        );
        assert_eq!(alerts.due(leaks_only, &[leaking(3000)], 0).len(), 1);
    }

    #[test]
    fn leaves_desktop_apps_alone() {
        let mut alerts = Alerts::default();
        let mut app = leaking(6443);
        app.kind = Kind::Other;
        assert!(alerts.due(AlertSettings::default(), &[app], 0).is_empty());
    }

    #[test]
    fn sends_nothing_while_alerts_are_off() {
        let mut alerts = Alerts::default();
        let off = AlertSettings {
            enabled: false,
            threshold_bytes: None,
        };
        let mut big_leak = leaking(5173);
        big_leak.memory_bytes = 8 * GIB;
        assert!(alerts.due(off, &[big_leak], 0).is_empty());
    }
}
