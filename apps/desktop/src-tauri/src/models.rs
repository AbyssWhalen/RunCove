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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatusEvent {
    pub profile_id: String,
    pub status: RunStatus,
    pub pid: Option<u32>,
    pub message: Option<String>,
    #[serde(default)]
    pub unexpected: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEvent {
    pub profile_id: String,
    pub stream: LogStream,
    pub line: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
        }
    }
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub started_profile_ids: Vec<String>,
    pub failed_profile_id: Option<String>,
    pub error: Option<String>,
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
