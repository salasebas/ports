//! Checks which servers serve a web page, over HTTPS or HTTP, and reads the
//! page's title and icon. A listening socket alone doesn't say: databases
//! and custom TCP services listen too.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::{Url, header, redirect};
use serde::Serialize;

use super::icons::{self, Icon, Icons};
use super::network;
use super::project;
use super::sampler::{Kind, Server};

/// How often Ports looks for servers due a check.
pub const CHECK_EVERY_MS: u64 = 10_000;
/// A server that just started may still be compiling before it answers.
const STARTING_MS: u64 = 2 * 60_000;
const STARTING_RECHECK_MS: u64 = 15_000;
/// A server that took the connection but didn't answer in time may be
/// compiling its first page.
const TIMED_OUT_RECHECK_MS: u64 = 60_000;
/// A server that answered neither is asked again rarely: each check is a
/// connection, which would keep a forgotten server from turning idle.
const NO_PAGE_RECHECK_MS: u64 = 10 * 60_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// Generous enough for a dev server compiling its first page.
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HTML_BYTES: u64 = 512 * 1024;
const MAX_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

/// Whether a server serves a web page. Mirrors `Web` in `src/lib/ports/types.ts`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Web {
    /// Not checked: Ports doesn't check databases, desktop apps or other
    /// users' processes, and a new server waits for the next round.
    #[default]
    Unknown,
    /// Answered an HTTPS or HTTP request for `/`, with any status.
    Page {
        scheme: Scheme,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// Answered neither: it speaks another protocol, or didn't answer in time.
    None,
}

/// A server's identity in the snapshot.
type Key = String;

/// What one check found.
#[derive(Debug, Default)]
pub struct Found {
    web: Web,
    icon: Option<Arc<Icon>>,
    /// Some request connected but got no answer in time.
    timed_out: bool,
}

struct Checked {
    at: u64,
    found: Found,
}

/// What the latest checks found, and when the next round is due.
#[derive(Default)]
pub struct Pages {
    checked: HashMap<Key, Checked>,
    checking: bool,
    last_round_at: u64,
}

#[derive(Debug, Clone)]
pub struct Target {
    key: Key,
    address: SocketAddr,
    started_at: u64,
}

/// Servers that may serve a page: those in a project, containers, and
/// processes whose command is a web server. Databases are left out, in a
/// container too, since some log an HTTP request as an attack.
pub fn target(server: &Server) -> Option<Target> {
    if server.kind == Kind::Database
        || server.framework.is_some_and(project::is_database)
        || server.pid.is_none() && server.container.is_none()
    {
        return None;
    }
    if !matches!(server.kind, Kind::Dev | Kind::Container) && !server.http {
        return None;
    }
    Some(Target {
        key: server.id.clone(),
        address: network::local_address(server.port, &server.addresses),
        started_at: server.started_at,
    })
}

impl Pages {
    /// Starts a round once the last one is old enough; returns the servers
    /// due a check, and forgets the ones that are gone.
    pub fn due(&mut self, targets: Vec<Target>, now: u64) -> Option<Vec<Target>> {
        if self.checking || now.saturating_sub(self.last_round_at) < CHECK_EVERY_MS {
            return None;
        }
        let keys: HashSet<&Key> = targets.iter().map(|target| &target.key).collect();
        self.checked.retain(|key, _| keys.contains(key));
        let due: Vec<Target> = targets
            .into_iter()
            .filter(|target| {
                self.checked.get(&target.key).is_none_or(|checked| {
                    // A page is checked once per server: a `GET /` renders it,
                    // and that work would keep the server from turning idle.
                    // A restarted server has a new key and is checked again.
                    let every = match &checked.found {
                        Found {
                            web: Web::Page { .. },
                            ..
                        } => return false,
                        _ if now.saturating_sub(target.started_at) < STARTING_MS => {
                            STARTING_RECHECK_MS
                        }
                        Found {
                            timed_out: true, ..
                        } => TIMED_OUT_RECHECK_MS,
                        _ => NO_PAGE_RECHECK_MS,
                    };
                    now.saturating_sub(checked.at) >= every
                })
            })
            .collect();
        self.last_round_at = now;
        self.checking = !due.is_empty();
        self.checking.then_some(due)
    }

    /// Records a round's results; true when any server's page changed.
    pub fn finish(&mut self, results: Vec<(Key, Found)>, now: u64) -> bool {
        self.checking = false;
        let mut changed = false;
        for (key, found) in results {
            let icon = found.icon.as_ref().map(|icon| icon.key.clone());
            let web = found.web.clone();
            let previous = self.checked.insert(key, Checked { at: now, found });
            changed |= previous.is_none_or(|previous| {
                previous.found.web != web
                    || previous.found.icon.map(|icon| icon.key.clone()) != icon
            });
        }
        changed
    }

    pub fn cancel(&mut self) {
        self.checking = false;
    }

    /// Adds what the checks found to the snapshot. A page's own icon wins
    /// over the project's icon file.
    pub fn apply(&self, servers: &mut [Server]) {
        for server in servers {
            if let Some(checked) = self.checked.get(&server.id) {
                server.web = checked.found.web.clone();
                if let Some(icon) = &checked.found.icon {
                    server.icon = Some(icon.key.clone());
                }
            }
        }
    }
}

/// Checks every target at once.
pub fn check_all(targets: &[Target], icons: &Icons) -> Vec<(Key, Found)> {
    thread::scope(|scope| {
        let checks: Vec<_> = targets
            .iter()
            .map(|target| scope.spawn(move || (target.key.clone(), check(target.address, icons))))
            .collect();
        checks
            .into_iter()
            .filter_map(|check| check.join().ok())
            .collect()
    })
}

/// Asks for `/` over HTTPS, then over HTTP. HTTPS goes first because some
/// HTTPS servers, like nginx, answer a plain request with an HTTP error page.
/// Development servers use certificates the system doesn't trust, so the
/// check accepts any: it only learns the scheme and reads the page, and the
/// browser still warns about the certificate.
fn check(address: SocketAddr, icons: &Icons) -> Found {
    let mut timed_out = false;
    for scheme in [Scheme::Https, Scheme::Http] {
        let Some((client, url)) = client(address, scheme) else {
            continue;
        };
        let response = match client.get(url).send() {
            Ok(response) => response,
            Err(error) => {
                timed_out |= error.is_timeout() && !error.is_connect();
                continue;
            }
        };
        let page = response.url().clone();
        // The head comes first, so a long page still has its title.
        let head = is_html(&response)
            .then(|| read(response, MAX_HTML_BYTES))
            .flatten()
            .map(|html| read_head(&String::from_utf8_lossy(&html)))
            .unwrap_or_default();
        return Found {
            web: Web::Page {
                scheme,
                title: head.title,
            },
            icon: fetch_icon(&client, &page, head.icon.as_deref(), icons),
            timed_out: false,
        };
    }
    Found {
        web: Web::None,
        icon: None,
        timed_out,
    }
}

/// A client for one server: `localhost` reaches its exact address, so the
/// request carries the host name a browser sends and dev servers allow.
fn client(address: SocketAddr, scheme: Scheme) -> Option<(Client, Url)> {
    let scheme = scheme.as_str();
    let url = if address.ip().is_loopback() {
        format!("{scheme}://localhost:{}/", address.port())
    } else {
        format!("{scheme}://{address}/")
    };
    let client = Client::builder()
        .resolve("localhost", address)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT)
        .no_proxy()
        .user_agent("Ports")
        .tls_danger_accept_invalid_certs(true)
        .tls_danger_accept_invalid_hostnames(true)
        // Only within the server: a redirect elsewhere says nothing more about it.
        .redirect(redirect::Policy::custom(|attempt| {
            let first = &attempt.previous()[0];
            if attempt.previous().len() < 5 && same_origin(attempt.url(), first) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .ok()?;
    Some((client, Url::parse(&url).ok()?))
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.origin() == b.origin()
}

fn is_html(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| value.to_ascii_lowercase().contains("text/html"))
}

/// Up to `limit` bytes of the body.
fn read(response: Response, limit: u64) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    response.take(limit).read_to_end(&mut body).ok()?;
    Some(body)
}

/// The icon the page links to on the same server, else `/favicon.ico`:
/// Ports makes no request anywhere else.
fn fetch_icon(client: &Client, page: &Url, href: Option<&str>, icons: &Icons) -> Option<Arc<Icon>> {
    let url = href
        .and_then(|href| page.join(href).ok())
        .filter(|url| same_origin(url, page))
        .or_else(|| page.join("/favicon.ico").ok())?;
    let response = client.get(url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return None;
    }
    // A longer file isn't an icon, and a cut one wouldn't show.
    let bytes = read(response, icons::MAX_BYTES + 1)?;
    (!bytes.is_empty() && bytes.len() as u64 <= icons::MAX_BYTES)
        .then(|| icons.insert(&mime, bytes))
}

/// What Ports reads from a page's `<head>`.
#[derive(Debug, Default, PartialEq)]
struct Head {
    title: Option<String>,
    /// The `href` of its icon link.
    icon: Option<String>,
}

/// Reads the title and icon link of an HTML page. Tags are found by name,
/// case-insensitively, which is all a `<head>` needs.
fn read_head(html: &str) -> Head {
    // ASCII lowercasing keeps every byte offset, so positions carry over to `html`.
    let lower = html.to_ascii_lowercase();
    let end = lower.find("<body").unwrap_or(lower.len());
    let mut head = Head::default();
    let mut touch_icon = None;
    let mut at = 0;
    while let Some(start) = lower.get(at..end).and_then(|rest| rest.find('<')) {
        let start = at + start;
        let Some(close) = lower[start..].find('>').map(|i| start + i) else {
            break;
        };
        let tag = &lower[start + 1..close];
        let name = tag
            .split(|c: char| c.is_ascii_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
        match name {
            "title" if head.title.is_none() => {
                let text_end = lower[close..].find("</title").map_or(end, |i| close + i);
                let title = decode_entities(&html[close + 1..text_end.max(close + 1)]);
                let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
                head.title =
                    (!title.is_empty()).then(|| title.chars().take(MAX_TITLE_CHARS).collect());
            }
            "link" => {
                let attributes = attributes(&html[start + 1 + name.len()..close]);
                let rel = attributes.get("rel").map(|rel| rel.to_ascii_lowercase());
                let href = attributes
                    .get("href")
                    .filter(|href| !href.starts_with("data:"));
                if let (Some(rel), Some(href)) = (rel, href) {
                    let tokens: Vec<_> = rel.split_ascii_whitespace().collect();
                    if tokens.contains(&"icon") && head.icon.is_none() {
                        head.icon = Some(decode_entities(href));
                    } else if tokens.contains(&"apple-touch-icon") && touch_icon.is_none() {
                        touch_icon = Some(decode_entities(href));
                    }
                }
            }
            _ => {}
        }
        at = close + 1;
    }
    head.icon = head.icon.or(touch_icon);
    head
}

/// `name="value"`, `name='value'` and `name=value` pairs, names lowercased.
fn attributes(tag: &str) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let mut rest = tag.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (text, remaining) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &after[1..];
                    let end = inner.find(quote).unwrap_or(inner.len());
                    (&inner[..end], inner.get(end + 1..).unwrap_or_default())
                }
                _ => {
                    let end = after
                        .find(|c: char| c.is_ascii_whitespace())
                        .unwrap_or(after.len());
                    (&after[..end], &after[end..])
                }
            };
            value = text.to_string();
            rest = remaining;
        } else if name.is_empty() {
            // A stray `/` of a self-closing tag.
            rest = &rest[1.min(rest.len())..];
        }
        if !name.is_empty() {
            attributes.entry(name).or_insert(value);
        }
        rest = rest.trim_start();
    }
    attributes
}

/// The character references titles use; others stay as written.
fn decode_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        // References are short; looking further would make `&&&…` quadratic.
        let reference = rest
            .bytes()
            .take(12)
            .position(|byte| byte == b';')
            .map(|end| (&rest[1..end], end));
        let character = reference.and_then(|(name, _)| match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => {
                let number = name.strip_prefix('#')?;
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (character, reference) {
            (Some(character), Some((_, end))) => {
                decoded.push(character);
                rest = &rest[end + 1..];
            }
            _ => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    use super::*;

    #[test]
    fn reads_the_title_and_icon_of_a_page() {
        let html = r#"<!doctype html><HTML><head>
            <meta charset="utf-8">
            <link rel="apple-touch-icon" href="/touch.png">
            <link rel=stylesheet href=app.css>
            <LINK REL="Shortcut Icon" type='image/svg+xml' href='/vite.svg?v=1&amp;x=2' />
            <title>
              Billing &amp; Payments &#8212; Dev
            </title></head><body><title>Not this</title></body></html>"#;
        assert_eq!(
            read_head(html),
            Head {
                title: Some("Billing & Payments — Dev".into()),
                icon: Some("/vite.svg?v=1&x=2".into()),
            }
        );
        assert_eq!(
            read_head(
                r#"<link rel="icon" href="data:image/svg+xml,<svg>"><link rel="apple-touch-icon" href="t.png">"#
            ),
            Head {
                title: None,
                icon: Some("t.png".into()),
            }
        );
        assert_eq!(read_head("not html at all"), Head::default());
    }

    /// Answers `connections` requests with `answer`; the first line of each request goes to the returned channel.
    fn serve(
        listener: TcpListener,
        connections: usize,
        answer: impl Fn(&str) -> Vec<u8> + Send + 'static,
    ) -> thread::JoinHandle<Vec<String>> {
        thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..connections {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                stream.set_read_timeout(Some(TIMEOUT)).unwrap();
                let mut buffer = [0; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let line = request.lines().next().unwrap_or_default().to_string();
                let _ = stream.write_all(&answer(&line));
                requests.push(line);
            }
            requests
        })
    }

    fn http(status: &str, mime: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn an_http_server_is_a_page_with_its_title_and_icon() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        // The HTTPS attempt, the page, then its icon.
        let server = serve(listener, 3, |line| match line {
            "GET / HTTP/1.1" => http(
                "200 OK",
                "text/html; charset=utf-8",
                r#"<head><title>Storefront</title><link rel="icon" href="/favicon.svg"></head>"#,
            ),
            "GET /favicon.svg HTTP/1.1" => http("200 OK", "image/svg+xml", "<svg/>"),
            _ => http("400 Bad Request", "text/plain", ""),
        });
        let icons = Icons::default();
        let found = check(address, &icons);
        assert_eq!(
            found.web,
            Web::Page {
                scheme: Scheme::Http,
                title: Some("Storefront".into())
            }
        );
        assert_eq!(icons.respond(&found.icon.unwrap().key).body(), b"<svg/>");
        server.join().unwrap();
    }

    #[test]
    fn an_https_server_is_a_page_over_https() {
        let identity = native_tls::Identity::from_pkcs8(
            include_bytes!("../../tests/fixtures/localhost.crt"),
            include_bytes!("../../tests/fixtures/localhost.key"),
        )
        .unwrap();
        let acceptor = native_tls::TlsAcceptor::new(identity).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            // The page, then the icon it doesn't have.
            for status in ["200 OK", "404 Not Found"] {
                let (stream, _) = listener.accept().unwrap();
                let mut stream = acceptor.accept(stream).unwrap();
                let _ = stream.read(&mut [0; 4096]);
                let body = "<title>Secure</title>";
                let _ = stream.write_all(&http(status, "text/html", body));
            }
        });
        let found = check(address, &Icons::default());
        assert_eq!(
            found.web,
            Web::Page {
                scheme: Scheme::Https,
                title: Some("Secure".into())
            }
        );
        assert!(found.icon.is_none());
        server.join().unwrap();
    }

    // A custom TCP service in a Vite project, or a database: it speaks its own protocol.
    #[test]
    fn a_tcp_service_that_does_not_speak_http_is_not_a_page() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = serve(listener, 2, |_| b"+PONG\r\n".to_vec());
        let found = check(address, &Icons::default());
        assert_eq!(found.web, Web::None);
        assert!(found.icon.is_none());
        server.join().unwrap();
    }

    #[test]
    fn a_closed_port_is_not_a_page() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        // Reserve a port nothing listens on.
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let address = client.local_addr().unwrap();
        assert_eq!(check(address, &Icons::default()).web, Web::None);
    }

    fn target(key: &str, started_at: u64) -> Target {
        Target {
            key: key.into(),
            address: "127.0.0.1:1".parse().unwrap(),
            started_at,
        }
    }

    fn found(web: Web, timed_out: bool) -> Vec<(Key, Found)> {
        vec![(
            "web".into(),
            Found {
                web,
                icon: None,
                timed_out,
            },
        )]
    }

    /// Whether the server is due at `now`, as the only server in a fresh round.
    fn due_at(pages: &mut Pages, started_at: u64, now: u64) -> bool {
        pages.last_round_at = 0;
        let due = pages.due(vec![target("web", started_at)], now);
        pages.cancel();
        due.is_some()
    }

    #[test]
    fn checks_a_page_once_and_a_server_without_one_now_and_then() {
        let mut pages = Pages::default();
        let start = 1_000_000;
        assert!(pages.due(vec![target("web", start)], start).is_some());
        // One round at a time.
        assert!(
            pages
                .due(vec![target("web", start)], start + CHECK_EVERY_MS)
                .is_none()
        );
        // A server still starting gets another look soon.
        assert!(pages.finish(found(Web::None, false), start));
        assert!(!due_at(&mut pages, start, start + STARTING_RECHECK_MS - 1));
        assert!(due_at(&mut pages, start, start + STARTING_RECHECK_MS));
        // Later, one that timed out sooner than one that answered something else.
        let later = start + STARTING_MS;
        pages.finish(found(Web::None, true), later);
        assert!(!due_at(&mut pages, start, later + STARTING_RECHECK_MS));
        assert!(due_at(&mut pages, start, later + TIMED_OUT_RECHECK_MS));
        assert!(!pages.finish(found(Web::None, false), later));
        assert!(!due_at(&mut pages, start, later + TIMED_OUT_RECHECK_MS));
        assert!(due_at(&mut pages, start, later + NO_PAGE_RECHECK_MS));
        // A page isn't asked for again while the same server runs.
        let page = Web::Page {
            scheme: Scheme::Http,
            title: None,
        };
        assert!(pages.finish(found(page, false), later));
        assert!(!due_at(&mut pages, start, later + 100 * NO_PAGE_RECHECK_MS));
        // A server that is gone is forgotten.
        assert!(
            pages
                .due(Vec::new(), later + 101 * NO_PAGE_RECHECK_MS)
                .is_none()
        );
        assert!(pages.checked.is_empty());
    }

    #[test]
    fn leaves_databases_alone_even_in_a_container() {
        let mut redis = crate::ports::alerts::tests::server(6379, 0);
        redis.kind = Kind::Container;
        redis.pid = None;
        redis.container = Some(super::super::sampler::ContainerRef {
            id: "c".into(),
            image: "redis".into(),
        });
        assert!(super::target(&redis).is_some());
        redis.framework = Some("Redis");
        assert!(super::target(&redis).is_none());
    }
}
