import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import { HelpDrawer } from "./HelpDrawer";

function renderHelp(
  initialTopic: React.ComponentProps<typeof HelpDrawer>["initialTopic"],
  language: "en" | "zh-CN" = "en",
  overrides: Partial<React.ComponentProps<typeof HelpDrawer>> = {},
) {
  window.localStorage.setItem("runcove.language", language);
  const props: React.ComponentProps<typeof HelpDrawer> = {
    initialTopic,
    closeBehavior: "ask",
    onClose: vi.fn(),
    onNavigate: vi.fn(),
    onResetCloseBehavior: vi.fn(),
    ...overrides,
  };
  render(<I18nProvider><HelpDrawer {...props} /></I18nProvider>);
  return props;
}

describe("HelpDrawer", () => {
  it("opens on the requested initial topic", () => {
    renderHelp("projects");

    expect(screen.getByRole("dialog")).toHaveAccessibleName("RunCove Help");
    expect(screen.getByRole("tab", { name: "Projects" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Manage projects and launch profiles");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Unknown means readiness cannot be confirmed");
  });

  it("switches topics with tabs and the keyboard", async () => {
    const user = userEvent.setup();
    renderHelp("quickStart");

    const portsTab = screen.getByRole("tab", { name: "Ports" });
    await user.click(portsTab);
    expect(portsTab).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Ports combines live Windows listeners");

    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Projects" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Manage projects and launch profiles");
  });

  it("navigates to the view linked from the active topic", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    renderHelp("ports", "en", { onNavigate });

    await user.click(screen.getByRole("button", { name: "Open Ports" }));
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(onNavigate).toHaveBeenCalledWith("ports");
  });

  it("closes on Escape", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    renderHelp("safety", "en", { onClose });

    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("shows and resets a remembered title-bar close behavior", async () => {
    const user = userEvent.setup();
    const onResetCloseBehavior = vi.fn();
    renderHelp("safety", "en", { closeBehavior: "quit", onResetCloseBehavior });

    expect(screen.getByRole("tabpanel")).toHaveTextContent("Current: Quit RunCove");
    await user.click(screen.getByRole("button", { name: "Ask every time" }));
    expect(onResetCloseBehavior).toHaveBeenCalledOnce();
  });

  it("renders the run-history guidance in Chinese", () => {
    renderHelp("history", "zh-CN");

    expect(screen.getByRole("dialog")).toHaveAccessibleName("RunCove 使用帮助");
    expect(screen.getByRole("tab", { name: "运行历史" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "看懂运行历史" })).toBeVisible();
    expect(screen.getByRole("tabpanel")).toHaveTextContent("冲突与恢复失败");
    // The retired copy promised history logs were never kept. Archiving replaced that
    // promise with an opt-in, so both halves of the new one are pinned here: memory by
    // default, a file only for runs started after the switch is on.
    expect(screen.getByRole("heading", { name: "归档历史日志是可选的" })).toBeVisible();
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      "日志始终保存在当前应用会话的有界内存缓冲中",
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      "在日志抽屉里开启“归档运行日志”后，此后启动的每次运行还会把输出写入文件",
    );
  });
});
