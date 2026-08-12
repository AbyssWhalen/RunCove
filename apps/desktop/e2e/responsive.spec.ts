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

for (const viewport of viewports) {
  test(`${viewport.name} primary workflows stay inside ${viewport.width}x${viewport.height}`, async ({ page: initialPage }) => {
    const browserErrors: string[] = [];
    const captureErrors = (candidate: Page) => {
      candidate.on("console", (message) => {
        if (message.type() === "error" || message.type() === "warning") {
          browserErrors.push(`${message.type()}: ${message.text()}`);
        }
      });
      candidate.on("pageerror", (error) => browserErrors.push(`pageerror: ${error.message}`));
    };
    captureErrors(initialPage);
    initialPage.context().on("page", captureErrors);
    const page = await openAppAfterEdgeColdStart(initialPage, viewport);

    await expect(page.getByRole("heading", { name: "Launch profiles" })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    if (viewport.name === "compact") {
      const language = page.locator(".language-picker select");
      await language.selectOption("zh-CN");
      await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
      await expect(page.getByRole("heading", { name: "启动配置" })).toBeVisible();
      await expectNoHorizontalOverflow(page);
      await language.selectOption("en");
      await expect(page.getByRole("heading", { name: "Launch profiles" })).toBeVisible();
    }

    await page.getByRole("button", { name: "Help and usage guide" }).click();
    const helpDialog = page.getByRole("dialog", { name: "RunCove Help" });
    await expect(helpDialog).toBeVisible();
    await expectInsideViewport(page, helpDialog);
    await expectNoHorizontalOverflow(page);
    await helpDialog.getByRole("tab", { name: "Ports" }).click();
    await expect(helpDialog.getByRole("heading", { name: "Understand the Ports view" })).toBeVisible();
    await expectInsideViewport(page, helpDialog);
    await expectNoHorizontalOverflow(page);
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

    await page.getByRole("button", { name: "Projects" }).click();
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
