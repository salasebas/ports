use std::collections::HashSet;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

use super::history::Status;
use super::network;
use super::sampler::Server;

pub const PROBE_EVERY_MS: u64 = 10_000;
pub const TIMEOUT: Duration = Duration::from_secs(3);

pub type Key = String;

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

pub fn unresponsive(targets: &[Target]) -> HashSet<Key> {
    thread::scope(|scope| {
        let probes: Vec<_> = targets
            .iter()
            .map(|target| {
                scope.spawn(move || {
                    (
                        target.key.clone(),
                        probe(target.address, target.probe, TIMEOUT),
                    )
                })
            })
            .collect();
        probes
            .into_iter()
            .filter_map(|probe| probe.join().ok())
            .filter(|(_, answered)| *answered == Some(false))
            .map(|(key, _)| key)
            .collect()
    })
}

pub fn apply(unresponsive: &HashSet<Key>, servers: &mut [Server]) {
    for server in servers {
        if unresponsive.contains(&server.id) {
            server.responding = false;
            if server.status != Status::Idle {
                server.status = Status::Attention;
            }
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

    use super::*;

    const SHORT: Duration = Duration::from_millis(300);

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
