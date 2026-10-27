//! TCP sockets on this machine, grouped by local port.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::net::{IpAddr, SocketAddr};

use listeners::{Protocol, SocketState};

#[derive(Debug, Default)]
pub struct Sockets {
    /// PIDs listening on each port; the IPv4 and IPv6 sockets of a port share one entry.
    pub listening: BTreeMap<u16, BTreeSet<u32>>,
    /// Listening ports whose process this user can't see, with the owner's uid when known.
    pub unowned: BTreeMap<u16, Option<u32>>,
    /// Addresses each listening port is bound to, e.g. `127.0.0.1` or `0.0.0.0`.
    pub addresses: BTreeMap<u16, BTreeSet<IpAddr>>,
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
                    .map(|l| (l.socket, l.process.pid, l.state)),
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
                .filter(|(socket, _)| !sockets.listening.contains_key(&socket.port()))
                .partition(|(_, pid)| Self::belongs_to_another_user(*pid));
            for (socket, _) in current.iter().chain(&stale) {
                sockets.bound(*socket);
            }
            for (socket, pid) in current {
                sockets
                    .listening
                    .entry(socket.port())
                    .or_default()
                    .insert(pid);
            }
            for (socket, _) in stale {
                if !sockets.listening.contains_key(&socket.port()) {
                    sockets.unowned.insert(socket.port(), None);
                }
            }
        }
        #[cfg(target_os = "linux")]
        for (socket, uid) in proc_net::listening() {
            if !sockets.listening.contains_key(&socket.port()) {
                sockets.bound(socket);
                sockets.unowned.insert(socket.port(), Some(uid));
            }
        }
        sockets
    }

    #[cfg(target_os = "macos")]
    fn belongs_to_another_user(pid: u32) -> bool {
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        !alive && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn from_entries(entries: impl IntoIterator<Item = (SocketAddr, u32, SocketState)>) -> Self {
        let mut sockets = Self::default();
        for (socket, pid, state) in entries {
            match state {
                SocketState::Listen if socket.port() != 0 => {
                    sockets
                        .listening
                        .entry(socket.port())
                        .or_default()
                        .insert(pid);
                    sockets.bound(socket);
                }
                SocketState::Established => {
                    sockets.established.insert((socket.port(), pid));
                }
                _ => {}
            }
        }
        sockets
    }

    fn bound(&mut self, socket: SocketAddr) {
        self.addresses
            .entry(socket.port())
            .or_default()
            // Dual-stack sockets report IPv4 addresses as `::ffff:127.0.0.1`.
            .insert(socket.ip().to_canonical());
    }

    /// The addresses `port` is bound to, wildcards (`0.0.0.0`, `::`) included.
    pub fn addresses(&self, port: u16) -> Vec<IpAddr> {
        self.addresses
            .get(&port)
            .map(|addresses| addresses.iter().copied().collect())
            .unwrap_or_default()
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
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    const XINPGEN_LEN: usize = 24;
    const XSO_SOCKET: u32 = 0x001;
    const XSO_INPCB: u32 = 0x010;
    const XSO_TCPCB: u32 = 0x020;
    /// `xinpcb_n.inp_lport`, in network byte order.
    const INPCB_LPORT: usize = 18;
    /// `xinpcb_n.inp_vflag`, with `INP_IPV4` and `INP_IPV6`.
    const INPCB_VFLAG: usize = 44;
    const INP_IPV6: u8 = 0x2;
    /// `xinpcb_n.inp_dependladdr`: an `in6_addr`, or an `in_addr_4in6` whose last four bytes are the IPv4 address.
    const INPCB_LADDR: usize = 64;
    /// `xsocket_n.so_last_pid`.
    const SOCKET_LAST_PID: usize = 68;
    /// `xtcpcb_n.t_state`.
    const TCPCB_STATE: usize = 36;
    const TCPS_LISTEN: u32 = 1;

    /// Local address and PID of every listening TCP socket, IPv4 and IPv6.
    pub fn listening() -> Vec<(SocketAddr, u32)> {
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
    fn local_ip(record: &[u8]) -> Option<IpAddr> {
        let address: [u8; 16] = record.get(INPCB_LADDR..INPCB_LADDR + 16)?.try_into().ok()?;
        Some(if record.get(INPCB_VFLAG)? & INP_IPV6 != 0 {
            IpAddr::V6(Ipv6Addr::from(address))
        } else {
            IpAddr::V4(Ipv4Addr::new(
                address[12],
                address[13],
                address[14],
                address[15],
            ))
        })
    }

    fn parse(buffer: &[u8]) -> Vec<(SocketAddr, u32)> {
        let mut found = Vec::new();
        let Some(header) = u32_at(buffer, 0) else {
            return found;
        };
        let mut offset = round_up_8(header as usize);
        let (mut socket, mut pid) = (None, None);
        while let (Some(len), Some(kind)) = (u32_at(buffer, offset), u32_at(buffer, offset + 4)) {
            let len = len as usize;
            if len <= XINPGEN_LEN {
                break;
            }
            match kind {
                XSO_INPCB => {
                    let record = buffer.get(offset..offset + len).unwrap_or_default();
                    let port = record
                        .get(INPCB_LPORT..INPCB_LPORT + 2)
                        .map(|b| u16::from_be_bytes([b[0], b[1]]));
                    socket = port
                        .zip(local_ip(record))
                        .map(|(port, ip)| SocketAddr::new(ip, port));
                    pid = None;
                }
                XSO_SOCKET => pid = u32_at(buffer, offset + SOCKET_LAST_PID),
                XSO_TCPCB => {
                    if let (Some(socket), Some(pid), Some(TCPS_LISTEN)) =
                        (socket, pid, u32_at(buffer, offset + TCPCB_STATE))
                        && socket.port() != 0
                        && pid != 0
                    {
                        found.push((socket, pid));
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
        fn finds_a_listener_with_its_address_and_pid() {
            for address in ["127.0.0.1:0", "0.0.0.0:0", "[::1]:0", "[::]:0"] {
                let listener = TcpListener::bind(address).unwrap();
                let socket = listener.local_addr().unwrap();
                assert!(
                    super::listening().contains(&(socket, std::process::id())),
                    "{socket} is missing"
                );
            }
        }
    }
}

/// `/proc/net/tcp{,6}`, readable by every user, with each socket's owner uid.
#[cfg(any(target_os = "linux", test))]
mod proc_net {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    const LISTEN: &str = "0A";

    #[cfg(target_os = "linux")]
    pub fn listening() -> Vec<(SocketAddr, u32)> {
        ["/proc/net/tcp", "/proc/net/tcp6"]
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .flat_map(|table| parse(&table))
            .collect()
    }

    /// The kernel prints each 32-bit word of the address in host byte order.
    fn address(hex: &str) -> Option<IpAddr> {
        let words = (0..hex.len() / 8)
            .map(|i| u32::from_str_radix(hex.get(i * 8..i * 8 + 8)?, 16).ok())
            .collect::<Option<Vec<u32>>>()?;
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_ne_bytes()).collect();
        match bytes.len() {
            4 => Some(IpAddr::V4(Ipv4Addr::new(
                bytes[0], bytes[1], bytes[2], bytes[3],
            ))),
            16 => Some(IpAddr::V6(Ipv6Addr::from(
                <[u8; 16]>::try_from(bytes).ok()?,
            ))),
            _ => None,
        }
    }

    /// (local address, uid) of the listening sockets in one table.
    pub fn parse(table: &str) -> Vec<(SocketAddr, u32)> {
        table
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.get(3) != Some(&LISTEN) {
                    return None;
                }
                let (ip, port) = fields.get(1)?.rsplit_once(':')?;
                let port = u16::from_str_radix(port, 16).ok()?;
                Some((
                    SocketAddr::new(address(ip)?, port),
                    fields.get(7)?.parse().ok()?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(address: &str) -> SocketAddr {
        address.parse().unwrap()
    }

    #[test]
    fn merges_ipv4_and_ipv6_listeners_of_a_port() {
        let sockets = Sockets::from_entries([
            (at("127.0.0.1:5173"), 10, SocketState::Listen),
            (at("[::1]:5173"), 10, SocketState::Listen),
            (at("0.0.0.0:5432"), 20, SocketState::Listen),
        ]);
        assert_eq!(
            sockets.listening,
            BTreeMap::from([(5173, BTreeSet::from([10])), (5432, BTreeSet::from([20]))])
        );
        assert_eq!(
            sockets.addresses(5173),
            vec![at("127.0.0.1:0").ip(), at("[::1]:0").ip()]
        );
    }

    // A JVM server bound to 127.0.0.1 shows up in Linux's IPv6 table.
    #[test]
    fn reads_ipv4_mapped_addresses_as_ipv4() {
        let sockets =
            Sockets::from_entries([(at("[::ffff:127.0.0.1]:8080"), 10, SocketState::Listen)]);
        assert_eq!(sockets.addresses(8080), vec![at("127.0.0.1:0").ip()]);
    }

    #[test]
    fn counts_only_connections_accepted_by_the_server() {
        let sockets = Sockets::from_entries([
            (at("127.0.0.1:3000"), 10, SocketState::Listen),
            (at("127.0.0.1:3000"), 10, SocketState::Established),
            // The browser's side of the same connection.
            (at("127.0.0.1:61000"), 99, SocketState::Established),
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
        assert_eq!(
            proc_net::parse(table),
            vec![(at("127.0.0.1:5432"), 113), (at("0.0.0.0:22"), 0)]
        );
    }

    #[test]
    fn reads_ipv6_addresses_from_proc_net_tcp6() {
        // `::` and `::1` as an x86 kernel prints them.
        let table = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 1 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000001000000:0BB8 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 2 1 0000000000000000 100 0 0 10 0
";
        // The words are in host byte order, so this reads right on little-endian machines only.
        if cfg!(target_endian = "little") {
            assert_eq!(
                proc_net::parse(table),
                vec![(at("[::]:8080"), 1000), (at("[::1]:3000"), 1000)]
            );
        }
    }
}
