use crate::models::{DiscoveredProject, ExpectedPortDraft};
use runcove::model::ConnectionState;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedProcess {
    pid: u32,
    parent_pid: Option<u32>,
    cwd: Option<String>,
    command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ObservedListener {
    pid: u32,
    port: u16,
    protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceInvocation {
    manager: &'static str,
    script: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProfileKey {
    project: usize,
    profile: usize,
}

#[derive(Debug, Clone)]
struct InvocationObservation {
    args: Vec<String>,
    runtime_declared_ports: HashSet<u16>,
    listeners: HashSet<(u16, String)>,
}

/// Adds runtime-derived launch details to an import preview. The overlay is
/// deliberately best-effort: port inspection failure must not block imports.
pub fn overlay_local_runtime(projects: &mut [DiscoveredProject]) {
    let Ok(entries) = runcove::scanner::create_scanner().scan() else {
        return;
    };
    let listeners = entries
        .into_iter()
        .filter(|entry| entry.state == ConnectionState::Listen)
        .filter_map(|entry| {
            entry.pid.map(|pid| ObservedListener {
                pid,
                port: entry.port,
                protocol: entry.protocol.to_string().to_ascii_lowercase(),
            })
        })
        .collect::<Vec<_>>();

    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let processes = system
        .processes()
        .iter()
        .map(|(pid, process)| ObservedProcess {
            pid: pid.as_u32(),
            parent_pid: process.parent().map(Pid::as_u32),
            cwd: process
                .cwd()
                .map(|path| path.to_string_lossy().into_owned()),
            command: process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().into_owned())
                .collect(),
        })
        .collect::<Vec<_>>();

    overlay_observations(projects, &processes, &listeners);
}

fn overlay_observations(
    projects: &mut [DiscoveredProject],
    processes: &[ObservedProcess],
    listeners: &[ObservedListener],
) {
    let process_by_pid = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    let mut observations: HashMap<ProfileKey, HashMap<u32, InvocationObservation>> = HashMap::new();

    for listener in listeners {
        let chain = ancestry(listener.pid, &process_by_pid);
        let matches = chain
            .iter()
            .enumerate()
            .filter_map(|(index, process)| {
                let invocation = parse_service_invocation(&process.command)?;
                let cwd = process.cwd.as_deref()?;
                let profiles =
                    matching_profiles(projects, cwd, invocation.manager, &invocation.script);
                (profiles.len() == 1).then(|| (profiles[0].clone(), index, process.pid, invocation))
            })
            .collect::<Vec<_>>();

        if matches.len() != 1 {
            continue;
        }
        let (profile, invocation_index, invocation_pid, invocation) = &matches[0];
        let Some(runtime_declared_ports) =
            chain[..=*invocation_index]
                .iter()
                .try_fold(HashSet::new(), |mut ports, process| {
                    ports.extend(declared_ports(&process.command)?);
                    Some(ports)
                })
        else {
            continue;
        };
        if !runtime_declared_ports.is_empty() && !runtime_declared_ports.contains(&listener.port) {
            continue;
        }
        let by_invocation = observations.entry(profile.clone()).or_default();
        let observation =
            by_invocation
                .entry(*invocation_pid)
                .or_insert_with(|| InvocationObservation {
                    args: invocation.args.clone(),
                    runtime_declared_ports: HashSet::new(),
                    listeners: HashSet::new(),
                });
        if observation.args != invocation.args {
            continue;
        }
        observation
            .runtime_declared_ports
            .extend(runtime_declared_ports);
        observation
            .listeners
            .insert((listener.port, listener.protocol.clone()));
    }

    for (key, by_invocation) in observations {
        if by_invocation.len() != 1 {
            continue;
        }
        let Some(observation) = by_invocation.into_values().next() else {
            continue;
        };
        if contains_sensitive_argument(&observation.args) {
            continue;
        }
        let Some(expected_ports) = expected_ports(
            &observation.args,
            &observation.runtime_declared_ports,
            &observation.listeners,
        ) else {
            continue;
        };
        let profile = &mut projects[key.project].profiles[key.profile];
        profile.args = observation.args;
        profile.expected_ports = expected_ports;
        profile.observed_runtime = true;
    }
}

fn ancestry<'a>(
    pid: u32,
    processes: &'a HashMap<u32, &'a ObservedProcess>,
) -> Vec<&'a ObservedProcess> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(pid);
    while let Some(pid) = current.filter(|pid| seen.insert(*pid)) {
        let Some(process) = processes.get(&pid).copied() else {
            break;
        };
        result.push(process);
        current = process.parent_pid;
    }
    result
}

fn matching_profiles(
    projects: &[DiscoveredProject],
    cwd: &str,
    manager: &str,
    script: &str,
) -> Vec<ProfileKey> {
    projects
        .iter()
        .enumerate()
        .filter(|(_, project)| project.package_manager.eq_ignore_ascii_case(manager))
        .flat_map(|(project_index, project)| {
            project
                .profiles
                .iter()
                .enumerate()
                .filter(move |(_, profile)| {
                    matches!(
                        profile.args.as_slice(),
                        [run, candidate]
                            if run.eq_ignore_ascii_case("run")
                                && candidate.eq_ignore_ascii_case(script)
                    ) && same_path(Path::new(&profile.cwd), Path::new(cwd))
                })
                .map(move |(profile_index, _)| ProfileKey {
                    project: project_index,
                    profile: profile_index,
                })
        })
        .collect()
}

fn parse_service_invocation(command: &[String]) -> Option<ServiceInvocation> {
    let (manager, command_args) = package_manager_command(command)?;
    let (script, suffix) = match (manager, command_args) {
        ("npm" | "pnpm", [run, script, suffix @ ..])
            if run.eq_ignore_ascii_case("run") && is_service_script(script) =>
        {
            (script, suffix)
        }
        ("pnpm", [script, suffix @ ..]) if is_service_script(script) => (script, suffix),
        _ => return None,
    };

    let script = script.to_ascii_lowercase();
    let mut args = vec!["run".to_owned(), script.clone()];
    args.extend(suffix.iter().cloned());
    Some(ServiceInvocation {
        manager,
        script,
        args,
    })
}

fn is_service_script(script: &str) -> bool {
    ["dev", "start", "serve", "preview"]
        .iter()
        .any(|candidate| script.eq_ignore_ascii_case(candidate))
}

fn contains_sensitive_argument(args: &[String]) -> bool {
    const SENSITIVE_PARTS: [&str; 18] = [
        "auth",
        "authorization",
        "credential",
        "credentials",
        "define",
        "header",
        "key",
        "keys",
        "password",
        "passwords",
        "passwd",
        "secret",
        "secrets",
        "token",
        "tokens",
        "private",
        "certificate",
        "certificates",
    ];

    args.iter().skip(2).any(|argument| {
        let name = argument
            .trim_start_matches('-')
            .split_once('=')
            .map_or_else(|| argument.trim_start_matches('-'), |(name, _)| name);
        let mut normalized = String::with_capacity(name.len());
        let mut previous_was_lowercase_or_digit = false;
        for character in name.chars() {
            if character.is_ascii_uppercase() && previous_was_lowercase_or_digit {
                normalized.push('-');
            }
            if character.is_ascii_alphanumeric() {
                normalized.push(character.to_ascii_lowercase());
                previous_was_lowercase_or_digit =
                    character.is_ascii_lowercase() || character.is_ascii_digit();
            } else {
                normalized.push('-');
                previous_was_lowercase_or_digit = false;
            }
        }
        normalized
            .split('-')
            .any(|part| SENSITIVE_PARTS.contains(&part))
    })
}

fn package_manager_command(command: &[String]) -> Option<(&'static str, &[String])> {
    let executable = command.first().map(|part| file_name(part))?;
    if executable.eq_ignore_ascii_case("npm") || executable.eq_ignore_ascii_case("npm.cmd") {
        return Some(("npm", &command[1..]));
    }
    if executable.eq_ignore_ascii_case("pnpm")
        || executable.eq_ignore_ascii_case("pnpm.cmd")
        || executable.eq_ignore_ascii_case("pnpm.exe")
    {
        return Some(("pnpm", &command[1..]));
    }
    if !executable.eq_ignore_ascii_case("node") && !executable.eq_ignore_ascii_case("node.exe") {
        return None;
    }

    let wrapper = command.get(1).map(|part| file_name(part))?;
    let manager = if wrapper.eq_ignore_ascii_case("npm-cli.js") {
        "npm"
    } else if wrapper.eq_ignore_ascii_case("pnpm.mjs") || wrapper.eq_ignore_ascii_case("pnpm.cjs") {
        "pnpm"
    } else {
        return None;
    };
    Some((manager, &command[2..]))
}

fn file_name(value: &str) -> &str {
    value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .trim_matches('"')
}

fn expected_ports(
    args: &[String],
    runtime_declared_ports: &HashSet<u16>,
    listeners: &HashSet<(u16, String)>,
) -> Option<Vec<ExpectedPortDraft>> {
    let wrapper_declared_ports = declared_ports(args)?;
    let declared = if wrapper_declared_ports.is_empty() {
        runtime_declared_ports
    } else {
        &wrapper_declared_ports
    };
    let selected = if declared.is_empty() {
        (listeners.len() == 1).then(|| listeners.iter().cloned().collect::<Vec<_>>())?
    } else {
        let matching = listeners
            .iter()
            .filter(|(port, _)| declared.contains(port))
            .cloned()
            .collect::<Vec<_>>();
        let observed_ports = matching
            .iter()
            .map(|(port, _)| *port)
            .collect::<HashSet<_>>();
        (&observed_ports == declared).then_some(matching)?
    };

    let mut expected = selected
        .into_iter()
        .map(|(port, protocol)| ExpectedPortDraft { port, protocol })
        .collect::<Vec<_>>();
    expected.sort_by(|left, right| (left.port, &left.protocol).cmp(&(right.port, &right.protocol)));
    Some(expected)
}

/// `None` means an explicit port flag was malformed and the observation is not
/// safe to import. An empty set means no port was declared by the wrapper.
fn declared_ports(args: &[String]) -> Option<HashSet<u16>> {
    let mut ports = HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument.eq_ignore_ascii_case("--port") {
            let port = args.get(index + 1)?.parse::<u16>().ok()?;
            ports.insert(port);
            index += 2;
            continue;
        }
        if let Some((flag, port)) = argument.split_once('=') {
            if flag.eq_ignore_ascii_case("--port") {
                ports.insert(port.parse::<u16>().ok()?);
            }
        }
        index += 1;
    }
    Some(ports)
}

fn same_path(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        let value = path.to_string_lossy().replace('/', "\\");
        let value = if cfg!(windows) {
            if let Some(path) = value.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{path}")
            } else {
                value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
            }
        } else {
            value
        };
        let value = value.trim_end_matches('\\');
        if cfg!(windows) {
            value.to_ascii_lowercase()
        } else {
            value.to_owned()
        }
    };
    normalize(left) == normalize(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DiscoveredProfile;

    fn project(profiles: Vec<DiscoveredProfile>) -> DiscoveredProject {
        DiscoveredProject {
            name: "fixture".into(),
            path: r"\\?\C:\Code\fixture".into(),
            package_manager: "pnpm".into(),
            workspace_patterns: Vec::new(),
            profiles,
        }
    }

    fn profile(name: &str, script: &str) -> DiscoveredProfile {
        DiscoveredProfile {
            name: name.into(),
            program: "pnpm.cmd".into(),
            args: vec!["run".into(), script.into()],
            cwd: r"\\?\C:\Code\fixture".into(),
            expected_ports: Vec::new(),
            observed_runtime: false,
        }
    }

    fn process(pid: u32, parent_pid: Option<u32>, command: &[&str]) -> ObservedProcess {
        ObservedProcess {
            pid,
            parent_pid,
            cwd: Some(r"c:/code/fixture/".into()),
            command: command.iter().map(|part| (*part).into()).collect(),
        }
    }

    #[test]
    fn exact_pnpm_dev_enriches_only_dev_with_structured_args_and_port() {
        let mut projects = vec![project(vec![
            profile("dev", "dev"),
            profile("start", "start"),
            profile("preview", "preview"),
        ])];
        let processes = vec![
            process(
                10,
                None,
                &[
                    "node.exe",
                    r"C:\tools\pnpm\pnpm.mjs",
                    "dev",
                    "--hostname",
                    "127.0.0.1",
                    "--port",
                    "3100",
                ],
            ),
            process(20, Some(10), &["node.exe", "next", "dev"]),
        ];
        let listeners = vec![ObservedListener {
            pid: 20,
            port: 3100,
            protocol: "tcp".into(),
        }];

        overlay_observations(&mut projects, &processes, &listeners);

        let profiles = &projects[0].profiles;
        assert_eq!(profiles[0].program, "pnpm.cmd");
        assert_eq!(
            profiles[0].args,
            ["run", "dev", "--hostname", "127.0.0.1", "--port", "3100"]
        );
        assert_eq!(
            profiles[0].expected_ports,
            [ExpectedPortDraft {
                port: 3100,
                protocol: "tcp".into()
            }]
        );
        assert!(profiles[0].observed_runtime);
        assert_eq!(profiles[1].args, ["run", "start"]);
        assert!(profiles[1].expected_ports.is_empty());
        assert!(!profiles[1].observed_runtime);
        assert_eq!(profiles[2].args, ["run", "preview"]);
        assert!(profiles[2].expected_ports.is_empty());
    }

    #[test]
    fn exact_npm_start_enriches_only_the_running_start_profile() {
        let mut npm_project = project(vec![profile("dev", "dev"), profile("start", "start")]);
        npm_project.package_manager = "npm".into();
        for profile in &mut npm_project.profiles {
            profile.program = "npm.cmd".into();
        }
        let mut projects = vec![npm_project];
        let processes = vec![
            process(
                10,
                None,
                &[
                    "node.exe",
                    r"C:\node\npm-cli.js",
                    "run",
                    "start",
                    "--port",
                    "8080",
                ],
            ),
            process(20, Some(10), &["node.exe", "server.js"]),
        ];
        let listeners = vec![ObservedListener {
            pid: 20,
            port: 8080,
            protocol: "tcp".into(),
        }];

        overlay_observations(&mut projects, &processes, &listeners);

        assert_eq!(projects[0].profiles[0].args, ["run", "dev"]);
        assert!(projects[0].profiles[0].expected_ports.is_empty());
        assert_eq!(
            projects[0].profiles[1].args,
            ["run", "start", "--port", "8080"]
        );
        assert_eq!(
            projects[0].profiles[1].expected_ports,
            [ExpectedPortDraft {
                port: 8080,
                protocol: "tcp".into()
            }]
        );
        assert!(projects[0].profiles[1].observed_runtime);
    }

    #[test]
    fn descendant_declared_port_excludes_auxiliary_listeners() {
        let mut npm_project = project(vec![profile("dev", "dev")]);
        npm_project.package_manager = "npm".into();
        npm_project.profiles[0].program = "npm.cmd".into();
        let mut projects = vec![npm_project];
        let processes = vec![
            process(10, None, &["node.exe", r"C:\node\npm-cli.js", "run", "dev"]),
            process(
                20,
                Some(10),
                &["node.exe", "astro.js", "dev", "--port", "4321", "--host"],
            ),
            process(30, Some(20), &["workerd.exe", "serve"]),
        ];
        let listeners = vec![
            ObservedListener {
                pid: 20,
                port: 4321,
                protocol: "tcp".into(),
            },
            ObservedListener {
                pid: 20,
                port: 62_853,
                protocol: "tcp".into(),
            },
            ObservedListener {
                pid: 30,
                port: 62_859,
                protocol: "tcp".into(),
            },
        ];

        overlay_observations(&mut projects, &processes, &listeners);

        let profile = &projects[0].profiles[0];
        assert_eq!(profile.args, ["run", "dev"]);
        assert_eq!(
            profile.expected_ports,
            [ExpectedPortDraft {
                port: 4321,
                protocol: "tcp".into()
            }]
        );
        assert!(profile.observed_runtime);
    }

    #[test]
    fn different_script_and_ambiguous_dev_runs_do_not_enrich() {
        let mut preview_project = vec![project(vec![profile("dev", "dev")])];
        let preview_processes = vec![process(
            10,
            None,
            &["node.exe", r"C:\tools\pnpm\pnpm.mjs", "preview"],
        )];
        let preview_listeners = vec![ObservedListener {
            pid: 10,
            port: 4173,
            protocol: "tcp".into(),
        }];
        overlay_observations(&mut preview_project, &preview_processes, &preview_listeners);
        assert_eq!(preview_project[0].profiles[0].args, ["run", "dev"]);
        assert!(preview_project[0].profiles[0].expected_ports.is_empty());

        let mut ambiguous_project = vec![project(vec![profile("dev", "dev")])];
        let ambiguous_processes = vec![
            process(
                20,
                None,
                &[
                    "node.exe",
                    r"C:\tools\pnpm\pnpm.mjs",
                    "dev",
                    "--port",
                    "3000",
                ],
            ),
            process(
                30,
                None,
                &[
                    "node.exe",
                    r"C:\tools\pnpm\pnpm.mjs",
                    "dev",
                    "--port",
                    "3001",
                ],
            ),
        ];
        let ambiguous_listeners = vec![
            ObservedListener {
                pid: 20,
                port: 3000,
                protocol: "tcp".into(),
            },
            ObservedListener {
                pid: 30,
                port: 3001,
                protocol: "tcp".into(),
            },
        ];
        overlay_observations(
            &mut ambiguous_project,
            &ambiguous_processes,
            &ambiguous_listeners,
        );
        assert_eq!(ambiguous_project[0].profiles[0].args, ["run", "dev"]);
        assert!(ambiguous_project[0].profiles[0].expected_ports.is_empty());
        assert!(parse_service_invocation(&[
            "npm.cmd".into(),
            "run".into(),
            "deploy".into(),
            "--port".into(),
            "9000".into(),
        ])
        .is_none());
    }

    #[test]
    fn npm_passthrough_separator_is_preserved_and_port_must_be_observed() {
        let invocation = parse_service_invocation(&[
            "node.exe".into(),
            r"C:\node\npm-cli.js".into(),
            "run".into(),
            "dev".into(),
            "--".into(),
            "--port=4321".into(),
        ])
        .unwrap();
        assert_eq!(invocation.manager, "npm");
        assert_eq!(invocation.script, "dev");
        assert_eq!(invocation.args, ["run", "dev", "--", "--port=4321"]);
        assert!(expected_ports(
            &invocation.args,
            &HashSet::new(),
            &HashSet::from([(4_300, "tcp".into())])
        )
        .is_none());
        assert_eq!(
            expected_ports(
                &invocation.args,
                &HashSet::new(),
                &HashSet::from([(4_321, "tcp".into())])
            )
            .unwrap(),
            [ExpectedPortDraft {
                port: 4_321,
                protocol: "tcp".into()
            }]
        );
    }

    #[test]
    fn sensitive_runtime_arguments_are_not_copied_into_the_import_preview() {
        let mut projects = vec![project(vec![profile("dev", "dev")])];
        let processes = vec![
            process(
                10,
                None,
                &[
                    "pnpm.cmd",
                    "dev",
                    "--port",
                    "3100",
                    "--access-token=do-not-store",
                ],
            ),
            process(20, Some(10), &["node.exe", "server.js"]),
        ];
        let listeners = vec![ObservedListener {
            pid: 20,
            port: 3100,
            protocol: "tcp".into(),
        }];

        overlay_observations(&mut projects, &processes, &listeners);

        let profile = &projects[0].profiles[0];
        assert_eq!(profile.args, ["run", "dev"]);
        assert!(profile.expected_ports.is_empty());
        assert!(!profile.observed_runtime);
    }

    #[test]
    fn camel_case_and_container_secret_flags_are_rejected() {
        for flag in [
            "--accessToken=fake",
            "--apiKey=fake",
            "--privateKey=fake",
            "--clientSecret=fake",
            "--authorization=fake",
            "--certificatePath=fake",
            "--header",
            "--define",
        ] {
            assert!(
                contains_sensitive_argument(&["run".into(), "dev".into(), flag.into()]),
                "expected {flag} to block runtime observation"
            );
        }
    }
}
