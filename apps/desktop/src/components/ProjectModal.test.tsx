import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import type { DiscoveredProject, Project } from "../types";
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
  it("copies a profile without persisted IDs or observed-runtime metadata", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockResolvedValue(undefined);
    const storedProject: Project = {
      id: "project-persisted",
      name: safeProject.name,
      path: safeProject.path,
      createdAt: 1,
      updatedAt: 2,
      profiles: [{
        id: "profile-persisted",
        projectId: "project-persisted",
        name: "dev",
        program: "npm.cmd",
        args: ["run", "dev"],
        cwd: safeProject.path,
        status: "idle",
        expectedPorts: [{
          id: "port-persisted",
          profileId: "profile-persisted",
          port: 3100,
          protocol: "tcp",
        }],
      }],
    };
    renderModal({
      project: storedProject,
      onSave,
    });

    await user.click(screen.getByRole("button", { name: "Copy profile 1" }));

    expect(screen.getByText("Profile 2")).toBeInTheDocument();
    expect(screen.getAllByDisplayValue("npm.cmd")).toHaveLength(2);
    expect(screen.getAllByDisplayValue("D:\\projects\\web-app")).toHaveLength(3);
    expect(screen.getAllByDisplayValue("run")).toHaveLength(2);
    expect(screen.getAllByDisplayValue("3100")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Save project" }));

    expect(onSave).toHaveBeenCalledTimes(1);
    const saved = onSave.mock.calls[0][0];
    expect(saved.profiles[1]).toEqual({
      name: "dev Copy",
      program: "npm.cmd",
      args: ["run", "dev"],
      cwd: "D:\\projects\\web-app",
      expectedPorts: [{ port: 3100, protocol: "tcp" }],
    });
    expect(saved.profiles[1]).not.toHaveProperty("id");
    expect(saved.profiles[1]).not.toHaveProperty("observedRuntime");
    expect(saved.profiles[1].expectedPorts[0]).not.toHaveProperty("id");
  });

  it("marks invalid fields and duplicate ports when an invalid form is submitted", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockResolvedValue(undefined);
    renderModal({
      onDiscover: vi.fn().mockResolvedValue(safeProject),
      onSave,
    });

    await user.type(screen.getByLabelText("Project directory"), safeProject.path);
    await user.click(screen.getByRole("button", { name: "Inspect directory" }));

    await screen.findByDisplayValue("Web App");
    const name = screen.getByLabelText("Project name");
    await user.clear(name);
    await user.clear(screen.getByLabelText("Name"));
    await user.clear(screen.getByLabelText("Program"));
    await user.clear(screen.getByLabelText("Working directory"));
    await user.click(screen.getByRole("button", { name: "Add argument" }));
    await user.click(screen.getByRole("button", { name: "Add expected port" }));
    await user.click(screen.getByRole("button", { name: "Add expected port" }));
    await user.click(screen.getByRole("button", { name: "Save project" }));

    expect(name).toHaveAttribute("aria-invalid", "true");
    expect(name).toHaveAttribute("aria-describedby", "project-name-error");
    expect(screen.getByLabelText("Profile 1 argument 3")).toHaveAttribute("aria-invalid", "true");
    const duplicatePorts = screen.getAllByDisplayValue("3000");
    expect(duplicatePorts).toHaveLength(2);
    expect(duplicatePorts[0]).toHaveAttribute("aria-invalid", "true");
    expect(duplicatePorts[1]).toHaveAttribute("aria-invalid", "true");
    expect(screen.getAllByText("This port and protocol pair is already listed in this profile.")).toHaveLength(2);
    expect(screen.getAllByRole("alert").some((alert) =>
      alert.textContent?.includes("Review the highlighted fields before saving."),
    )).toBe(true);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps every field reachable by its own name once validation errors are showing", async () => {
    const user = userEvent.setup();
    const fields = ["Project name", "Project directory", "Name", "Program", "Working directory"];
    renderModal({
      onDiscover: vi.fn().mockResolvedValue(safeProject),
      onSave: vi.fn().mockResolvedValue(undefined),
    });

    await user.type(screen.getByLabelText("Project directory"), safeProject.path);
    await user.click(screen.getByRole("button", { name: "Inspect directory" }));
    await screen.findByDisplayValue("Web App");

    for (const field of fields) {
      await user.clear(screen.getByLabelText(field));
    }
    await user.click(screen.getByRole("button", { name: "Save project" }));

    // Each cleared field now renders its error message inside the same <label>,
    // so a field that took its name from the label's text would answer to
    // "Program This field is required." and to nothing a user could guess.
    for (const field of fields) {
      expect(screen.getByLabelText(field)).toHaveAttribute("aria-invalid", "true");
    }
  });

  it("localizes profile copying and field-level validation in Chinese", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockResolvedValue(undefined);
    const storedProject: Project = {
      id: "project-zh",
      name: safeProject.name,
      path: safeProject.path,
      createdAt: 1,
      updatedAt: 2,
      profiles: [{
        id: "profile-zh",
        projectId: "project-zh",
        name: "dev",
        program: "npm.cmd",
        args: ["run", "dev"],
        cwd: safeProject.path,
        status: "idle",
        expectedPorts: [],
      }],
    };
    renderModal({ project: storedProject, onSave }, "zh-CN");

    await user.click(screen.getByRole("button", { name: "复制配置 1" }));
    expect(screen.getByDisplayValue("dev 副本")).toBeInTheDocument();

    await user.clear(screen.getByLabelText("项目名称"));
    await user.clear(screen.getAllByLabelText("名称")[0]);
    await user.clear(screen.getAllByLabelText("程序")[0]);
    await user.clear(screen.getAllByLabelText("工作目录")[0]);
    await user.click(screen.getAllByRole("button", { name: "添加参数" })[0]);
    await user.click(screen.getAllByRole("button", { name: "添加预期端口" })[0]);
    await user.click(screen.getAllByRole("button", { name: "添加预期端口" })[0]);
    await user.click(screen.getByRole("button", { name: "保存项目" }));

    expect(screen.getAllByText("此字段为必填项。")).toHaveLength(4);
    expect(screen.getByText("参数不能为空；如果不需要，请移除此参数。")).toBeVisible();
    expect(screen.getAllByText("此配置中已经存在相同的端口和协议。")).toHaveLength(2);
    expect(screen.getByText("请检查标出的字段后再保存。")).toBeVisible();
    expect(onSave).not.toHaveBeenCalled();
  });

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

  it("removes successfully imported candidates while keeping the failed candidate selected", () => {
    const workerProject: DiscoveredProject = {
      ...safeProject,
      name: "Worker Lab",
      path: "D:\\projects\\worker-lab",
      profiles: [{
        ...safeProject.profiles[0],
        name: "start",
        cwd: "D:\\projects\\worker-lab",
      }],
    };
    const baseProps: React.ComponentProps<typeof ProjectModal> = {
      initialImportMode: "root",
      initialRoot: "D:\\projects",
      initialRootProjects: [safeProject, workerProject],
      onDiscover: vi.fn(),
      onScanDevelopmentRoot: vi.fn(),
      onPickDirectory: vi.fn().mockResolvedValue(null),
      onSave: vi.fn(),
      onSaveMany: vi.fn(),
      registeredPaths: [],
      onClose: vi.fn(),
    };
    const view = render(
      <I18nProvider><ProjectModal {...baseProps} /></I18nProvider>,
    );

    expect(screen.getByRole("checkbox", { name: "Select Web App" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Select Worker Lab" })).toBeChecked();

    view.rerender(
      <I18nProvider>
        <ProjectModal {...baseProps} initialRootProjects={[workerProject]} />
      </I18nProvider>,
    );

    expect(screen.queryByText("Web App", { exact: true })).not.toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select Worker Lab" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Import 1 project" })).toBeEnabled();
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
