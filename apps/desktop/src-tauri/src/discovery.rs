use crate::error::{invalid, AppResult};
use crate::models::{DiscoveredProfile, DiscoveredProject};
use glob::{glob, Pattern};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::{Component, Path};

const MAX_SCAN_DEPTH: usize = 8;
const MAX_SCANNED_DIRECTORIES: usize = 10_000;
const MAX_DISCOVERED_PROJECTS: usize = 200;
const IGNORED_DIRECTORIES: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    "out",
    "coverage",
    ".cache",
    ".pnpm-store",
    ".yarn",
    ".venv",
    "venv",
];

pub fn discover(directory: &str) -> AppResult<DiscoveredProject> {
    let root = Path::new(directory)
        .canonicalize()
        .map_err(|error| invalid(format!("Cannot access project directory: {error}")))?;
    if !root.is_dir() {
        return Err(invalid("Project path must be a directory"));
    }

    let manifest_path = root.join("package.json");
    let manifest = read_manifest(&manifest_path)?;
    let package_manager = detect_package_manager(&root);
    let workspace_patterns = workspace_patterns(&root, &manifest)?;
    let mut manifests = vec![manifest_path];

    for directory in expand_workspace_directories(&root, &workspace_patterns)? {
        let package_manifest = directory.join("package.json");
        if package_manifest.is_file() {
            manifests.push(package_manifest);
        }
    }

    manifests.sort();
    manifests.dedup();
    let root_name = package_name(&manifest).unwrap_or_else(|| {
        root.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let mut profiles = Vec::new();
    for path in manifests {
        let package = read_manifest(&path)?;
        let cwd = path.parent().unwrap_or(&root);
        let package_label = package_name(&package).unwrap_or_else(|| {
            cwd.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
        let is_root = cwd == root;
        if let Some(scripts) = package.get("scripts").and_then(Value::as_object) {
            let mut names: Vec<_> = scripts.keys().cloned().collect();
            names.sort_by_key(|name| script_priority(name));
            for script in names {
                if !is_default_launch_script(&script) {
                    continue;
                }
                let display_name = if is_root {
                    script.clone()
                } else {
                    format!("{package_label}: {script}")
                };
                profiles.push(DiscoveredProfile {
                    name: display_name,
                    program: executable_name(&package_manager),
                    args: vec!["run".into(), script],
                    cwd: cwd.to_string_lossy().into_owned(),
                    expected_ports: Vec::new(),
                    observed_runtime: false,
                });
            }
        }
    }

    Ok(DiscoveredProject {
        name: root_name,
        path: root.to_string_lossy().into_owned(),
        package_manager,
        workspace_patterns,
        profiles,
    })
}

pub fn scan_development_root(directory: &str) -> AppResult<Vec<DiscoveredProject>> {
    let root = Path::new(directory)
        .canonicalize()
        .map_err(|error| invalid(format!("Cannot access development root: {error}")))?;
    if !root.is_dir() {
        return Err(invalid("Development root must be a directory"));
    }

    let mut queue = VecDeque::from([(root.clone(), 0_usize)]);
    let mut visited = HashSet::new();
    let mut workspace_roots = Vec::new();
    let mut projects = Vec::new();

    while let Some((directory, depth)) = queue.pop_front() {
        let Ok(canonical) = directory.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&root) || !visited.insert(canonical.clone()) {
            continue;
        }
        if visited.len() > MAX_SCANNED_DIRECTORIES {
            break;
        }
        if workspace_roots
            .iter()
            .any(|workspace: &std::path::PathBuf| canonical.starts_with(workspace))
        {
            continue;
        }

        if canonical.join("package.json").is_file() {
            if let Ok(project) = discover(&canonical.to_string_lossy()) {
                if let Ok(workspaces) =
                    expand_workspace_directories(&canonical, &project.workspace_patterns)
                {
                    workspace_roots.extend(workspaces);
                }
                projects.push(project);
                if projects.len() >= MAX_DISCOVERED_PROJECTS {
                    break;
                }
            }
        }

        if depth == MAX_SCAN_DEPTH {
            continue;
        }
        let Ok(mut children) = scan_children(&canonical) else {
            continue;
        };
        children.sort();
        queue.extend(children.into_iter().map(|child| (child, depth + 1)));
    }

    projects.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(projects)
}

fn scan_children(directory: &Path) -> AppResult<Vec<std::path::PathBuf>> {
    let entries = fs::read_dir(directory).map_err(|error| {
        invalid(format!(
            "Cannot scan directory '{}': {error}",
            directory.to_string_lossy()
        ))
    })?;
    let mut children = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            invalid(format!(
                "Cannot read an entry in '{}': {error}",
                directory.to_string_lossy()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            invalid(format!(
                "Cannot inspect '{}': {error}",
                entry.path().to_string_lossy()
            ))
        })?;
        if file_type.is_dir() && !ignored_directory(&entry.file_name().to_string_lossy()) {
            children.push(entry.path());
        }
    }
    Ok(children)
}

fn ignored_directory(name: &str) -> bool {
    IGNORED_DIRECTORIES
        .iter()
        .any(|ignored| name.eq_ignore_ascii_case(ignored))
}

fn expand_workspace_directories(
    root: &Path,
    patterns: &[String],
) -> AppResult<Vec<std::path::PathBuf>> {
    let mut directories = BTreeSet::new();
    for pattern in patterns.iter().filter(|pattern| !pattern.starts_with('!')) {
        let pattern_text = root.join(pattern).to_string_lossy().into_owned();
        let matches = glob(&pattern_text)
            .map_err(|error| invalid(format!("Invalid workspace pattern '{pattern}': {error}")))?;
        for item in matches.flatten() {
            if let Ok(canonical) = item.canonicalize() {
                if canonical.is_dir()
                    && canonical.starts_with(root)
                    && !canonical
                        .components()
                        .any(|part| part.as_os_str() == "node_modules")
                    && !workspace_directory_is_excluded(root, &canonical, patterns)?
                {
                    directories.insert(canonical);
                }
            }
        }
    }
    Ok(directories.into_iter().collect())
}

fn workspace_directory_is_excluded(
    root: &Path,
    directory: &Path,
    patterns: &[String],
) -> AppResult<bool> {
    let relative = directory.strip_prefix(root).map_err(|_| {
        invalid(format!(
            "Workspace directory '{}' escaped the project root",
            directory.to_string_lossy()
        ))
    })?;
    for pattern in patterns
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('!'))
    {
        let matcher = Pattern::new(pattern)
            .map_err(|error| invalid(format!("Invalid workspace pattern '!{pattern}': {error}")))?;
        if matcher.matches_path(relative) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_manifest(path: &Path) -> AppResult<Value> {
    let contents = fs::read_to_string(path)
        .map_err(|error| invalid(format!("Cannot read {}: {error}", path.to_string_lossy())))?;
    Ok(serde_json::from_str(&contents)?)
}

fn package_name(manifest: &Value) -> Option<String> {
    manifest
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn detect_package_manager(root: &Path) -> String {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm".into()
    } else {
        "npm".into()
    }
}

fn workspace_patterns(root: &Path, manifest: &Value) -> AppResult<Vec<String>> {
    let values = match manifest.get("workspaces") {
        Some(Value::Array(values)) => Some(values),
        Some(Value::Object(object)) => object.get("packages").and_then(Value::as_array),
        _ => None,
    };
    let mut patterns = values
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    patterns.extend(read_pnpm_workspace_patterns(root)?);
    Ok(patterns.into_iter().collect())
}

fn read_pnpm_workspace_patterns(root: &Path) -> AppResult<Vec<String>> {
    let path = root.join("pnpm-workspace.yaml");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path)
        .map_err(|error| invalid(format!("Cannot read {}: {error}", path.to_string_lossy())))?;
    parse_pnpm_workspace_patterns(&contents)
        .map_err(|error| invalid(format!("Invalid {}: {error}", path.to_string_lossy())))
}

fn parse_pnpm_workspace_patterns(contents: &str) -> AppResult<Vec<String>> {
    let mut patterns = BTreeSet::new();
    let mut in_packages = false;

    for (line_index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indentation = line.len() - trimmed.len();

        if !in_packages {
            if indentation != 0 {
                continue;
            }
            let declaration = strip_yaml_comment(trimmed).trim();
            if declaration == "packages:" {
                in_packages = true;
            } else if let Some(value) = declaration.strip_prefix("packages:") {
                for pattern in parse_inline_yaml_list(value.trim(), line_index + 1)? {
                    validate_workspace_pattern(&pattern, line_index + 1)?;
                    patterns.insert(pattern);
                }
                return Ok(patterns.into_iter().collect());
            }
            continue;
        }

        if indentation == 0 {
            break;
        }
        let item = strip_yaml_comment(trimmed).trim();
        if item.is_empty() {
            continue;
        }
        let Some(value) = item.strip_prefix('-') else {
            return Err(invalid(format!(
                "line {} must be a list item under 'packages:'",
                line_index + 1
            )));
        };
        let pattern = parse_yaml_string(value.trim(), line_index + 1)?;
        validate_workspace_pattern(&pattern, line_index + 1)?;
        patterns.insert(pattern);
    }

    Ok(patterns.into_iter().collect())
}

fn parse_inline_yaml_list(value: &str, line: usize) -> AppResult<Vec<String>> {
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(invalid(format!(
            "line {line} must use a block or inline list under 'packages:'"
        )));
    }
    let inner = &value[1..value.len() - 1];
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let mut current = String::new();
    let mut characters = inner.chars().peekable();
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    while let Some(character) = characters.next() {
        if double_quoted {
            current.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            current.push(character);
            if character == '\'' {
                if characters.peek() == Some(&'\'') {
                    current.push(characters.next().expect("peeked quote exists"));
                } else {
                    single_quoted = false;
                }
            }
            continue;
        }
        match character {
            '\'' => {
                single_quoted = true;
                current.push(character);
            }
            '"' => {
                double_quoted = true;
                current.push(character);
            }
            ',' => {
                items.push(parse_yaml_string(current.trim(), line)?);
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if single_quoted || double_quoted || escaped {
        return Err(invalid(format!(
            "line {line} contains an unterminated quoted workspace pattern"
        )));
    }
    items.push(parse_yaml_string(current.trim(), line)?);
    Ok(items)
}

fn strip_yaml_comment(value: &str) -> &str {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut previous_was_space = true;

    for (index, character) in value.char_indices() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
        } else if single_quoted {
            if character == '\'' {
                single_quoted = false;
            }
        } else {
            match character {
                '\'' => single_quoted = true,
                '"' => double_quoted = true,
                '#' if previous_was_space => return &value[..index],
                _ => {}
            }
        }
        previous_was_space = character.is_whitespace();
    }
    value
}

fn parse_yaml_string(value: &str, line: usize) -> AppResult<String> {
    if value.is_empty() {
        return Err(invalid(format!(
            "line {line} contains an empty workspace pattern"
        )));
    }
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value).map_err(|error| {
            invalid(format!(
                "line {line} contains an invalid double-quoted pattern: {error}"
            ))
        });
    }
    if value.starts_with('\'') {
        if value.len() < 2 || !value.ends_with('\'') {
            return Err(invalid(format!(
                "line {line} contains an unterminated single-quoted pattern"
            )));
        }
        return Ok(value[1..value.len() - 1].replace("''", "'"));
    }
    Ok(value.to_owned())
}

fn validate_workspace_pattern(pattern: &str, line: usize) -> AppResult<()> {
    let path_pattern = pattern.strip_prefix('!').unwrap_or(pattern);
    if path_pattern.is_empty() {
        return Err(invalid(format!(
            "line {line} contains an empty workspace pattern"
        )));
    }
    let path = Path::new(path_pattern);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(invalid(format!(
            "line {line} workspace pattern must stay inside the project directory"
        )));
    }
    Pattern::new(path_pattern).map_err(|error| {
        invalid(format!(
            "line {line} contains an invalid workspace pattern: {error}"
        ))
    })?;
    Ok(())
}

fn executable_name(package_manager: &str) -> String {
    let suffix = if cfg!(windows) { ".cmd" } else { "" };
    format!("{package_manager}{suffix}")
}

fn script_priority(name: &str) -> (u8, String) {
    let rank = match name {
        "dev" => 0,
        "start" => 1,
        "serve" => 2,
        "preview" => 3,
        _ => 4,
    };
    (rank, name.to_owned())
}

fn is_default_launch_script(name: &str) -> bool {
    matches!(name, "dev" | "start" | "serve" | "preview")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discovers_root_and_workspace_scripts() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("packages").join("app");
        fs::create_dir_all(&app).unwrap();
        fs::write(temp.path().join("pnpm-lock.yaml"), "lockfileVersion: 9").unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"suite","workspaces":["packages/*"],"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            app.join("package.json"),
            r#"{"name":"web","scripts":{"start":"node server.js"}}"#,
        )
        .unwrap();

        let result = discover(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(result.name, "suite");
        assert_eq!(result.package_manager, "pnpm");
        assert_eq!(result.profiles.len(), 2);
        assert!(result
            .profiles
            .iter()
            .any(|profile| profile.name == "web: start"));
    }

    #[test]
    fn discovers_pnpm_workspace_scripts_with_quotes_comments_and_exclusions() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("packages").join("web");
        let ignored = temp.path().join("packages").join("ignored");
        let fixture = temp
            .path()
            .join("packages")
            .join("web")
            .join("fixtures")
            .join("demo");
        let api = temp.path().join("services").join("api");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&ignored).unwrap();
        fs::create_dir_all(&fixture).unwrap();
        fs::create_dir_all(&api).unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"pnpm-suite","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("pnpm-workspace.yaml"),
            r#"packages:
  - 'packages/*' # single-quoted include
  - 'packages/**/fixtures/*'
  - "services/*" # double-quoted include
  - '!packages/ignored' # excluded package
  - '!**/fixtures/**'

catalog:
  react: 19.0.0
"#,
        )
        .unwrap();
        fs::write(
            web.join("package.json"),
            r#"{"name":"web","scripts":{"start":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            ignored.join("package.json"),
            r#"{"name":"ignored","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            fixture.join("package.json"),
            r#"{"name":"fixture","scripts":{"preview":"vite preview"}}"#,
        )
        .unwrap();
        fs::write(
            api.join("package.json"),
            r#"{"name":"api","scripts":{"serve":"node server.js"}}"#,
        )
        .unwrap();

        let result = discover(temp.path().to_str().unwrap()).unwrap();
        let profile_names = result
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(result.package_manager, "npm");
        assert_eq!(
            result.workspace_patterns,
            vec![
                "!**/fixtures/**",
                "!packages/ignored",
                "packages/*",
                "packages/**/fixtures/*",
                "services/*"
            ]
        );
        assert_eq!(
            profile_names,
            BTreeSet::from(["api: serve", "dev", "web: start"])
        );
    }

    #[test]
    fn discovers_pnpm_workspace_scripts_from_an_inline_package_list() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("apps").join("web");
        let ignored = temp.path().join("apps").join("ignored");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&ignored).unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name":"suite"}"#).unwrap();
        fs::write(
            temp.path().join("pnpm-workspace.yaml"),
            "packages: ['apps/*', '!apps/ignored'] # inline list\n",
        )
        .unwrap();
        fs::write(
            web.join("package.json"),
            r#"{"name":"web","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            ignored.join("package.json"),
            r#"{"name":"ignored","scripts":{"start":"node server.js"}}"#,
        )
        .unwrap();

        let result = discover(temp.path().to_str().unwrap()).unwrap();

        assert_eq!(result.workspace_patterns, vec!["!apps/ignored", "apps/*"]);
        assert_eq!(result.profiles.len(), 1);
        assert_eq!(result.profiles[0].name, "web: dev");
    }

    #[test]
    fn only_discovers_exact_service_scripts_for_root_and_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("packages").join("app");
        fs::create_dir_all(&app).unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{
                "name":"suite",
                "workspaces":["packages/*"],
                "scripts":{
                    "dev":"vite",
                    "start":"node server.js",
                    "serve":"vite preview",
                    "preview":"vite preview",
                    "dev:web":"vite",
                    "deploy":"cloud deploy",
                    "db:migrate":"database migrate",
                    "kill":"taskkill /f",
                    "test":"vitest",
                    "build":"vite build"
                }
            }"#,
        )
        .unwrap();
        fs::write(
            app.join("package.json"),
            r#"{
                "name":"web",
                "scripts":{
                    "start":"node server.js",
                    "preview":"vite preview",
                    "deploy":"cloud deploy",
                    "test":"vitest"
                }
            }"#,
        )
        .unwrap();

        let result = discover(temp.path().to_str().unwrap()).unwrap();
        let names = result
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            names,
            BTreeSet::from([
                "dev",
                "preview",
                "serve",
                "start",
                "web: preview",
                "web: start",
            ])
        );
    }

    #[test]
    fn development_root_does_not_offer_non_service_scripts_as_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let maintenance = temp.path().join("maintenance");
        let web = temp.path().join("web");
        fs::create_dir_all(&maintenance).unwrap();
        fs::create_dir_all(&web).unwrap();
        fs::write(
            maintenance.join("package.json"),
            r#"{"name":"maintenance","scripts":{"deploy":"cloud deploy","db":"database console"}}"#,
        )
        .unwrap();
        fs::write(
            web.join("package.json"),
            r#"{"name":"web","scripts":{"dev":"vite","build":"vite build"}}"#,
        )
        .unwrap();

        let projects = scan_development_root(temp.path().to_str().unwrap()).unwrap();
        let maintenance = projects
            .iter()
            .find(|project| project.name == "maintenance")
            .unwrap();
        let web = projects
            .iter()
            .find(|project| project.name == "web")
            .unwrap();

        assert!(maintenance.profiles.is_empty());
        assert_eq!(web.profiles.len(), 1);
        assert_eq!(web.profiles[0].name, "dev");
    }

    #[test]
    fn rejects_directory_without_manifest() {
        let temp = tempfile::tempdir().unwrap();
        assert!(discover(temp.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn scans_multiple_independent_projects_in_stable_order() {
        let temp = tempfile::tempdir().unwrap();
        let alpha = temp.path().join("alpha");
        let nested = temp.path().join("group").join("beta");
        fs::create_dir_all(&alpha).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(alpha.join("package.json"), r#"{"name":"alpha"}"#).unwrap();
        fs::write(nested.join("package.json"), r#"{"name":"beta"}"#).unwrap();

        let projects = scan_development_root(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(
            projects
                .iter()
                .map(|project| project.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
    }

    #[test]
    fn deep_unrelated_tree_does_not_discard_valid_projects() {
        let temp = tempfile::tempdir().unwrap();
        let valid = temp.path().join("web");
        fs::create_dir_all(&valid).unwrap();
        fs::write(
            valid.join("package.json"),
            r#"{"name":"web","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();

        let mut deep = temp.path().join("archive");
        for level in 0..=MAX_SCAN_DEPTH {
            deep = deep.join(format!("level-{level}"));
        }
        fs::create_dir_all(deep.join("still-deeper")).unwrap();

        let projects = scan_development_root(temp.path().to_str().unwrap()).unwrap();

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "web");
    }

    #[test]
    fn workspace_packages_are_not_returned_as_top_level_projects() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("packages").join("app");
        fs::create_dir_all(&app).unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"suite","workspaces":["packages/*"]}"#,
        )
        .unwrap();
        fs::write(
            app.join("package.json"),
            r#"{"name":"app","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();

        let projects = scan_development_root(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "suite");
        assert!(projects[0]
            .profiles
            .iter()
            .any(|profile| profile.name == "app: dev"));
    }

    #[test]
    fn pnpm_workspace_packages_are_not_top_level_but_exclusions_are() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("packages").join("web");
        let standalone = temp.path().join("packages").join("standalone");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&standalone).unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name":"suite"}"#).unwrap();
        fs::write(
            temp.path().join("pnpm-workspace.yaml"),
            "packages:\n  - packages/*\n  - !packages/standalone # independent project\n",
        )
        .unwrap();
        fs::write(
            web.join("package.json"),
            r#"{"name":"web","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            standalone.join("package.json"),
            r#"{"name":"standalone","scripts":{"start":"node server.js"}}"#,
        )
        .unwrap();

        let projects = scan_development_root(temp.path().to_str().unwrap()).unwrap();

        assert_eq!(
            projects
                .iter()
                .map(|project| project.name.as_str())
                .collect::<Vec<_>>(),
            vec!["suite", "standalone"]
        );
        let suite = projects
            .iter()
            .find(|project| project.name == "suite")
            .unwrap();
        assert!(suite
            .profiles
            .iter()
            .any(|profile| profile.name == "web: dev"));
        assert!(!suite
            .profiles
            .iter()
            .any(|profile| profile.name == "standalone: start"));
    }

    #[test]
    fn ignored_directories_are_not_scanned() {
        let temp = tempfile::tempdir().unwrap();
        for ignored in ["node_modules", ".git", "target", "dist", ".venv", "venv"] {
            let project = temp.path().join(ignored).join("hidden");
            fs::create_dir_all(&project).unwrap();
            fs::write(
                project.join("package.json"),
                format!(r#"{{"name":"{ignored}"}}"#),
            )
            .unwrap();
        }

        assert!(scan_development_root(temp.path().to_str().unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn empty_development_root_returns_no_projects() {
        let temp = tempfile::tempdir().unwrap();
        assert!(scan_development_root(temp.path().to_str().unwrap())
            .unwrap()
            .is_empty());
    }
}
