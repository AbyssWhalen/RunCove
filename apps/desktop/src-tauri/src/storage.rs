use crate::error::{invalid, AppResult};
use crate::models::{
    AppSettings, AssociationSource, ExpectedPort, LaunchProfile, PortAssociation, Project,
    ProjectInput, RestoreSet, RunSession, RunStatus,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: &Path) -> AppResult<Self> {
        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut connection)?;
        recover_unfinished_sessions(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[cfg(test)]
    fn in_memory() -> AppResult<Self> {
        let mut connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn save_project(&self, input: ProjectInput) -> AppResult<Project> {
        let canonical = Path::new(&input.path)
            .canonicalize()
            .map_err(|error| invalid(format!("Cannot access project path: {error}")))?;
        if !canonical.is_dir() {
            return Err(invalid("Project path must be a directory"));
        }
        if input.name.trim().is_empty() {
            return Err(invalid("Project name cannot be empty"));
        }
        validate_project(&input)?;

        let now = now_ms();
        let project_id = input.id.unwrap_or_else(new_id);
        let mut connection = self.connection.lock().expect("database mutex poisoned");
        let transaction = connection.transaction()?;
        let created_at: Option<i64> = transaction
            .query_row(
                "SELECT created_at FROM projects WHERE id = ?1",
                [&project_id],
                |row| row.get(0),
            )
            .optional()?;
        transaction.execute(
            "INSERT INTO projects (id, name, path, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, path=excluded.path, updated_at=excluded.updated_at",
            params![project_id, input.name.trim(), canonical.to_string_lossy(), created_at.unwrap_or(now)],
        )?;

        let existing_ids = profile_ids_for(&transaction, &project_id)?;
        let mut retained_ids = Vec::new();
        for (sort_order, profile) in input.profiles.into_iter().enumerate() {
            let profile_id = profile.id.unwrap_or_else(new_id);
            let owner: Option<String> = transaction
                .query_row(
                    "SELECT project_id FROM launch_profiles WHERE id=?1",
                    [&profile_id],
                    |row| row.get(0),
                )
                .optional()?;
            if owner.as_deref().is_some_and(|owner| owner != project_id) {
                return Err(invalid("Launch profile ID belongs to another project"));
            }
            transaction.execute(
                "INSERT INTO launch_profiles (id, project_id, name, program, args_json, cwd, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    name=excluded.name, program=excluded.program, args_json=excluded.args_json,
                    cwd=excluded.cwd, sort_order=excluded.sort_order",
                params![
                    profile_id,
                    project_id,
                    profile.name.trim(),
                    profile.program,
                    serde_json::to_string(&profile.args).expect("string vector serializes"),
                    profile.cwd,
                    sort_order as i64,
                ],
            )?;
            transaction.execute(
                "DELETE FROM expected_ports WHERE profile_id=?1",
                [&profile_id],
            )?;
            for port in profile.expected_ports {
                let protocol = normalize_protocol(&port.protocol)?;
                transaction.execute(
                    "INSERT INTO expected_ports (id, profile_id, port, protocol) VALUES (?1, ?2, ?3, ?4)",
                    params![port.id.unwrap_or_else(new_id), profile_id, i64::from(port.port), protocol],
                )?;
            }
            retained_ids.push(profile_id);
        }
        for old_id in existing_ids {
            if !retained_ids.contains(&old_id) {
                transaction.execute("DELETE FROM launch_profiles WHERE id=?1", [&old_id])?;
            }
        }
        transaction.commit()?;
        drop(connection);
        self.get_project(&project_id)?
            .ok_or_else(|| invalid("Saved project could not be loaded"))
    }

    pub fn delete_project(&self, project_id: &str) -> AppResult<()> {
        let changed = self
            .connection
            .lock()
            .expect("database mutex poisoned")
            .execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
        if changed == 0 {
            return Err(invalid("Project not found"));
        }
        Ok(())
    }

    pub fn list_projects(&self) -> AppResult<Vec<Project>> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, name, path, created_at, updated_at FROM projects ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut projects = Vec::new();
        for row in rows {
            let (id, name, path, created_at, updated_at) = row?;
            projects.push(Project {
                profiles: profiles_for(&connection, &id)?,
                id,
                name,
                path,
                created_at,
                updated_at,
            });
        }
        Ok(projects)
    }

    pub fn get_project(&self, project_id: &str) -> AppResult<Option<Project>> {
        Ok(self
            .list_projects()?
            .into_iter()
            .find(|project| project.id == project_id))
    }

    pub fn get_profile(&self, profile_id: &str) -> AppResult<Option<LaunchProfile>> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        profile_by_id(&connection, profile_id)
    }

    pub fn list_associations(&self) -> AppResult<Vec<PortAssociation>> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, project_id, profile_id, port, protocol, source, first_seen_at, last_seen_at
             FROM port_associations ORDER BY last_seen_at DESC",
        )?;
        let rows = statement.query_map([], association_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn upsert_managed_association(
        &self,
        project_id: &str,
        profile_id: &str,
        port: u16,
        protocol: &str,
    ) -> AppResult<()> {
        let now = now_ms();
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "INSERT INTO port_associations
             (id, project_id, profile_id, port, protocol, source, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'managed', ?6, ?6)
             ON CONFLICT(project_id, profile_id, port, protocol)
             DO UPDATE SET source='managed', last_seen_at=excluded.last_seen_at",
                params![
                    new_id(),
                    project_id,
                    profile_id,
                    i64::from(port),
                    normalize_protocol(protocol)?,
                    now
                ],
            )?;
        Ok(())
    }

    pub fn confirm_association(
        &self,
        project_id: &str,
        profile_id: Option<&str>,
        port: u16,
        protocol: &str,
    ) -> AppResult<PortAssociation> {
        let protocol = normalize_protocol(protocol)?;
        let now = now_ms();
        let id = new_id();
        let mut connection = self.connection.lock().expect("database mutex poisoned");
        let transaction = connection.transaction()?;
        let project_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [project_id],
            |row| row.get(0),
        )?;
        if !project_exists {
            return Err(invalid("Project not found"));
        }
        if let Some(profile_id) = profile_id {
            let owner: Option<String> = transaction
                .query_row(
                    "SELECT project_id FROM launch_profiles WHERE id=?1",
                    [profile_id],
                    |row| row.get(0),
                )
                .optional()?;
            if owner.as_deref() != Some(project_id) {
                return Err(invalid("Launch profile does not belong to the project"));
            }
        }
        let first_seen_at = transaction
            .query_row(
                "SELECT first_seen_at FROM port_associations
                 WHERE project_id=?1 AND IFNULL(profile_id, '')=IFNULL(?2, '')
                   AND port=?3 AND protocol=?4
                 ORDER BY first_seen_at LIMIT 1",
                params![project_id, profile_id, i64::from(port), protocol],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(now);
        transaction.execute(
            "DELETE FROM port_associations
             WHERE port=?3 AND protocol=?4
               AND (source='confirmed'
                    OR (project_id=?1 AND IFNULL(profile_id, '')=IFNULL(?2, '')))",
            params![project_id, profile_id, i64::from(port), protocol],
        )?;
        transaction.execute(
            "INSERT INTO port_associations
             (id, project_id, profile_id, port, protocol, source, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'confirmed', ?6, ?7)",
            params![
                id,
                project_id,
                profile_id,
                i64::from(port),
                protocol,
                first_seen_at,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(PortAssociation {
            id,
            project_id: project_id.into(),
            profile_id: profile_id.map(str::to_owned),
            port,
            protocol,
            source: AssociationSource::Confirmed,
            first_seen_at,
            last_seen_at: now,
        })
    }

    pub fn touch_association(&self, association_id: &str) -> AppResult<()> {
        let changed = self
            .connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "UPDATE port_associations SET last_seen_at=?2 WHERE id=?1",
                params![association_id, now_ms()],
            )?;
        if changed == 0 {
            Err(invalid("Port association no longer exists"))
        } else {
            Ok(())
        }
    }

    pub fn begin_session(&self, profile_id: &str, profile_name: &str) -> AppResult<String> {
        let id = new_id();
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "INSERT INTO run_sessions (id, profile_id, profile_name, started_at, status)
             VALUES (?1, ?2, ?3, ?4, 'starting')",
                params![id, profile_id, profile_name, now_ms()],
            )?;
        Ok(id)
    }

    pub fn set_session_pid(&self, session_id: &str, pid: u32) -> AppResult<()> {
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "UPDATE run_sessions SET pid=?2, status='running' WHERE id=?1",
                params![session_id, i64::from(pid)],
            )?;
        Ok(())
    }

    pub fn finish_session(&self, session_id: &str, exit_code: Option<i32>) -> AppResult<()> {
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "UPDATE run_sessions SET ended_at=?2, exit_code=?3, status='exited' WHERE id=?1",
                params![session_id, now_ms(), exit_code],
            )?;
        Ok(())
    }

    pub fn list_sessions(&self, limit: usize) -> AppResult<Vec<RunSession>> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, profile_id, profile_name, pid, started_at, ended_at, exit_code, status
             FROM run_sessions ORDER BY started_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok(RunSession {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                profile_name: row.get(2)?,
                pid: row.get::<_, Option<i64>>(3)?.map(|value| value as u32),
                started_at: row.get(4)?,
                ended_at: row.get(5)?,
                exit_code: row.get(6)?,
                status: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn restore_set(&self) -> AppResult<RestoreSet> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        let mut statement =
            connection.prepare("SELECT profile_id FROM restore_set ORDER BY position")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        let profile_ids = rows.collect::<Result<Vec<_>, _>>()?;
        let saved_at = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key='restore_saved_at'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| value.parse().ok());
        Ok(RestoreSet {
            profile_ids,
            saved_at,
        })
    }

    pub fn save_restore_set(&self, profile_ids: &[String]) -> AppResult<()> {
        let mut connection = self.connection.lock().expect("database mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM restore_set", [])?;
        for (position, profile_id) in profile_ids.iter().enumerate() {
            transaction.execute(
                "INSERT INTO restore_set (position, profile_id) VALUES (?1, ?2)",
                params![position as i64, profile_id],
            )?;
        }
        transaction.execute(
            "INSERT INTO app_settings (key, value) VALUES ('restore_saved_at', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [now_ms().to_string()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn settings(&self) -> AppResult<AppSettings> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|json| serde_json::from_str(&json).map_err(|error| invalid(error.to_string())))
            .unwrap_or_else(|| Ok(AppSettings::default()))
    }

    pub fn save_settings(&self, settings: &AppSettings) -> AppResult<()> {
        let value = serde_json::to_string(settings).map_err(|error| invalid(error.to_string()))?;
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "INSERT INTO app_settings (key, value) VALUES ('settings', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [value],
            )?;
        Ok(())
    }

    pub fn remember_development_root(&self, directory: &Path) -> AppResult<AppSettings> {
        let canonical = directory
            .canonicalize()
            .map_err(|error| invalid(format!("Cannot access development root: {error}")))?;
        if !canonical.is_dir() {
            return Err(invalid("Development root must be a directory"));
        }
        let mut settings = self.settings()?;
        settings.recent_development_root = Some(canonical.to_string_lossy().into_owned());
        self.save_settings(&settings)?;
        Ok(settings)
    }
}

fn profiles_for(connection: &Connection, project_id: &str) -> AppResult<Vec<LaunchProfile>> {
    let mut statement = connection.prepare(
        "SELECT id, project_id, name, program, args_json, cwd
         FROM launch_profiles WHERE project_id=?1 ORDER BY sort_order, name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([project_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut profiles = Vec::new();
    for row in rows {
        let (id, project_id, name, program, args_json, cwd) = row?;
        profiles.push(LaunchProfile {
            expected_ports: expected_ports_for(connection, &id)?,
            id,
            project_id,
            name,
            program,
            args: serde_json::from_str(&args_json).unwrap_or_default(),
            cwd,
            status: RunStatus::Idle,
            pid: None,
        });
    }
    Ok(profiles)
}

fn profile_ids_for(transaction: &Transaction<'_>, project_id: &str) -> AppResult<Vec<String>> {
    let mut statement =
        transaction.prepare("SELECT id FROM launch_profiles WHERE project_id=?1")?;
    let rows = statement.query_map([project_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn profile_by_id(connection: &Connection, profile_id: &str) -> AppResult<Option<LaunchProfile>> {
    let row = connection
        .query_row(
            "SELECT id, project_id, name, program, args_json, cwd FROM launch_profiles WHERE id=?1",
            [profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    row.map(|(id, project_id, name, program, args_json, cwd)| {
        Ok(LaunchProfile {
            expected_ports: expected_ports_for(connection, &id)?,
            id,
            project_id,
            name,
            program,
            args: serde_json::from_str(&args_json).unwrap_or_default(),
            cwd,
            status: RunStatus::Idle,
            pid: None,
        })
    })
    .transpose()
}

fn expected_ports_for(connection: &Connection, profile_id: &str) -> AppResult<Vec<ExpectedPort>> {
    let mut statement = connection.prepare(
        "SELECT id, profile_id, port, protocol FROM expected_ports WHERE profile_id=?1 ORDER BY port",
    )?;
    let rows = statement.query_map([profile_id], |row| {
        Ok(ExpectedPort {
            id: row.get(0)?,
            profile_id: row.get(1)?,
            port: row.get::<_, i64>(2)? as u16,
            protocol: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn association_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PortAssociation> {
    let source: String = row.get(5)?;
    Ok(PortAssociation {
        id: row.get(0)?,
        project_id: row.get(1)?,
        profile_id: row.get(2)?,
        port: row.get::<_, i64>(3)? as u16,
        protocol: row.get(4)?,
        source: match source.as_str() {
            "managed" => AssociationSource::Managed,
            "confirmed" => AssociationSource::Confirmed,
            _ => AssociationSource::Suggested,
        },
        first_seen_at: row.get(6)?,
        last_seen_at: row.get(7)?,
    })
}

fn validate_project(input: &ProjectInput) -> AppResult<()> {
    if input.profiles.is_empty() {
        return Err(invalid("Project must have at least one launch profile"));
    }
    for profile in &input.profiles {
        validate_profile(profile)?;
    }
    Ok(())
}

fn validate_profile(profile: &crate::models::LaunchProfileInput) -> AppResult<()> {
    if profile.name.trim().is_empty() {
        return Err(invalid("Launch profile name cannot be empty"));
    }
    if profile.program.trim().is_empty() {
        return Err(invalid("Launch program cannot be empty"));
    }
    if profile
        .args
        .iter()
        .any(|argument| argument.trim().is_empty())
    {
        return Err(invalid("Launch arguments cannot contain empty items"));
    }
    if !Path::new(&profile.cwd).is_dir() {
        return Err(invalid("Launch working directory does not exist"));
    }
    let mut expected_ports = HashSet::new();
    for expected in &profile.expected_ports {
        if expected.port == 0 {
            return Err(invalid("Expected port must be between 1 and 65535"));
        }
        let protocol = normalize_protocol(&expected.protocol)?;
        if !expected_ports.insert((expected.port, protocol)) {
            return Err(invalid(
                "Expected ports cannot contain duplicate protocol and port pairs",
            ));
        }
    }
    Ok(())
}

fn normalize_protocol(protocol: &str) -> AppResult<String> {
    match protocol.to_ascii_uppercase().as_str() {
        "TCP" => Ok("tcp".into()),
        "UDP" => Ok("udp".into()),
        _ => Err(invalid("Protocol must be TCP or UDP")),
    }
}

fn migrate(connection: &mut Connection) -> AppResult<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 1 {
        return Err(invalid(format!(
            "Database schema version {version} is newer than this app supports"
        )));
    }
    if version == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
             );
             CREATE TABLE launch_profiles (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                name TEXT NOT NULL, program TEXT NOT NULL, args_json TEXT NOT NULL, cwd TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE expected_ports (
                id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES launch_profiles(id) ON DELETE CASCADE,
                port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
                protocol TEXT NOT NULL CHECK(protocol IN ('tcp','udp')),
                UNIQUE(profile_id, port, protocol)
             );
             CREATE TABLE port_associations (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                profile_id TEXT REFERENCES launch_profiles(id) ON DELETE CASCADE,
                port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
                protocol TEXT NOT NULL CHECK(protocol IN ('tcp','udp')),
                source TEXT NOT NULL CHECK(source IN ('managed','confirmed')),
                first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
                UNIQUE(project_id, profile_id, port, protocol)
             );
             CREATE TABLE run_sessions (
                id TEXT PRIMARY KEY, profile_id TEXT REFERENCES launch_profiles(id) ON DELETE SET NULL,
                profile_name TEXT NOT NULL, pid INTEGER, started_at INTEGER NOT NULL, ended_at INTEGER, exit_code INTEGER,
                status TEXT NOT NULL
             );
             CREATE TABLE restore_set (
                position INTEGER PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES launch_profiles(id) ON DELETE CASCADE
             );
             CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO app_settings (key, value) VALUES
                ('settings', '{\"pollIntervalMs\":2000,\"logCapacity\":2000}');
             PRAGMA user_version=1;",
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn recover_unfinished_sessions(connection: &Connection) -> AppResult<()> {
    connection.execute(
        "UPDATE run_sessions SET status='interrupted', ended_at=?1
         WHERE status IN ('starting', 'running') AND ended_at IS NULL",
        [now_ms()],
    )?;
    Ok(())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CloseBehavior, ExpectedPortInput, LanguagePreference, LaunchProfileInput};

    fn sample_project(path: &Path) -> ProjectInput {
        ProjectInput {
            id: None,
            name: "Sample".into(),
            path: path.to_string_lossy().into_owned(),
            profiles: vec![LaunchProfileInput {
                id: None,
                name: "dev".into(),
                program: "npm".into(),
                args: vec!["run".into(), "dev".into()],
                cwd: path.to_string_lossy().into_owned(),
                expected_ports: vec![ExpectedPortInput {
                    id: None,
                    port: 5173,
                    protocol: "tcp".into(),
                }],
            }],
        }
    }

    #[test]
    fn migration_is_idempotent_and_sets_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.db");
        Storage::open(&path).unwrap();
        let storage = Storage::open(&path).unwrap();
        let version: i64 = storage
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn language_preference_persists_in_existing_settings_row() {
        let storage = Storage::in_memory().unwrap();
        let mut settings = storage.settings().unwrap();
        assert_eq!(settings.language_preference, LanguagePreference::System);

        settings.language_preference = LanguagePreference::SimplifiedChinese;
        storage.save_settings(&settings).unwrap();

        let reloaded = storage.settings().unwrap();
        assert_eq!(
            reloaded.language_preference,
            LanguagePreference::SimplifiedChinese
        );
        let schema_version: i64 = storage
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(schema_version, 1);
    }

    #[test]
    fn close_behavior_persists_without_a_schema_change() {
        let storage = Storage::in_memory().unwrap();
        let mut settings = storage.settings().unwrap();
        assert_eq!(settings.close_behavior, CloseBehavior::Ask);

        settings.close_behavior = CloseBehavior::HideToTray;
        storage.save_settings(&settings).unwrap();

        assert_eq!(
            storage.settings().unwrap().close_behavior,
            CloseBehavior::HideToTray
        );
        let schema_version: i64 = storage
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(schema_version, 1);
    }

    #[test]
    fn recent_development_root_persists_without_a_schema_change() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let canonical_text = canonical.to_string_lossy().into_owned();

        let settings = storage.remember_development_root(temp.path()).unwrap();
        assert_eq!(
            settings.recent_development_root.as_deref(),
            Some(canonical_text.as_str())
        );
        assert_eq!(
            storage
                .settings()
                .unwrap()
                .recent_development_root
                .as_deref(),
            settings.recent_development_root.as_deref()
        );
        let schema_version: i64 = storage
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(schema_version, 1);
    }

    #[test]
    fn project_round_trip_and_cascade_delete() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let project = storage.save_project(sample_project(temp.path())).unwrap();
        assert_eq!(project.profiles[0].expected_ports[0].port, 5173);
        assert_eq!(project.profiles[0].expected_ports[0].protocol, "tcp");
        storage.delete_project(&project.id).unwrap();
        assert!(storage.list_projects().unwrap().is_empty());
    }

    #[test]
    fn project_requires_a_valid_launch_profile_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let mut project = sample_project(temp.path());
        project.profiles.clear();
        assert_eq!(
            storage.save_project(project).unwrap_err().to_string(),
            "Project must have at least one launch profile"
        );
        assert!(storage.list_projects().unwrap().is_empty());

        let invalid_profiles = [
            ("name", "Launch profile name cannot be empty"),
            ("program", "Launch program cannot be empty"),
            ("argument", "Launch arguments cannot contain empty items"),
            (
                "duplicate-port",
                "Expected ports cannot contain duplicate protocol and port pairs",
            ),
        ];
        for (invalid_field, expected) in invalid_profiles {
            let mut project = sample_project(temp.path());
            match invalid_field {
                "name" => project.profiles[0].name = "   ".into(),
                "program" => project.profiles[0].program = "  ".into(),
                "argument" => project.profiles[0].args.push("\t".into()),
                "duplicate-port" => {
                    project.profiles[0].expected_ports.push(ExpectedPortInput {
                        id: None,
                        port: 5173,
                        protocol: "TCP".into(),
                    });
                }
                _ => unreachable!(),
            }
            assert_eq!(
                storage.save_project(project).unwrap_err().to_string(),
                expected
            );
            assert!(storage.list_projects().unwrap().is_empty());
        }
    }

    #[test]
    fn project_validation_allows_programs_resolved_through_path() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();

        let project = storage.save_project(sample_project(temp.path())).unwrap();

        assert_eq!(project.profiles[0].program, "npm");
    }

    #[test]
    fn project_validation_rejects_missing_working_directory_and_zero_port() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let mut missing_cwd = sample_project(temp.path());
        missing_cwd.profiles[0].cwd = temp.path().join("missing").display().to_string();
        assert_eq!(
            storage.save_project(missing_cwd).unwrap_err().to_string(),
            "Launch working directory does not exist"
        );

        let mut zero_port = sample_project(temp.path());
        zero_port.profiles[0].expected_ports[0].port = 0;
        assert_eq!(
            storage.save_project(zero_port).unwrap_err().to_string(),
            "Expected port must be between 1 and 65535"
        );
    }

    #[test]
    fn restore_set_preserves_launch_order() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let project = storage.save_project(sample_project(temp.path())).unwrap();
        let ids = vec![project.profiles[0].id.clone()];
        storage.save_restore_set(&ids).unwrap();
        let restore = storage.restore_set().unwrap();
        assert_eq!(restore.profile_ids, ids);
        assert!(restore.saved_at.is_some());
    }

    #[test]
    fn reopening_storage_marks_unfinished_sessions_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.db");
        let storage = Storage::open(&path).unwrap();
        let project = storage.save_project(sample_project(temp.path())).unwrap();
        let starting = storage
            .begin_session(&project.profiles[0].id, &project.profiles[0].name)
            .unwrap();
        let running = storage
            .begin_session(&project.profiles[0].id, &project.profiles[0].name)
            .unwrap();
        storage.set_session_pid(&running, 123).unwrap();
        let exited = storage
            .begin_session(&project.profiles[0].id, &project.profiles[0].name)
            .unwrap();
        storage.finish_session(&exited, Some(0)).unwrap();
        drop(storage);

        let reopened = Storage::open(&path).unwrap();
        let sessions = reopened.list_sessions(10).unwrap();
        let by_id = sessions
            .into_iter()
            .map(|session| (session.id.clone(), session))
            .collect::<std::collections::HashMap<_, _>>();

        for session_id in [starting, running] {
            let session = &by_id[&session_id];
            assert_eq!(session.status, "interrupted");
            assert!(session.ended_at.is_some());
            assert_eq!(session.exit_code, None);
        }
        let completed = &by_id[&exited];
        assert_eq!(completed.status, "exited");
        assert!(completed.ended_at.is_some());
        assert_eq!(completed.exit_code, Some(0));
    }

    #[test]
    fn run_history_is_newest_first_and_honors_the_limit() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let project = storage.save_project(sample_project(temp.path())).unwrap();
        let profile = &project.profiles[0];
        let first = storage.begin_session(&profile.id, &profile.name).unwrap();
        let second = storage.begin_session(&profile.id, &profile.name).unwrap();
        {
            let connection = storage.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE run_sessions SET started_at=10 WHERE id=?1",
                    [&first],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE run_sessions SET started_at=20 WHERE id=?1",
                    [&second],
                )
                .unwrap();
        }

        let sessions = storage.list_sessions(1).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, second);

        let connection = storage.connection.lock().unwrap();
        connection
            .execute("UPDATE run_sessions SET started_at=30", [])
            .unwrap();
        drop(connection);
        let tied_sessions = storage.list_sessions(2).unwrap();
        assert_eq!(
            tied_sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec![second.as_str(), first.as_str()]
        );
    }

    #[test]
    fn removing_profile_preserves_session_history() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let project = storage.save_project(sample_project(temp.path())).unwrap();
        let profile = &project.profiles[0];
        let session = storage.begin_session(&profile.id, &profile.name).unwrap();
        storage.set_session_pid(&session, 123).unwrap();
        storage.finish_session(&session, Some(0)).unwrap();
        let retained_profile = LaunchProfileInput {
            id: None,
            name: "preview".into(),
            program: "npm".into(),
            args: vec!["run".into(), "preview".into()],
            cwd: temp.path().to_string_lossy().into_owned(),
            expected_ports: Vec::new(),
        };
        storage
            .save_project(ProjectInput {
                id: Some(project.id),
                name: "Sample".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![retained_profile],
            })
            .unwrap();

        let sessions = storage.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].profile_id, None);
        assert_eq!(sessions[0].profile_name, "dev");
    }

    #[test]
    fn deleting_project_preserves_orphaned_session_history() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let project = storage.save_project(sample_project(temp.path())).unwrap();
        let profile = &project.profiles[0];
        let session = storage.begin_session(&profile.id, &profile.name).unwrap();
        storage.finish_session(&session, Some(7)).unwrap();

        storage.delete_project(&project.id).unwrap();

        let sessions = storage.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].profile_id, None);
        assert_eq!(sessions[0].profile_name, "dev");
        assert_eq!(sessions[0].exit_code, Some(7));
    }

    #[test]
    fn confirmed_association_validates_profile_ownership_and_replaces_duplicate() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let first = storage
            .save_project(sample_project(first_dir.path()))
            .unwrap();
        let second = storage
            .save_project(sample_project(second_dir.path()))
            .unwrap();
        let profile_id = first.profiles[0].id.as_str();
        let initial = storage
            .confirm_association(&first.id, Some(profile_id), 5173, "TCP")
            .unwrap();
        let repeated = storage
            .confirm_association(&first.id, Some(profile_id), 5173, "tcp")
            .unwrap();
        assert_eq!(repeated.first_seen_at, initial.first_seen_at);
        assert_eq!(storage.list_associations().unwrap().len(), 1);
        assert!(storage
            .confirm_association(&second.id, Some(profile_id), 5173, "tcp")
            .is_err());

        let second_profile_id = second.profiles[0].id.as_str();
        storage
            .confirm_association(&second.id, Some(second_profile_id), 5173, "tcp")
            .unwrap();
        let confirmed = storage
            .list_associations()
            .unwrap()
            .into_iter()
            .filter(|association| association.source == AssociationSource::Confirmed)
            .collect::<Vec<_>>();
        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0].project_id, second.id);
    }

    #[test]
    fn observed_confirmed_association_updates_last_seen_time() {
        let project_dir = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let project = storage
            .save_project(sample_project(project_dir.path()))
            .unwrap();
        let association = storage
            .confirm_association(&project.id, None, 4_321, "tcp")
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));
        storage.touch_association(&association.id).unwrap();

        let updated = storage
            .list_associations()
            .unwrap()
            .into_iter()
            .find(|item| item.id == association.id)
            .unwrap();
        assert!(updated.last_seen_at > association.last_seen_at);
    }
}
