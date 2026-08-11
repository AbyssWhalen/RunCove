//! Scanner parsing, filtering, and live Windows socket coverage.

#[cfg(windows)]
mod live_scan_tests {
    use runcove::model::{ConnectionState, Protocol};
    use std::net::{TcpListener, UdpSocket};
    use std::time::{Duration, Instant};

    #[test]
    fn maps_real_tcp_and_udp_listeners_to_the_current_pid() {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp.local_addr().unwrap().port();
        let udp_port = udp.local_addr().unwrap().port();
        let pid = std::process::id();
        let deadline = Instant::now() + Duration::from_secs(2);

        loop {
            let entries = runcove::scanner::create_scanner().scan().unwrap();
            let tcp_found = entries.iter().any(|entry| {
                entry.port == tcp_port
                    && entry.protocol == Protocol::TCP
                    && entry.state == ConnectionState::Listen
                    && entry.pid == Some(pid)
            });
            let udp_found = entries.iter().any(|entry| {
                entry.port == udp_port && entry.protocol == Protocol::UDP && entry.pid == Some(pid)
            });
            if tcp_found && udp_found {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "live scanner did not map TCP {tcp_port} and UDP {udp_port} to PID {pid}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(test)]
mod model_tests {
    use runcove::model::*;
    use std::net::IpAddr;

    #[test]
    fn test_parse_linux_addr_port_loopback() {
        let (ip, port) = parse_linux_addr_port("0100007F:0050").unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        assert_eq!(port, 80);
    }

    #[test]
    fn test_parse_linux_addr_port_any() {
        let (ip, port) = parse_linux_addr_port("00000000:1F90").unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        assert_eq!(port, 8080);
    }

    #[test]
    fn test_parse_linux_addr_port_specific() {
        // 192.168.1.100 = 0x6401A8C0 in little-endian
        let (ip, port) = parse_linux_addr_port("6401A8C0:01BB").unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 100)));
        assert_eq!(port, 443);
    }

    #[test]
    fn test_parse_linux_addr_port_v6_loopback() {
        // ::1 in /proc/net/tcp6 format
        let (ip, port) = parse_linux_addr_port_v6("00000000000000000000000001000000:0050").unwrap();
        assert_eq!(port, 80);
        // Should be IPv6 loopback or mapped
        match ip {
            IpAddr::V6(v6) => assert_eq!(v6, std::net::Ipv6Addr::LOCALHOST),
            IpAddr::V4(v4) => assert_eq!(v4, std::net::Ipv4Addr::new(1, 0, 0, 0)), // mapped form
        }
    }

    #[test]
    fn test_parse_linux_addr_port_v6_any() {
        let (ip, port) = parse_linux_addr_port_v6("00000000000000000000000000000000:0CEA").unwrap();
        assert_eq!(port, 3306);
        assert!(is_public_bind(&ip));
    }

    #[test]
    fn test_is_public_bind_v4() {
        assert!(is_public_bind(&IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)));
        assert!(!is_public_bind(&IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
        assert!(!is_public_bind(&"10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_is_public_bind_v6() {
        assert!(is_public_bind(&IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)));
        assert!(!is_public_bind(&IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_connection_state_display() {
        assert_eq!(format!("{}", ConnectionState::Listen), "LISTEN");
        assert_eq!(format!("{}", ConnectionState::Established), "ESTABLISHED");
        assert_eq!(format!("{}", ConnectionState::TimeWait), "TIME_WAIT");
    }

    #[test]
    fn test_protocol_display() {
        assert_eq!(format!("{}", Protocol::TCP), "TCP");
        assert_eq!(format!("{}", Protocol::UDP), "UDP");
    }

    #[test]
    fn test_well_known_ports() {
        assert_eq!(well_known_service(22), Some("ssh"));
        assert_eq!(well_known_service(80), Some("http"));
        assert_eq!(well_known_service(443), Some("https"));
        assert_eq!(well_known_service(3000), None);
        assert_eq!(well_known_service(8080), Some("http-alt"));
    }

    #[test]
    fn test_connection_state_windows_mapping() {
        assert_eq!(
            ConnectionState::from_windows_state(2),
            ConnectionState::Listen
        );
        assert_eq!(
            ConnectionState::from_windows_state(5),
            ConnectionState::Established
        );
        assert_eq!(
            ConnectionState::from_windows_state(11),
            ConnectionState::TimeWait
        );
        assert_eq!(
            ConnectionState::from_windows_state(99),
            ConnectionState::Unknown
        );
    }
}

#[cfg(test)]
mod filter_tests {
    use runcove::model::*;
    use std::net::IpAddr;

    fn make_entry(port: u16, state: ConnectionState, process: Option<&str>) -> PortEntry {
        PortEntry {
            port,
            protocol: Protocol::TCP,
            state,
            pid: Some(1234),
            process_name: process.map(|s| s.to_string()),
            bind_address: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            is_public: false,
        }
    }

    #[test]
    fn test_filter_by_state() {
        let entries = [
            make_entry(80, ConnectionState::Listen, Some("nginx")),
            make_entry(8080, ConnectionState::Established, Some("curl")),
            make_entry(443, ConnectionState::TimeWait, Some("nginx")),
        ];

        let listen_only: Vec<_> = entries
            .iter()
            .filter(|e| e.state == ConnectionState::Listen)
            .collect();

        assert_eq!(listen_only.len(), 1);
        assert_eq!(listen_only[0].port, 80);
    }

    #[test]
    fn test_filter_by_process_name() {
        let entries = [
            make_entry(80, ConnectionState::Listen, Some("nginx")),
            make_entry(3000, ConnectionState::Listen, Some("node")),
            make_entry(5432, ConnectionState::Listen, Some("postgres")),
        ];

        let node_entries: Vec<_> = entries
            .iter()
            .filter(|e| {
                e.process_name
                    .as_ref()
                    .map(|n| n.to_lowercase().contains("node"))
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(node_entries.len(), 1);
        assert_eq!(node_entries[0].port, 3000);
    }

    #[test]
    fn test_filter_by_port() {
        let entries = [
            make_entry(80, ConnectionState::Listen, Some("nginx")),
            make_entry(443, ConnectionState::Listen, Some("nginx")),
            make_entry(8080, ConnectionState::Listen, Some("node")),
        ];

        let port_443: Vec<_> = entries.iter().filter(|e| e.port == 443).collect();
        assert_eq!(port_443.len(), 1);
    }

    #[test]
    fn test_filter_by_range() {
        let entries = [
            make_entry(80, ConnectionState::Listen, Some("nginx")),
            make_entry(3000, ConnectionState::Listen, Some("node")),
            make_entry(3001, ConnectionState::Listen, Some("node")),
            make_entry(8080, ConnectionState::Listen, Some("proxy")),
        ];

        let in_range: Vec<_> = entries
            .iter()
            .filter(|e| e.port >= 3000 && e.port <= 4000)
            .collect();

        assert_eq!(in_range.len(), 2);
    }
}
