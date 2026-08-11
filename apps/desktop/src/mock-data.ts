import type {
  DashboardSnapshot,
  DiscoveredProject,
  ExternalProcessRequest,
  Project,
  ProjectInput,
  RestoreResult,
  RunCoveApi,
  RunLogEvent,
  RunStatusEvent,
} from "./types";

const fixedNow = Date.parse("2026-08-07T08:30:00.000Z");

const initialSnapshot: DashboardSnapshot = {
  generatedAt: fixedNow,
  scanError: null,
  privilege: {
    elevated: false,
    elevationAvailable: true,
    monitorOnly: false,
  },
  settings: {
    pollIntervalMs: 2_000,
    logCapacity: 500,
    languagePreference: "system",
    recentDevelopmentRoot: "D:\\CodexProject\\personal-projects",
    closeBehavior: "ask",
  },
  restoreSet: {
    profileIds: ["profile-studio-web", "profile-docs"],
    savedAt: Date.parse("2026-08-06T16:41:12.000Z"),
  },
  projects: [
    {
      id: "project-studio",
      name: "Abyss Studio",
      path: "D:\\CodexProject\\personal-projects\\abyss-studio",
      createdAt: Date.parse("2026-07-18T10:00:00.000Z"),
      updatedAt: fixedNow,
      profiles: [
        {
          id: "profile-studio-web",
          projectId: "project-studio",
          name: "Web",
          program: "npm.cmd",
          args: ["run", "dev"],
          cwd: "D:\\CodexProject\\personal-projects\\abyss-studio",
          expectedPorts: [
            { id: "expected-5173", profileId: "profile-studio-web", port: 5173, protocol: "tcp" },
          ],
          status: "running",
          pid: 18424,
        },
        {
          id: "profile-studio-api",
          projectId: "project-studio",
          name: "API",
          program: "npm.cmd",
          args: ["run", "api"],
          cwd: "D:\\CodexProject\\personal-projects\\abyss-studio",
          expectedPorts: [
            { id: "expected-4010", profileId: "profile-studio-api", port: 4010, protocol: "tcp" },
          ],
          status: "idle",
          pid: null,
        },
      ],
    },
    {
      id: "project-docs",
      name: "Docs Lab",
      path: "D:\\CodexProject\\personal-projects\\docs-lab",
      createdAt: Date.parse("2026-07-24T03:00:00.000Z"),
      updatedAt: fixedNow,
      profiles: [
        {
          id: "profile-docs",
          projectId: "project-docs",
          name: "Astro",
          program: "pnpm.cmd",
          args: ["dev"],
          cwd: "D:\\CodexProject\\personal-projects\\docs-lab",
          expectedPorts: [
            { id: "expected-4321", profileId: "profile-docs", port: 4321, protocol: "tcp" },
          ],
          status: "exited",
          pid: null,
        },
      ],
    },
  ],
  ports: [
    {
      port: 5173,
      protocol: "tcp",
      state: "LISTEN",
      bindAddress: "127.0.0.1",
      isPublic: false,
      active: true,
      pid: 18424,
      processName: "node.exe",
      executablePath: "C:\\Program Files\\nodejs\\node.exe",
      commandLine: "node vite --host 127.0.0.1",
      processStartedAt: Date.parse("2026-08-07T08:12:24.000Z"),
      lastSeenAt: fixedNow,
      projectId: "project-studio",
      profileId: "profile-studio-web",
      associationSource: "managed",
    },
    {
      port: 4000,
      protocol: "tcp",
      state: "LISTEN",
      bindAddress: "0.0.0.0",
      isPublic: true,
      active: true,
      pid: 9208,
      processName: "node.exe",
      executablePath: "C:\\Program Files\\nodejs\\node.exe",
      commandLine: "node server.js",
      processStartedAt: Date.parse("2026-08-07T07:52:03.000Z"),
      lastSeenAt: fixedNow,
      projectId: "project-studio",
      profileId: null,
      associationSource: "suggested",
    },
    {
      port: 5432,
      protocol: "tcp",
      state: "LISTEN",
      bindAddress: "127.0.0.1",
      isPublic: false,
      active: true,
      pid: 6612,
      processName: "postgres.exe",
      executablePath: "C:\\Program Files\\PostgreSQL\\bin\\postgres.exe",
      commandLine: null,
      processStartedAt: Date.parse("2026-08-07T00:02:18.000Z"),
      lastSeenAt: fixedNow,
      projectId: null,
      profileId: null,
      associationSource: null,
    },
    {
      port: 8787,
      protocol: "tcp",
      state: "CLOSED",
      bindAddress: "127.0.0.1",
      isPublic: false,
      active: false,
      pid: null,
      processName: null,
      executablePath: null,
      commandLine: null,
      lastSeenAt: Date.parse("2026-08-05T11:18:09.000Z"),
      projectId: "project-docs",
      profileId: "profile-docs",
      associationSource: "confirmed",
    },
  ],
};

const initialLogs: Record<string, RunLogEvent[]> = {
  "profile-studio-web": [
    {
      profileId: "profile-studio-web",
      stream: "system",
      line: "Started npm.cmd run dev (PID 18424)",
      timestamp: Date.parse("2026-08-07T08:12:24.000Z"),
    },
    {
      profileId: "profile-studio-web",
      stream: "stdout",
      line: "VITE ready in 361 ms",
      timestamp: Date.parse("2026-08-07T08:12:25.000Z"),
    },
    {
      profileId: "profile-studio-web",
      stream: "stdout",
      line: "Local: http://localhost:5173/",
      timestamp: Date.parse("2026-08-07T08:12:25.000Z"),
    },
  ],
};

function clone<T>(value: T): T {
  return structuredClone(value);
}

function nextId(prefix: string): string {
  return `${prefix}-${mockSequence++}`;
}

function discoveredProject(
  directory: string,
  packageManager = "npm",
  observedRuntime = false,
): DiscoveredProject {
  const folder = directory.replace(/[\\/]+$/, "").split(/[\\/]/).at(-1) || "project";
  const name = folder
    .split(/[-_]/)
    .filter(Boolean)
    .map((part) => part[0]?.toUpperCase() + part.slice(1))
    .join(" ");
  return {
    name,
    path: directory,
    packageManager,
    workspacePatterns: [],
    profiles: [
      {
        name: "Dev",
        program: packageManager === "pnpm" ? "pnpm.cmd" : "npm.cmd",
        args: observedRuntime ? ["run", "dev", "--port", "3100"] : ["run", "dev"],
        cwd: directory,
        expectedPorts: observedRuntime ? [{ port: 3100, protocol: "tcp" }] : [],
        observedRuntime,
      },
    ],
  };
}

let mockSequence = 100;

export type MockRunCoveApi = RunCoveApi & { reset(): void };

export function createMockApi(): MockRunCoveApi {
  let snapshot = clone(initialSnapshot);
  let logs = clone(initialLogs);
  const statusHandlers = new Set<(event: RunStatusEvent) => void>();
  const logHandlers = new Set<(event: RunLogEvent) => void>();

  const emitStatus = (profileId: string, status: RunStatusEvent["status"]): RunStatusEvent => {
    const event: RunStatusEvent = {
      profileId,
      status,
      pid: status === "running" ? 21040 : null,
      timestamp: fixedNow,
    };
    const profile = snapshot.projects.flatMap((project) => project.profiles).find((item) => item.id === profileId);
    if (profile) {
      profile.status = status;
      profile.pid = event.pid;
    }
    statusHandlers.forEach((handler) => handler(clone(event)));
    return clone(event);
  };

  const appendLog = (profileId: string, line: string): void => {
    const entries = logs[profileId] ?? [];
    const event: RunLogEvent = {
      profileId,
      stream: "system",
      line,
      timestamp: fixedNow,
    };
    logs[profileId] = [...entries, event].slice(-snapshot.settings.logCapacity);
    logHandlers.forEach((handler) => handler(clone(event)));
  };

  return {
    reset() {
      snapshot = clone(initialSnapshot);
      logs = clone(initialLogs);
      mockSequence = 100;
      statusHandlers.clear();
      logHandlers.clear();
    },
    async getDashboardSnapshot() {
      snapshot.generatedAt = fixedNow;
      return clone(snapshot);
    },
    async discoverProject(directory) {
      return discoveredProject(directory, "npm", true);
    },
    async scanDevelopmentRoot(directory) {
      const root = directory.replace(/[\\/]+$/, "");
      snapshot.settings.recentDevelopmentRoot = root;
      return [
        discoveredProject(`${root}\\signal-console`),
        discoveredProject(`${root}\\worker-lab`, "pnpm"),
      ];
    },
    async scanSavedDevelopmentRoot() {
      const root = snapshot.settings.recentDevelopmentRoot;
      if (!root) throw new Error("No development root has been saved yet");
      return [
        discoveredProject(`${root}\\signal-console`),
        discoveredProject(`${root}\\worker-lab`, "pnpm"),
      ];
    },
    async saveProject(input: ProjectInput) {
      const existing = input.id ? snapshot.projects.find((project) => project.id === input.id) : undefined;
      const projectId = existing?.id ?? nextId("project");
      const project: Project = {
        id: projectId,
        name: input.name,
        path: input.path,
        createdAt: existing?.createdAt ?? fixedNow,
        updatedAt: fixedNow,
        profiles: input.profiles.map((profileInput) => {
          const profileId = profileInput.id ?? nextId("profile");
          const oldProfile = existing?.profiles.find((profile) => profile.id === profileId);
          return {
            id: profileId,
            projectId,
            name: profileInput.name,
            program: profileInput.program,
            args: [...profileInput.args],
            cwd: profileInput.cwd,
            expectedPorts: profileInput.expectedPorts.map((port) => ({
              id: port.id ?? nextId("expected"),
              profileId,
              port: port.port,
              protocol: port.protocol,
            })),
            status: oldProfile?.status ?? "idle",
            pid: oldProfile?.pid ?? null,
          };
        }),
      };
      snapshot.projects = existing
        ? snapshot.projects.map((item) => (item.id === projectId ? project : item))
        : [...snapshot.projects, project];
      return clone(project);
    },
    async deleteProject(projectId) {
      const removedProfileIds = new Set(
        snapshot.projects
          .find((project) => project.id === projectId)
          ?.profiles.map((profile) => profile.id) ?? [],
      );
      snapshot.projects = snapshot.projects.filter((project) => project.id !== projectId);
      snapshot.restoreSet.profileIds = snapshot.restoreSet.profileIds.filter(
        (profileId) => !removedProfileIds.has(profileId),
      );
    },
    async startProfile(profileId) {
      appendLog(profileId, "Start requested from browser preview");
      return emitStatus(profileId, "running");
    },
    async stopProfile(profileId) {
      appendLog(profileId, "Process tree stopped");
      return emitStatus(profileId, "idle");
    },
    async restartProfile(profileId) {
      appendLog(profileId, "Process tree restarted");
      return emitStatus(profileId, "running");
    },
    async restoreLastRunSet() {
      const result: RestoreResult = { startedProfileIds: [] };
      for (const profileId of snapshot.restoreSet.profileIds) {
        emitStatus(profileId, "running");
        appendLog(profileId, "Restored from last run set");
        result.startedProfileIds.push(profileId);
      }
      return result;
    },
    async terminateExternalProcess(request: ExternalProcessRequest) {
      snapshot.ports = snapshot.ports.map((port) =>
        port.port === request.port &&
        port.protocol === request.protocol &&
        port.pid === request.pid
          ? { ...port, active: false, state: "CLOSED", pid: null, lastSeenAt: fixedNow }
          : port,
      );
    },
    async confirmPortAssociation(request) {
      snapshot.ports = snapshot.ports.map((item) =>
        item.port === request.port && item.protocol === request.protocol
          ? {
            ...item,
            projectId: request.projectId,
            profileId: request.profileId ?? item.profileId,
            associationSource: "confirmed",
          }
          : item,
      );
    },
    async clearLogs(profileId) {
      logs[profileId] = [];
    },
    async getLogs(profileId) {
      return clone(logs[profileId] ?? []);
    },
    async getRunHistory() {
      return [];
    },
    async openPort() {},
    async openProjectDirectory() {},
    async shutdownApp() {},
    async hideToTray() {},
    async setCloseBehavior(behavior) {
      snapshot.settings.closeBehavior = behavior;
      return clone(snapshot.settings);
    },
    async setLanguagePreference(preference) {
      snapshot.settings.languagePreference = preference;
      return clone(snapshot.settings);
    },
    async requestElevatedMonitoring() {
      snapshot.privilege = { elevated: true, elevationAvailable: true, monitorOnly: true };
    },
    async pickProjectDirectory() {
      return "D:\\CodexProject\\personal-projects\\selected-project";
    },
    async onRunStatus(handler) {
      statusHandlers.add(handler);
      return () => statusHandlers.delete(handler);
    },
    async onRunLog(handler) {
      logHandlers.add(handler);
      return () => logHandlers.delete(handler);
    },
    async onPortSnapshot() {
      return () => {};
    },
    async onLifecycleError() {
      return () => {};
    },
    async onTrayRestoreRequested() {
      return () => {};
    },
    async onTrayQuitRequested() {
      return () => {};
    },
    async onWindowCloseChoiceRequested(handler) {
      const listener = () => handler();
      window.addEventListener("runcove:window-close-choice-requested", listener);
      return () => window.removeEventListener("runcove:window-close-choice-requested", listener);
    },
  };
}
