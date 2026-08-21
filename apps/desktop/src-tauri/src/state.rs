use crate::error::AppResult;
use crate::models::{
    AssociationSource, DashboardSnapshot, LaunchProfile, PortAssociation, PortSnapshot, Project,
    RunStatus,
};
use crate::processes::ProcessManager;
use crate::storage::{now_ms, Storage};
use runcove::model::ConnectionState;
use std::collections::HashSet;
use std::sync::Arc;
use sysinfo::{Pid, ProcessesToUpdate, System};

pub struct AppState {
    pub storage: Arc<Storage>,
    pub processes: Arc<ProcessManager>,
}

impl AppState {
    pub fn dashboard(&self) -> AppResult<DashboardSnapshot> {
        let mut projects = self.storage.list_projects()?;
        apply_runtime_status(&mut projects, &self.processes);
        let restore_set = self.storage.restore_set()?;
        let settings = self.storage.settings()?;
        let associations = self.storage.list_associations()?;
        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let (entries, scan_error, scan_complete) =
            dashboard_scan_result(runcove::scanner::create_scanner().scan_report());
        let mut ports = Vec::new();
        let mut active_keys = HashSet::new();

        for entry in entries
            .into_iter()
            .filter(|entry| entry.state == ConnectionState::Listen)
        {
            let protocol = entry.protocol.to_string().to_ascii_lowercase();
            let key = (entry.port, protocol.clone());
            active_keys.insert(key);
            let process = entry.pid.and_then(|pid| system.process(Pid::from_u32(pid)));
            let managed = entry
                .pid
                .and_then(|pid| self.processes.owns_pid(pid, &system));
            let confirmed = associations.iter().find(|association| {
                association.port == entry.port
                    && association.protocol == protocol
                    && association.source == AssociationSource::Confirmed
            });
            let suggested = managed
                .is_none()
                .then(|| {
                    entry
                        .pid
                        .and_then(|pid| suggested_owner(pid, entry.port, &system, &projects))
                })
                .flatten();
            let confirmed = confirmed
                .filter(|association| confirmed_matches_suggestion(association, suggested));
            let (project_id, profile_id, source) = if let Some(info) = &managed {
                persist_managed_association(
                    &self.storage,
                    &info.project_id,
                    &info.profile_id,
                    entry.port,
                    &protocol,
                )?;
                (
                    Some(info.project_id.clone()),
                    Some(info.profile_id.clone()),
                    Some(AssociationSource::Managed),
                )
            } else if let Some(association) = confirmed {
                touch_confirmed_association(&self.storage, &association.id)?;
                (
                    Some(association.project_id.clone()),
                    association.profile_id.clone(),
                    Some(AssociationSource::Confirmed),
                )
            } else if let Some(owner) = suggested {
                (
                    Some(owner.project.id.clone()),
                    owner.profile.map(|profile| profile.id.clone()),
                    Some(AssociationSource::Suggested),
                )
            } else {
                (None, None, None)
            };
            ports.push(PortSnapshot {
                port: entry.port,
                protocol,
                state: entry.state.to_string(),
                bind_address: Some(entry.bind_address.to_string()),
                is_public: entry.is_public,
                active: true,
                pid: entry.pid,
                process_name: process
                    .map(|value| value.name().to_string_lossy().into_owned())
                    .or(entry.process_name),
                executable_path: process
                    .and_then(|value| value.exe())
                    .map(|path| path.to_string_lossy().into_owned()),
                command_line: process.map(|value| {
                    value
                        .cmd()
                        .iter()
                        .map(|part| part.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                process_started_at: entry
                    .pid
                    .and_then(|pid| process_started_at_ms(pid, &system)),
                last_seen_at: Some(now_ms()),
                project_id,
                profile_id,
                association_source: source,
            });
        }

        if scan_complete {
            reconcile_expected_port_conflicts(&mut projects, &self.processes, &ports);
        }
        append_inactive_expected_ports(&mut ports, &projects, &active_keys);
        append_inactive_associations(&mut ports, &associations);
        ports.sort_by_key(|port| (port.port, port.protocol.clone(), !port.active));
        Ok(DashboardSnapshot {
            ports,
            projects,
            restore_set,
            settings,
            privilege: crate::privileges::current_status()?,
            generated_at: now_ms(),
            scan_error,
            run_log_archive: self.processes.archive().state(),
        })
    }
}

fn dashboard_scan_result(
    scan: Result<runcove::scanner::ScanReport, runcove::scanner::ScanError>,
) -> (Vec<runcove::model::PortEntry>, Option<String>, bool) {
    match scan {
        Ok(report) if report.warnings.is_empty() => (report.entries, None, true),
        Ok(report) => {
            let warning = format!("Partial port scan: {}", report.warnings.join("; "));
            (report.entries, Some(warning), false)
        }
        Err(error) => (Vec::new(), Some(error.to_string()), false),
    }
}

fn persist_managed_association(
    storage: &Storage,
    project_id: &str,
    profile_id: &str,
    port: u16,
    protocol: &str,
) -> AppResult<()> {
    storage.upsert_managed_association(project_id, profile_id, port, protocol)
}

fn touch_confirmed_association(storage: &Storage, association_id: &str) -> AppResult<()> {
    storage.touch_association(association_id)
}

#[derive(Clone, Copy)]
struct SuggestedOwner<'a> {
    project: &'a Project,
    profile: Option<&'a LaunchProfile>,
}

fn suggested_owner<'a>(
    pid: u32,
    port: u16,
    system: &System,
    projects: &'a [Project],
) -> Option<SuggestedOwner<'a>> {
    let processes = process_ancestry(pid, system);
    let project = projects
        .iter()
        .filter(|project| {
            let root = normalize_windows_path(&project.path);
            processes.iter().any(|process| {
                let cwd = process
                    .cwd()
                    .map(|path| normalize_windows_path(&path.to_string_lossy()));
                let command = process_command(process);
                cwd.as_deref()
                    .is_some_and(|candidate| path_is_within(candidate, &root))
                    || command_mentions_path(&command, &root)
            })
        })
        .max_by_key(|project| normalize_windows_path(&project.path).len())?;
    let commands = processes
        .iter()
        .map(|process| process_command(process))
        .collect::<Vec<_>>();
    Some(SuggestedOwner {
        project,
        profile: suggested_profile(project, port, &commands),
    })
}

fn process_ancestry(pid: u32, system: &System) -> Vec<&sysinfo::Process> {
    let mut processes = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(Pid::from_u32(pid));
    while let Some(pid) = current.filter(|pid| seen.insert(*pid)) {
        let Some(process) = system.process(pid) else {
            break;
        };
        processes.push(process);
        current = process.parent();
    }
    processes
}

fn process_command(process: &sysinfo::Process) -> String {
    process
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn suggested_profile<'a>(
    project: &'a Project,
    port: u16,
    commands: &[String],
) -> Option<&'a LaunchProfile> {
    let expected = project
        .profiles
        .iter()
        .filter(|profile| {
            profile
                .expected_ports
                .iter()
                .any(|expected| expected.port == port)
        })
        .collect::<Vec<_>>();
    if expected.len() == 1 {
        return expected.into_iter().next();
    }

    let candidates = if expected.is_empty() {
        project.profiles.iter().collect::<Vec<_>>()
    } else {
        expected
    };
    let command_matches = candidates
        .iter()
        .copied()
        .filter(|profile| profile_matches_commands(profile, commands))
        .collect::<Vec<_>>();
    if command_matches.len() == 1 {
        return command_matches.into_iter().next();
    }
    (project.profiles.len() == 1).then(|| &project.profiles[0])
}

fn profile_matches_commands(profile: &LaunchProfile, commands: &[String]) -> bool {
    let script = match profile.args.as_slice() {
        [run, script, ..] if run.eq_ignore_ascii_case("run") => script.as_str(),
        [script, ..] => script.as_str(),
        [] => return false,
    };
    commands
        .iter()
        .any(|command| command_contains_token(command, script))
}

fn command_contains_token(command: &str, token: &str) -> bool {
    let command = command.to_ascii_lowercase();
    let token = token.to_ascii_lowercase();
    command.match_indices(&token).any(|(index, _)| {
        let is_token_character = |character: char| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':')
        };
        let before = command[..index].chars().next_back();
        let after = command[index + token.len()..].chars().next();
        before.map_or(true, |character| !is_token_character(character))
            && after.map_or(true, |character| !is_token_character(character))
    })
}

fn confirmed_matches_suggestion(
    association: &PortAssociation,
    suggestion: Option<SuggestedOwner<'_>>,
) -> bool {
    match suggestion {
        None => true,
        Some(suggestion) => {
            association.project_id == suggestion.project.id
                && match (&association.profile_id, suggestion.profile) {
                    (Some(confirmed), Some(profile)) => confirmed == &profile.id,
                    _ => true,
                }
        }
    }
}

fn normalize_windows_path(path: &str) -> String {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    let without_extended_prefix = if let Some(path) = normalized.strip_prefix(r"\\?\unc\") {
        format!(r"\\{path}")
    } else {
        normalized
            .strip_prefix(r"\\?\")
            .unwrap_or(&normalized)
            .to_owned()
    };
    without_extended_prefix.trim_end_matches('\\').to_owned()
}

fn path_is_within(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn command_mentions_path(command: &str, root: &str) -> bool {
    let command = normalize_windows_path(command);
    command.match_indices(root).any(|(index, _)| {
        let before = command[..index].chars().next_back();
        let after = command[index + root.len()..].chars().next();
        before.map_or(true, |character| {
            character.is_whitespace() || "\"'=".contains(character)
        }) && after.map_or(true, |character| {
            character == '\\' || character.is_whitespace() || "\"'".contains(character)
        })
    })
}

fn process_started_at_ms(pid: u32, _system: &System) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, FILETIME};
        use windows::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let result =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        let _ = unsafe { CloseHandle(handle) };
        result.ok()?;
        let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        const WINDOWS_TO_UNIX_TICKS: u64 = 116_444_736_000_000_000;
        Some(ticks.saturating_sub(WINDOWS_TO_UNIX_TICKS) / 10_000)
    }

    #[cfg(not(windows))]
    {
        _system
            .process(Pid::from_u32(pid))
            .map(|process| process.start_time().saturating_mul(1_000))
    }
}

fn apply_runtime_status(projects: &mut [Project], processes: &ProcessManager) {
    for project in projects {
        for profile in &mut project.profiles {
            apply_profile_runtime_status(
                profile,
                processes.status(&profile.id),
                processes.info(&profile.id).map(|info| info.pid),
            );
        }
    }
}

fn apply_profile_runtime_status(
    profile: &mut LaunchProfile,
    status: Option<RunStatus>,
    pid: Option<u32>,
) {
    if let Some(status) = status {
        profile.status = status;
    }
    profile.pid = pid;
}

fn reconcile_expected_port_conflicts(
    projects: &mut [Project],
    processes: &ProcessManager,
    active_ports: &[PortSnapshot],
) {
    for profile in projects
        .iter_mut()
        .flat_map(|project| project.profiles.iter_mut())
    {
        if profile.status == RunStatus::Starting {
            continue;
        }

        let expected_owners = profile.expected_ports.iter().map(|expected| {
            let mut matching = active_ports.iter().filter(|port| {
                port.active
                    && port.port == expected.port
                    && port.protocol.eq_ignore_ascii_case(&expected.protocol)
            });
            let owned_by_profile = matching.clone().any(|port| {
                port.association_source == Some(AssociationSource::Managed)
                    && port.profile_id.as_deref() == Some(profile.id.as_str())
            });
            let occupied_by_other = matching.any(|port| {
                port.association_source != Some(AssociationSource::Managed)
                    || port.profile_id.as_deref() != Some(profile.id.as_str())
            });
            (owned_by_profile, occupied_by_other)
        });

        let expected_owners = expected_owners.collect::<Vec<_>>();
        let has_conflict = expected_owners
            .iter()
            .any(|(_, occupied_by_other)| *occupied_by_other);
        let all_expected_ports_managed = !expected_owners.is_empty()
            && expected_owners
                .iter()
                .all(|(owned_by_profile, _)| *owned_by_profile);
        let managed_process_alive = profile.pid.is_some();
        let next = if has_conflict {
            Some(RunStatus::Conflict)
        } else if managed_process_alive && all_expected_ports_managed {
            Some(RunStatus::Running)
        } else if managed_process_alive && !profile.expected_ports.is_empty() {
            Some(RunStatus::Unknown)
        } else if profile.status == RunStatus::Conflict {
            Some(RunStatus::Idle)
        } else {
            None
        };
        if let Some(next) = next.filter(|next| *next != profile.status) {
            profile.status = next;
            processes.set_status(&profile.id, next);
        }
    }
}

fn append_inactive_expected_ports(
    ports: &mut Vec<PortSnapshot>,
    projects: &[Project],
    active: &HashSet<(u16, String)>,
) {
    for project in projects {
        for profile in &project.profiles {
            for expected in &profile.expected_ports {
                if !active.contains(&(expected.port, expected.protocol.clone())) {
                    ports.push(inactive_port(
                        expected.port,
                        &expected.protocol,
                        Some(project.id.clone()),
                        Some(profile.id.clone()),
                        None,
                        None,
                    ));
                }
            }
        }
    }
}

fn append_inactive_associations(
    ports: &mut Vec<PortSnapshot>,
    associations: &[crate::models::PortAssociation],
) {
    for association in associations {
        if let Some(expected) = ports.iter_mut().find(|port| {
            !port.active
                && port.port == association.port
                && port.protocol == association.protocol
                && port.project_id.as_deref() == Some(&association.project_id)
                && port.profile_id == association.profile_id
                && port.association_source.is_none()
        }) {
            expected.association_source = Some(association.source);
            expected.last_seen_at = Some(association.last_seen_at);
            continue;
        }
        let already_present = ports.iter().any(|port| {
            port.port == association.port
                && port.protocol == association.protocol
                && port.project_id.as_deref() == Some(&association.project_id)
                && port.profile_id == association.profile_id
                && port.association_source == Some(association.source)
        });
        if !already_present {
            ports.push(inactive_port(
                association.port,
                &association.protocol,
                Some(association.project_id.clone()),
                association.profile_id.clone(),
                Some(association.source),
                Some(association.last_seen_at),
            ));
        }
    }
}

fn inactive_port(
    port: u16,
    protocol: &str,
    project_id: Option<String>,
    profile_id: Option<String>,
    source: Option<AssociationSource>,
    last_seen_at: Option<i64>,
) -> PortSnapshot {
    PortSnapshot {
        port,
        protocol: protocol.into(),
        state: "IDLE".into(),
        bind_address: None,
        is_public: false,
        active: false,
        pid: None,
        process_name: None,
        executable_path: None,
        command_line: None,
        process_started_at: None,
        last_seen_at,
        project_id,
        profile_id,
        association_source: source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, profiles: Vec<LaunchProfile>) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: format!(r"C:\code\{id}"),
            profiles,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn profile(id: &str, project_id: &str, script: &str, port: u16) -> LaunchProfile {
        LaunchProfile {
            id: id.into(),
            project_id: project_id.into(),
            name: script.into(),
            program: "npm.cmd".into(),
            args: vec!["run".into(), script.into()],
            cwd: format!(r"C:\code\{project_id}"),
            expected_ports: vec![crate::models::ExpectedPort {
                id: format!("expected-{id}"),
                profile_id: id.into(),
                port,
                protocol: "tcp".into(),
            }],
            status: RunStatus::Idle,
            pid: None,
        }
    }

    #[test]
    fn project_path_matching_respects_boundaries() {
        let root = normalize_windows_path("C:/code/app");
        assert!(path_is_within(
            &normalize_windows_path("C:/code/app/src"),
            &root
        ));
        assert!(!path_is_within(
            &normalize_windows_path("C:/code/app-two"),
            &root
        ));
        assert!(command_mentions_path("node C:/code/app/server.js", &root));
        assert!(!command_mentions_path(
            "node C:/code/app-two/server.js",
            &root
        ));
        assert_eq!(normalize_windows_path(r"\\?\C:\Code\App\"), r"c:\code\app");
        assert_eq!(
            normalize_windows_path(r"//?/UNC/Server/Share/App/"),
            r"\\server\share\app"
        );
        assert!(path_is_within(
            &normalize_windows_path(r"\\?\C:\CODE\APP\src"),
            &root,
        ));
        assert!(!path_is_within(
            &normalize_windows_path(r"\\?\C:\CODE\APP-two"),
            &root,
        ));
    }

    #[test]
    fn suggestion_resolves_the_profile_from_expected_port_or_script() {
        let project = project(
            "app",
            vec![
                profile("web", "app", "dev", 3_100),
                profile("api", "app", "serve:api", 4_100),
            ],
        );
        assert_eq!(
            suggested_profile(&project, 3_100, &[]).map(|profile| profile.id.as_str()),
            Some("web")
        );
        assert_eq!(
            suggested_profile(&project, 9_999, &["npm.cmd run serve:api".into()])
                .map(|profile| profile.id.as_str()),
            Some("api")
        );
        assert!(!command_contains_token("npm.cmd run dev:fresh", "dev"));
    }

    #[test]
    fn stale_confirmed_owner_is_not_applied_to_a_different_current_project() {
        let old_project = project("old", vec![profile("old-dev", "old", "dev", 3_100)]);
        let new_project = project("new", vec![profile("new-dev", "new", "dev", 3_100)]);
        let association = PortAssociation {
            id: "association".into(),
            project_id: old_project.id.clone(),
            profile_id: Some(old_project.profiles[0].id.clone()),
            port: 3_100,
            protocol: "tcp".into(),
            source: AssociationSource::Confirmed,
            first_seen_at: 1,
            last_seen_at: 2,
        };

        assert!(!confirmed_matches_suggestion(
            &association,
            Some(SuggestedOwner {
                project: &new_project,
                profile: Some(&new_project.profiles[0]),
            })
        ));
    }

    #[test]
    fn confirmed_owner_remains_authoritative_without_a_contradictory_suggestion() {
        let owner = project("owner", vec![profile("owner-dev", "owner", "dev", 3_100)]);
        let association = PortAssociation {
            id: "association".into(),
            project_id: owner.id.clone(),
            profile_id: Some(owner.profiles[0].id.clone()),
            port: 3_100,
            protocol: "tcp".into(),
            source: AssociationSource::Confirmed,
            first_seen_at: 1,
            last_seen_at: 2,
        };

        assert!(confirmed_matches_suggestion(&association, None));
        assert!(confirmed_matches_suggestion(
            &association,
            Some(SuggestedOwner {
                project: &owner,
                profile: Some(&owner.profiles[0]),
            })
        ));
    }

    #[test]
    fn runtime_pid_does_not_promote_a_starting_profile_before_readiness() {
        let mut profile = profile("web", "app", "dev", 3_100);

        apply_profile_runtime_status(&mut profile, Some(RunStatus::Starting), Some(42));

        assert_eq!(profile.status, RunStatus::Starting);
        assert_eq!(profile.pid, Some(42));
    }

    #[test]
    fn partial_scan_warnings_preserve_entries_and_disable_status_reconciliation() {
        let bind_address = "127.0.0.1".parse().unwrap();
        let report = runcove::scanner::ScanReport {
            entries: vec![runcove::model::PortEntry {
                port: 3_100,
                protocol: runcove::model::Protocol::TCP,
                state: ConnectionState::Listen,
                pid: Some(42),
                process_name: Some("node.exe".into()),
                bind_address,
                is_public: false,
            }],
            warnings: vec!["could not scan IPv6 TCP: access denied".into()],
        };

        let (entries, warning, complete) = dashboard_scan_result(Ok(report));

        assert_eq!(entries.len(), 1);
        assert_eq!(
            warning.as_deref(),
            Some("Partial port scan: could not scan IPv6 TCP: access denied")
        );
        assert!(!complete);
    }

    #[test]
    fn fatal_scan_errors_return_no_entries_and_disable_status_reconciliation() {
        let (entries, error, complete) = dashboard_scan_result(Err(
            runcove::scanner::ScanError::PlatformError("IPv4 unavailable".into()),
        ));

        assert!(entries.is_empty());
        assert_eq!(error.as_deref(), Some("Platform error: IPv4 unavailable"));
        assert!(!complete);
    }

    #[test]
    fn external_expected_port_occupancy_sets_and_clears_conflict() {
        let processes = ProcessManager::new(10);
        let mut projects = vec![project("app", vec![profile("web", "app", "dev", 3_100)])];
        let occupied = vec![active_port(
            3_100,
            Some("other-project"),
            Some("other-profile"),
            None,
        )];

        reconcile_expected_port_conflicts(&mut projects, &processes, &occupied);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Conflict);
        assert_eq!(processes.status("web"), Some(RunStatus::Conflict));

        reconcile_expected_port_conflicts(&mut projects, &processes, &[]);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Idle);
        assert_eq!(processes.status("web"), Some(RunStatus::Idle));
    }

    #[test]
    fn managed_expected_port_does_not_create_a_conflict_for_its_owner() {
        let processes = ProcessManager::new(10);
        let mut projects = vec![project("app", vec![profile("web", "app", "dev", 3_100)])];
        projects[0].profiles[0].status = RunStatus::Running;
        projects[0].profiles[0].pid = Some(42);
        let occupied = vec![active_port(
            3_100,
            Some("app"),
            Some("web"),
            Some(AssociationSource::Managed),
        )];

        reconcile_expected_port_conflicts(&mut projects, &processes, &occupied);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Running);
    }

    #[test]
    fn running_profile_becomes_unknown_when_its_expected_listener_disappears() {
        let processes = ProcessManager::new(10);
        let mut projects = vec![project("app", vec![profile("web", "app", "dev", 3_100)])];
        projects[0].profiles[0].status = RunStatus::Running;
        projects[0].profiles[0].pid = Some(42);

        reconcile_expected_port_conflicts(&mut projects, &processes, &[]);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Unknown);
        assert_eq!(processes.status("web"), Some(RunStatus::Unknown));
    }

    #[test]
    fn running_profile_becomes_conflict_when_an_expected_port_has_an_external_owner() {
        let processes = ProcessManager::new(10);
        let mut projects = vec![project("app", vec![profile("web", "app", "dev", 3_100)])];
        projects[0].profiles[0].status = RunStatus::Running;
        projects[0].profiles[0].pid = Some(42);
        let occupied = vec![active_port(
            3_100,
            Some("other-project"),
            Some("other-profile"),
            None,
        )];

        reconcile_expected_port_conflicts(&mut projects, &processes, &occupied);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Conflict);
        assert_eq!(processes.status("web"), Some(RunStatus::Conflict));
    }

    #[test]
    fn unknown_profile_returns_to_running_when_all_expected_listeners_are_managed() {
        let processes = ProcessManager::new(10);
        let mut projects = vec![project("app", vec![profile("web", "app", "dev", 3_100)])];
        projects[0].profiles[0].status = RunStatus::Unknown;
        projects[0].profiles[0].pid = Some(42);
        let occupied = vec![active_port(
            3_100,
            Some("app"),
            Some("web"),
            Some(AssociationSource::Managed),
        )];

        reconcile_expected_port_conflicts(&mut projects, &processes, &occupied);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Running);
        assert_eq!(processes.status("web"), Some(RunStatus::Running));
    }

    #[test]
    fn starting_profile_is_not_reclassified_by_port_polling() {
        let processes = ProcessManager::new(10);
        let mut projects = vec![project("app", vec![profile("web", "app", "dev", 3_100)])];
        projects[0].profiles[0].status = RunStatus::Starting;
        projects[0].profiles[0].pid = Some(42);
        let occupied = vec![active_port(
            3_100,
            Some("app"),
            Some("web"),
            Some(AssociationSource::Managed),
        )];

        reconcile_expected_port_conflicts(&mut projects, &processes, &occupied);

        assert_eq!(projects[0].profiles[0].status, RunStatus::Starting);
        assert_eq!(processes.status("web"), None);
    }

    #[test]
    fn managed_association_persistence_errors_reach_dashboard_callers() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(&temp.path().join("managed-association.sqlite3")).unwrap();

        let result = persist_managed_association(
            &storage,
            "missing-project",
            "missing-profile",
            3_100,
            "tcp",
        );

        assert!(result.is_err());
    }

    #[test]
    fn confirmed_association_touch_errors_reach_dashboard_callers() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(&temp.path().join("confirmed-association.sqlite3")).unwrap();

        let result = touch_confirmed_association(&storage, "missing-association");

        assert!(result.is_err());
    }

    fn active_port(
        port: u16,
        project_id: Option<&str>,
        profile_id: Option<&str>,
        association_source: Option<AssociationSource>,
    ) -> PortSnapshot {
        PortSnapshot {
            port,
            protocol: "tcp".into(),
            state: "LISTEN".into(),
            bind_address: Some("127.0.0.1".into()),
            is_public: false,
            active: true,
            pid: Some(42),
            process_name: Some("node.exe".into()),
            executable_path: None,
            command_line: None,
            process_started_at: Some(1),
            last_seen_at: Some(2),
            project_id: project_id.map(str::to_owned),
            profile_id: profile_id.map(str::to_owned),
            association_source,
        }
    }

    #[test]
    fn active_port_reuse_keeps_the_old_association_as_history() {
        let mut ports = vec![PortSnapshot {
            port: 3_100,
            protocol: "tcp".into(),
            state: "LISTEN".into(),
            bind_address: Some("127.0.0.1".into()),
            is_public: false,
            active: true,
            pid: Some(42),
            process_name: Some("node.exe".into()),
            executable_path: None,
            command_line: None,
            process_started_at: Some(1),
            last_seen_at: Some(2),
            project_id: Some("new".into()),
            profile_id: Some("new-dev".into()),
            association_source: Some(AssociationSource::Suggested),
        }];
        let associations = vec![PortAssociation {
            id: "old-association".into(),
            project_id: "old".into(),
            profile_id: Some("old-dev".into()),
            port: 3_100,
            protocol: "tcp".into(),
            source: AssociationSource::Confirmed,
            first_seen_at: 1,
            last_seen_at: 2,
        }];

        append_inactive_associations(&mut ports, &associations);

        assert_eq!(ports.len(), 2);
        assert!(!ports[1].active);
        assert_eq!(ports[1].project_id.as_deref(), Some("old"));
    }

    #[test]
    fn inactive_expected_port_uses_matching_association_metadata_without_duplication() {
        let owner = project("owner", vec![profile("owner-dev", "owner", "dev", 3_100)]);
        let mut ports = Vec::new();
        append_inactive_expected_ports(&mut ports, std::slice::from_ref(&owner), &HashSet::new());
        let association = PortAssociation {
            id: "association".into(),
            project_id: owner.id.clone(),
            profile_id: Some(owner.profiles[0].id.clone()),
            port: 3_100,
            protocol: "tcp".into(),
            source: AssociationSource::Confirmed,
            first_seen_at: 10,
            last_seen_at: 20,
        };

        append_inactive_associations(&mut ports, &[association]);

        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0].association_source,
            Some(AssociationSource::Confirmed)
        );
        assert_eq!(ports[0].last_seen_at, Some(20));
    }
}
