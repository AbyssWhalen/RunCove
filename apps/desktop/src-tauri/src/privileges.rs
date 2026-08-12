use crate::error::{invalid, AppResult};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrivilegeStatus {
    pub elevated: bool,
    pub elevation_available: bool,
    /// An elevated instance is deliberately limited to inspection. It must
    /// never launch or terminate a user-controlled process with admin rights.
    pub monitor_only: bool,
}

pub const MONITOR_ONLY_ACTION_MESSAGE: &str =
    "Administrator monitoring mode is read-only; process actions are disabled";

fn process_action_allowed(monitor_only: bool) -> AppResult<()> {
    if monitor_only {
        Err(invalid(MONITOR_ONLY_ACTION_MESSAGE))
    } else {
        Ok(())
    }
}

pub fn ensure_process_action_allowed() -> AppResult<()> {
    process_action_allowed(current_status()?.monitor_only)
}

#[cfg(windows)]
fn shell_execute_result(code: isize) -> AppResult<()> {
    match code {
        code if code > 32 => Ok(()),
        5 => Err(invalid(
            "The administrator request was cancelled or denied by Windows",
        )),
        code => Err(invalid(format!(
            "Windows could not start the administrator instance (ShellExecuteW error {code})"
        ))),
    }
}

pub fn current_status() -> AppResult<PrivilegeStatus> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
            .map_err(|error| invalid(format!("Could not inspect process privileges: {error}")))?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        let result = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        };
        let _ = unsafe { CloseHandle(token) };
        result
            .map_err(|error| invalid(format!("Could not inspect process privileges: {error}")))?;
        Ok(PrivilegeStatus {
            elevated: elevation.TokenIsElevated != 0,
            elevation_available: true,
            monitor_only: elevation.TokenIsElevated != 0,
        })
    }

    #[cfg(not(windows))]
    {
        Ok(PrivilegeStatus {
            elevated: false,
            elevation_available: false,
            monitor_only: false,
        })
    }
}

pub fn validate_elevated_relaunch() -> AppResult<()> {
    if current_status()?.elevated {
        Ok(())
    } else {
        Err(invalid(
            "The elevated monitoring startup flag requires administrator privileges",
        ))
    }
}

#[cfg(windows)]
pub fn launch_elevated_copy() -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    if current_status()?.elevated {
        return Err(invalid(
            "RunCove is already running with administrator privileges",
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| invalid(format!("Could not locate the RunCove executable: {error}")))?;
    let directory = executable
        .parent()
        .ok_or_else(|| invalid("Could not resolve the RunCove executable directory"))?;
    let verb = wide(std::ffi::OsStr::new("runas"));
    let executable = wide(executable.as_os_str());
    let parameters = wide(std::ffi::OsStr::new("--elevated-monitor"));
    let directory = wide(directory.as_os_str());
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(executable.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR(directory.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    shell_execute_result(result.0 as isize)
}

#[cfg(not(windows))]
pub fn launch_elevated_copy() -> AppResult<()> {
    Err(invalid("Enhanced monitoring is available only on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privilege_status_is_available_without_requesting_elevation() {
        let status = current_status().unwrap();
        if status.elevated {
            assert!(status.elevation_available);
            assert!(validate_elevated_relaunch().is_ok());
        } else {
            assert_eq!(
                validate_elevated_relaunch().unwrap_err().to_string(),
                "The elevated monitoring startup flag requires administrator privileges"
            );
        }
        assert!(process_action_allowed(false).is_ok());
        assert_eq!(
            process_action_allowed(true).unwrap_err().to_string(),
            MONITOR_ONLY_ACTION_MESSAGE
        );
        #[cfg(not(windows))]
        assert!(!status.elevation_available);
    }

    #[cfg(windows)]
    #[test]
    fn shell_execute_result_distinguishes_denial_from_other_failures() {
        assert!(shell_execute_result(33).is_ok());
        assert_eq!(
            shell_execute_result(5).unwrap_err().to_string(),
            "The administrator request was cancelled or denied by Windows"
        );
        assert_eq!(
            shell_execute_result(2).unwrap_err().to_string(),
            "Windows could not start the administrator instance (ShellExecuteW error 2)"
        );
    }
}
