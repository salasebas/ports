//! Docker containers that publish ports. Docker Desktop, OrbStack, Colima and
//! Podman forward a published port through one helper process, so the socket
//! alone names the runtime, not the container. Their Engine API names the
//! container and stops it the way `docker stop` does, without touching the
//! runtime.

use std::collections::{BTreeSet, HashMap};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// Long enough for a container to finish `docker stop`'s own 10 s grace period.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub started_at: u64,
    /// The Engine API socket of the runtime that runs it.
    socket: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Listed {
    id: String,
    #[serde(default)]
    names: Vec<String>,
    image: String,
    created: u64,
    #[serde(default)]
    ports: Vec<Published>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Published {
    public_port: Option<u16>,
    #[serde(rename = "Type")]
    protocol: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Inspected {
    state: State,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct State {
    started_at: String,
}

/// Engine API sockets of the runtimes people use, deduplicated: apps opened
/// from the Dock don't inherit `DOCKER_HOST`, and `/var/run/docker.sock` is
/// often a link to one of the others.
fn sockets(home: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var("DOCKER_HOST")
        .ok()
        .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
    {
        candidates.push(path);
    }
    candidates.push("/var/run/docker.sock".into());
    for path in [
        ".docker/run/docker.sock",
        ".docker/desktop/docker.sock",
        ".orbstack/run/docker.sock",
        ".colima/default/docker.sock",
        ".rd/docker.sock",
    ] {
        candidates.push(home.join(path));
    }
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        candidates.push(Path::new(&runtime).join("docker.sock"));
        candidates.push(Path::new(&runtime).join("podman/podman.sock"));
    }
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter_map(|path| path.canonicalize().ok())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// One request per connection: the response ends at EOF because the
/// connection closes, which Docker only does when asked. Large bodies still
/// come chunked, whatever the request's HTTP version.
fn request(socket: &Path, method: &str, path: &str, timeout: Duration) -> Result<String, String> {
    let failed = |error: std::io::Error| match error.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => "Docker didn't answer in time".to_string(),
        _ => format!("Can't reach Docker: {error}"),
    };
    let mut stream = UnixStream::connect(socket).map_err(failed)?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .and_then(|()| {
            write!(
                stream,
                "{method} {path} HTTP/1.0\r\nHost: docker\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            )
        })
        .map_err(failed)?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(failed)?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or("Docker sent an invalid response")?;
    let status: u16 = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or("Docker sent an invalid response")?;
    let chunked = head.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
        })
    });
    let body = if chunked {
        unchunk(body).ok_or("Docker sent an invalid response")?
    } else {
        body.to_string()
    };
    // 304: the container was already stopped.
    if (200..300).contains(&status) || status == 304 {
        return Ok(body);
    }
    #[derive(Deserialize)]
    struct Failure {
        message: String,
    }
    Err(serde_json::from_str::<Failure>(&body)
        .map_or_else(|_| format!("Docker answered {status}"), |f| f.message))
}

/// A chunked body: each chunk is its size in hex, CRLF, the data, CRLF, until
/// a chunk of size 0.
fn unchunk(mut rest: &str) -> Option<String> {
    let mut body = String::new();
    loop {
        let (size, after) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            return Some(body);
        }
        body.push_str(after.get(..size)?);
        rest = after.get(size..)?.strip_prefix("\r\n")?;
    }
}

/// Running containers by the host port they publish over TCP, from every
/// runtime that answers. Empty when none does, e.g. Docker isn't installed
/// or the user isn't allowed to use it.
pub fn published(home: &Path) -> HashMap<u16, Container> {
    let mut ports = HashMap::new();
    for socket in sockets(home) {
        let Some(listed) = request(&socket, "GET", "/containers/json", LIST_TIMEOUT)
            .ok()
            .and_then(|body| serde_json::from_str::<Vec<Listed>>(&body).ok())
        else {
            continue;
        };
        for (port, mut container) in by_port(listed, &socket) {
            if let Some(started_at) = started_at(&socket, &container.id) {
                container.started_at = started_at;
            }
            ports.entry(port).or_insert(container);
        }
    }
    ports
}

fn by_port(listed: Vec<Listed>, socket: &Path) -> Vec<(u16, Container)> {
    let mut ports = Vec::new();
    for listed in listed {
        let container = Container {
            name: listed
                .names
                .first()
                .map_or(&listed.id[..12.min(listed.id.len())], |name| {
                    name.trim_start_matches('/')
                })
                .to_string(),
            id: listed.id,
            image: listed.image,
            // Replaced by the start time; a restart keeps the creation time.
            started_at: listed.created * 1000,
            socket: socket.to_path_buf(),
        };
        let tcp: BTreeSet<u16> = listed
            .ports
            .iter()
            .filter(|port| port.protocol == "tcp")
            .filter_map(|port| port.public_port)
            .collect();
        ports.extend(tcp.into_iter().map(|port| (port, container.clone())));
    }
    ports
}

fn started_at(socket: &Path, id: &str) -> Option<u64> {
    let body = request(
        socket,
        "GET",
        &format!("/containers/{id}/json"),
        LIST_TIMEOUT,
    )
    .ok()?;
    let inspected: Inspected = serde_json::from_str(&body).ok()?;
    parse_time(&inspected.state.started_at)
}

/// Milliseconds since the epoch for Docker's UTC times, e.g.
/// `2026-10-27T07:21:04.123456789Z`.
fn parse_time(text: &str) -> Option<u64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let numbers = |text: &str, separator| -> Option<Vec<i64>> {
        text.split(separator).map(|n| n.parse().ok()).collect()
    };
    let [year, month, day] = numbers(date, '-')?[..] else {
        return None;
    };
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let [hour, minute, second] = numbers(clock, ':')?[..] else {
        return None;
    };
    let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    // Days since 1970-01-01, from Howard Hinnant's `days_from_civil`.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds * 1_000 + millis).ok()
}

/// What the image is, e.g. `postgres` for `docker.io/library/postgres:16-alpine`.
pub fn image_name(image: &str) -> &str {
    let without_digest = image.split('@').next().unwrap_or(image);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last.split(':').next().unwrap_or(last)
}

/// The container that publishes `port`, if it's still the one with `id`.
fn locate(home: &Path, port: u16, id: &str) -> Result<Container, String> {
    published(home)
        .remove(&port)
        .filter(|container| container.id == id)
        .ok_or_else(|| format!(":{port} is no longer published by that container"))
}

pub fn stop(home: &Path, port: u16, id: &str) -> Result<(), String> {
    let container = locate(home, port, id)?;
    let path = format!("/containers/{}/stop", container.id);
    request(&container.socket, "POST", &path, CONTROL_TIMEOUT).map(drop)
}

pub fn restart(home: &Path, port: u16, id: &str) -> Result<(), String> {
    let container = locate(home, port, id)?;
    let path = format!("/containers/{}/restart", container.id);
    request(&container.socket, "POST", &path, CONTROL_TIMEOUT).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_each_published_tcp_port_to_its_container() {
        // Trimmed from Docker Engine API 1.47 `GET /containers/json`.
        let body = r#"[
            {"Id":"4a1b2c3d4e5f6a7b8c9d","Names":["/shop-db"],"Image":"postgres:16-alpine","Created":1761530000,
             "Ports":[{"IP":"0.0.0.0","PrivatePort":5432,"PublicPort":5433,"Type":"tcp"},
                      {"IP":"::","PrivatePort":5432,"PublicPort":5433,"Type":"tcp"}]},
            {"Id":"9f8e7d6c5b4a39281706","Names":["/dns"],"Image":"coredns/coredns","Created":1761530001,
             "Ports":[{"IP":"0.0.0.0","PrivatePort":53,"PublicPort":5353,"Type":"udp"},
                      {"PrivatePort":8080,"Type":"tcp"}]}
        ]"#;
        let ports = by_port(
            serde_json::from_str(body).unwrap(),
            Path::new("/docker.sock"),
        );
        assert_eq!(ports.len(), 1);
        let (port, db) = &ports[0];
        assert_eq!(*port, 5433);
        assert_eq!(db.name, "shop-db");
        assert_eq!(db.image, "postgres:16-alpine");
    }

    #[test]
    fn joins_a_chunked_body() {
        // The shape OrbStack's Docker 29 sends for a long container list.
        assert_eq!(
            unchunk("5\r\n[{\"a\"\r\n4\r\n:1}]\r\n0\r\n\r\n").as_deref(),
            Some("[{\"a\":1}]")
        );
        assert_eq!(unchunk("5\r\nabc"), None);
    }

    #[test]
    fn reads_the_start_time_docker_reports() {
        // Expected values from Python's `datetime(..., tzinfo=timezone.utc).timestamp()`.
        assert_eq!(
            parse_time("2026-10-27T07:21:04.500123456Z"),
            Some(1_793_085_664_500)
        );
        assert_eq!(parse_time("2000-02-29T23:59:59Z"), Some(951_868_799_000));
        assert_eq!(parse_time("0001-01-01T00:00:00Z"), None);
    }

    #[test]
    fn names_the_image_without_registry_or_tag() {
        assert_eq!(
            image_name("docker.io/library/postgres:16-alpine"),
            "postgres"
        );
        assert_eq!(image_name("localhost:5000/team/api:dev"), "api");
        assert_eq!(image_name("redis@sha256:abc"), "redis");
        assert_eq!(image_name("mongo"), "mongo");
    }
}
