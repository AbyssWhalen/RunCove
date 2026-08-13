import { describe, expect, it } from "vitest";

import { renderMessage } from "./context";
import type { MessageKey, MessageParams } from "./messages";

function expectBilingual(
  key: MessageKey,
  english: string,
  chinese: string,
  params: MessageParams = {},
) {
  expect(renderMessage("en", key, params)).toBe(english);
  expect(renderMessage("zh-CN", key, params)).toBe(chinese);
}

describe("localized message formatting", () => {
  it("uses the correct English singular and plural conflict labels", () => {
    expect(renderMessage("en", "count.conflicts", { count: 1 })).toBe("1 conflict");
    expect(renderMessage("en", "count.conflicts", { count: 2 })).toBe("2 conflicts");
  });

  it("provides Chinese fallbacks for clipboard and package-manager failures", () => {
    expect(renderMessage("zh-CN", "logs.copyUnavailable")).toBe("无法复制日志：当前无法访问剪贴板");
    expect(renderMessage("zh-CN", "logs.loadFailed", { detail: "IPC 不可用" })).toBe(
      "无法加载日志：IPC 不可用",
    );
    expect(renderMessage("zh-CN", "project.unknownPackageManager")).toBe("未知包管理器");
  });

  it("explains that a tray language failure happens after the preference is saved", () => {
    expect(renderMessage("en", "error.languageTray", { detail: "menu rebuild failed" })).toBe(
      "Language preference was saved, but the system tray could not be updated: menu rebuild failed",
    );
    expect(renderMessage("zh-CN", "error.languageTray", { detail: "菜单重建失败" })).toBe(
      "语言偏好已保存，但无法更新系统托盘：菜单重建失败",
    );
  });

  it("reports partial restore progress in both languages", () => {
    const params = { count: 1, profile: "Astro", detail: "port 4321 is occupied" };
    expect(renderMessage("en", "error.restorePartial", params)).toBe(
      "1 profile was restored before Astro failed: port 4321 is occupied",
    );
    expect(renderMessage("zh-CN", "error.restorePartial", params)).toBe(
      "已成功恢复 1 个配置，随后 Astro 恢复失败：port 4321 is occupied",
    );
  });

  it("provides every run-history label in both languages", () => {
    const messages: Array<[MessageKey, string, string, MessageParams?]> = [
      ["history.title", "Recent runs", "最近运行"],
      ["history.subtitle", "Profile sessions managed by RunCove, newest first.", "RunCove 托管的配置会话，按时间从新到旧排列。"],
      ["history.viewAll", "View all", "查看全部"],
      ["history.refresh", "Refresh history", "刷新历史"],
      ["history.refreshing", "Refreshing...", "正在刷新..."],
      ["history.retry", "Retry", "重试"],
      ["history.loading", "Loading run history...", "正在加载运行历史..."],
      ["history.loadError", "Run history could not be loaded: database unavailable", "无法加载运行历史：database unavailable", { detail: "database unavailable" }],
      ["history.empty", "No run history yet", "暂无运行历史"],
      ["history.noMatches", "No run history matches the current filters", "没有符合当前筛选条件的运行历史"],
      ["history.search", "Search project or profile", "搜索项目或配置"],
      ["history.clearSearch", "Clear history search", "清除历史搜索"],
      ["history.filterLabel", "Run history status filter", "运行历史状态筛选"],
      ["history.filter.all", "All", "全部"],
      ["history.filter.active", "Active", "活动"],
      ["history.filter.exited", "Exited", "已退出"],
      ["history.filter.interrupted", "Interrupted", "已中断"],
      ["history.project", "Project", "项目"],
      ["history.profile", "Profile", "配置"],
      ["history.pid", "PID", "PID"],
      ["history.started", "Started", "开始时间"],
      ["history.ended", "Ended", "结束时间"],
      ["history.duration", "Duration", "持续时间"],
      ["history.exitCode", "Exit code", "退出码"],
      ["history.status", "Status", "状态"],
      ["history.actions", "Actions", "操作"],
      ["history.unavailable", "Unavailable", "不可用"],
      ["history.stillRunning", "Still running", "仍在运行"],
      ["history.projectDeleted", "Project deleted", "项目已删除"],
      ["history.locateProject", "View Web / dev in Projects", "在项目页查看 Web / dev", { project: "Web", profile: "dev" }],
      ["history.close", "Close run history", "关闭运行历史"],
      ["history.sessionCount", "2 sessions", "2 次会话", { count: 2 }],
      ["history.resultCount", "3 of 8", "显示 3 / 8", { visible: 3, total: 8 }],
      ["history.status.starting", "Starting", "启动中"],
      ["history.status.running", "Running", "运行中"],
      ["history.status.exited", "Exited", "已退出"],
      ["history.status.interrupted", "Interrupted", "已中断"],
      ["history.status.unknown", "Unknown", "未知"],
    ];

    for (const [key, english, chinese, params] of messages) {
      expectBilingual(key, english, chinese, params);
    }
  });

  it("localizes run-history help and project validation copy", () => {
    const messages: Array<[MessageKey, string, string, MessageParams?]> = [
      ["help.topic.history", "Run history", "运行历史"],
      ["help.history.title", "Understand run history", "看懂运行历史"],
      ["help.history.intro", "Run history records the lifecycle of profiles managed by RunCove, without archiving their console output.", "运行历史记录由 RunCove 托管的配置生命周期，但不会归档控制台输出。"],
      ["help.history.item1Title", "Recent and complete history", "最近记录与完整历史"],
      ["help.history.item1Detail", "Overview shows the five most recent sessions. Open View all to search up to 200 stored sessions and filter active, exited, or interrupted runs.", "概览显示最近 5 次会话。点击“查看全部”可以搜索最多 200 条已存记录，并按活动、已退出或中断筛选。"],
      ["help.history.item2Title", "Session details", "会话详情"],
      ["help.history.item2Detail", "Each row shows its project, profile, PID, start and end times, duration, status, and exit code when one is available. Deleted projects remain labeled in history.", "每行显示项目、配置、PID、开始与结束时间、持续时间、状态，以及可用时的退出码。项目删除后，历史记录仍会保留并明确标记。"],
      ["help.history.item3Title", "Conflicts and restore failures", "冲突与恢复失败"],
      ["help.history.item3Detail", "When an expected port is occupied, View occupant refreshes the snapshot and locates that exact port. Restore stops before the next profile after a failure and keeps profiles that already started running.", "预期端口被占用时，“查看占用”会刷新快照并定位到该端口。恢复途中失败时，后续配置不会继续启动，已经成功运行的配置会保留。"],
      ["help.history.item4Title", "Historical logs are not stored", "不会保存历史日志"],
      ["help.history.item4Detail", "Logs stay only in a bounded memory buffer for the current app session. Run history cannot reopen old logs after RunCove exits or a session is cleared.", "日志只存在于当前应用会话的有界内存缓冲中。RunCove 退出或日志被清空后，运行历史无法重新打开旧日志。"],
      ["project.copyProfile", "Copy profile 2", "复制配置 2", { number: 2 }],
      ["project.copySuffix", "Copy", "副本"],
      ["project.validation.required", "This field is required.", "此字段为必填项。"],
      ["project.validation.profilesRequired", "Add at least one launch profile.", "请至少添加一个启动配置。"],
      ["project.validation.argumentRequired", "Arguments cannot be empty. Remove this argument if it is not needed.", "参数不能为空；如果不需要，请移除此参数。"],
      ["project.validation.portRange", "Enter a whole-number port from 1 to 65535.", "请输入 1 到 65535 之间的整数端口。"],
      ["project.validation.duplicatePort", "This port and protocol pair is already listed in this profile.", "此配置中已经存在相同的端口和协议。"],
      ["project.validation.fixErrors", "Review the highlighted fields before saving.", "请检查标出的字段后再保存。"],
    ];

    for (const [key, english, chinese, params] of messages) {
      expectBilingual(key, english, chinese, params);
    }
  });
});
