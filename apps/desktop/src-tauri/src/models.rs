use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub profiles: Vec<LaunchProfile>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProfile {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub expected_ports: Vec<ExpectedPort>,
    pub status: RunStatus,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedPort {
    pub id: String,
    pub profile_id: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    pub id: Option<String>,
    pub name: String,
    pub path: String,
    pub profiles: Vec<LaunchProfileInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub expected_ports: Vec<ExpectedPortInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedPortInput {
    pub id: Option<String>,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProject {
    pub name: String,
    pub path: String,
    pub package_manager: String,
    pub workspace_patterns: Vec<String>,
    pub profiles: Vec<DiscoveredProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProfile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub expected_ports: Vec<ExpectedPortDraft>,
    pub observed_runtime: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedPortDraft {
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Idle,
    Starting,
    Running,
    Conflict,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RelatedPort {
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatusEvent {
    pub profile_id: String,
    pub status: RunStatus,
    pub pid: Option<u32>,
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_port: Option<RelatedPort>,
    #[serde(default)]
    pub unexpected: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEvent {
    pub profile_id: String,
    pub stream: LogStream,
    pub line: String,
    pub timestamp: i64,
}

/// One run's archive reached its final row.
///
/// A close writes the session's remaining records and syncs the file, so it cannot
/// run inside the managed map's lock and necessarily lands after the exit event the
/// frontend reloads history on. Without this event that reload is the last one, and a
/// finished archive stays on screen as `finalizing` until something else refetches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveClosedEvent {
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortSnapshot {
    pub port: u16,
    pub protocol: String,
    pub state: String,
    pub bind_address: Option<String>,
    pub is_public: bool,
    pub active: bool,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub executable_path: Option<String>,
    pub command_line: Option<String>,
    pub process_started_at: Option<u64>,
    pub last_seen_at: Option<i64>,
    pub project_id: Option<String>,
    pub profile_id: Option<String>,
    pub association_source: Option<AssociationSource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AssociationSource {
    Managed,
    Confirmed,
    Suggested,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortAssociation {
    pub id: String,
    pub project_id: String,
    pub profile_id: Option<String>,
    pub port: u16,
    pub protocol: String,
    pub source: AssociationSource,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// One run log archive as the user interface sees it.
///
/// `status` and `reason` are strings, not enums, for the same reason
/// `RunSession::status` is: a database written by a newer build may carry values
/// this build does not know, and passing them through unchanged is better than
/// failing to deserialize the whole history. The database `CHECK` constraints,
/// not this type, are what keep impossible combinations out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogArchiveSummary {
    pub status: String,
    pub reason: Option<String>,
    pub line_count: i64,
    pub byte_size: i64,
    pub dropped_lines: i64,
    pub dropped_bytes: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSession {
    pub id: String,
    pub profile_id: Option<String>,
    pub profile_name: String,
    pub pid: Option<u32>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub status: String,
    /// `None` when this session has no archive row: archiving was off, or the
    /// session predates the version 2 schema.
    #[serde(default)]
    pub archive: Option<RunLogArchiveSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSet {
    pub profile_ids: Vec<String>,
    pub saved_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub poll_interval_ms: u64,
    pub log_capacity: usize,
    #[serde(default)]
    pub language_preference: LanguagePreference,
    #[serde(default)]
    pub recent_development_root: Option<String>,
    #[serde(default)]
    pub close_behavior: CloseBehavior,
    /// Whether new runs write their output to a plain-text file on disk.
    ///
    /// `#[serde(default)]` over a `bool` means off, and off is the only safe
    /// default: an archive holds whatever the child printed, secrets included, so
    /// nothing is written to disk until the user asks for it. A settings row saved
    /// by v0.2.1 has no such key and therefore reads back as off, which is the same
    /// treatment `close_behavior` and `recent_development_root` got — no migration,
    /// no new column.
    #[serde(default)]
    pub archive_run_logs: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CloseBehavior {
    #[default]
    #[serde(rename = "ask")]
    Ask,
    #[serde(rename = "hideToTray")]
    HideToTray,
    #[serde(rename = "quit")]
    Quit,
}

impl CloseBehavior {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(Self::Ask),
            "hideToTray" => Some(Self::HideToTray),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum LanguagePreference {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

impl LanguagePreference {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "en" => Some(Self::English),
            "zh-CN" => Some(Self::SimplifiedChinese),
            _ => None,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            poll_interval_ms: 2_000,
            log_capacity: 2_000,
            language_preference: LanguagePreference::System,
            recent_development_root: None,
            close_behavior: CloseBehavior::Ask,
            archive_run_logs: false,
        }
    }
}

/// What the run log archive can do right now, as the toggle and the viewer see
/// it.
///
/// `enabled` is what the user asked for and `available` is what this run can
/// actually do, and they are reported separately on purpose: an initialization
/// that failed must not render as "on", because that would imply output is being
/// captured when none is. A disabled feature is available and off; a broken one is
/// unavailable and says why.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogArchiveState {
    pub enabled: bool,
    pub available: bool,
    /// Why the archive cannot run, in the user's words, or `None` when it can.
    pub unavailable_reason: Option<String>,
}

/// One archived record as the viewer receives it.
///
/// The archive stores the decoded text of a `RunLogEvent`, so this is the same
/// three fields the live drawer shows, without the profile id the file's name
/// already implies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogArchiveRecord {
    pub stream: LogStream,
    pub line: String,
    pub timestamp: i64,
}

/// One page of an archive, oldest record first, plus everything the viewer needs
/// to ask for the page before it.
///
/// `file_length` is measured at read time and is exact, while the row counters are
/// as fresh as the writer's last refresh: a page of a session still being written
/// can legitimately hold more lines than `line_count` claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogArchivePage {
    pub session_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub line_count: i64,
    pub byte_size: i64,
    pub dropped_lines: i64,
    pub dropped_bytes: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub records: Vec<RunLogArchiveRecord>,
    /// The measured length of the file this page was read from.
    pub file_length: u64,
    /// Feed this back as `before_offset` to page towards the start.
    pub page_start_offset: u64,
    pub has_more_before: bool,
    /// Which bound ended the page: `lines`, `bytes`, or `start`.
    pub stopped_by: String,
    /// Bytes after the last newline were not a whole record and were skipped.
    pub incomplete_tail_skipped: bool,
    /// Records inside the page that were not readable JSON and were skipped.
    pub malformed_lines: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub ports: Vec<PortSnapshot>,
    pub projects: Vec<Project>,
    pub restore_set: RestoreSet,
    pub settings: AppSettings,
    pub privilege: crate::privileges::PrivilegeStatus,
    pub generated_at: i64,
    pub scan_error: Option<String>,
    /// What the archive can do right now, which `settings.archive_run_logs` alone
    /// cannot say: the stored setting can be on while this run's archive is broken.
    pub run_log_archive: RunLogArchiveState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub started_profile_ids: Vec<String>,
    pub failed_profile_id: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_port: Option<RelatedPort>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProcessRequest {
    pub port: u16,
    pub protocol: String,
    pub pid: u32,
    pub started_at: Option<u64>,
    pub executable_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmAssociationRequest {
    pub port: u16,
    pub protocol: String,
    pub project_id: String,
    pub profile_id: Option<String>,
    pub pid: u32,
    pub started_at: u64,
    pub executable_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_contract_uses_camel_case_and_lowercase_values() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(value["pollIntervalMs"], 2_000);
        assert_eq!(value["languagePreference"], "system");
        assert_eq!(value["closeBehavior"], "ask");
        assert!(value.get("poll_interval_ms").is_none());
        assert_eq!(serde_json::to_value(RunStatus::Running).unwrap(), "running");
        assert_eq!(
            serde_json::to_value(AssociationSource::Managed).unwrap(),
            "managed"
        );

        let port = ExpectedPort {
            id: "port".into(),
            profile_id: "profile".into(),
            port: 5173,
            protocol: "tcp".into(),
        };
        let port = serde_json::to_value(port).unwrap();
        assert_eq!(port["profileId"], "profile");
        assert_eq!(port["protocol"], "tcp");
    }

    #[test]
    fn run_events_use_optional_camel_case_related_port_context() {
        let event = RunStatusEvent {
            profile_id: "profile".into(),
            status: RunStatus::Conflict,
            pid: None,
            message: Some("Expected port 5173 is already occupied".into()),
            related_port: Some(RelatedPort {
                port: 5173,
                protocol: "tcp".into(),
            }),
            unexpected: false,
            timestamp: 1,
        };
        let with_context = serde_json::to_value(event).unwrap();
        assert_eq!(with_context["relatedPort"]["port"], 5173);
        assert_eq!(with_context["relatedPort"]["protocol"], "tcp");
        assert!(with_context.get("related_port").is_none());

        let without_context = serde_json::to_value(RestoreResult {
            started_profile_ids: Vec::new(),
            failed_profile_id: None,
            error: None,
            related_port: None,
        })
        .unwrap();
        assert!(without_context.get("relatedPort").is_none());
    }

    /// The frontend listens for `run-archive-closed` and reads `sessionId` off the
    /// payload to reload history. A rename on this side would leave that listener
    /// reading `undefined`, and a finished archive would keep rendering as
    /// "finalizing" — a silent failure, so the wire name is pinned here.
    #[test]
    fn the_archive_closed_event_names_its_session_in_camel_case() {
        let event = serde_json::to_value(ArchiveClosedEvent {
            session_id: "session".into(),
        })
        .unwrap();

        assert_eq!(event["sessionId"], "session");
        assert!(event.get("session_id").is_none());
    }

    #[test]
    fn settings_without_language_preference_remain_compatible() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"pollIntervalMs":1000,"logCapacity":500}"#).unwrap();

        assert_eq!(settings.language_preference, LanguagePreference::System);
        assert_eq!(settings.recent_development_root, None);
        assert_eq!(settings.close_behavior, CloseBehavior::Ask);
    }

    #[test]
    fn language_preferences_use_stable_wire_values() {
        assert_eq!(
            serde_json::to_value(LanguagePreference::SimplifiedChinese).unwrap(),
            "zh-CN"
        );
        assert_eq!(
            LanguagePreference::parse("zh-CN"),
            Some(LanguagePreference::SimplifiedChinese)
        );
        assert_eq!(LanguagePreference::parse("zh-cn"), None);
    }

    #[test]
    fn close_behaviors_use_stable_wire_values() {
        assert_eq!(
            serde_json::to_value(CloseBehavior::HideToTray).unwrap(),
            "hideToTray"
        );
        assert_eq!(CloseBehavior::parse("ask"), Some(CloseBehavior::Ask));
        assert_eq!(
            CloseBehavior::parse("hideToTray"),
            Some(CloseBehavior::HideToTray)
        );
        assert_eq!(CloseBehavior::parse("quit"), Some(CloseBehavior::Quit));
        assert_eq!(CloseBehavior::parse("hide_to_tray"), None);
    }

    #[test]
    fn process_identity_requests_accept_the_frontend_camel_case_shape() {
        let external: ExternalProcessRequest = serde_json::from_str(
            r#"{"port":5173,"protocol":"tcp","pid":42,"startedAt":1000,"executablePath":"C:\\node.exe"}"#,
        )
        .unwrap();
        assert_eq!(external.port, 5_173);
        assert_eq!(external.protocol, "tcp");
        assert_eq!(external.started_at, Some(1_000));

        let association: ConfirmAssociationRequest = serde_json::from_str(
            r#"{"port":5173,"protocol":"tcp","projectId":"project","profileId":"dev","pid":42,"startedAt":1000,"executablePath":"C:\\node.exe"}"#,
        )
        .unwrap();
        assert_eq!(association.project_id, "project");
        assert_eq!(association.profile_id.as_deref(), Some("dev"));
        assert_eq!(association.executable_path, r"C:\node.exe");
    }
}
