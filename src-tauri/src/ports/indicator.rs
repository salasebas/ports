use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::alerts;
use super::health::HealthState;
use super::rules::Rules;
use super::sampler::Server;

const IDLE_TOOLTIP: &str = "Ports";
const WARNING: &str = "⚠";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrayMetric {
    #[default]
    Off,
    Servers,
    Memory,
    Cpu,
    ServersAndMemory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Indicator {
    pub title: Option<String>,
    pub tooltip: String,
}

fn needs_attention(server: &Server) -> bool {
    server.health.state == HealthState::TimedOut
        || (server.growth.is_some() && server.snoozed_until.is_none())
}

pub fn indicator(metric: TrayMetric, rules: &Rules, servers: &[Server]) -> Indicator {
    if metric == TrayMetric::Off {
        return Indicator {
            title: None,
            tooltip: IDLE_TOOLTIP.to_string(),
        };
    }
    let shown: Vec<_> = servers
        .iter()
        .filter(|server| rules.shows(server))
        .collect();
    let mut counted = HashSet::new();
    let mut memory = 0;
    let mut cpu = 0.0;
    for resource in shown.iter().flat_map(|server| &server.resources) {
        if counted.insert(&resource.process) {
            memory += resource.memory_bytes;
            cpu += resource.cpu_percent;
        }
    }
    let count = shown.len();
    let memory = alerts::bytes(memory);
    let cpu = format!("{}%", cpu.round());
    let value = match metric {
        TrayMetric::Off | TrayMetric::Servers => count.to_string(),
        TrayMetric::Memory => memory.clone(),
        TrayMetric::Cpu => cpu.clone(),
        TrayMetric::ServersAndMemory => format!("{count} · {memory}"),
    };
    let servers = if count == 1 { "server" } else { "servers" };
    let summary = format!("{count} {servers} · {memory} · {cpu} CPU");
    let attention = shown
        .iter()
        .filter(|server| needs_attention(server))
        .count();
    if attention == 0 {
        Indicator {
            title: Some(value),
            tooltip: summary,
        }
    } else {
        Indicator {
            title: Some(format!("{WARNING} {value}")),
            tooltip: format!("{summary} · {attention} need attention"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::alerts::tests::server;
    use super::super::health::Health;
    use super::super::history::Growth;
    use super::super::identity::ProcessIdentity;
    use super::super::sampler::{Kind, ProcessUsage};
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn usage(pid: u32, memory_bytes: u64, cpu_percent: f64) -> ProcessUsage {
        ProcessUsage {
            process: ProcessIdentity {
                pid,
                start: "1".into(),
            },
            parent: None,
            name: "node".into(),
            ports: Vec::new(),
            memory_bytes,
            cpu_percent,
        }
    }

    fn measured(port: u16, resources: Vec<ProcessUsage>) -> Server {
        let mut server = server(port, 0);
        server.resources = resources;
        server
    }

    fn title(metric: TrayMetric, servers: &[Server]) -> Option<String> {
        indicator(metric, &Rules::default(), servers).title
    }

    #[test]
    fn stays_quiet_until_a_metric_is_chosen() {
        let off = indicator(TrayMetric::Off, &Rules::default(), &[server(3000, 0)]);
        assert_eq!(off.title, None);
        assert_eq!(off.tooltip, "Ports");
        assert_eq!(TrayMetric::default(), TrayMetric::Off);
    }

    #[test]
    fn shows_each_metric_of_the_servers() {
        let servers = [
            measured(3000, vec![usage(1, 1024 * MIB, 10.0)]),
            measured(5173, vec![usage(2, 512 * MIB, 2.4)]),
        ];
        let title = |metric| title(metric, &servers).unwrap();
        assert_eq!(title(TrayMetric::Servers), "2");
        assert_eq!(title(TrayMetric::Memory), "1.5 GB");
        assert_eq!(title(TrayMetric::Cpu), "12%");
        assert_eq!(title(TrayMetric::ServersAndMemory), "2 · 1.5 GB");
    }

    #[test]
    fn counts_a_process_shared_by_two_ports_once() {
        let shared = usage(1, 100 * MIB, 5.0);
        let servers = [
            measured(3000, vec![shared.clone()]),
            measured(3001, vec![shared, usage(2, 50 * MIB, 1.0)]),
        ];
        assert_eq!(title(TrayMetric::Memory, &servers).unwrap(), "150 MB");
        assert_eq!(title(TrayMetric::Cpu, &servers).unwrap(), "6%");
    }

    #[test]
    fn leaves_out_what_the_list_hides() {
        let visible = measured(3000, vec![usage(1, 100 * MIB, 1.0)]);
        let mut hidden = measured(3001, vec![usage(2, 100 * MIB, 1.0)]);
        hidden.hidden = true;
        let mut app = measured(3002, vec![usage(3, 100 * MIB, 1.0)]);
        app.kind = Kind::Other;
        let servers = [visible, hidden, app];
        assert_eq!(
            title(TrayMetric::ServersAndMemory, &servers).unwrap(),
            "1 · 100 MB"
        );
    }

    #[test]
    fn warns_with_a_symbol_and_words_not_just_color() {
        let mut stuck = server(3000, 0);
        stuck.health = Health {
            state: HealthState::TimedOut,
            checked_at: None,
        };
        let shown = indicator(TrayMetric::Servers, &Rules::default(), &[stuck]);
        assert_eq!(shown.title.unwrap(), "⚠ 1");
        assert!(shown.tooltip.ends_with("1 need attention"));
    }

    #[test]
    fn a_snoozed_server_stops_warning_about_growth() {
        let mut growing = server(3000, 0);
        growing.growth = Some(Growth {
            bytes: 100 * MIB,
            minutes: 10,
        });
        assert_eq!(
            title(TrayMetric::Servers, std::slice::from_ref(&growing)).unwrap(),
            "⚠ 1"
        );
        growing.snoozed_until = Some(1);
        assert_eq!(title(TrayMetric::Servers, &[growing]).unwrap(), "1");
    }

    #[test]
    fn a_hidden_server_does_not_warn() {
        let mut stuck = server(3000, 0);
        stuck.hidden = true;
        stuck.health = Health {
            state: HealthState::TimedOut,
            checked_at: None,
        };
        assert_eq!(title(TrayMetric::Servers, &[stuck]).unwrap(), "0");
    }
}
