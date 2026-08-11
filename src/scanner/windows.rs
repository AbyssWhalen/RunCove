use crate::model::*;
use crate::scanner::{PortScanner, ScanError, ScanReport};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

pub struct WindowsScanner;

impl PortScanner for WindowsScanner {
    fn scan(&self) -> Result<Vec<PortEntry>, ScanError> {
        let report = self.scan_report()?;
        for warning in &report.warnings {
            eprintln!("Warning: {warning}");
        }
        Ok(report.entries)
    }

    fn scan_report(&self) -> Result<ScanReport, ScanError> {
        let mut report = scan_tables(scan_tcp_v4, scan_tcp_v6, scan_udp_v4, scan_udp_v6)?;
        let process_names = process_name_snapshot();

        for entry in &mut report.entries {
            if let Some(pid) = entry.pid {
                entry.process_name = process_names.get(&pid).cloned();
            }
        }

        deduplicate_entries(&mut report.entries);
        Ok(report)
    }
}

fn scan_tables<TcpV4, TcpV6, UdpV4, UdpV6>(
    tcp_v4: TcpV4,
    tcp_v6: TcpV6,
    udp_v4: UdpV4,
    udp_v6: UdpV6,
) -> Result<ScanReport, ScanError>
where
    TcpV4: FnOnce(&mut Vec<PortEntry>) -> Result<(), ScanError>,
    TcpV6: FnOnce(&mut Vec<PortEntry>) -> Result<(), ScanError>,
    UdpV4: FnOnce(&mut Vec<PortEntry>) -> Result<(), ScanError>,
    UdpV6: FnOnce(&mut Vec<PortEntry>) -> Result<(), ScanError>,
{
    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    tcp_v4(&mut entries)?;
    if let Err(error) = tcp_v6(&mut entries) {
        warnings.push(format!("could not scan IPv6 TCP: {error}"));
    }
    udp_v4(&mut entries)?;
    if let Err(error) = udp_v6(&mut entries) {
        warnings.push(format!("could not scan IPv6 UDP: {error}"));
    }

    Ok(ScanReport { entries, warnings })
}

fn deduplicate_entries(entries: &mut Vec<PortEntry>) {
    let mut seen = HashSet::new();
    entries.retain(|entry| {
        seen.insert((
            entry.port,
            entry.protocol,
            entry.state,
            entry.pid,
            entry.bind_address,
        ))
    });
    entries.sort_by_key(|entry| entry.port);
}

pub(super) fn process_name_snapshot() -> HashMap<u32, String> {
    use ::windows::Win32::Foundation::CloseHandle;
    use ::windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(snapshot) => snapshot,
        Err(_) => return HashMap::new(),
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut names = HashMap::new();

    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let length = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            if length > 0 {
                names.insert(
                    entry.th32ProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..length]),
                );
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };
    names
}

// ---------------------------------------------------------------------------
// TCP IPv4
// ---------------------------------------------------------------------------

fn scan_tcp_v4(entries: &mut Vec<PortEntry>) -> Result<(), ScanError> {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    let buffer = call_get_extended_table(|buf, size| unsafe {
        WIN32_ERROR(GetExtendedTcpTable(
            Some(buf.cast()),
            size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        ))
    })?;

    let table_ptr = buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
    let num_entries = unsafe { (*table_ptr).dwNumEntries } as usize;
    let rows_ptr = unsafe { (*table_ptr).table.as_ptr() };
    let rows = unsafe { std::slice::from_raw_parts(rows_ptr, num_entries) };

    for row in rows {
        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        let ip_bytes = row.dwLocalAddr.to_ne_bytes();
        let bind_addr = IpAddr::V4(std::net::Ipv4Addr::new(
            ip_bytes[0],
            ip_bytes[1],
            ip_bytes[2],
            ip_bytes[3],
        ));
        let state = ConnectionState::from_windows_state(row.dwState);

        entries.push(PortEntry {
            port,
            protocol: Protocol::TCP,
            state,
            pid: Some(row.dwOwningPid),
            process_name: None,
            bind_address: bind_addr,
            is_public: is_public_bind(&bind_addr),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// TCP IPv6
// ---------------------------------------------------------------------------

fn scan_tcp_v6(entries: &mut Vec<PortEntry>) -> Result<(), ScanError> {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows::Win32::Networking::WinSock::AF_INET6;

    let buffer = call_get_extended_table(|buf, size| unsafe {
        WIN32_ERROR(GetExtendedTcpTable(
            Some(buf.cast()),
            size,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        ))
    })?;

    let table_ptr = buffer.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
    let num_entries = unsafe { (*table_ptr).dwNumEntries } as usize;
    let rows_ptr = unsafe { (*table_ptr).table.as_ptr() };
    let rows = unsafe { std::slice::from_raw_parts(rows_ptr, num_entries) };

    for row in rows {
        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        let bind_addr = IpAddr::V6(std::net::Ipv6Addr::from(row.ucLocalAddr));
        let state = ConnectionState::from_windows_state(row.dwState);

        entries.push(PortEntry {
            port,
            protocol: Protocol::TCP,
            state,
            pid: Some(row.dwOwningPid),
            process_name: None,
            bind_address: bind_addr,
            is_public: is_public_bind(&bind_addr),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// UDP IPv4
// ---------------------------------------------------------------------------

fn scan_udp_v4(entries: &mut Vec<PortEntry>) -> Result<(), ScanError> {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedUdpTable, MIB_UDPTABLE_OWNER_PID, UDP_TABLE_OWNER_PID,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    let buffer = call_get_extended_table(|buf, size| unsafe {
        WIN32_ERROR(GetExtendedUdpTable(
            Some(buf.cast()),
            size,
            false,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        ))
    })?;

    let table_ptr = buffer.as_ptr() as *const MIB_UDPTABLE_OWNER_PID;
    let num_entries = unsafe { (*table_ptr).dwNumEntries } as usize;
    let rows_ptr = unsafe { (*table_ptr).table.as_ptr() };
    let rows = unsafe { std::slice::from_raw_parts(rows_ptr, num_entries) };

    for row in rows {
        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        let ip_bytes = row.dwLocalAddr.to_ne_bytes();
        let bind_addr = IpAddr::V4(std::net::Ipv4Addr::new(
            ip_bytes[0],
            ip_bytes[1],
            ip_bytes[2],
            ip_bytes[3],
        ));

        entries.push(PortEntry {
            port,
            protocol: Protocol::UDP,
            state: ConnectionState::Listen,
            pid: Some(row.dwOwningPid),
            process_name: None,
            bind_address: bind_addr,
            is_public: is_public_bind(&bind_addr),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// UDP IPv6
// ---------------------------------------------------------------------------

fn scan_udp_v6(entries: &mut Vec<PortEntry>) -> Result<(), ScanError> {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedUdpTable, MIB_UDP6TABLE_OWNER_PID, UDP_TABLE_OWNER_PID,
    };
    use windows::Win32::Networking::WinSock::AF_INET6;

    let buffer = call_get_extended_table(|buf, size| unsafe {
        WIN32_ERROR(GetExtendedUdpTable(
            Some(buf.cast()),
            size,
            false,
            AF_INET6.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        ))
    })?;

    let table_ptr = buffer.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID;
    let num_entries = unsafe { (*table_ptr).dwNumEntries } as usize;
    let rows_ptr = unsafe { (*table_ptr).table.as_ptr() };
    let rows = unsafe { std::slice::from_raw_parts(rows_ptr, num_entries) };

    for row in rows {
        let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        let bind_addr = IpAddr::V6(std::net::Ipv6Addr::from(row.ucLocalAddr));

        entries.push(PortEntry {
            port,
            protocol: Protocol::UDP,
            state: ConnectionState::Listen,
            pid: Some(row.dwOwningPid),
            process_name: None,
            bind_address: bind_addr,
            is_public: is_public_bind(&bind_addr),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Generic helper for GetExtended*Table pattern
// ---------------------------------------------------------------------------

use windows::Win32::Foundation::WIN32_ERROR;

/// Calls a Windows API function following the standard retry pattern:
/// 1. Call with null buffer to get required size
/// 2. Allocate buffer of that size
/// 3. Call again with buffer; retry once if buffer was too small
///
/// `api_call` should return `WIN32_ERROR`. `NO_ERROR` (0) means success.
fn call_get_extended_table<F>(api_call: F) -> Result<Vec<u8>, ScanError>
where
    F: Fn(*mut u8, &mut u32) -> WIN32_ERROR,
{
    let no_error = WIN32_ERROR(0);

    // Step 1: determine required buffer size
    let mut size: u32 = 0;
    let _ = api_call(std::ptr::null_mut(), &mut size);

    if size == 0 {
        return Err(ScanError::PlatformError(
            "Failed to determine buffer size for port table".into(),
        ));
    }

    // Step 2: allocate and call
    let mut buffer = vec![0u8; size as usize];
    let ret = api_call(buffer.as_mut_ptr(), &mut size);

    if ret == no_error {
        return Ok(buffer);
    }

    // Step 3: retry with updated size (table may have grown between calls)
    buffer.resize(size as usize, 0);
    let ret = api_call(buffer.as_mut_ptr(), &mut size);

    if ret == no_error {
        return Ok(buffer);
    }

    Err(ScanError::PlatformError(format!(
        "Port table query failed with error code {}",
        ret.0
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(protocol: Protocol, pid: u32, address: &str) -> PortEntry {
        let bind_address = address.parse().unwrap();
        PortEntry {
            port: 5353,
            protocol,
            state: ConnectionState::Listen,
            pid: Some(pid),
            process_name: Some(format!("process-{pid}.exe")),
            bind_address,
            is_public: is_public_bind(&bind_address),
        }
    }

    #[test]
    fn interleaved_duplicate_endpoints_are_collapsed() {
        let mut different_state = entry(Protocol::UDP, 10, "0.0.0.0");
        different_state.state = ConnectionState::Established;
        let mut entries = vec![
            entry(Protocol::UDP, 10, "0.0.0.0"),
            entry(Protocol::UDP, 20, "0.0.0.0"),
            entry(Protocol::UDP, 10, "0.0.0.0"),
            entry(Protocol::UDP, 10, "::"),
            entry(Protocol::TCP, 10, "0.0.0.0"),
            different_state,
        ];

        deduplicate_entries(&mut entries);

        assert_eq!(entries.len(), 5);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| {
                    entry.protocol == Protocol::UDP
                        && entry.pid == Some(10)
                        && entry.state == ConnectionState::Listen
                })
                .count(),
            2
        );
        assert!(entries
            .iter()
            .any(|entry| entry.protocol == Protocol::UDP && entry.pid == Some(20)));
        assert!(entries
            .iter()
            .any(|entry| entry.protocol == Protocol::TCP && entry.pid == Some(10)));
        assert!(entries
            .iter()
            .any(|entry| entry.state == ConnectionState::Established));
    }

    #[test]
    fn partial_ipv6_failures_are_reported_without_losing_ipv4_entries() {
        let report = scan_tables(
            |entries| {
                entries.push(entry(Protocol::TCP, 10, "127.0.0.1"));
                Ok(())
            },
            |_| Err(ScanError::PlatformError("TCP v6 unavailable".into())),
            |entries| {
                entries.push(entry(Protocol::UDP, 20, "0.0.0.0"));
                Ok(())
            },
            |_| Err(ScanError::PermissionDenied("UDP v6 denied".into())),
        )
        .unwrap();

        assert_eq!(report.entries.len(), 2);
        assert_eq!(
            report.warnings,
            vec![
                "could not scan IPv6 TCP: Platform error: TCP v6 unavailable",
                "could not scan IPv6 UDP: Permission denied: UDP v6 denied",
            ]
        );
    }
}
