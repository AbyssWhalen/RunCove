import type {
  DashboardSnapshot,
  DiscoveredProject,
  ExternalProcessRequest,
  LaunchGroup,
  LaunchGroupInput,
  LaunchGroupStartResult,
  LaunchGroupStopResult,
  Project,
  ProjectInput,
  RestoreResult,
  RunCoveApi,
  RunLogArchivePage,
  RunLogArchiveRecord,
  RunLogArchiveSummary,
  RunLogEvent,
  RunSession,
  RunStatusEvent,
} from "./types";

const fixedNow = Date.parse("2026-08-07T08:30:00.000Z");
const savedRootScanFailureKey = "runcove:e2e:saved-root-scan-failure-once";
/** Set this key to preview the state where the archive failed to initialize. */
const archiveUnavailableKey = "runcove:e2e:archive-unavailable";

const initialSnapshot: DashboardSnapshot = {
  generatedAt: fixedNow,
  scanError: null,
  privilege: {
    elevated: false,
    elevationAvailable: true,
    monitorOnly: false,
  },
  runLogArchive: {
    enabled: false,
    available: true,
    unavailableReason: null,
  },
  settings: {
    pollIntervalMs: 2_000,
    logCapacity: 500,
    languagePreference: "system",
    recentDevelopmentRoot: "D:\\CodexProject\\personal-projects",
    closeBehavior: "ask",
    archiveRunLogs: false,
  },
  restoreSet: {
    profileIds: ["profile-studio-web", "profile-docs"],
    savedAt: Date.parse("2026-08-06T16:41:12.000Z"),
  },
  // Seeded half-running on purpose: Web is already up and API is not, so the preview
  // opens on the "partly running" badge rather than on either extreme.
  launchGroups: [
    {
      id: "group-studio",
      name: "Abyss Studio stack",
      profileIds: ["profile-studio-api", "profile-studio-web"],
      createdAt: Date.parse("2026-07-30T09:00:00.000Z"),
      updatedAt: Date.parse("2026-08-06T16:41:12.000Z"),
    },
  ],
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

/**
 * One archived session in the browser preview.
 *
 * The real archive is a file of one JSON record per line and pages backwards over
 * byte offsets. The mock keeps the records in memory but encodes them the same way
 * to derive offsets and sizes, so a viewer written against these numbers is written
 * against the real cursor contract.
 */
interface MockArchive {
  status: string;
  reason: string | null;
  droppedLines: number;
  droppedBytes: number;
  records: RunLogArchiveRecord[];
}

/**
 * The mock serves twelve records per page instead of the command's default of 500,
 * so the browser preview can page without fabricating hundreds of lines.
 */
const mockPageRecords = 12;

function encodedLength(record: RunLogArchiveRecord): number {
  const encoded = JSON.stringify({ t: record.timestamp, s: record.stream, l: record.line });
  return new TextEncoder().encode(`${encoded}\n`).length;
}

/** Byte offset of the end of every record, in file order. */
function recordEnds(records: RunLogArchiveRecord[]): number[] {
  let offset = 0;
  return records.map((record) => (offset += encodedLength(record)));
}

function archiveSummary(archive: MockArchive, endedAt: number | null): RunLogArchiveSummary {
  const ends = recordEnds(archive.records);
  return {
    status: archive.status,
    reason: archive.reason,
    lineCount: archive.records.length,
    byteSize: ends.at(-1) ?? 0,
    droppedLines: archive.droppedLines,
    droppedBytes: archive.droppedBytes,
    startedAt: archive.records[0]?.timestamp ?? fixedNow,
    endedAt,
  };
}

function viteStartup(profileId: string, startedAt: number, count: number): RunLogArchiveRecord[] {
  const lines = [
    "> abyss-studio@0.4.2 dev",
    "> vite --host 127.0.0.1",
    "VITE v6.0.5  ready in 361 ms",
    "  ➜  Local:   http://localhost:5173/",
    "  ➜  Network: use --host to expose",
    "12:12:26 [vite] hmr update /src/routes/dashboard.tsx",
    "12:12:31 [vite] page reload src/main.tsx",
  ];
  return Array.from({ length: count }, (_, index) => ({
    stream: index === 0 ? "system" : index % 9 === 8 ? "stderr" : "stdout",
    line:
      index === 0
        ? `Started npm.cmd run dev for ${profileId}`
        : index % 9 === 8
          ? `12:${13 + index}:04 [vite] warning: dynamic import cannot be analysed (line ${index})`
          : `${lines[index % lines.length]} (line ${index})`,
    timestamp: startedAt + index * 1_000,
  }));
}

const initialArchives: Record<string, MockArchive> = {
  "session-web-current": {
    status: "writing",
    reason: null,
    droppedLines: 0,
    droppedBytes: 0,
    records: viteStartup("profile-studio-web", Date.parse("2026-08-07T08:12:24.000Z"), 30),
  },
  "session-docs-exited": {
    status: "complete",
    reason: null,
    droppedLines: 0,
    droppedBytes: 0,
    records: [
      {
        stream: "system",
        line: "Started pnpm.cmd dev (PID 17320)",
        timestamp: Date.parse("2026-08-06T16:40:00.000Z"),
      },
      {
        stream: "stdout",
        line: "astro  v5.1.1 ready in 512 ms",
        timestamp: Date.parse("2026-08-06T16:40:01.000Z"),
      },
      {
        stream: "stdout",
        line: "┃ Local    http://localhost:4321/",
        timestamp: Date.parse("2026-08-06T16:40:01.000Z"),
      },
      {
        stream: "system",
        line: "Process exited with code 0",
        timestamp: Date.parse("2026-08-06T16:41:12.000Z"),
      },
    ],
  },
  "session-orphaned": {
    status: "partial",
    reason: "quota-exceeded",
    droppedLines: 184,
    droppedBytes: 61_440,
    records: [
      {
        stream: "system",
        line: "Started node.exe worker.js (PID 16004)",
        timestamp: Date.parse("2026-08-05T09:00:00.000Z"),
      },
      {
        stream: "stderr",
        line: "worker: retrying upstream connection (attempt 41)",
        timestamp: Date.parse("2026-08-05T09:01:00.000Z"),
      },
      {
        stream: "system",
        line: "[runcove] 184 lines (61440 bytes) dropped",
        timestamp: Date.parse("2026-08-05T09:03:05.000Z"),
      },
    ],
  },
};

const initialRunHistory: RunSession[] = [
  {
    id: "session-web-current",
    profileId: "profile-studio-web",
    profileName: "Web",
    pid: 18424,
    startedAt: Date.parse("2026-08-07T08:12:24.000Z"),
    endedAt: null,
    exitCode: null,
    status: "running",
    archive: archiveSummary(initialArchives["session-web-current"], null),
  },
  {
    id: "session-docs-exited",
    profileId: "profile-docs",
    profileName: "Astro",
    pid: 17320,
    startedAt: Date.parse("2026-08-06T16:40:00.000Z"),
    endedAt: Date.parse("2026-08-06T16:41:12.000Z"),
    exitCode: 0,
    status: "exited",
    archive: archiveSummary(
      initialArchives["session-docs-exited"],
      Date.parse("2026-08-06T16:41:12.000Z"),
    ),
  },
  {
    id: "session-orphaned",
    profileId: "deleted-profile",
    profileName: "Removed service",
    pid: 16004,
    startedAt: Date.parse("2026-08-05T09:00:00.000Z"),
    endedAt: Date.parse("2026-08-05T09:03:05.000Z"),
    exitCode: null,
    status: "interrupted",
    archive: archiveSummary(
      initialArchives["session-orphaned"],
      Date.parse("2026-08-05T09:03:05.000Z"),
    ),
  },
];

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
  let runHistory = clone(initialRunHistory);
  let archives = clone(initialArchives);
  const statusHandlers = new Set<(event: RunStatusEvent) => void>();
  const statusEventListeners = new Set<EventListener>();
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

  /**
   * Drop deleted profiles from the restore set and from every launch group, the way
   * the database's `ON DELETE CASCADE` does. A group whose last member goes is kept
   * and reported empty rather than deleted.
   */
  const forgetProfiles = (removed: Set<string>): void => {
    if (removed.size === 0) return;
    snapshot.restoreSet.profileIds = snapshot.restoreSet.profileIds.filter(
      (profileId) => !removed.has(profileId),
    );
    snapshot.launchGroups = snapshot.launchGroups.map((group) => ({
      ...group,
      profileIds: group.profileIds.filter((profileId) => !removed.has(profileId)),
    }));
  };

  const groupToRun = (groupId: string): LaunchGroup => {
    const group = snapshot.launchGroups.find((item) => item.id === groupId);
    if (!group) throw new Error("Launch group not found");
    if (group.profileIds.length === 0) {
      throw new Error("This launch group has no launch profiles");
    }
    return group;
  };

  return {
    reset() {
      snapshot = clone(initialSnapshot);
      logs = clone(initialLogs);
      runHistory = clone(initialRunHistory);
      archives = clone(initialArchives);
      mockSequence = 100;
      statusHandlers.clear();
      statusEventListeners.forEach((listener) => window.removeEventListener("runcove:mock-run-status", listener));
      statusEventListeners.clear();
      delete document.documentElement.dataset.mockRunStatusReady;
      logHandlers.clear();
    },
    async getDashboardSnapshot() {
      snapshot.generatedAt = fixedNow;
      if (sessionStorage.getItem(archiveUnavailableKey) === "1") {
        snapshot.runLogArchive = {
          enabled: snapshot.settings.archiveRunLogs,
          available: false,
          unavailableReason:
            "Could not create the run log archive directory: access is denied (mock preview)",
        };
      }
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
      if (sessionStorage.getItem(savedRootScanFailureKey) === "1") {
        sessionStorage.removeItem(savedRootScanFailureKey);
        throw new Error("Mock saved-root scan failed once");
      }
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
      const keptProfileIds = new Set(project.profiles.map((profile) => profile.id));
      forgetProfiles(
        new Set(
          existing?.profiles
            .map((profile) => profile.id)
            .filter((profileId) => !keptProfileIds.has(profileId)) ?? [],
        ),
      );
      return clone(project);
    },
    async deleteProject(projectId) {
      const removedProfileIds = new Set(
        snapshot.projects
          .find((project) => project.id === projectId)
          ?.profiles.map((profile) => profile.id) ?? [],
      );
      snapshot.projects = snapshot.projects.filter((project) => project.id !== projectId);
      forgetProfiles(removedProfileIds);
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
    async saveLaunchGroup(input: LaunchGroupInput) {
      const name = input.name.trim();
      if (!name) throw new Error("Launch group name cannot be empty");
      if (input.profileIds.length === 0) {
        throw new Error("Launch group must have at least one launch profile");
      }
      if (new Set(input.profileIds).size !== input.profileIds.length) {
        throw new Error("Launch group cannot list the same launch profile twice");
      }
      const clash = snapshot.launchGroups.find(
        (group) => group.id !== input.id && group.name.toLowerCase() === name.toLowerCase(),
      );
      if (clash) throw new Error("A launch group with this name already exists");
      const existing = input.id
        ? snapshot.launchGroups.find((group) => group.id === input.id)
        : undefined;
      const group: LaunchGroup = {
        id: existing?.id ?? nextId("group"),
        name,
        profileIds: [...input.profileIds],
        createdAt: existing?.createdAt ?? fixedNow,
        updatedAt: fixedNow,
      };
      snapshot.launchGroups = existing
        ? snapshot.launchGroups.map((item) => (item.id === group.id ? group : item))
        : [...snapshot.launchGroups, group];
      snapshot.launchGroups.sort((left, right) => left.name.localeCompare(right.name));
      return clone(group);
    },
    async deleteLaunchGroup(groupId) {
      if (!snapshot.launchGroups.some((group) => group.id === groupId)) {
        throw new Error("Launch group not found");
      }
      snapshot.launchGroups = snapshot.launchGroups.filter((group) => group.id !== groupId);
    },
    async startLaunchGroup(groupId) {
      const group = groupToRun(groupId);
      const result: LaunchGroupStartResult = { groupId, startedProfileIds: [] };
      for (const profileId of group.profileIds) {
        emitStatus(profileId, "running");
        appendLog(profileId, `Started as part of ${group.name}`);
        result.startedProfileIds.push(profileId);
      }
      return result;
    },
    async stopLaunchGroup(groupId) {
      const group = groupToRun(groupId);
      const result: LaunchGroupStopResult = { groupId, stoppedProfileIds: [], failures: [] };
      // Backwards, the way the real command walks it: whatever depends on a member
      // goes down before the member itself does.
      for (const profileId of [...group.profileIds].reverse()) {
        emitStatus(profileId, "idle");
        appendLog(profileId, `Stopped as part of ${group.name}`);
        result.stoppedProfileIds.push(profileId);
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
      return clone(runHistory).sort((left, right) => right.startedAt - left.startedAt).slice(0, 200);
    },
    async setRunLogArchiving(enabled) {
      if (snapshot.runLogArchive.available === false) {
        throw new Error("The run log archive is unavailable in this preview");
      }
      snapshot.settings.archiveRunLogs = enabled;
      snapshot.runLogArchive = { enabled, available: true, unavailableReason: null };
      return clone(snapshot.runLogArchive);
    },
    async readRunLogArchive(sessionId, beforeOffset, maxLines) {
      const archive = archives[sessionId];
      const session = runHistory.find((entry) => entry.id === sessionId);
      if (!archive) throw new Error(`No run log archive exists for session ${sessionId}`);
      if (archive.status === "removed") {
        throw new Error(
          `The run log archive for session ${sessionId} was removed (${archive.reason ?? "unknown"})`,
        );
      }

      const ends = recordEnds(archive.records);
      const fileLength = ends.at(-1) ?? 0;
      const cursor = beforeOffset ?? fileLength;
      if (beforeOffset != null && !ends.includes(beforeOffset)) {
        throw new Error(`Offset ${beforeOffset} is not a record boundary of this archive`);
      }
      const limit = Math.min(Math.max(maxLines ?? mockPageRecords, 1), 2_000);
      const endIndex = ends.filter((end) => end <= cursor).length;
      const startIndex = Math.max(endIndex - limit, 0);
      const page: RunLogArchivePage = {
        sessionId,
        status: archive.status,
        reason: archive.reason,
        lineCount: archive.records.length,
        byteSize: fileLength,
        droppedLines: archive.droppedLines,
        droppedBytes: archive.droppedBytes,
        startedAt: session?.startedAt ?? fixedNow,
        endedAt: session?.endedAt ?? null,
        records: clone(archive.records.slice(startIndex, endIndex)),
        fileLength,
        pageStartOffset: startIndex === 0 ? 0 : ends[startIndex - 1],
        hasMoreBefore: startIndex > 0,
        stoppedBy: startIndex === 0 ? "start" : "lines",
        incompleteTailSkipped: false,
        malformedLines: 0,
      };
      return page;
    },
    async deleteRunLogArchive(sessionId) {
      const archive = archives[sessionId];
      if (!archive) throw new Error(`No run log archive exists for session ${sessionId}`);
      if (archive.status === "writing") {
        throw new Error(
          `Session ${sessionId} is still being archived; stop the run before deleting it`,
        );
      }
      archives[sessionId] = { ...archive, status: "removed", reason: "user-deleted", records: [] };
      // The counters stay and `endedAt` becomes the removal time, the way the real
      // index does it: "42 lines, deleted" tells the user what they gave up, while
      // zeroing them would read as a run that printed nothing.
      runHistory = runHistory.map((session) =>
        session.id === sessionId && session.archive
          ? {
            ...session,
            archive: {
              ...session.archive,
              status: "removed",
              reason: "user-deleted",
              endedAt: fixedNow,
            },
          }
          : session,
      );
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
      document.documentElement.dataset.mockRunStatusReady = "true";
      const listener = (event: Event) => {
        const detail = (event as CustomEvent<RunStatusEvent>).detail;
        if (detail) handler(clone(detail));
      };
      statusEventListeners.add(listener);
      window.addEventListener("runcove:mock-run-status", listener);
      return () => {
        statusHandlers.delete(handler);
        statusEventListeners.delete(listener);
        window.removeEventListener("runcove:mock-run-status", listener);
        if (statusHandlers.size === 0) delete document.documentElement.dataset.mockRunStatusReady;
      };
    },
    async onRunLog(handler) {
      logHandlers.add(handler);
      return () => logHandlers.delete(handler);
    },
    // The mock closes an archive synchronously with the run, so there is nothing left
    // to announce; the listener exists so the real event has a mock counterpart.
    async onArchiveClosed() {
      return () => {};
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
