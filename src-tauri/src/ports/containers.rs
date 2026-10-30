use super::control::Shutdown;
use reqwest::Method;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(1);
const COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub started_at: u64,
    pub socket: PathBuf,
    pub bindings: BTreeSet<SocketAddr>,
    pub project_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerTarget {
    pub port: u16,
    pub id: String,
    pub engine: PathBuf,
    pub addresses: Vec<IpAddr>,
}

impl ContainerTarget {
    pub fn key(&self) -> String {
        format!("container:{}:{}", self.engine.display(), self.id)
    }
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
    #[serde(default)]
    labels: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Published {
    public_port: Option<u16>,
    #[serde(rename = "IP")]
    ip: Option<String>,
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

/// One client per engine for the app's lifetime: building a blocking client
/// starts a runtime thread, too costly to repeat on every sample.
fn client(socket: &Path) -> Result<Client, String> {
    static CLIENTS: OnceLock<Mutex<HashMap<PathBuf, Client>>> = OnceLock::new();
    let mut clients = CLIENTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(client) = clients.get(socket) {
        return Ok(client.clone());
    }
    let client = Client::builder()
        .unix_socket(socket.to_path_buf())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(CONTROL_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    clients.insert(socket.to_path_buf(), client.clone());
    Ok(client)
}

fn request(
    client: &Client,
    method: Method,
    path: &str,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let response = client
        .request(method, format!("http://docker{path}"))
        .timeout(timeout)
        .header("Connection", "close")
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("Docker response is too large".into());
    }
    if status.is_success() || status.as_u16() == 304 {
        return Ok(body);
    }
    #[derive(Deserialize)]
    struct Failure {
        message: String,
    }
    Err(serde_json::from_slice::<Failure>(&body)
        .map_or_else(|_| format!("Docker answered {status}"), |f| f.message))
}

pub fn published(home: &Path) -> Vec<Container> {
    published_at(&sockets(home))
}

pub(super) fn published_at(paths: &[PathBuf]) -> Vec<Container> {
    let deadline = Instant::now() + COLLECTION_TIMEOUT;
    let mut containers = Vec::new();
    for socket in paths {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let Ok(client) = client(socket) else {
            continue;
        };
        let listed = request(
            &client,
            Method::GET,
            "/containers/json",
            remaining.min(LIST_TIMEOUT),
        )
        .and_then(|body| {
            serde_json::from_slice::<Vec<Listed>>(&body).map_err(|error| error.to_string())
        });
        let listed = match listed {
            Ok(listed) => listed,
            Err(error) => {
                eprintln!("Could not list containers at {}: {error}", socket.display());
                continue;
            }
        };
        for mut container in by_container(listed, socket) {
            if let Some(remaining) = deadline.checked_duration_since(Instant::now())
                && let Ok(body) = request(
                    &client,
                    Method::GET,
                    &format!("/containers/{}/json", container.id),
                    remaining.min(LIST_TIMEOUT),
                )
                && let Ok(inspected) = serde_json::from_slice::<Inspected>(&body)
                && let Some(started) = parse_time(&inspected.state.started_at)
            {
                container.started_at = started;
            }
            containers.push(container);
        }
    }
    containers
}

fn by_container(listed: Vec<Listed>, socket: &Path) -> Vec<Container> {
    listed
        .into_iter()
        .filter_map(|listed| {
            let bindings: BTreeSet<_> = listed
                .ports
                .into_iter()
                .filter(|p| p.protocol == "tcp")
                .filter_map(|p| {
                    let port = p.public_port.filter(|p| *p != 0)?;
                    let ip = match p.ip {
                        Some(ip) => ip.parse::<IpAddr>().ok()?,
                        None => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    };
                    Some(SocketAddr::new(ip.to_canonical(), port))
                })
                .collect();
            if bindings.is_empty() {
                return None;
            }
            Some(Container {
                name: listed
                    .names
                    .first()
                    .map_or(&listed.id[..12.min(listed.id.len())], |name| {
                        name.trim_start_matches('/')
                    })
                    .to_string(),
                id: listed.id,
                image: listed.image,
                started_at: listed.created * 1000,
                socket: socket.to_path_buf(),
                bindings,
                project_dir: listed
                    .labels
                    .get("com.docker.compose.project.working_dir")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute()),
            })
        })
        .collect()
}

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
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds * 1_000 + millis).ok()
}

pub fn image_name(image: &str) -> &str {
    let without_digest = image.split('@').next().unwrap_or(image);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last.split(':').next().unwrap_or(last)
}

fn locate(home: &Path, target: &ContainerTarget) -> Result<Container, String> {
    if target.addresses.is_empty() || !sockets(home).contains(&target.engine) {
        return Err("The container runtime is no longer available".into());
    }
    published_at(std::slice::from_ref(&target.engine))
        .into_iter()
        .find(|c| {
            c.id == target.id
                && target
                    .addresses
                    .iter()
                    .all(|ip| c.bindings.contains(&SocketAddr::new(*ip, target.port)))
        })
        .ok_or_else(|| format!(":{} is no longer published by that container", target.port))
}

pub fn stop(home: &Path, target: &ContainerTarget, shutdown: Shutdown) -> Result<(), String> {
    if shutdown.force {
        control(home, target, "kill", Duration::ZERO)
    } else {
        control(home, target, "stop", shutdown.grace)
    }
}

pub fn restart(home: &Path, target: &ContainerTarget, shutdown: Shutdown) -> Result<(), String> {
    control(home, target, "restart", shutdown.grace)
}

fn control(
    home: &Path,
    target: &ContainerTarget,
    action: &str,
    grace: Duration,
) -> Result<(), String> {
    let container = locate(home, target)?;
    let path = match action {
        "kill" => format!("/containers/{}/kill", container.id),
        _ => format!(
            "/containers/{}/{action}?t={}",
            container.id,
            grace.as_secs()
        ),
    };
    request(
        &client(&container.socket)?,
        Method::POST,
        &path,
        CONTROL_TIMEOUT + grace,
    )
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    fn inspects_a_multiport_container_once_and_decodes_split_unicode_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("docker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let engine = std::thread::spawn(move || {
            let mut paths = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                paths.push(line.split_whitespace().nth(1).unwrap().to_string());
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                let body = if paths.len() == 1 {
                    r#"[{"Id":"abc123","Names":["/café"],"Image":"node","Created":1,"Ports":[{"PublicPort":3000,"IP":"127.0.0.1","Type":"tcp"},{"PublicPort":3001,"IP":"::1","Type":"tcp"},{"PublicPort":3002,"IP":"0.0.0.0","Type":"tcp"}]}]"#
                } else {
                    r#"{"State":{"StartedAt":"2026-10-27T07:21:04Z"}}"#
                };
                stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
                for byte in body.as_bytes() {
                    stream
                        .write_all(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n'])
                        .unwrap();
                }
                stream.write_all(b"0\r\n\r\n").unwrap();
            }
            paths
        });
        let result = published_at(&[socket]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "café");
        assert_eq!(result[0].bindings.len(), 3);
        assert_eq!(result[0].started_at, 1_793_085_664_000);
        assert_eq!(
            engine.join().unwrap(),
            ["/containers/json", "/containers/abc123/json"]
        );
    }

    #[test]
    fn maps_each_published_tcp_port_to_its_container() {
        let body = r#"[
            {"Id":"4a1b2c3d4e5f6a7b8c9d","Names":["/shop-db"],"Image":"postgres:16-alpine","Created":1761530000,
             "Labels":{"com.docker.compose.project.working_dir":"/work/shop"},
             "Ports":[{"IP":"0.0.0.0","PrivatePort":5432,"PublicPort":5433,"Type":"tcp"},
                      {"IP":"::","PrivatePort":5432,"PublicPort":5433,"Type":"tcp"}]},
            {"Id":"9f8e7d6c5b4a39281706","Names":["/dns"],"Image":"coredns/coredns","Created":1761530001,
             "Ports":[{"IP":"0.0.0.0","PrivatePort":53,"PublicPort":5353,"Type":"udp"},
                      {"PrivatePort":8080,"Type":"tcp"}]}
        ]"#;
        let ports = by_container(
            serde_json::from_str(body).unwrap(),
            Path::new("/docker.sock"),
        );
        assert_eq!(ports.len(), 1);
        let db = &ports[0];
        assert_eq!(
            db.bindings,
            BTreeSet::from([
                "0.0.0.0:5433".parse().unwrap(),
                "[::]:5433".parse().unwrap()
            ])
        );
        assert_eq!(db.name, "shop-db");
        assert_eq!(db.image, "postgres:16-alpine");
        assert_eq!(db.project_dir.as_deref(), Some(Path::new("/work/shop")));
    }

    #[test]
    fn reads_the_start_time_docker_reports() {
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
