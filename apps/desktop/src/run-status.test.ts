import { describe, expect, it } from "vitest";

import { renderMessage } from "./i18n/context";
import type { MessageKey, MessageParams } from "./i18n/messages";
import { describeRunStatusReason, runStatusText } from "./run-status";

const en = (key: MessageKey, params?: MessageParams) => renderMessage("en", key, params);
const zh = (key: MessageKey, params?: MessageParams) => renderMessage("zh-CN", key, params);

describe("describeRunStatusReason", () => {
  it.each([
    { kind: "stop-requested", english: "Stop requested", chinese: "已请求停止" },
    { kind: "user-stop", english: "Stopped by you", chinese: "已由你停止" },
    { kind: "shutdown", english: "Stopped while RunCove was closing", chinese: "RunCove 关闭时已停止" },
    {
      kind: "startup-not-ready",
      english: "Stopped because startup never became ready",
      chinese: "启动一直未就绪，已停止",
    },
    { kind: "exited-normally", english: "Process exited normally", chinese: "进程正常退出" },
    { kind: "already-running", english: "This profile is already running", chinese: "该配置已在运行" },
  ])("names every reason the backend can send without data: $kind", ({ kind, english, chinese }) => {
    expect(describeRunStatusReason({ kind }, en)).toBe(english);
    expect(describeRunStatusReason({ kind }, zh)).toBe(chinese);
  });

  it("keeps the exit code when there is one and drops the clause when there is not", () => {
    const withCode = { kind: "exited-unexpectedly", code: 7 };
    expect(describeRunStatusReason(withCode, en)).toBe("Process exited unexpectedly with code 7");
    expect(describeRunStatusReason(withCode, zh)).toBe("进程异常退出，退出码 7");

    const withoutCode = { kind: "exited-unexpectedly", code: null };
    expect(describeRunStatusReason(withoutCode, en)).toBe("Process exited unexpectedly");
    expect(describeRunStatusReason(withoutCode, zh)).toBe("进程异常退出");
  });

  it("passes the operating system's own words through a wait failure", () => {
    const reason = { kind: "wait-failed", detail: "Access is denied. (os error 5)" };
    expect(describeRunStatusReason(reason, en)).toBe(
      "Could not wait for the process: Access is denied. (os error 5)",
    );
    expect(describeRunStatusReason(reason, zh)).toBe(
      "无法等待进程结束：Access is denied. (os error 5)",
    );
  });

  it("declines a reason this build cannot render, so the caller can fall back", () => {
    // A newer backend, or a wait failure that lost the one detail explaining it.
    expect(describeRunStatusReason({ kind: "quarantined" }, zh)).toBeNull();
    expect(describeRunStatusReason({ kind: "wait-failed" }, zh)).toBeNull();
    expect(describeRunStatusReason({ kind: "" }, zh)).toBeNull();
    expect(describeRunStatusReason(null, zh)).toBeNull();
    expect(describeRunStatusReason(undefined, zh)).toBeNull();
  });
});

describe("runStatusText", () => {
  it("prefers RunCove's own reason over the English sentence beside it", () => {
    const event = { reason: { kind: "user-stop" }, message: "Stopped by user" };
    expect(runStatusText(event, zh)).toBe("已由你停止");
  });

  it("shows the backend's text when the reason is absent or unknown", () => {
    expect(runStatusText({ message: "Expected port 5173 is occupied" }, zh)).toBe(
      "Expected port 5173 is occupied",
    );
    expect(runStatusText({ reason: { kind: "quarantined" }, message: "Quarantined" }, zh)).toBe(
      "Quarantined",
    );
    expect(runStatusText({}, zh)).toBe("");
  });
});
