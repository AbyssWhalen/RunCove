use crate::model::{ConnectionState, PortEntry};
use colored::*;
use std::io::{self, Write};

#[cfg(windows)]
pub fn windows_system32_executable(name: &str) -> io::Result<std::path::PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buffer = vec![0_u16; 260];
    loop {
        let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < buffer.len() {
            let directory = OsString::from_wide(&buffer[..length]);
            return Ok(std::path::PathBuf::from(directory).join(name));
        }
        buffer.resize(length + 1, 0);
    }
}

/// Kill the process occupying the specified port.
pub fn kill_on_port(
    port: u16,
    force: bool,
    entries: &[PortEntry],
    no_color: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let pids = listener_pids_on_port(port, entries);
    if pids.is_empty() {
        if no_color {
            eprintln!("✗ No process found on port {}.", port);
        } else {
            eprintln!(
                "{} No process found on port {}.",
                "✗".red(),
                port.to_string().bold()
            );
        }
        return Ok(());
    }

    for pid in &pids {
        verify_pid_owns_port(port, *pid)?;
        let identity = process_identity(*pid)?;
        let proc_name = entries
            .iter()
            .find(|entry| {
                entry.port == port
                    && entry.state == ConnectionState::Listen
                    && entry.pid == Some(*pid)
            })
            .and_then(|e| e.process_name.as_deref())
            .unwrap_or("unknown");

        if !force {
            if no_color {
                print!("Kill {} (PID {}) on port {}? [y/N] ", proc_name, pid, port);
            } else {
                print!(
                    "Kill {} (PID {}) on port {}? [y/N] ",
                    proc_name.yellow(),
                    pid.to_string().bold(),
                    port.to_string().bold()
                );
            }
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Skipped.");
                continue;
            }
        }

        verify_pid_owns_port(port, *pid)?;
        with_verified_process(*pid, &identity, || kill_process(*pid))?;
        if no_color {
            println!("✓ Killed {} (PID {})", proc_name, pid);
        } else {
            println!("{} Killed {} (PID {})", "✓".green(), proc_name, pid);
        }
    }

    Ok(())
}

fn listener_pids_on_port(port: u16, entries: &[PortEntry]) -> Vec<u32> {
    let mut pids = entries
        .iter()
        .filter(|entry| entry.port == port && entry.state == ConnectionState::Listen)
        .filter_map(|entry| entry.pid)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn verify_pid_owns_port(port: u16, pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    let entries = crate::scanner::create_scanner().scan()?;
    if entries.iter().any(|entry| {
        entry.port == port && entry.state == ConnectionState::Listen && entry.pid == Some(pid)
    }) {
        Ok(())
    } else {
        Err(format!("Port ownership changed; PID {pid} no longer owns port {port}").into())
    }
}

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct ProcessIdentity {
    creation_time: u64,
    executable_path: Vec<u16>,
}

#[cfg(windows)]
fn open_process_identity(
    pid: u32,
) -> Result<(ProcessIdentity, ::windows::Win32::Foundation::HANDLE), Box<dyn std::error::Error>> {
    use ::windows::core::PWSTR;
    use ::windows::Win32::Foundation::{CloseHandle, FILETIME};
    use ::windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| format!("Cannot inspect PID {pid} before termination: {error}"))?;
    let result = (|| {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
            .map_err(|error| format!("Cannot read start time for PID {pid}: {error}"))?;

        let mut executable_path = vec![0u16; 32_768];
        let mut path_length = executable_path.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(executable_path.as_mut_ptr()),
                &mut path_length,
            )
        }
        .map_err(|error| format!("Cannot read executable path for PID {pid}: {error}"))?;
        executable_path.truncate(path_length as usize);

        Ok::<_, Box<dyn std::error::Error>>(ProcessIdentity {
            creation_time: (u64::from(creation.dwHighDateTime) << 32)
                | u64::from(creation.dwLowDateTime),
            executable_path,
        })
    })();
    match result {
        Ok(identity) => Ok((identity, handle)),
        Err(error) => {
            let _ = unsafe { CloseHandle(handle) };
            Err(error)
        }
    }
}

#[cfg(windows)]
fn process_identity(pid: u32) -> Result<ProcessIdentity, Box<dyn std::error::Error>> {
    use ::windows::Win32::Foundation::CloseHandle;

    let (identity, handle) = open_process_identity(pid)?;
    let _ = unsafe { CloseHandle(handle) };
    Ok(identity)
}

#[cfg(windows)]
fn with_verified_process<T>(
    pid: u32,
    expected: &ProcessIdentity,
    action: impl FnOnce() -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    use ::windows::Win32::Foundation::CloseHandle;

    let (current, handle) = open_process_identity(pid)?;
    let result = if current == *expected {
        action()
    } else {
        Err(
            format!("Process identity changed for PID {pid}; refusing to terminate a reused PID")
                .into(),
        )
    };
    let _ = unsafe { CloseHandle(handle) };
    result
}

#[cfg(not(windows))]
fn process_identity(_pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(not(windows))]
fn with_verified_process<T>(
    _pid: u32,
    _expected: &(),
    action: impl FnOnce() -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    action()
}

/// Open `http://localhost:<port>` in the default browser.
pub fn open_port(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("http://localhost:{}", port);
    println!("Opening {} ...", url.cyan());

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", &url])
            .spawn()?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&url).spawn()?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&url).spawn()?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Platform-specific process killing
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn kill_process(pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Command;

    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()?;

    if !status.success() {
        return Err(format!("Failed to kill process {}", pid).into());
    }
    Ok(())
}

#[cfg(windows)]
fn kill_process(pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    let status = std::process::Command::new(windows_system32_executable("taskkill.exe")?)
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()?;

    if !status.success() {
        return Err(format!("Failed to kill process {}", pid).into());
    }
    Ok(())
}

#[cfg(windows)]
#[cfg(test)]
mod windows_path_tests {
    use super::windows_system32_executable;

    #[test]
    fn system_helper_is_resolved_under_windows_system_directory() {
        let path = windows_system32_executable("taskkill.exe").unwrap();
        assert!(path.is_absolute());
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("taskkill.exe")
        );
        assert!(path.parent().is_some_and(|parent| {
            parent
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with("\\system32")
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionState, Protocol};
    use std::net::{IpAddr, Ipv4Addr};

    fn entry(state: ConnectionState, pid: u32) -> PortEntry {
        PortEntry {
            port: 3_000,
            protocol: Protocol::TCP,
            state,
            pid: Some(pid),
            process_name: Some("node.exe".into()),
            bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            is_public: false,
        }
    }

    #[test]
    fn established_connection_is_not_a_kill_candidate() {
        let entries = [entry(ConnectionState::Established, 41)];

        assert!(listener_pids_on_port(3_000, &entries).is_empty());
    }

    #[test]
    fn listening_process_is_a_kill_candidate() {
        let entries = [
            entry(ConnectionState::Listen, 42),
            entry(ConnectionState::Listen, 42),
        ];

        assert_eq!(listener_pids_on_port(3_000, &entries), vec![42]);
    }
}
