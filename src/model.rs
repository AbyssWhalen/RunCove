use serde::Serialize;
use std::net::IpAddr;

/// Core data structure representing a port in use.
#[derive(Debug, Clone, Serialize)]
pub struct PortEntry {
    pub port: u16,
    pub protocol: Protocol,
    pub state: ConnectionState,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub bind_address: IpAddr,
    pub is_public: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Protocol {
    TCP,
    UDP,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::TCP => write!(f, "TCP"),
            Protocol::UDP => write!(f, "UDP"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ConnectionState {
    Listen,
    Established,
    TimeWait,
    CloseWait,
    SynSent,
    SynRecv,
    FinWait1,
    FinWait2,
    Closing,
    LastAck,
    Unknown,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConnectionState::Listen => "LISTEN",
            ConnectionState::Established => "ESTABLISHED",
            ConnectionState::TimeWait => "TIME_WAIT",
            ConnectionState::CloseWait => "CLOSE_WAIT",
            ConnectionState::SynSent => "SYN_SENT",
            ConnectionState::SynRecv => "SYN_RECV",
            ConnectionState::FinWait1 => "FIN_WAIT1",
            ConnectionState::FinWait2 => "FIN_WAIT2",
            ConnectionState::Closing => "CLOSING",
            ConnectionState::LastAck => "LAST_ACK",
            ConnectionState::Unknown => "UNKNOWN",
        };
        write!(f, "{}", s)
    }
}

impl ConnectionState {
    /// Parse Linux /proc/net/tcp state hex code.
    /// See: <https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/include/net/tcp_states.h>
    pub fn from_linux_hex(hex: &str) -> Self {
        match hex.to_uppercase().as_str() {
            "0A" => ConnectionState::Listen,
            "01" => ConnectionState::Established,
            "06" => ConnectionState::TimeWait,
            "08" => ConnectionState::CloseWait,
            "02" => ConnectionState::SynSent,
            "03" => ConnectionState::SynRecv,
            "04" => ConnectionState::FinWait1,
            "05" => ConnectionState::FinWait2,
            "0B" => ConnectionState::Closing,
            "09" => ConnectionState::LastAck,
            _ => ConnectionState::Unknown,
        }
    }

    /// Parse Windows TCP state (MIB_TCP_STATE constants).
    pub fn from_windows_state(state: u32) -> Self {
        match state {
            1 => ConnectionState::Unknown, // CLOSED
            2 => ConnectionState::Listen,
            3 => ConnectionState::SynSent,
            4 => ConnectionState::SynRecv,
            5 => ConnectionState::Established,
            6 => ConnectionState::FinWait1,
            7 => ConnectionState::FinWait2,
            8 => ConnectionState::CloseWait,
            9 => ConnectionState::Closing,
            10 => ConnectionState::LastAck,
            11 => ConnectionState::TimeWait,
            12 => ConnectionState::Unknown, // DELETE_TCB
            _ => ConnectionState::Unknown,
        }
    }
}

/// Convert a u32 in network byte order (as stored in /proc/net/tcp) to an IPv4 address.
pub fn u32_to_ipv4(addr: u32) -> IpAddr {
    let bytes = addr.to_ne_bytes();
    IpAddr::V4(std::net::Ipv4Addr::new(
        bytes[0], bytes[1], bytes[2], bytes[3],
    ))
}

/// Parse a hex "address:port" pair from /proc/net/tcp format.
/// Example: "0100007F:1F90" → (127.0.0.1, 8080)
pub fn parse_linux_addr_port(s: &str) -> Option<(IpAddr, u16)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let addr_raw = u32::from_str_radix(parts[0], 16).ok()?;
    let port = u16::from_str_radix(parts[1], 16).ok()?;
    let ip = u32_to_ipv4(addr_raw);

    Some((ip, port))
}

/// Parse a hex "address:port" pair from /proc/net/tcp6 format.
/// Example: "00000000000000000000000000000000:1F90" → (::, 8080)
pub fn parse_linux_addr_port_v6(s: &str) -> Option<(IpAddr, u16)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let hex_addr = parts[0];
    if hex_addr.len() != 32 {
        return None;
    }

    let port = u16::from_str_radix(parts[1], 16).ok()?;

    // /proc/net/tcp6 stores IPv6 in 4 groups of 4 bytes, each group in host byte order
    let b =
        |start: usize| -> u32 { u32::from_str_radix(&hex_addr[start..start + 8], 16).unwrap_or(0) };

    let g0 = b(0).to_ne_bytes();
    let g1 = b(8).to_ne_bytes();
    let g2 = b(16).to_ne_bytes();
    let g3 = b(24).to_ne_bytes();

    let mut octets = [0u8; 16];
    octets[0..4].copy_from_slice(&g0);
    octets[4..8].copy_from_slice(&g1);
    octets[8..12].copy_from_slice(&g2);
    octets[12..16].copy_from_slice(&g3);

    let ipv6 = std::net::Ipv6Addr::from(octets);

    // Check if it's a mapped IPv4 address (::ffff:x.x.x.x)
    if let Some(ipv4) = ipv6.to_ipv4_mapped() {
        Some((IpAddr::V4(ipv4), port))
    } else {
        Some((IpAddr::V6(ipv6), port))
    }
}

/// Check if an IP address is bound to all interfaces.
pub fn is_public_bind(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(ipv4) => ipv4.is_unspecified(), // 0.0.0.0
        IpAddr::V6(ipv6) => ipv6.is_unspecified(), // ::
    }
}

/// Well-known port → service name lookup.
pub fn well_known_service(port: u16) -> Option<&'static str> {
    match port {
        20 => Some("ftp-data"),
        21 => Some("ftp"),
        22 => Some("ssh"),
        23 => Some("telnet"),
        25 => Some("smtp"),
        53 => Some("dns"),
        67 => Some("dhcp-server"),
        68 => Some("dhcp-client"),
        80 => Some("http"),
        110 => Some("pop3"),
        123 => Some("ntp"),
        143 => Some("imap"),
        161 => Some("snmp"),
        389 => Some("ldap"),
        443 => Some("https"),
        445 => Some("smb"),
        465 => Some("smtps"),
        587 => Some("submission"),
        631 => Some("ipp"),
        993 => Some("imaps"),
        995 => Some("pop3s"),
        1433 => Some("mssql"),
        1521 => Some("oracle"),
        3306 => Some("mysql"),
        3389 => Some("rdp"),
        5432 => Some("postgresql"),
        5672 => Some("amqp"),
        5900 => Some("vnc"),
        6379 => Some("redis"),
        8080 => Some("http-alt"),
        8443 => Some("https-alt"),
        9090 => Some("prometheus"),
        9200 => Some("elasticsearch"),
        11211 => Some("memcached"),
        27017 => Some("mongodb"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_linux_addr_port() {
        let (ip, port) = parse_linux_addr_port("0100007F:1F90").unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));
        assert_eq!(port, 8080);
    }

    #[test]
    fn test_parse_linux_addr_port_all_interfaces() {
        let (ip, port) = parse_linux_addr_port("00000000:0050").unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        assert_eq!(port, 80);
    }

    #[test]
    fn test_is_public_bind() {
        assert!(is_public_bind(&"0.0.0.0".parse().unwrap()));
        assert!(is_public_bind(&"::".parse().unwrap()));
        assert!(!is_public_bind(&"127.0.0.1".parse().unwrap()));
        assert!(!is_public_bind(&"192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_connection_state_from_linux_hex() {
        assert_eq!(
            ConnectionState::from_linux_hex("0A"),
            ConnectionState::Listen
        );
        assert_eq!(
            ConnectionState::from_linux_hex("01"),
            ConnectionState::Established
        );
        assert_eq!(
            ConnectionState::from_linux_hex("06"),
            ConnectionState::TimeWait
        );
        assert_eq!(
            ConnectionState::from_linux_hex("FF"),
            ConnectionState::Unknown
        );
    }

    #[test]
    fn test_well_known_service() {
        assert_eq!(well_known_service(80), Some("http"));
        assert_eq!(well_known_service(443), Some("https"));
        assert_eq!(well_known_service(5432), Some("postgresql"));
        assert_eq!(well_known_service(9999), None);
    }

    #[test]
    fn test_u32_to_ipv4() {
        // 127.0.0.1 in little-endian u32 = 0x0100007F
        let ip = u32_to_ipv4(0x0100007F);
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));
    }
}
