import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import type { DiscoveredProject } from "../types";
import { ProjectModal } from "./ProjectModal";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

const safeProject: DiscoveredProject = {
  name: "Web App",
  path: "D:\\projects\\web-app",
  packageManager: "npm",
  workspacePatterns: [],
  profiles: [{
    name: "dev",
    program: "npm.cmd",
    args: ["run", "dev"],
    cwd: "D:\\projects\\web-app",
    expectedPorts: [],
  }],
};

const maintenanceProject: DiscoveredProject = {
  name: "Maintenance",
  path: "D:\\projects\\maintenance",
  packageManager: "npm",
  workspacePatterns: [],
  profiles: [],
};

function renderModal(
  overrides: Partial<React.ComponentProps<typeof ProjectModal>> = {},
  language: "en" | "zh-CN" = "en",
) {
  const props: React.ComponentProps<typeof ProjectModal> = {
    onDiscover: vi.fn(),
    onScanDevelopmentRoot: vi.fn(),
    onPickDirectory: vi.fn().mockResolvedValue(null),
    onSave: vi.fn(),
    onSaveMany: vi.fn(),
    registeredPaths: [],
    onClose: vi.fn(),
    ...overrides,
  };
  window.localStorage.setItem("runcove.language", language);
  render(<I18nProvider><ProjectModal {...props} /></I18nProvider>);
  return props;
}

describe("ProjectModal safe discovery imports", () => {
  it("marks runtime-observed launch details for review", async () => {
    const user = userEvent.setup();
    const observedProject: DiscoveredProject = {
      ...safeProject,
      profiles: [{
        ...safeProject.profiles[0],
        args: ["run", "dev", "--port", "3100"],
        expectedPorts: [{ port: 3100, protocol: "tcp" }],
        observedRuntime: true,
      }],
    };
    renderModal({ onDiscover: vi.fn().mockResolvedValue(observedProject) });

    await user.type(screen.getByLabelText("Project directory"), observedProject.path);
    await user.click(screen.getByRole("button", { name: "Inspect directory" }));

    expect(await screen.findByText("Observed")).toBeInTheDocument();
    expect(screen.getByLabelText("Profile 1 expected port 1")).toHaveValue(3100);
    expect(screen.getByDisplayValue("--port")).toBeInTheDocument();
  });

  it("requires manual review when a single project has no service scripts", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockResolvedValue(undefined);
    renderModal({
      onDiscover: vi.fn().mockResolvedValue(maintenanceProject),
      onSave,
    });

    await user.type(screen.getByLabelText("Project directory"), maintenanceProject.path);
    await user.click(screen.getByRole("button", { name: "Inspect directory" }));

    expect(await screen.findByDisplayValue("Maintenance")).toBeInTheDocument();
    expect(screen.getByText(/Only exact dev, start, serve, and preview scripts/)).toBeInTheDocument();
    expect(screen.getByText(/Add a reviewed profile manually to continue/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save project" })).toBeDisabled();
    expect(onSave).not.toHaveBeenCalled();
  });

  it("does not select or persist projects without service launch profiles", async () => {
    const user = userEvent.setup();
    const onSaveMany = vi.fn().mockResolvedValue(undefined);
    renderModal({
      onScanDevelopmentRoot: vi.fn().mockResolvedValue([safeProject, maintenanceProject]),
      onSaveMany,
    });

    await user.click(screen.getByRole("button", { name: "Development root" }));
    await user.type(screen.getByLabelText("Development root"), "D:\\projects");
    await user.click(screen.getByRole("button", { name: "Scan root" }));

    expect(await screen.findByRole("checkbox", { name: "Select Web App" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Select Maintenance" })).toBeDisabled();
    expect(screen.getByText("No service scripts")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Import 1 project" }));
    expect(onSaveMany).toHaveBeenCalledWith([
      expect.objectContaining({ name: "Web App", profiles: [expect.objectContaining({ name: "dev" })] }),
    ]);
  });

  it("localizes an unavailable package manager", async () => {
    const projectWithoutManager: DiscoveredProject = {
      ...safeProject,
      packageManager: null,
    };
    renderModal({
      onScanDevelopmentRoot: vi.fn().mockResolvedValue([projectWithoutManager]),
    }, "zh-CN");

    await userEvent.click(screen.getByRole("button", { name: "开发根目录" }));
    await userEvent.type(screen.getByLabelText("开发根目录"), "D:\\projects");
    await userEvent.click(screen.getByRole("button", { name: "扫描根目录" }));

    expect(await screen.findByText("未知包管理器")).toBeInTheDocument();
  });

  it("keeps every dismissal path disabled while a project save is pending", async () => {
    const save = deferred<void>();
    renderModal({
      onDiscover: vi.fn().mockResolvedValue(safeProject),
      onSave: vi.fn().mockReturnValue(save.promise),
    });

    await userEvent.type(screen.getByLabelText("Project directory"), safeProject.path);
    await userEvent.click(screen.getByRole("button", { name: "Inspect directory" }));
    await screen.findByDisplayValue("Web App");
    await userEvent.click(screen.getByRole("button", { name: "Save project" }));

    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Close" })).toBeDisabled();

    save.resolve();
  });

  it("surfaces directory-picker failures and allows retrying", async () => {
    const onPickDirectory = vi.fn().mockRejectedValue(new Error("picker unavailable"));
    renderModal({ onPickDirectory });

    await userEvent.click(screen.getByRole("button", { name: "Choose project directory" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("picker unavailable");
    expect(screen.getByRole("button", { name: "Choose project directory" })).toBeEnabled();
  });

  it("prevents Enter from starting concurrent discovery requests", async () => {
    const discovery = deferred<DiscoveredProject>();
    const onDiscover = vi.fn().mockReturnValue(discovery.promise);
    renderModal({ onDiscover });

    const input = screen.getByLabelText("Project directory");
    await userEvent.type(input, safeProject.path);
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onDiscover).toHaveBeenCalledTimes(1);
    expect(input).toBeDisabled();

    discovery.resolve(safeProject);
    expect(await screen.findByDisplayValue("Web App")).toBeInTheDocument();
  });
});
