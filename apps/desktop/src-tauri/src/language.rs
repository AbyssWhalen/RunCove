use crate::models::LanguagePreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayLanguage {
    English,
    SimplifiedChinese,
}

#[derive(Debug, Clone, Copy)]
pub struct TrayLabels {
    pub open: &'static str,
    pub restore: &'static str,
    pub restore_monitor_only: &'static str,
    pub stop_all: &'static str,
    pub stop_all_monitor_only: &'static str,
    pub quit: &'static str,
    pub tagline: &'static str,
    pub monitor_only_status: &'static str,
}

pub fn resolve(preference: LanguagePreference) -> DisplayLanguage {
    resolve_for_locale(preference, system_locale().as_deref())
}

fn resolve_for_locale(
    preference: LanguagePreference,
    system_locale: Option<&str>,
) -> DisplayLanguage {
    match preference {
        LanguagePreference::English => DisplayLanguage::English,
        LanguagePreference::SimplifiedChinese => DisplayLanguage::SimplifiedChinese,
        LanguagePreference::System => {
            let is_chinese = system_locale.is_some_and(|locale| {
                locale
                    .replace('_', "-")
                    .to_ascii_lowercase()
                    .starts_with("zh-")
                    || locale.eq_ignore_ascii_case("zh")
            });
            if is_chinese {
                DisplayLanguage::SimplifiedChinese
            } else {
                DisplayLanguage::English
            }
        }
    }
}

pub fn tray_labels(language: DisplayLanguage) -> TrayLabels {
    match language {
        DisplayLanguage::English => TrayLabels {
            open: "Open RunCove",
            restore: "Restore previous run",
            restore_monitor_only: "Restore previous run (monitor-only)",
            stop_all: "Stop all",
            stop_all_monitor_only: "Stop all (monitor-only)",
            quit: "Exit",
            tagline: "RunCove - Local dev services, under control.",
            monitor_only_status: "Administrator monitor-only",
        },
        DisplayLanguage::SimplifiedChinese => TrayLabels {
            open: "打开 RunCove",
            restore: "恢复上次运行",
            restore_monitor_only: "恢复上次运行（仅监控）",
            stop_all: "停止全部",
            stop_all_monitor_only: "停止全部（仅监控）",
            quit: "退出",
            tagline: "RunCove - 本地开发服务，尽在掌控。",
            monitor_only_status: "管理员仅监控模式",
        },
    }
}

pub fn tray_status_text(
    language: DisplayLanguage,
    running: usize,
    conflicts: usize,
    unexpected_exits: usize,
) -> String {
    match language {
        DisplayLanguage::English => format!(
            "{running} running | {conflicts} {} | {unexpected_exits} {}",
            if conflicts == 1 {
                "conflict"
            } else {
                "conflicts"
            },
            if unexpected_exits == 1 {
                "unexpected exit"
            } else {
                "unexpected exits"
            }
        ),
        DisplayLanguage::SimplifiedChinese => {
            format!("运行中 {running} | 冲突 {conflicts} | 异常退出 {unexpected_exits}")
        }
    }
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    // LOCALE_NAME_MAX_LENGTH is 85, including the terminating null.
    let mut buffer = [0_u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if length <= 1 {
        return None;
    }
    String::from_utf16(&buffer[..length as usize - 1]).ok()
}

#[cfg(not(windows))]
fn system_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_override_the_system_locale() {
        assert_eq!(
            resolve_for_locale(LanguagePreference::English, Some("zh-CN")),
            DisplayLanguage::English
        );
        assert_eq!(
            resolve_for_locale(LanguagePreference::SimplifiedChinese, Some("en-US")),
            DisplayLanguage::SimplifiedChinese
        );
    }

    #[test]
    fn system_preference_recognizes_chinese_locale_variants() {
        assert_eq!(
            resolve_for_locale(LanguagePreference::System, Some("zh_CN")),
            DisplayLanguage::SimplifiedChinese
        );
        assert_eq!(
            resolve_for_locale(LanguagePreference::System, Some("en-US")),
            DisplayLanguage::English
        );
        assert_eq!(
            resolve_for_locale(LanguagePreference::System, None),
            DisplayLanguage::English
        );
    }

    #[test]
    fn tray_copy_is_localized_and_english_counts_are_grammatical() {
        let labels = tray_labels(DisplayLanguage::SimplifiedChinese);
        assert_eq!(labels.open, "打开 RunCove");
        assert_eq!(labels.restore, "恢复上次运行");
        assert_eq!(labels.restore_monitor_only, "恢复上次运行（仅监控）");
        assert_eq!(labels.quit, "退出");
        let english_labels = tray_labels(DisplayLanguage::English);
        assert_eq!(english_labels.restore, "Restore previous run");
        assert_eq!(
            tray_status_text(DisplayLanguage::English, 2, 1, 1),
            "2 running | 1 conflict | 1 unexpected exit"
        );
        assert_eq!(
            tray_status_text(DisplayLanguage::SimplifiedChinese, 2, 1, 3),
            "运行中 2 | 冲突 1 | 异常退出 3"
        );
    }
}
