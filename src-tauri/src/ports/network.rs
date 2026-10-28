//! Whether other devices can reach a server, and the URL they open it at.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

/// Interfaces of virtual machines and containers: on this machine, not on the network.
const VIRTUAL: [&str; 7] = [
    "bridge", "docker", "br-", "veth", "vmnet", "vboxnet", "virbr",
];

/// An IPv4 address on the local network, as another device on the same
/// Wi-Fi or Ethernet reaches this machine; `None` while offline.
pub fn lan_ip() -> Option<IpAddr> {
    let interfaces: Vec<Candidate> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| {
            interface.is_oper_up() && !interface.is_loopback() && !interface.is_link_local()
        })
        .map(|interface| Candidate {
            ip: interface.ip(),
            // VPN tunnels are point-to-point.
            physical: !interface.is_p2p()
                && !VIRTUAL
                    .iter()
                    .any(|prefix| interface.name.starts_with(prefix)),
        })
        .collect();
    pick(route_ip(), &interfaces)
}

struct Candidate {
    ip: IpAddr,
    physical: bool,
}

/// The address of the interface traffic to the internet leaves from.
/// Connecting a UDP socket sends nothing, it only picks the route.
fn route_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // TEST-NET-1 (RFC 5737): routed like any internet address, never reached.
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// The default route's address when it's a private one on a physical
/// interface; otherwise the first such address, since a full-tunnel VPN
/// moves the default route onto the tunnel while the Wi-Fi address still works.
fn pick(route: Option<IpAddr>, interfaces: &[Candidate]) -> Option<IpAddr> {
    let lan: Vec<IpAddr> = interfaces
        .iter()
        .filter(|candidate| {
            candidate.physical && matches!(candidate.ip, IpAddr::V4(ip) if ip.is_private())
        })
        .map(|candidate| candidate.ip)
        .collect();
    route
        .filter(|ip| lan.contains(ip))
        .or_else(|| lan.first().copied())
        .or(route)
}

/// Bound to a wildcard (`0.0.0.0`, `::`) or to a network interface, so
/// anyone on the same network can connect, not only this machine.
pub fn is_exposed(addresses: &[IpAddr]) -> bool {
    addresses.iter().any(|ip| !ip.is_loopback())
}

/// The `host:port` another device reaches the server at, when it can. The
/// windows add the scheme the page check found, so Open, Copy and the QR
/// code agree. A `::` socket answers IPv4 too (Node's `listen(port)`) unless `v6only`.
pub fn lan_host(
    port: u16,
    addresses: &[IpAddr],
    lan_ip: Option<IpAddr>,
    v6only: bool,
) -> Option<String> {
    let ip = lan_ip
        .filter(|lan| {
            addresses.iter().any(|bound| {
                bound.is_unspecified()
                    && (bound.is_ipv4() == lan.is_ipv4() || bound.is_ipv6() && !v6only)
            })
        })
        .or_else(|| {
            addresses
                .iter()
                .copied()
                .find(|ip| !ip.is_loopback() && !ip.is_unspecified())
        })?;
    Some(SocketAddr::new(ip, port).to_string())
}

/// Where Ports connects to reach the server itself: loopback for a wildcard.
pub fn local_address(port: u16, addresses: &[IpAddr]) -> SocketAddr {
    let ip = match addresses.first() {
        Some(IpAddr::V6(ip)) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        Some(ip) if !ip.is_unspecified() => *ip,
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    SocketAddr::new(ip, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(list: &[&str]) -> Vec<IpAddr> {
        list.iter().map(|ip| ip.parse().unwrap()).collect()
    }

    const LAN: Option<IpAddr> = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)));

    // `vite` listens on localhost only; `vite --host` on every interface.
    #[test]
    fn loopback_stays_on_this_machine() {
        let vite = ips(&["127.0.0.1", "::1"]);
        assert!(!is_exposed(&vite));
        assert_eq!(lan_host(5173, &vite, LAN, false), None);
    }

    #[test]
    fn a_wildcard_is_reachable_at_the_lan_address() {
        for wildcard in [ips(&["0.0.0.0"]), ips(&["::"]), ips(&["0.0.0.0", "::"])] {
            assert!(is_exposed(&wildcard));
            assert_eq!(
                lan_host(5173, &wildcard, LAN, false).as_deref(),
                Some("192.168.1.20:5173")
            );
        }
        assert_eq!(lan_host(5173, &ips(&["0.0.0.0"]), None, false), None);
        assert_eq!(
            lan_host(
                5173,
                &ips(&["0.0.0.0"]),
                Some("fd00::5".parse().unwrap()),
                false
            ),
            None
        );
        // nginx's `listen [::]` is IPv6-only: the IPv4 LAN address can't reach it.
        assert_eq!(lan_host(5173, &ips(&["::"]), LAN, true), None);
        assert_eq!(
            lan_host(5173, &ips(&["0.0.0.0", "::"]), LAN, true).as_deref(),
            Some("192.168.1.20:5173")
        );
        assert_eq!(
            lan_host(5173, &ips(&["::"]), Some("fd00::5".parse().unwrap()), true).as_deref(),
            Some("[fd00::5]:5173")
        );
    }

    #[test]
    fn an_interface_address_is_used_as_it_is() {
        let bound = ips(&["10.0.0.5"]);
        assert!(is_exposed(&bound));
        assert_eq!(
            lan_host(3000, &bound, LAN, false).as_deref(),
            Some("10.0.0.5:3000")
        );
        assert_eq!(
            lan_host(3000, &ips(&["fd00::5"]), LAN, false).as_deref(),
            Some("[fd00::5]:3000")
        );
    }

    fn candidate(ip: &str, physical: bool) -> Candidate {
        Candidate {
            ip: ip.parse().unwrap(),
            physical,
        }
    }

    #[test]
    fn picks_the_wifi_address_over_a_vpn_tunnel() {
        let wifi = candidate("192.168.1.20", true);
        let vpn = candidate("10.8.0.2", false);
        let docker = candidate("172.17.0.1", false);
        // With a full-tunnel VPN the default route leaves from the tunnel.
        assert_eq!(pick(Some(vpn.ip), &[vpn, wifi, docker]), LAN);
        // Without one it leaves from the Wi-Fi, which wins over Ethernet listed first.
        let ethernet = candidate("10.0.0.7", true);
        let wifi = candidate("192.168.1.20", true);
        assert_eq!(pick(LAN, &[ethernet, wifi]), LAN);
        // A public address on the route is still better than nothing.
        let public = candidate("203.0.113.5", true);
        assert_eq!(pick(Some(public.ip), &[public]), Some(public_ip()));
        assert_eq!(pick(None, &[]), None);
    }

    fn public_ip() -> IpAddr {
        "203.0.113.5".parse().unwrap()
    }

    #[test]
    fn connects_to_loopback_for_a_wildcard() {
        assert_eq!(
            local_address(80, &ips(&["0.0.0.0"])),
            "127.0.0.1:80".parse().unwrap()
        );
        assert_eq!(
            local_address(80, &ips(&["::"])),
            "[::1]:80".parse().unwrap()
        );
        assert_eq!(
            local_address(80, &ips(&["::1"])),
            "[::1]:80".parse().unwrap()
        );
        assert_eq!(local_address(80, &[]), "127.0.0.1:80".parse().unwrap());
    }
}
