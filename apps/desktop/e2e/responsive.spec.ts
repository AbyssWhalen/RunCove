import { expect, test, type Locator, type Page } from "@playwright/test";

const viewports = [
  { name: "compact", width: 900, height: 600 },
  { name: "desktop", width: 1280, height: 720 },
  { name: "wide", width: 1440, height: 900 },
] as const;

async function expectInsideViewport(page: Page, locator: Locator) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  const viewport = page.viewportSize();
  expect(viewport).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport!.width);
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport!.height);
}

async function expectNoHorizontalOverflow(page: Page) {
  const widths = await page.evaluate(() => ({
    viewport: window.innerWidth,
    document: document.documentElement.scrollWidth,
    body: document.body.scrollWidth,
    content: document.querySelector<HTMLElement>(".content-area")?.scrollWidth ?? 0,
  }));
  expect(widths.document).toBeLessThanOrEqual(widths.viewport);
  expect(widths.body).toBeLessThanOrEqual(widths.viewport);
  expect(widths.content).toBeLessThanOrEqual(widths.viewport);
}

async function openAppAfterEdgeColdStart(initialPage: Page, viewport: { width: number; height: number }) {
  const context = initialPage.context();
  let page = initialPage;
  for (const timeout of [20_000, 60_000]) {
    await page.setViewportSize(viewport);
    try {
      const response = await page.goto("/", { waitUntil: "commit", timeout });
      expect(response?.ok()).toBe(true);
      return page;
    } catch (error) {
      if (page.url() !== "about:blank" || timeout === 60_000) throw error;
      await page.close();
      page = await context.newPage();
    }
  }
  throw new Error("RunCove navigation did not start");
}

function captureBrowserErrors(page: Page): string[] {
  const browserErrors: string[] = [];
  const captureErrors = (candidate: Page) => {
    candidate.on("console", (message) => {
      if (message.type() === "error" || message.type() === "warning") {
        browserErrors.push(`${message.type()}: ${message.text()}`);
      }
    });
    candidate.on("pageerror", (error) => browserErrors.push(`pageerror: ${error.message}`));
  };
  captureErrors(page);
  page.context().on("page", captureErrors);
  return browserErrors;
}

for (const viewport of viewports) {
  test(`${viewport.name} primary workflows stay inside ${viewport.width}x${viewport.height}`, async ({ page: initialPage }) => {
    const browserErrors = captureBrowserErrors(initialPage);
    const page = await openAppAfterEdgeColdStart(initialPage, viewport);

    await expect(page.getByRole("heading", { name: "Launch profiles" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Recent runs" })).toBeVisible();
    await expectNoHorizontalOverflow(page);

    const language = page.locator(".language-picker select");
    await language.selectOption("zh-CN");
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
    await expect(page.getByRole("heading", { name: "启动配置" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "最近运行" })).toBeVisible();
    await expectNoHorizontalOverflow(page);

    await page.getByRole("button", { name: "查看全部" }).click();
    const chineseHistoryDialog = page.getByRole("dialog", { name: "最近运行" });
    await expect(chineseHistoryDialog).toBeVisible();
    await expect(chineseHistoryDialog.getByText("项目已删除", { exact: true })).toBeVisible();
    await chineseHistoryDialog.getByRole("button", { name: "已中断" }).click();
    await expect(chineseHistoryDialog.getByText("Removed service", { exact: true })).toBeVisible();
    await expectInsideViewport(page, chineseHistoryDialog);
    await expectNoHorizontalOverflow(page);
    await chineseHistoryDialog.getByRole("button", { name: "关闭运行历史" }).click();

    await page.getByRole("button", { name: "帮助与使用指南" }).click();
    const chineseHelpDialog = page.getByRole("dialog", { name: "RunCove 使用帮助" });
    await expect(chineseHelpDialog).toBeVisible();
    await chineseHelpDialog.getByRole("tab", { name: "端口" }).click();
    await expect(chineseHelpDialog.getByRole("heading", { name: "看懂端口页面" })).toBeVisible();
    await chineseHelpDialog.getByRole("tab", { name: "运行历史" }).click();
    await expect(chineseHelpDialog.getByRole("heading", { name: "看懂运行历史" })).toBeVisible();
    await expect(chineseHelpDialog.getByRole("heading", { name: "归档历史日志是可选的" })).toBeVisible();
    await expectInsideViewport(page, chineseHelpDialog);
    await expectNoHorizontalOverflow(page);
    await chineseHelpDialog.getByRole("button", { name: "关闭", exact: true }).click();

    await language.selectOption("en");
    await expect(page.getByRole("heading", { name: "Launch profiles" })).toBeVisible();

    await page.getByRole("button", { name: "View all" }).click();
    const historyDialog = page.getByRole("dialog", { name: "Recent runs" });
    await expect(historyDialog).toBeVisible();
    await expect(historyDialog.getByText("Project deleted", { exact: true })).toBeVisible();
    await expectInsideViewport(page, historyDialog);
    await expectNoHorizontalOverflow(page);
    await historyDialog.getByRole("button", { name: "Interrupted" }).click();
    await expect(historyDialog.getByText("Removed service", { exact: true })).toBeVisible();
    await historyDialog.getByRole("button", { name: "Close run history" }).click();
    await expect(historyDialog).not.toBeVisible();

    // The archive viewer is the widest drawer RunCove opens, so the walk proves it
    // stays inside 900x600 as well as that it opens at the end of the file: the mock
    // session holds 30 records and one page is 12.
    await page.getByRole("button", { name: "View archived Web logs" }).click();
    const archiveDialog = page.getByRole("dialog", { name: "Web" });
    await expect(archiveDialog).toBeVisible();
    await expect(archiveDialog).toContainText("12 shown · 30 recorded");
    await expect(archiveDialog).toContainText("This run is still being archived");
    // Still being written, so the file cannot be deleted from here.
    await expect(archiveDialog.getByRole("button", { name: "Delete archived Web logs" })).toHaveCount(0);
    await expectInsideViewport(page, archiveDialog);
    await expectNoHorizontalOverflow(page);
    await archiveDialog.getByRole("button", { name: "Load earlier lines" }).click();
    await expect(archiveDialog).toContainText("24 shown · 30 recorded");
    await expectInsideViewport(page, archiveDialog);
    await archiveDialog.getByRole("button", { name: "Close archived logs" }).click();
    await expect(archiveDialog).not.toBeVisible();

    await page.getByRole("button", { name: "Help and usage guide" }).click();
    const helpDialog = page.getByRole("dialog", { name: "RunCove Help" });
    await expect(helpDialog).toBeVisible();
    await expectInsideViewport(page, helpDialog);
    await expectNoHorizontalOverflow(page);
    await helpDialog.getByRole("tab", { name: "Ports" }).click();
    await expect(helpDialog.getByRole("heading", { name: "Understand the Ports view" })).toBeVisible();
    await expectInsideViewport(page, helpDialog);
    await expectNoHorizontalOverflow(page);
    await helpDialog.getByRole("tab", { name: "Run history" }).click();
    await expect(helpDialog.getByRole("heading", { name: "Understand run history" })).toBeVisible();
    await expect(helpDialog).toContainText("Logs always stay in a bounded memory buffer");
    await expect(helpDialog).toContainText("Turn on Archive run logs in a log drawer");
    await helpDialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(helpDialog).not.toBeVisible();

    await expect(page.getByRole("button", { name: "Hide to system tray", exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Quit RunCove", exact: true })).toHaveCount(0);
    await page.evaluate(() => {
      window.dispatchEvent(new Event("runcove:window-close-choice-requested"));
    });
    const closeChoiceDialog = page.getByRole("dialog", { name: "When closing the window" });
    await expect(closeChoiceDialog).toBeVisible();
    await expectInsideViewport(page, closeChoiceDialog);
    await expectNoHorizontalOverflow(page);
    await expect(closeChoiceDialog.getByRole("button", { name: /Hide to system tray/ })).toBeVisible();
    await expect(closeChoiceDialog.getByRole("button", { name: /Quit RunCove/ })).toBeVisible();
    await expect(
      closeChoiceDialog.getByRole("checkbox", {
        name: "Remember this choice and don't ask again",
      }),
    ).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(closeChoiceDialog).not.toBeVisible();

    await page.getByRole("button", { name: /View Web logs/ }).click();
    const logDrawer = page.getByRole("dialog", { name: "Web" });
    await expect(logDrawer).toBeVisible();
    await expectInsideViewport(page, logDrawer);
    await page.getByRole("button", { name: "Close logs" }).click();

    await page.getByRole("button", { name: "Ports" }).click();
    await expect(page.getByRole("heading", { name: "Ports", level: 1 })).toBeVisible();
    await page.getByRole("button", { name: "View details for port 5173" }).click();
    const portDetails = page.getByRole("region", { name: "Port 5173 details" });
    await expect(portDetails).toBeVisible();
    await portDetails.scrollIntoViewIfNeeded();
    await expectInsideViewport(page, portDetails);
    await expectNoHorizontalOverflow(page);

    await page.getByRole("button", { name: "Projects", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Projects", level: 1 })).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: "Stop every running profile in Abyss Studio before deleting it",
      }),
    ).toBeDisabled();
    await page.getByRole("button", { name: "Delete Docs Lab" }).click();
    const deleteDialog = page.getByRole("alertdialog", { name: "Delete project" });
    await expect(deleteDialog).toBeVisible();
    await expect(deleteDialog).toContainText("files in the project folder will not be deleted");
    await expectInsideViewport(page, deleteDialog);
    await expectNoHorizontalOverflow(page);
    await deleteDialog.getByRole("button", { name: "Cancel" }).click();
    await expect(deleteDialog).not.toBeVisible();
    await page.getByRole("button", { name: "Import project" }).click();
    const projectDialog = page.getByRole("dialog", { name: "Import project" });
    await expect(projectDialog).toBeVisible();
    await expectInsideViewport(page, projectDialog);
    await expectNoHorizontalOverflow(page);

    expect(browserErrors).toEqual([]);
  });
}

test("project editor copies a profile and reports field-level validation", async ({ page: initialPage }) => {
  const browserErrors = captureBrowserErrors(initialPage);
  const page = await openAppAfterEdgeColdStart(initialPage, viewports[1]);

  await page.getByRole("button", { name: "Projects", exact: true }).click();
  await page.getByRole("button", { name: "Edit Docs Lab" }).click();
  const dialog = page.getByRole("dialog", { name: "Edit project" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Copy profile 1" }).click();

  const copiedProfile = dialog.locator(".profile-editor").nth(1);
  await expect(copiedProfile.getByLabel("Name", { exact: true })).toHaveValue("Astro Copy");
  await expect(copiedProfile.getByLabel("Program", { exact: true })).toHaveValue("pnpm.cmd");
  await expect(copiedProfile.getByLabel("Working directory", { exact: true })).toHaveValue("D:\\CodexProject\\personal-projects\\docs-lab");
  await expect(copiedProfile.getByLabel("Profile 2 argument 1")).toHaveValue("dev");
  await expect(copiedProfile.getByLabel("Profile 2 expected port 1")).toHaveValue("4321");

  await copiedProfile.getByLabel("Name", { exact: true }).fill("");
  await copiedProfile.getByRole("button", { name: "Add argument" }).click();
  await copiedProfile.getByRole("button", { name: "Add expected port" }).click();
  await copiedProfile.getByLabel("Profile 2 expected port 2").fill("4321");
  await dialog.getByRole("button", { name: "Save project" }).click();

  await expect(copiedProfile.getByText("This field is required.", { exact: true })).toBeVisible();
  await expect(copiedProfile.getByText("Arguments cannot be empty. Remove this argument if it is not needed.", { exact: true })).toBeVisible();
  await expect(copiedProfile.getByText("This port and protocol pair is already listed in this profile.", { exact: true })).toHaveCount(2);
  await expect(dialog.getByText("Review the highlighted fields before saving.", { exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(browserErrors).toEqual([]);
});

test("port conflict action refreshes and focuses the exact occupant", async ({ page: initialPage }) => {
  const browserErrors = captureBrowserErrors(initialPage);
  const page = await openAppAfterEdgeColdStart(initialPage, viewports[1]);
  await expect(page.locator("html")).toHaveAttribute("data-mock-run-status-ready", "true");

  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent("runcove:mock-run-status", {
      detail: {
        profileId: "profile-docs",
        status: "conflict",
        pid: null,
        message: "Docs Lab / Astro cannot start because TCP port 5432 is occupied",
        relatedPort: { port: 5432, protocol: "tcp" },
        timestamp: Date.now() + 1_000,
      },
    }));
  });

  const conflict = page.getByRole("alert").filter({ hasText: "TCP port 5432 is occupied" });
  await expect(conflict).toBeVisible();
  await conflict.getByRole("button", { name: "View occupant" }).click();
  await expect(page.getByRole("heading", { name: "Ports", level: 1 })).toBeVisible();
  await expect(page.getByPlaceholder("Search ports")).toHaveValue("5432 tcp");
  await expect(page.getByRole("region", { name: "Port 5432 details" })).toBeVisible();
  await expect(page.locator("tr.is-focused-port")).toContainText("5432");
  await expect(page.getByRole("button", { name: "View details for port 5173" })).toHaveCount(0);
  expect(browserErrors).toEqual([]);
});

test("automatic discovery explains a saved-root failure and retries successfully", async ({ page: initialPage }) => {
  await initialPage.addInitScript(() => {
    sessionStorage.setItem("runcove:e2e:saved-root-scan-failure-once", "1");
  });
  const browserErrors = captureBrowserErrors(initialPage);
  const page = await openAppAfterEdgeColdStart(initialPage, viewports[1]);

  await page.getByRole("button", { name: "Projects", exact: true }).click();
  const discoveryError = page.getByRole("alert").filter({ hasText: "The saved development root could not be scanned." });
  await expect(discoveryError).toContainText("Mock saved-root scan failed once");
  await discoveryError.getByRole("button", { name: "Retry" }).click();

  const dialog = page.getByRole("dialog", { name: "Import project" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Signal Console", { exact: true })).toBeVisible();
  await expect(dialog.getByText("Worker Lab", { exact: true })).toBeVisible();
  await expect(dialog.getByRole("checkbox", { name: "Select Signal Console" })).toBeChecked();
  await expect(dialog.getByRole("checkbox", { name: "Select Worker Lab" })).toBeChecked();
  await expectInsideViewport(page, dialog);
  await expectNoHorizontalOverflow(page);
  expect(browserErrors).toEqual([]);
});
