use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::history::Status;
use super::network;
use super::sampler::{Server, now_ms};

const WARMING_MS: u64 = 60_000;
const EVERY_SECONDS: [u64; 4] = [5, 10, 30, 60];
const TIMEOUT_SECONDS: [u64; 4] = [1, 3, 5, 10];

pub type Key = String;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthSettings {
    pub enabled: bool,
    pub every_seconds: u64,
    pub timeout_seconds: u64,
}

impl Default for HealthSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            every_seconds: 10,
            timeout_seconds: 3,
        }
    }
}

impl HealthSettings {
    pub fn is_valid(&self) -> bool {
        EVERY_SECONDS.contains(&self.every_seconds)
            && TIMEOUT_SECONDS.contains(&self.timeout_seconds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthState {
    Unknown,
    Warming,
    Responsive,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub state: HealthState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
}

impl Health {
    pub const UNKNOWN: Self = Self {
        state: HealthState::Unknown,
        checked_at: None,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Check {
    answered: bool,
    at: u64,
}

pub type Checks = HashMap<Key, Check>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Probe {
    Http,
    Tcp,
}

#[derive(Debug, Clone)]
pub struct Target {
    key: Key,
    address: SocketAddr,
    probe: Probe,
}

pub fn target(server: &Server) -> Option<Target> {
    server.pid?;
    Some(Target {
        key: server.id.clone(),
        address: network::local_address(server.port, &server.addresses),
        probe: if server.http { Probe::Http } else { Probe::Tcp },
    })
}

pub fn check_all(targets: &[Target], timeout: Duration) -> Checks {
    let answers: Vec<_> = thread::scope(|scope| {
        let probes: Vec<_> = targets
            .iter()
            .map(|target| {
                scope.spawn(move || (&target.key, probe(target.address, target.probe, timeout)))
            })
            .collect();
        probes
            .into_iter()
            .filter_map(|probe| probe.join().ok())
            .collect()
    });
    let at = now_ms();
    answers
        .into_iter()
        .filter_map(|(key, answered)| {
            Some((
                key.clone(),
                Check {
                    answered: answered?,
                    at,
                },
            ))
        })
        .collect()
}

pub fn same_outcomes(a: &Checks, b: &Checks) -> bool {
    a.len() == b.len()
        && a.iter().all(|(key, check)| {
            b.get(key)
                .is_some_and(|other| other.answered == check.answered)
        })
}

pub fn apply(checks: &Checks, enabled: bool, servers: &mut [Server], now: u64) {
    if !enabled {
        return;
    }
    for server in servers.iter_mut().filter(|server| server.pid.is_some()) {
        let warming = now.saturating_sub(server.listener_started_at) < WARMING_MS;
        let check = checks.get(&server.id);
        let state = match check {
            Some(check) if check.answered => HealthState::Responsive,
            _ if warming => HealthState::Warming,
            Some(_) => HealthState::TimedOut,
            None => HealthState::Unknown,
        };
        server.health = Health {
            state,
            checked_at: check.map(|check| check.at),
        };
        if state == HealthState::TimedOut && server.status != Status::Idle {
            server.status = Status::Attention;
        }
    }
}

fn probe(address: SocketAddr, probe: Probe, timeout: Duration) -> Option<bool> {
    let mut stream = match TcpStream::connect_timeout(&address, timeout) {
        Ok(stream) => stream,
        // The accept queue is full: the server stopped taking connections.
        Err(error) if error.kind() == ErrorKind::TimedOut => return Some(false),
        Err(_) => return None,
    };
    if probe == Probe::Tcp {
        return Some(true);
    }
    // RFC 9110 §9.3.7: `OPTIONS *` asks about the server itself, as a ping,
    // so it runs no route of the app.
    let request = format!(
        "OPTIONS * HTTP/1.1\r\nHost: localhost:{}\r\nUser-Agent: Ports\r\nConnection: close\r\n\r\n",
        address.port()
    );
    let answered = stream
        .set_write_timeout(Some(timeout))
        .and_then(|()| stream.set_read_timeout(Some(timeout)))
        .and_then(|()| stream.write_all(request.as_bytes()))
        .and_then(|()| stream.read(&mut [0; 1]));
    Some(match answered {
        Ok(_) => true,
        Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
    })
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::super::alerts::tests::server;
    use super::*;

    const SHORT: Duration = Duration::from_millis(300);
    const STARTED: u64 = 1_000_000;

    fn check(answered: bool) -> Check {
        Check {
            answered,
            at: STARTED + 5_000,
        }
    }

    fn started() -> Server {
        let mut server = server(3000, 0);
        server.listener_started_at = STARTED;
        server
    }

    fn state_at(checks: &Checks, enabled: bool, now: u64) -> (HealthState, Status) {
        let mut servers = [started()];
        apply(checks, enabled, &mut servers, now);
        (servers[0].health.state, servers[0].status)
    }

    #[test]
    fn a_new_server_warms_up_before_a_timeout_counts_against_it() {
        let silent = Checks::from([("test:3000".to_string(), check(false))]);
        assert_eq!(
            state_at(&Checks::new(), true, STARTED + 1_000),
            (HealthState::Warming, Status::Running)
        );
        assert_eq!(
            state_at(&silent, true, STARTED + WARMING_MS - 1),
            (HealthState::Warming, Status::Running)
        );
        assert_eq!(
            state_at(&silent, true, STARTED + WARMING_MS),
            (HealthState::TimedOut, Status::Attention)
        );
    }

    #[test]
    fn an_answer_ends_warming_and_records_when_it_was_checked() {
        let answered = Checks::from([("test:3000".to_string(), check(true))]);
        let mut servers = [started()];
        apply(&answered, true, &mut servers, STARTED + 6_000);
        assert_eq!(
            servers[0].health,
            Health {
                state: HealthState::Responsive,
                checked_at: Some(STARTED + 5_000),
            }
        );
    }

    #[test]
    fn a_server_nothing_checked_stays_unknown() {
        assert_eq!(
            state_at(&Checks::new(), true, STARTED + WARMING_MS).0,
            HealthState::Unknown
        );
    }

    #[test]
    fn turning_checks_off_shows_no_failure() {
        let silent = Checks::from([("test:3000".to_string(), check(false))]);
        assert_eq!(
            state_at(&silent, false, STARTED + WARMING_MS),
            (HealthState::Unknown, Status::Running)
        );
    }

    #[test]
    fn a_replacement_process_ignores_the_previous_result() {
        let stale = Checks::from([("test:3000".to_string(), check(false))]);
        let mut replacement = started();
        replacement.id = "process:new:3000".into();
        let mut servers = [replacement];
        apply(&stale, true, &mut servers, STARTED + WARMING_MS);
        assert_eq!(servers[0].health, Health::UNKNOWN);
    }

    #[test]
    fn comparing_rounds_ignores_when_they_ran() {
        let earlier = Checks::from([("a".to_string(), check(true))]);
        let mut later = earlier.clone();
        later.get_mut("a").unwrap().at += 10_000;
        assert!(same_outcomes(&earlier, &later));
        later.get_mut("a").unwrap().answered = false;
        assert!(!same_outcomes(&earlier, &later));
    }

    #[test]
    fn only_the_offered_probe_settings_are_valid() {
        assert!(HealthSettings::default().is_valid());
        assert!(
            !HealthSettings {
                timeout_seconds: 0,
                ..HealthSettings::default()
            }
            .is_valid()
        );
        assert!(
            !HealthSettings {
                every_seconds: 1,
                ..HealthSettings::default()
            }
            .is_valid()
        );
    }

    #[test]
    fn a_server_that_answers_is_responding() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 512]);
            stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").unwrap();
        });
        assert_eq!(probe(address, Probe::Http, SHORT), Some(true));
        server.join().unwrap();
    }

    // A process stuck in a loop never accepts: the kernel completes the
    // handshake for it, then nothing reads the request.
    #[test]
    fn a_server_that_never_answers_is_not_responding() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        assert_eq!(probe(address, Probe::Http, SHORT), Some(false));
        assert_eq!(probe(address, Probe::Tcp, SHORT), Some(true));
    }

    #[test]
    fn a_free_port_has_nothing_to_probe() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let connection = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let nothing_listens = connection.local_addr().unwrap();
        assert_eq!(probe(nothing_listens, Probe::Http, SHORT), None);
    }
}
