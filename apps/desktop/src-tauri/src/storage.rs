use crate::archive::{ArchiveCounters, ArchiveIndex, ArchiveReason, ArchiveRow, ArchiveStatus};
use crate::error::{invalid, AppResult};
use crate::models::{
    AppSettings, AssociationSource, ExpectedPort, LaunchProfile, PortAssociation, Project,
    ProjectInput, RestoreSet, RunLogArchiveSummary, RunSession, RunStatus,
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
            "SELECT session.id, session.profile_id, session.profile_name, session.pid,
                    session.started_at, session.ended_at, session.exit_code, session.status,
                    archive.status, archive.reason, archive.line_count, archive.byte_size,
                    archive.dropped_lines, archive.dropped_bytes, archive.started_at,
                    archive.ended_at
             FROM run_sessions AS session
             LEFT JOIN run_log_archives AS archive ON archive.session_id = session.id
             ORDER BY session.started_at DESC, session.rowid DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            // `session_id` is the archive's primary key, so the join adds at most
            // one row per session. A null status means there is no archive row.
            let archive = match row.get::<_, Option<String>>(8)? {
                Some(status) => Some(RunLogArchiveSummary {
                    status,
                    reason: row.get(9)?,
                    line_count: row.get(10)?,
                    byte_size: row.get(11)?,
                    dropped_lines: row.get(12)?,
                    dropped_bytes: row.get(13)?,
                    started_at: row.get(14)?,
                    ended_at: row.get(15)?,
                }),
                None => None,
            };
            Ok(RunSession {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                profile_name: row.get(2)?,
                pid: row.get::<_, Option<i64>>(3)?.map(|value| value as u32),
                started_at: row.get(4)?,
                ended_at: row.get(5)?,
                exit_code: row.get(6)?,
                status: row.get(7)?,
                archive,
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

/// The run log archive's index, backed by the version 2 `run_log_archives` table.
///
/// Every method takes the connection mutex for exactly one statement and has
/// released it by the time it returns, so the archive writer — which calls these
/// with no file handle borrowed and none of its own state locks held — never makes
/// another RunCove thread wait on the database for the length of a disk write.
/// Nothing here opens, reads, or removes a file: the writer owns the bytes and
/// this owns the rows.
///
/// The `WHERE` clauses carry a status guard on purpose. An update that matches no
/// row is reported instead of passing silently, because every caller here believes
/// something about the row it is changing — that this session has an open `writing`
/// row, or that its archive still exists — and a write that quietly changed nothing
/// would leave that belief wrong with no way to notice. The column-level rules are
/// left to the table's own `CHECK` constraints rather than restated here, so there
/// is one place to keep in step; only the two things the schema cannot express are
/// checked in Rust.
impl ArchiveIndex for Storage {
    fn insert_writing(&self, session_id: &str, file_name: &str, started_at: i64) -> AppResult<()> {
        // A plain insert, never an upsert: the writer generates a session id once
        // per run and creates the file with `create_new`, so a second `writing` row
        // for the same session is a bug, and letting it overwrite the first row's
        // file name would point the index at an archive nobody is writing to.
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "INSERT INTO run_log_archives
                   (session_id, file_name, status, reason, line_count, byte_size,
                    dropped_lines, dropped_bytes, started_at, ended_at)
                 VALUES (?1, ?2, 'writing', NULL, 0, 0, 0, 0, ?3, NULL)",
                params![session_id, file_name, started_at],
            )?;
        Ok(())
    }

    fn update_counters(&self, session_id: &str, counters: ArchiveCounters) -> AppResult<()> {
        let changed = self
            .connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "UPDATE run_log_archives
                    SET line_count=?2, byte_size=?3, dropped_lines=?4, dropped_bytes=?5
                  WHERE session_id=?1 AND status='writing'",
                params![
                    session_id,
                    counters.line_count,
                    counters.byte_size,
                    counters.dropped_lines,
                    counters.dropped_bytes,
                ],
            )?;
        expect_open_archive_row(changed, session_id, "refresh the counters of")
    }

    fn close(
        &self,
        session_id: &str,
        status: ArchiveStatus,
        reason: Option<ArchiveReason>,
        counters: ArchiveCounters,
        ended_at: i64,
    ) -> AppResult<()> {
        // The one rule the table cannot state: `close` ends an archive, and the two
        // states an ended archive can be in are `complete` and `partial`. `writing`
        // and `removed` are refused here rather than reaching a `CHECK` that would
        // report them as a database error, or — for `removed` with a removal reason —
        // accept them and lose the archive without freeing a byte.
        if !matches!(status, ArchiveStatus::Complete | ArchiveStatus::Partial) {
            return Err(invalid(format!(
                "An archive closes as complete or partial, not as {}",
                status.as_str()
            )));
        }
        let changed = self
            .connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "UPDATE run_log_archives
                    SET status=?2, reason=?3, line_count=?4, byte_size=?5,
                        dropped_lines=?6, dropped_bytes=?7, ended_at=?8
                  WHERE session_id=?1 AND status='writing'",
                params![
                    session_id,
                    status.as_str(),
                    reason.map(ArchiveReason::as_str),
                    counters.line_count,
                    counters.byte_size,
                    counters.dropped_lines,
                    counters.dropped_bytes,
                    ended_at,
                ],
            )?;
        expect_open_archive_row(changed, session_id, "close")
    }

    fn mark_removed(
        &self,
        session_id: &str,
        reason: ArchiveReason,
        ended_at: i64,
    ) -> AppResult<()> {
        // The counters stay: an evicted archive that says "42 lines, reclaimed to
        // stay under the size limit" tells the user what they lost, and zeroing them
        // would make it indistinguishable from a run that printed nothing. They are
        // not the quota's numbers — the writer credits the bytes it measured on disk
        // — so keeping them here cannot hold space against the cap.
        //
        // `ended_at` is overwritten even for a row that already had one, because for
        // a removed archive the useful moment is when it stopped existing, and the
        // run's own end time is still on its `run_sessions` row.
        let changed = self
            .connection
            .lock()
            .expect("database mutex poisoned")
            .execute(
                "UPDATE run_log_archives
                    SET status='removed', reason=?2, ended_at=?3
                  WHERE session_id=?1 AND status<>'removed'",
                params![session_id, reason.as_str(), ended_at],
            )?;
        if changed == 0 {
            return Err(invalid(format!(
                "There is no archive left to remove for session {session_id}"
            )));
        }
        Ok(())
    }

    fn rows(&self) -> AppResult<Vec<ArchiveRow>> {
        let connection = self.connection.lock().expect("database mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT session_id, file_name, status, reason, line_count, byte_size,
                    dropped_lines, dropped_bytes, started_at, ended_at
             FROM run_log_archives ORDER BY started_at, session_id",
        )?;
        let rows = statement.query_map([], read_archive_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn row(&self, session_id: &str) -> AppResult<Option<ArchiveRow>> {
        self.connection
            .lock()
            .expect("database mutex poisoned")
            .query_row(
                "SELECT session_id, file_name, status, reason, line_count, byte_size,
                        dropped_lines, dropped_bytes, started_at, ended_at
                 FROM run_log_archives WHERE session_id=?1",
                [session_id],
                read_archive_row,
            )
            .optional()
            .map_err(Into::into)
    }
}

/// One `run_log_archives` row, in the column order every statement above selects.
///
/// `status` and `reason` are read as the strings they are stored as, without
/// parsing: a database a newer build has written may hold values this one does not
/// know, and the sweep has to be able to report such a row instead of failing to
/// read the whole index.
fn read_archive_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveRow> {
    Ok(ArchiveRow {
        session_id: row.get(0)?,
        file_name: row.get(1)?,
        status: row.get(2)?,
        reason: row.get(3)?,
        counters: ArchiveCounters {
            line_count: row.get(4)?,
            byte_size: row.get(5)?,
            dropped_lines: row.get(6)?,
            dropped_bytes: row.get(7)?,
        },
        started_at: row.get(8)?,
        ended_at: row.get(9)?,
    })
}

/// Turn "the update matched nothing" into an error naming what was attempted.
///
/// Both callers are writing to a row they believe is open, and both are reached
/// only while this build holds that session's archive file. So a miss means the row
/// was closed, removed, or never inserted behind the writer's back — a state worth
/// a failed pump and a `partial` close, not a silent no-op that leaves the row
/// claiming numbers from minutes ago.
fn expect_open_archive_row(changed: usize, session_id: &str, attempt: &str) -> AppResult<()> {
    if changed == 0 {
        return Err(invalid(format!(
            "Session {session_id} has no open run log archive to {attempt}"
        )));
    }
    Ok(())
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

/// The schema version this build writes and is willing to open.
const SCHEMA_VERSION: i64 = 2;

fn migrate(connection: &mut Connection) -> AppResult<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
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
    if version <= 1 {
        upgrade_to_version_2(connection)?;
    }
    Ok(())
}

/// Adds the run log archive index. One transaction, `user_version` last: if any
/// statement fails the whole upgrade rolls back and the database stays at
/// version 1, openable by this build and by v0.2.1.
///
/// The other direction is not symmetric. Once this commits, the database is at
/// version 2 and v0.2.1 refuses to open it, because that build rejects any
/// version above 1. There is no downgrade path and this is not a rollback.
///
/// `CREATE TABLE` deliberately omits `IF NOT EXISTS`: a pre-existing object with
/// this name is a database this build does not understand, and adopting it
/// silently would be worse than failing.
fn upgrade_to_version_2(connection: &mut Connection) -> AppResult<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE run_log_archives (
            session_id    TEXT PRIMARY KEY REFERENCES run_sessions(id) ON DELETE CASCADE,
            file_name     TEXT NOT NULL,
            status        TEXT NOT NULL
                          CHECK(status IN ('writing','complete','partial','removed')),
            reason        TEXT,
            line_count    INTEGER NOT NULL DEFAULT 0 CHECK(line_count >= 0),
            byte_size     INTEGER NOT NULL DEFAULT 0 CHECK(byte_size >= 0),
            dropped_lines INTEGER NOT NULL DEFAULT 0 CHECK(dropped_lines >= 0),
            dropped_bytes INTEGER NOT NULL DEFAULT 0 CHECK(dropped_bytes >= 0),
            started_at    INTEGER NOT NULL,
            ended_at      INTEGER,
            CHECK ((status IN ('writing','complete') AND reason IS NULL)
                OR (status = 'partial' AND reason IS NOT NULL AND reason IN
                     ('write-error','quota-exceeded','queue-overflow','interrupted',
                      'user-disabled'))
                OR (status = 'removed' AND reason IS NOT NULL AND reason IN
                     ('quota-evicted','user-deleted','file-missing'))),
            CHECK ((status = 'writing') = (ended_at IS NULL)),
            -- Every archived byte belongs to some line, so bytes cannot be lost
            -- without losing a line. The converse is false and must stay legal:
            -- a captured line may be empty, so dropping one costs 1 line and
            -- 0 bytes.
            CHECK (dropped_bytes = 0 OR dropped_lines > 0)
         );
         CREATE INDEX idx_run_log_archives_status_ended
            ON run_log_archives (status, ended_at);
         PRAGMA user_version=2;",
    )?;
    transaction.commit()?;
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

    /// The schema version this build must produce. Deliberately a literal rather
    /// than a production constant, so bumping the constant cannot make the
    /// migration tests pass on its own.
    const CURRENT_SCHEMA_VERSION: i64 = 2;

    fn read_schema_version(connection: &Connection) -> i64 {
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    fn schema_version_of(storage: &Storage) -> i64 {
        read_schema_version(&storage.connection.lock().unwrap())
    }

    #[test]
    fn migration_is_idempotent_and_sets_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.db");
        Storage::open(&path).unwrap();
        let storage = Storage::open(&path).unwrap();
        assert_eq!(schema_version_of(&storage), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn language_preference_persists_in_existing_settings_row() {
        let storage = Storage::in_memory().unwrap();
        let version_before = schema_version_of(&storage);
        let mut settings = storage.settings().unwrap();
        assert_eq!(settings.language_preference, LanguagePreference::System);

        settings.language_preference = LanguagePreference::SimplifiedChinese;
        storage.save_settings(&settings).unwrap();

        let reloaded = storage.settings().unwrap();
        assert_eq!(
            reloaded.language_preference,
            LanguagePreference::SimplifiedChinese
        );
        assert_eq!(schema_version_of(&storage), version_before);
    }

    #[test]
    fn close_behavior_persists_without_a_schema_change() {
        let storage = Storage::in_memory().unwrap();
        let version_before = schema_version_of(&storage);
        let mut settings = storage.settings().unwrap();
        assert_eq!(settings.close_behavior, CloseBehavior::Ask);

        settings.close_behavior = CloseBehavior::HideToTray;
        storage.save_settings(&settings).unwrap();

        assert_eq!(
            storage.settings().unwrap().close_behavior,
            CloseBehavior::HideToTray
        );
        assert_eq!(schema_version_of(&storage), version_before);
    }

    #[test]
    fn recent_development_root_persists_without_a_schema_change() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::in_memory().unwrap();
        let version_before = schema_version_of(&storage);
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
        assert_eq!(schema_version_of(&storage), version_before);
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

    // -----------------------------------------------------------------------
    // Schema version 1 -> 2 migration.
    //
    // `V1_SCHEMA` is a pinned copy of the version 1 schema as shipped in
    // v0.2.1. It is the historical database these tests upgrade from, so it
    // must never be edited to follow the production migration.
    // -----------------------------------------------------------------------
    const V1_SCHEMA: &str = r#"
        CREATE TABLE projects (
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
           ('settings', '{"pollIntervalMs":2000,"logCapacity":2000}');
        PRAGMA user_version=1;
"#;

    /// A version 1 database with real user data: two profiles, expected ports, a
    /// confirmed association, three sessions (one still `running`, one orphaned
    /// by a deleted profile), an ordered restore set, and non-default settings.
    const V1_FIXTURE: &str = r#"
        INSERT INTO projects (id, name, path, created_at, updated_at) VALUES
           ('proj-1', 'Legacy Web App', 'D:\legacy\web', 1000, 2000);
        INSERT INTO launch_profiles (id, project_id, name, program, args_json, cwd, sort_order) VALUES
           ('prof-1', 'proj-1', 'dev', 'npm', '["run","dev"]', 'D:\legacy\web', 0),
           ('prof-2', 'proj-1', 'build', 'npm', '["run","build"]', 'D:\legacy\web', 1);
        INSERT INTO expected_ports (id, profile_id, port, protocol) VALUES
           ('port-1', 'prof-1', 5173, 'tcp'),
           ('port-2', 'prof-1', 3000, 'tcp');
        INSERT INTO port_associations
           (id, project_id, profile_id, port, protocol, source, first_seen_at, last_seen_at) VALUES
           ('assoc-1', 'proj-1', 'prof-1', 5173, 'tcp', 'confirmed', 3000, 4000);
        INSERT INTO run_sessions
           (id, profile_id, profile_name, pid, started_at, ended_at, exit_code, status) VALUES
           ('sess-exited', 'prof-1', 'dev', 4242, 10000, 11000, 0, 'exited'),
           ('sess-running', 'prof-1', 'dev', 4243, 20000, NULL, NULL, 'running'),
           ('sess-orphan', NULL, 'removed profile', NULL, 5000, 6000, NULL, 'interrupted');
        INSERT INTO restore_set (position, profile_id) VALUES (0, 'prof-2'), (1, 'prof-1');
        UPDATE app_settings SET value = '{"pollIntervalMs":3500,"logCapacity":1234,"languagePreference":"zh-CN","recentDevelopmentRoot":"D:\\legacy","closeBehavior":"hideToTray"}'
           WHERE key = 'settings';
        INSERT INTO app_settings (key, value) VALUES ('restore_saved_at', '4242');
"#;
    /// The version 2 addition, duplicated from the plan on purpose: it pins the
    /// target shape independently of the production migration, so drift between
    /// the two fails a test instead of silently redefining the schema.
    const V2_ADDITION: &str = r#"
        CREATE TABLE run_log_archives (
          session_id    TEXT PRIMARY KEY REFERENCES run_sessions(id) ON DELETE CASCADE,
          file_name     TEXT NOT NULL,
          status        TEXT NOT NULL
                        CHECK(status IN ('writing','complete','partial','removed')),
          reason        TEXT,
          line_count    INTEGER NOT NULL DEFAULT 0 CHECK(line_count >= 0),
          byte_size     INTEGER NOT NULL DEFAULT 0 CHECK(byte_size >= 0),
          dropped_lines INTEGER NOT NULL DEFAULT 0 CHECK(dropped_lines >= 0),
          dropped_bytes INTEGER NOT NULL DEFAULT 0 CHECK(dropped_bytes >= 0),
          started_at    INTEGER NOT NULL,
          ended_at      INTEGER,
          CHECK ((status IN ('writing','complete') AND reason IS NULL)
              OR (status = 'partial' AND reason IS NOT NULL AND reason IN
                   ('write-error','quota-exceeded','queue-overflow','interrupted',
                    'user-disabled'))
              OR (status = 'removed' AND reason IS NOT NULL AND reason IN
                   ('quota-evicted','user-deleted','file-missing'))),
          CHECK ((status = 'writing') = (ended_at IS NULL)),
          -- Every archived byte belongs to some line, so bytes cannot be lost
          -- without losing a line. The converse is false and must stay legal:
          -- a captured line may be empty, so dropping one costs 1 line and
          -- 0 bytes.
          CHECK (dropped_bytes = 0 OR dropped_lines > 0)
        );
        CREATE INDEX idx_run_log_archives_status_ended
          ON run_log_archives (status, ended_at);
"#;

    fn open_raw(path: &Path) -> Connection {
        let connection = Connection::open(path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
    }

    /// Creates a populated version 1 database without going through `Storage`,
    /// so the fixture cannot be quietly upgraded by the code under test.
    fn create_version_1_database(path: &Path) {
        let connection = open_raw(path);
        connection.execute_batch(V1_SCHEMA).unwrap();
        connection.execute_batch(V1_FIXTURE).unwrap();
        assert_eq!(read_schema_version(&connection), 1);
    }

    fn object_exists(connection: &Connection, kind: &str, name: &str) -> bool {
        connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2",
                [kind, name],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap()
            .is_some()
    }

    fn column_names(connection: &Connection, table: &str) -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .unwrap();
        let rows = statement
            .query_map([table], |row| row.get::<_, String>(0))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }
    #[test]
    fn a_populated_version_1_database_upgrades_to_version_2() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);

        let storage = Storage::open(&path).unwrap();

        assert_eq!(schema_version_of(&storage), CURRENT_SCHEMA_VERSION);
        let connection = storage.connection.lock().unwrap();
        assert!(object_exists(&connection, "table", "run_log_archives"));
        assert!(object_exists(
            &connection,
            "index",
            "idx_run_log_archives_status_ended"
        ));
        assert_eq!(
            column_names(&connection, "run_log_archives").join(","),
            "session_id,file_name,status,reason,line_count,byte_size,\
             dropped_lines,dropped_bytes,started_at,ended_at"
        );
        // The upgrade only adds a table. It must not invent archive rows for
        // history that was recorded before archiving existed.
        let archive_rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM run_log_archives", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(archive_rows, 0);
    }
    #[test]
    fn version_1_user_data_survives_the_upgrade() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);

        let storage = Storage::open(&path).unwrap();

        let projects = storage.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.id, "proj-1");
        assert_eq!(project.name, "Legacy Web App");
        assert_eq!(project.path, r"D:\legacy\web");
        assert_eq!(
            project
                .profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["dev", "build"]
        );
        assert_eq!(project.profiles[0].args, ["run", "dev"]);
        let mut ports = project.profiles[0]
            .expected_ports
            .iter()
            .map(|port| port.port)
            .collect::<Vec<_>>();
        ports.sort_unstable();
        assert_eq!(ports, [3000, 5173]);

        let associations = storage.list_associations().unwrap();
        assert_eq!(associations.len(), 1);
        assert_eq!(associations[0].source, AssociationSource::Confirmed);
        assert_eq!(associations[0].port, 5173);

        let restore = storage.restore_set().unwrap();
        assert_eq!(restore.profile_ids, ["prof-2", "prof-1"]);
        assert_eq!(restore.saved_at, Some(4242));
        let settings = storage.settings().unwrap();
        assert_eq!(settings.poll_interval_ms, 3500);
        assert_eq!(settings.log_capacity, 1234);
        assert_eq!(
            settings.language_preference,
            LanguagePreference::SimplifiedChinese
        );
        assert_eq!(settings.close_behavior, CloseBehavior::HideToTray);
        assert_eq!(
            settings.recent_development_root.as_deref(),
            Some(r"D:\legacy")
        );

        let sessions = storage.list_sessions(10).unwrap();
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-running", "sess-exited", "sess-orphan"]
        );
        let exited = sessions
            .iter()
            .find(|session| session.id == "sess-exited")
            .unwrap();
        assert_eq!(exited.pid, Some(4242));
        assert_eq!(exited.exit_code, Some(0));
        assert_eq!(exited.status, "exited");
        // Recovery still runs after the upgrade: the session that was running
        // when the previous build died is closed out, not left as `running`.
        let running = sessions
            .iter()
            .find(|session| session.id == "sess-running")
            .unwrap();
        assert_eq!(running.status, "interrupted");
        assert!(running.ended_at.is_some());
    }
    #[test]
    fn reopening_an_upgraded_database_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);
        drop(Storage::open(&path).unwrap());

        // A row written by the first run must survive every later open: the
        // upgrade may not re-create or clear the table it already created.
        {
            let connection = open_raw(&path);
            connection
                .execute(
                    "INSERT INTO run_log_archives
                       (session_id, file_name, status, reason, line_count, byte_size,
                        dropped_lines, dropped_bytes, started_at, ended_at)
                     VALUES ('sess-exited', 'sess-exited.jsonl', 'complete', NULL,
                             12, 480, 0, 0, 10000, 11000)",
                    [],
                )
                .unwrap();
        }

        let storage = Storage::open(&path).unwrap();

        assert_eq!(schema_version_of(&storage), CURRENT_SCHEMA_VERSION);
        let connection = storage.connection.lock().unwrap();
        let summary: (i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(line_count), 0) FROM run_log_archives",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(summary, (1, 12));
    }
    #[test]
    fn a_version_2_database_opens_and_only_a_higher_version_is_rejected() {
        let temp = tempfile::tempdir().unwrap();

        let current = temp.path().join("current.sqlite3");
        create_version_1_database(&current);
        {
            let connection = open_raw(&current);
            connection.execute_batch(V2_ADDITION).unwrap();
            connection.pragma_update(None, "user_version", 2).unwrap();
        }
        let storage = Storage::open(&current).unwrap();
        assert_eq!(schema_version_of(&storage), CURRENT_SCHEMA_VERSION);
        drop(storage);

        let future = temp.path().join("future.sqlite3");
        create_version_1_database(&future);
        {
            let connection = open_raw(&future);
            connection.execute_batch(V2_ADDITION).unwrap();
            connection.pragma_update(None, "user_version", 3).unwrap();
        }
        let error = match Storage::open(&future) {
            Ok(_) => panic!("a database newer than version 2 must be rejected"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("newer than this app supports"),
            "unexpected error: {error}"
        );
        // Rejecting must be read-only: a newer database is not downgraded.
        assert_eq!(read_schema_version(&open_raw(&future)), 3);
    }
    #[test]
    fn a_failed_migration_leaves_the_version_1_database_intact() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);
        // Occupying the name the migration must create is the only way to force
        // a mid-migration failure without adding a test-only seam to `migrate`.
        {
            let connection = open_raw(&path);
            connection
                .execute_batch(
                    "CREATE TABLE run_log_archives (session_id TEXT PRIMARY KEY, junk TEXT);",
                )
                .unwrap();
        }

        assert!(
            Storage::open(&path).is_err(),
            "a migration that cannot create its table must fail, not half-apply"
        );

        let connection = open_raw(&path);
        assert_eq!(read_schema_version(&connection), 1);
        assert!(!object_exists(
            &connection,
            "index",
            "idx_run_log_archives_status_ended"
        ));
        assert_eq!(
            column_names(&connection, "run_log_archives").join(","),
            "session_id,junk"
        );
        let sessions: i64 = connection
            .query_row("SELECT COUNT(*) FROM run_sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 3);
        connection
            .execute_batch("DROP TABLE run_log_archives;")
            .unwrap();
        drop(connection);

        // With the conflict gone the upgrade proceeds, proving the failed
        // attempt left the file usable rather than wedged.
        let storage = Storage::open(&path).unwrap();
        assert_eq!(schema_version_of(&storage), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn the_archive_index_rejects_impossible_rows() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);
        let storage = Storage::open(&path).unwrap();
        let connection = storage.connection.lock().unwrap();
        // Without this guard every rejection below would pass vacuously on a
        // database that has no archive table at all.
        assert!(object_exists(&connection, "table", "run_log_archives"));
        let insert = |values: &str| {
            connection.execute(
                &format!(
                    "INSERT INTO run_log_archives
                       (session_id, file_name, status, reason, line_count, byte_size,
                        dropped_lines, dropped_bytes, started_at, ended_at)
                     VALUES {values}"
                ),
                [],
            )
        };

        for (case, values) in [
            (
                "writing must mean not ended",
                "('sess-exited','a.jsonl','writing',NULL,0,0,0,0,10,11)",
            ),
            (
                "complete must not carry a reason",
                "('sess-exited','a.jsonl','complete','interrupted',0,0,0,0,10,11)",
            ),
            (
                "partial must carry a reason",
                "('sess-exited','a.jsonl','partial',NULL,0,0,0,0,10,11)",
            ),
            (
                "removed must carry a reason",
                "('sess-exited','a.jsonl','removed',NULL,0,0,0,0,10,11)",
            ),
            (
                "partial must not borrow a removal reason",
                "('sess-exited','a.jsonl','partial','quota-evicted',0,0,0,0,10,11)",
            ),
            (
                "status is closed",
                "('sess-exited','a.jsonl','bogus',NULL,0,0,0,0,10,11)",
            ),
            // A `"drop counters agree"` case rejecting `5 lines / 0 bytes` used to
            // sit here. That row is ordinary data, because a captured line may be
            // empty and dropping it costs no bytes. The two directions are pinned
            // instead by `an_archive_row_may_lose_a_line_that_carried_no_bytes` and
            // `an_archive_row_may_not_lose_bytes_without_losing_a_line`, which hold
            // the migrated and the pinned schema to the same rule.
            (
                "counters are not negative",
                "('sess-exited','a.jsonl','complete',NULL,-1,0,0,0,10,11)",
            ),
            (
                "the session must exist",
                "('sess-missing','a.jsonl','complete',NULL,0,0,0,0,10,11)",
            ),
        ] {
            assert!(insert(values).is_err(), "{case}: {values} was accepted");
        }

        insert("('sess-exited','sess-exited.jsonl','partial','write-error',3,90,1,40,10,11)")
            .expect("a well-formed partial archive row must be accepted");
    }

    /// Inserts one `run_log_archives` row from a literal VALUES tuple, so a test
    /// can state which shapes the version 2 constraints accept and which they
    /// refuse without repeating the column list each time.
    fn insert_archive_row(connection: &Connection, values: &str) -> rusqlite::Result<usize> {
        connection.execute(
            &format!(
                "INSERT INTO run_log_archives
                   (session_id, file_name, status, reason, line_count, byte_size,
                    dropped_lines, dropped_bytes, started_at, ended_at)
                 VALUES {values}"
            ),
            [],
        )
    }

    /// A version 2 database built straight from `V2_ADDITION`, so a test can hold
    /// the pinned copy of the schema to the same rules as the production
    /// migration and catch a change that lands on only one of the two.
    fn create_pinned_version_2_database(path: &Path) -> Connection {
        create_version_1_database(path);
        let connection = open_raw(path);
        connection.execute_batch(V2_ADDITION).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection
    }

    #[test]
    fn an_archive_row_may_lose_a_line_that_carried_no_bytes() {
        // `capture_stream` turns a lone newline into a real log event whose line
        // is empty, so a dropped record can cost one line and zero bytes. Five
        // dropped empty lines are just as possible, which is why the `5 lines /
        // 0 bytes` case in `the_archive_index_rejects_impossible_rows` is not
        // impossible data.
        let rows = [
            (
                "one dropped empty line",
                "('sess-exited','sess-exited.jsonl','partial','queue-overflow',2,60,1,0,10,11)",
            ),
            (
                "five dropped empty lines",
                "('sess-running','sess-running.jsonl','partial','queue-overflow',2,60,5,0,20,21)",
            ),
        ];

        let temp = tempfile::tempdir().unwrap();

        let migrated = temp.path().join("migrated.sqlite3");
        create_version_1_database(&migrated);
        let storage = Storage::open(&migrated).unwrap();
        let connection = storage.connection.lock().unwrap();
        // Without this guard the inserts below would pass vacuously on a
        // database that has no archive table at all.
        assert!(object_exists(&connection, "table", "run_log_archives"));
        for (case, values) in rows {
            insert_archive_row(&connection, values).unwrap_or_else(|error| {
                panic!("migrated schema, {case}: {values} was rejected: {error}")
            });
        }
        drop(connection);
        drop(storage);

        // The same rule must hold for the pinned copy of the schema, otherwise a
        // correction could land on one definition and not the other.
        let pinned = temp.path().join("pinned.sqlite3");
        let connection = create_pinned_version_2_database(&pinned);
        for (case, values) in rows {
            insert_archive_row(&connection, values).unwrap_or_else(|error| {
                panic!("pinned schema, {case}: {values} was rejected: {error}")
            });
        }
    }

    #[test]
    fn an_archive_row_may_not_lose_bytes_without_losing_a_line() {
        // The genuinely impossible direction: text that never was a line cannot
        // have cost bytes. Pinning it keeps a fix for the empty-line case from
        // discarding the relationship between the two counters altogether.
        let rows = [
            (
                "bytes without a line",
                "('sess-exited','a.jsonl','partial','queue-overflow',2,60,0,40,10,11)",
            ),
            (
                "a negative dropped line count",
                "('sess-exited','a.jsonl','partial','queue-overflow',2,60,-1,40,10,11)",
            ),
            (
                "a negative dropped byte count",
                "('sess-exited','a.jsonl','partial','queue-overflow',2,60,1,-1,10,11)",
            ),
        ];

        let temp = tempfile::tempdir().unwrap();

        let migrated = temp.path().join("migrated.sqlite3");
        create_version_1_database(&migrated);
        let storage = Storage::open(&migrated).unwrap();
        let connection = storage.connection.lock().unwrap();
        assert!(object_exists(&connection, "table", "run_log_archives"));
        for (case, values) in rows {
            assert!(
                insert_archive_row(&connection, values).is_err(),
                "migrated schema, {case}: {values} was accepted"
            );
        }
        drop(connection);
        drop(storage);

        let pinned = temp.path().join("pinned.sqlite3");
        let connection = create_pinned_version_2_database(&pinned);
        for (case, values) in rows {
            assert!(
                insert_archive_row(&connection, values).is_err(),
                "pinned schema, {case}: {values} was accepted"
            );
        }
    }

    #[test]
    fn run_history_reports_the_archive_summary_when_one_exists() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);
        let storage = Storage::open(&path).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO run_log_archives
                   (session_id, file_name, status, reason, line_count, byte_size,
                    dropped_lines, dropped_bytes, started_at, ended_at)
                 VALUES ('sess-exited','sess-exited.jsonl','partial','write-error',
                         42, 4096, 7, 350, 10000, 11000)",
                [],
            )
            .unwrap();

        let sessions = storage.list_sessions(10).unwrap();

        // The join must not change the order or the row count.
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-running", "sess-exited", "sess-orphan"]
        );
        let archived = sessions
            .iter()
            .find(|session| session.id == "sess-exited")
            .unwrap();
        let archive = archived
            .archive
            .as_ref()
            .expect("the archive row must be reported");
        assert_eq!(archive.status, "partial");
        assert_eq!(archive.reason.as_deref(), Some("write-error"));
        assert_eq!(archive.line_count, 42);
        assert_eq!(archive.byte_size, 4096);
        assert_eq!(archive.dropped_lines, 7);
        assert_eq!(archive.dropped_bytes, 350);
        // The archive carries its own timestamps, not the session's.
        assert_eq!(archive.started_at, 10_000);
        assert_eq!(archive.ended_at, Some(11_000));
        // Sessions with no row report no archive rather than an empty one.
        assert_eq!(
            sessions
                .iter()
                .filter(|session| session.archive.is_none())
                .count(),
            2
        );
        assert_eq!(storage.list_sessions(1).unwrap().len(), 1);
    }

    /// A version 2 database carrying the fixture's three sessions, so an archive
    /// row inserted by these tests has a session for its foreign key to point at.
    fn archive_index_storage(temp: &tempfile::TempDir) -> Storage {
        let path = temp.path().join("runcove.sqlite3");
        create_version_1_database(&path);
        Storage::open(&path).unwrap()
    }

    /// One more ended `run_sessions` row. A test that walks every reason needs one
    /// session per reason, because `session_id` is the archive's primary key.
    fn seed_session(storage: &Storage, session_id: &str) {
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO run_sessions
                   (id, profile_id, profile_name, pid, started_at, ended_at, exit_code, status)
                 VALUES (?1, 'prof-1', 'dev', NULL, 30000, 31000, 0, 'exited')",
                [session_id],
            )
            .unwrap();
    }

    fn sample_counters() -> ArchiveCounters {
        ArchiveCounters {
            line_count: 12,
            byte_size: 3456,
            dropped_lines: 2,
            dropped_bytes: 80,
        }
    }

    #[test]
    fn the_storage_archive_index_moves_one_row_from_writing_to_complete() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);

        storage
            .insert_writing("sess-exited", "sess-exited.jsonl", 10_500)
            .unwrap();

        let opened = storage
            .row("sess-exited")
            .unwrap()
            .expect("the writing row");
        assert_eq!(
            opened,
            ArchiveRow {
                session_id: "sess-exited".into(),
                file_name: "sess-exited.jsonl".into(),
                status: "writing".into(),
                reason: None,
                counters: ArchiveCounters::default(),
                started_at: 10_500,
                ended_at: None,
            }
        );

        storage
            .update_counters("sess-exited", sample_counters())
            .unwrap();
        let refreshed = storage.row("sess-exited").unwrap().expect("the same row");
        assert_eq!(refreshed.counters, sample_counters());
        // A refresh touches the counters and nothing else: the row is still open,
        // still names the same file, and still remembers when it opened.
        assert_eq!(refreshed.status, "writing");
        assert_eq!(refreshed.ended_at, None);
        assert_eq!(refreshed.file_name, "sess-exited.jsonl");
        assert_eq!(refreshed.started_at, 10_500);

        storage
            .close(
                "sess-exited",
                ArchiveStatus::Complete,
                None,
                sample_counters(),
                12_000,
            )
            .unwrap();

        let closed = storage.row("sess-exited").unwrap().expect("the closed row");
        assert_eq!(closed.status, "complete");
        assert_eq!(closed.reason, None);
        assert_eq!(closed.counters, sample_counters());
        assert_eq!(closed.ended_at, Some(12_000));
        // The same row throughout, never a second one.
        assert_eq!(storage.rows().unwrap(), vec![closed]);
    }

    #[test]
    fn the_storage_archive_index_refuses_a_second_writing_row_for_one_session() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);
        storage
            .insert_writing("sess-exited", "sess-exited.jsonl", 10_500)
            .unwrap();

        // A different name is the case that matters: an upsert here would point the
        // index at a file no writer holds open.
        assert!(storage
            .insert_writing("sess-exited", "sess-other.jsonl", 99_000)
            .is_err());

        let row = storage.row("sess-exited").unwrap().expect("the first row");
        assert_eq!(row.file_name, "sess-exited.jsonl");
        assert_eq!(row.started_at, 10_500);
        assert_eq!(storage.rows().unwrap().len(), 1);
    }

    #[test]
    fn the_storage_archive_index_refuses_to_write_a_row_it_did_not_leave_open() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);
        storage
            .insert_writing("sess-exited", "sess-exited.jsonl", 10_500)
            .unwrap();
        storage
            .close(
                "sess-exited",
                ArchiveStatus::Partial,
                Some(ArchiveReason::Interrupted),
                sample_counters(),
                12_000,
            )
            .unwrap();
        let closed = storage.row("sess-exited").unwrap().expect("the closed row");

        // A late counter refresh must not resurrect a closed row's numbers, and a
        // second close must not overwrite the first verdict.
        assert!(storage
            .update_counters("sess-exited", ArchiveCounters::default())
            .is_err());
        assert!(storage
            .close(
                "sess-exited",
                ArchiveStatus::Complete,
                None,
                ArchiveCounters::default(),
                13_000,
            )
            .is_err());
        // A session with no archive row at all is the same refusal.
        assert!(storage
            .update_counters("sess-running", sample_counters())
            .is_err());
        assert!(storage
            .close(
                "sess-running",
                ArchiveStatus::Complete,
                None,
                sample_counters(),
                13_000,
            )
            .is_err());

        assert_eq!(storage.row("sess-exited").unwrap(), Some(closed));
        assert_eq!(storage.row("sess-running").unwrap(), None);
        assert_eq!(storage.rows().unwrap().len(), 1);
    }

    #[test]
    fn the_storage_archive_index_closes_only_as_complete_or_partial() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);
        storage
            .insert_writing("sess-exited", "sess-exited.jsonl", 10_500)
            .unwrap();

        for status in [ArchiveStatus::Writing, ArchiveStatus::Removed] {
            let refused = storage
                .close(
                    "sess-exited",
                    status,
                    Some(ArchiveReason::UserDeleted),
                    sample_counters(),
                    12_000,
                )
                .expect_err("a close names an ended archive's two states only");
            assert!(
                refused.to_string().contains(status.as_str()),
                "the refusal must name the status it refused: {refused}"
            );
        }

        // Refused before the statement ran, so the row is still the open one.
        let row = storage
            .row("sess-exited")
            .unwrap()
            .expect("the writing row");
        assert_eq!(row.status, "writing");
        assert_eq!(row.ended_at, None);
        assert_eq!(row.counters, ArchiveCounters::default());
    }

    #[test]
    fn the_storage_archive_index_keeps_what_a_removed_archive_held() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);
        storage
            .insert_writing("sess-exited", "sess-exited.jsonl", 10_500)
            .unwrap();
        storage
            .close(
                "sess-exited",
                ArchiveStatus::Complete,
                None,
                sample_counters(),
                12_000,
            )
            .unwrap();

        storage
            .mark_removed("sess-exited", ArchiveReason::QuotaEvicted, 14_000)
            .unwrap();

        let removed = storage
            .row("sess-exited")
            .unwrap()
            .expect("the removed row");
        assert_eq!(removed.status, "removed");
        assert_eq!(removed.reason.as_deref(), Some("quota-evicted"));
        // What the archive held is why the user can be told what the eviction cost.
        assert_eq!(removed.counters, sample_counters());
        assert_eq!(removed.file_name, "sess-exited.jsonl");
        assert_eq!(removed.started_at, 10_500);
        // The removal time replaces the close time; the run's own end time is still
        // on its session row.
        assert_eq!(removed.ended_at, Some(14_000));

        // Nothing is left to remove, so a second removal is refused rather than
        // rewriting the reason the archive disappeared.
        assert!(storage
            .mark_removed("sess-exited", ArchiveReason::UserDeleted, 15_000)
            .is_err());
        assert!(storage
            .mark_removed("sess-running", ArchiveReason::UserDeleted, 15_000)
            .is_err());
        assert_eq!(storage.row("sess-exited").unwrap(), Some(removed));
    }

    #[test]
    fn the_storage_archive_index_writes_every_reason_the_schema_accepts() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);

        // A `writing` row can be removed without ever being closed: that is how the
        // sweep records an archive whose file went missing between two runs.
        for (index, reason) in [
            ArchiveReason::QuotaEvicted,
            ArchiveReason::UserDeleted,
            ArchiveReason::FileMissing,
        ]
        .into_iter()
        .enumerate()
        {
            let session_id = format!("sess-removed-{index}");
            seed_session(&storage, &session_id);
            storage
                .insert_writing(&session_id, &format!("{session_id}.jsonl"), 30_000)
                .unwrap();
            storage.mark_removed(&session_id, reason, 31_000).unwrap();
            let row = storage.row(&session_id).unwrap().expect("the removed row");
            assert_eq!(row.status, "removed");
            assert_eq!(row.reason.as_deref(), Some(reason.as_str()));
        }

        for (index, reason) in [
            ArchiveReason::WriteError,
            ArchiveReason::QuotaExceeded,
            ArchiveReason::QueueOverflow,
            ArchiveReason::Interrupted,
            ArchiveReason::UserDisabled,
        ]
        .into_iter()
        .enumerate()
        {
            let session_id = format!("sess-partial-{index}");
            seed_session(&storage, &session_id);
            storage
                .insert_writing(&session_id, &format!("{session_id}.jsonl"), 30_000)
                .unwrap();
            storage
                .close(
                    &session_id,
                    ArchiveStatus::Partial,
                    Some(reason),
                    sample_counters(),
                    31_000,
                )
                .unwrap();
            let row = storage.row(&session_id).unwrap().expect("the partial row");
            assert_eq!(row.status, "partial");
            assert_eq!(row.reason.as_deref(), Some(reason.as_str()));
        }

        // Every row this build can write is readable back, and `rows` reports them
        // all rather than the newest few.
        assert_eq!(storage.rows().unwrap().len(), 8);
        assert!(storage.row("sess-exited").unwrap().is_none());
    }

    #[test]
    fn the_storage_archive_index_reports_the_rows_in_a_stable_order() {
        let temp = tempfile::tempdir().unwrap();
        let storage = archive_index_storage(&temp);
        assert!(storage.rows().unwrap().is_empty());

        storage
            .insert_writing("sess-running", "sess-running.jsonl", 20_000)
            .unwrap();
        storage
            .insert_writing("sess-orphan", "sess-orphan.jsonl", 5_000)
            .unwrap();
        storage
            .insert_writing("sess-exited", "sess-exited.jsonl", 10_000)
            .unwrap();

        assert_eq!(
            storage
                .rows()
                .unwrap()
                .iter()
                .map(|row| row.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-orphan", "sess-exited", "sess-running"]
        );
    }
}
