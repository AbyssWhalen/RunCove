import { describe, expect, it } from "vitest";

import { renderMessage } from "./context";

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
});
