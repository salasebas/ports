//! TCP sockets on this machine, grouped by local port.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use listeners::{Protocol, SocketState};

#[derive(Debug, Default)]
pub struct Sockets {
    /// PIDs listening on each port; the IPv4 and IPv6 sockets of a port share one entry.
    pub listening: BTreeMap<u16, BTreeSet<u32>>,
    /// Listening ports whose process this user can't see, with the owner's uid when known.
    pub unowned: BTreeMap<u16, Option<u32>>,
    /// Accepted connections, as (local port, owning PID).
    established: HashSet<(u16, u32)>,
}

impl Sockets {
    /// `listeners` maps sockets to processes through their file descriptors,
    /// which only works for this user's processes. Listeners of other users
    /// (root, `postgres`) come from tables any user can read instead.
    pub fn scan() -> Self {
        let mut sockets = match listeners::get_all() {
            Ok(all) => Self::from_entries(
                all.into_iter()
                    .filter(|l| l.protocol == Protocol::TCP)
                    .map(|l| (l.socket.port(), l.process.pid, l.state)),
            ),
            Err(error) => {
                eprintln!("could not list sockets: {error}");
                Self::default()
            }
        };
        #[cfg(target_os = "macos")]
        {
            // The kernel keeps the last process that used each socket. One
            // this user may signal, or one that's gone, is stale: `listeners`
            // already found every socket this user's processes hold, so the
            // holder now is a process of another user, e.g. after a fork.
            let (current, stale): (Vec<_>, Vec<_>) = pcblist::listening()
                .into_iter()
                .filter(|(port, _)| !sockets.listening.contains_key(port))
                .partition(|(_, pid)| Self::belongs_to_another_user(*pid));
            for (port, pid) in current {
                sockets.listening.entry(port).or_default().insert(pid);
            }
            for (port, _) in stale {
                if !sockets.listening.contains_key(&port) {
                    sockets.unowned.insert(port, None);
                }
            }
        }
        #[cfg(target_os = "linux")]
        for (port, uid) in proc_net::listening() {
            if !sockets.listening.contains_key(&port) {
                sockets.unowned.insert(port, Some(uid));
            }
        }
        sockets
    }

    #[cfg(target_os = "macos")]
    fn belongs_to_another_user(pid: u32) -> bool {
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        !alive && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn from_entries(entries: impl IntoIterator<Item = (u16, u32, SocketState)>) -> Self {
        let mut sockets = Self::default();
        for (port, pid, state) in entries {
            match state {
                SocketState::Listen if port != 0 => {
                    sockets.listening.entry(port).or_default().insert(pid);
                }
                SocketState::Established => {
                    sockets.established.insert((port, pid));
                }
                _ => {}
            }
        }
        sockets
    }

    /// Whether any of `pids` holds a connection accepted on `port`.
    pub fn has_connections(&self, port: u16, pids: &BTreeSet<u32>) -> bool {
        pids.iter()
            .any(|pid| self.established.contains(&(port, *pid)))
    }
}

/// The kernel's TCP table, as `netstat -anv` reads it. It includes the PID
/// that last used each socket, for every user. Layouts come from xnu's
/// `bsd/netinet/in_pcb.h`, `bsd/sys/socketvar.h` and `bsd/netinet/tcp_var.h`,
/// all declared under `#pragma pack(4)`.
#[cfg(target_os = "macos")]
mod pcblist {
    const XINPGEN_LEN: usize = 24;
    const XSO_SOCKET: u32 = 0x001;
    const XSO_INPCB: u32 = 0x010;
    const XSO_TCPCB: u32 = 0x020;
    /// `xinpcb_n.inp_lport`, in network byte order.
    const INPCB_LPORT: usize = 18;
    /// `xsocket_n.so_last_pid`.
    const SOCKET_LAST_PID: usize = 68;
    /// `xtcpcb_n.t_state`.
    const TCPCB_STATE: usize = 36;
    const TCPS_LISTEN: u32 = 1;

    /// (port, PID) of every listening TCP socket, IPv4 and IPv6.
    pub fn listening() -> Vec<(u16, u32)> {
        read().map(|buffer| parse(&buffer)).unwrap_or_default()
    }

    fn read() -> Option<Vec<u8>> {
        let name = c"net.inet.tcp.pcblist_n";
        let mut len = 0;
        let sized = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if sized != 0 {
            return None;
        }
        // Room for sockets opened between the two calls.
        len += len / 4;
        let mut buffer = vec![0u8; len];
        let read = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return None;
        }
        buffer.truncate(len);
        Some(buffer)
    }

    fn u32_at(buffer: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_ne_bytes(
            buffer.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }

    fn round_up_8(len: usize) -> usize {
        len.div_ceil(8) * 8
    }

    /// A header, then for each socket an inpcb record, a socket record, buffer
    /// and stats records and a tcpcb record, each padded to 8 bytes, then a trailer.
    fn parse(buffer: &[u8]) -> Vec<(u16, u32)> {
        let mut found = Vec::new();
        let Some(header) = u32_at(buffer, 0) else {
            return found;
        };
        let mut offset = round_up_8(header as usize);
        let (mut port, mut pid) = (None, None);
        while let (Some(len), Some(kind)) = (u32_at(buffer, offset), u32_at(buffer, offset + 4)) {
            let len = len as usize;
            if len <= XINPGEN_LEN {
                break;
            }
            match kind {
                XSO_INPCB => {
                    port = buffer
                        .get(offset + INPCB_LPORT..offset + INPCB_LPORT + 2)
                        .map(|b| u16::from_be_bytes([b[0], b[1]]));
                    pid = None;
                }
                XSO_SOCKET => pid = u32_at(buffer, offset + SOCKET_LAST_PID),
                XSO_TCPCB => {
                    if let (Some(port), Some(pid), Some(TCPS_LISTEN)) =
                        (port, pid, u32_at(buffer, offset + TCPCB_STATE))
                        && port != 0
                        && pid != 0
                    {
                        found.push((port, pid));
                    }
                }
                _ => {}
            }
            offset += round_up_8(len);
        }
        found
    }

    #[cfg(test)]
    mod tests {
        use std::net::TcpListener;

        #[test]
        fn finds_a_listener_with_its_pid() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            assert!(super::listening().contains(&(port, std::process::id())));
        }
    }
}

/// `/proc/net/tcp{,6}`, readable by every user, with each socket's owner uid.
#[cfg(any(target_os = "linux", test))]
mod proc_net {
    const LISTEN: &str = "0A";

    #[cfg(target_os = "linux")]
    pub fn listening() -> Vec<(u16, u32)> {
        ["/proc/net/tcp", "/proc/net/tcp6"]
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .flat_map(|table| parse(&table))
            .collect()
    }

    /// (port, uid) of the listening sockets in one table.
    pub fn parse(table: &str) -> Vec<(u16, u32)> {
        table
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.get(3) != Some(&LISTEN) {
                    return None;
                }
                let (_, port) = fields.get(1)?.rsplit_once(':')?;
                Some((
                    u16::from_str_radix(port, 16).ok()?,
                    fields.get(7)?.parse().ok()?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_ipv4_and_ipv6_listeners_of_a_port() {
        let sockets = Sockets::from_entries([
            (5173, 10, SocketState::Listen),
            (5173, 10, SocketState::Listen),
            (5432, 20, SocketState::Listen),
        ]);
        assert_eq!(
            sockets.listening,
            BTreeMap::from([(5173, BTreeSet::from([10])), (5432, BTreeSet::from([20]))])
        );
    }

    #[test]
    fn counts_only_connections_accepted_by_the_server() {
        let sockets = Sockets::from_entries([
            (3000, 10, SocketState::Listen),
            (3000, 10, SocketState::Established),
            // The browser's side of the same connection.
            (61000, 99, SocketState::Established),
        ]);
        assert!(sockets.has_connections(3000, &BTreeSet::from([10])));
        assert!(!sockets.has_connections(3000, &BTreeSet::from([99])));
    }

    #[test]
    fn reads_listeners_of_every_user_from_proc_net_tcp() {
        // Postgres (uid 113) and sshd (root) listening, plus an established connection.
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1538 00000000:0000 0A 00000000:00000000 00:00000000 00000000   113        0 23154 1 0000000000000000 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 19876 1 0000000000000000 100 0 0 10 0
   2: 0100007F:1538 0100007F:A2C4 01 00000000:00000000 00:00000000 00000000   113        0 99999 1 0000000000000000 20 4 30 10 -1
";
        assert_eq!(proc_net::parse(table), vec![(5432, 113), (22, 0)]);
    }
}
