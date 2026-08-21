export type Protocol = "tcp" | "udp";
export type ProfileStatus =
  | "idle"
  | "starting"
  | "running"
  | "conflict"
  | "exited"
  | "unknown";
export type AssociationSource = "managed" | "confirmed" | "suggested";

export interface ExpectedPort {
  id: string;
  profileId: string;
  port: number;
  protocol: Protocol;
}

export interface LaunchProfile {
  id: string;
  projectId: string;
  name: string;
  program: string;
  args: string[];
  cwd: string;
  expectedPorts: ExpectedPort[];
  status: ProfileStatus;
  pid?: number | null;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  profiles: LaunchProfile[];
  createdAt: number;
  updatedAt: number;
}

export interface PortSnapshot {
  port: number;
  protocol: Protocol;
  state: string;
  bindAddress?: string | null;
  isPublic: boolean;
  active: boolean;
  pid?: number | null;
  processName?: string | null;
  executablePath?: string | null;
  commandLine?: string | null;
  processStartedAt?: number | null;
  lastSeenAt?: number | null;
  projectId?: string | null;
  profileId?: string | null;
  associationSource?: AssociationSource | null;
}

export interface RestoreSet {
  profileIds: string[];
  savedAt?: number | null;
}

export interface AppSettings {
  pollIntervalMs: number;
  logCapacity: number;
  languagePreference: "system" | "en" | "zh-CN";
  recentDevelopmentRoot?: string | null;
  closeBehavior: CloseBehavior;
  /** Whether runs started from now on also write their output to a file on disk. */
  archiveRunLogs: boolean;
}

export type CloseBehavior = "ask" | "hideToTray" | "quit";

export interface DashboardSnapshot {
  ports: PortSnapshot[];
  projects: Project[];
  restoreSet: RestoreSet;
  settings: AppSettings;
  privilege: PrivilegeStatus;
  generatedAt: number;
  scanError?: string | null;
  runLogArchive: RunLogArchiveState;
}

export interface PrivilegeStatus {
  elevated: boolean;
  elevationAvailable: boolean;
  monitorOnly: boolean;
}

export interface DiscoveredProfile {
  name: string;
  program: string;
  args: string[];
  cwd: string;
  expectedPorts: Array<Pick<ExpectedPort, "port" | "protocol">>;
  observedRuntime?: boolean;
}

export interface DiscoveredProject {
  name: string;
  path: string;
  packageManager?: string | null;
  workspacePatterns: string[];
  profiles: DiscoveredProfile[];
}

export interface ExpectedPortInput {
  id?: string;
  port: number;
  protocol: Protocol;
}

export interface LaunchProfileInput {
  id?: string;
  name: string;
  program: string;
  args: string[];
  cwd: string;
  expectedPorts: ExpectedPortInput[];
  observedRuntime?: boolean;
}

export interface ProjectInput {
  id?: string;
  name: string;
  path: string;
  profiles: LaunchProfileInput[];
}

export interface RunStatusEvent {
  profileId: string;
  status: ProfileStatus;
  pid?: number | null;
  message?: string | null;
  unexpected?: boolean;
  relatedPort?: RelatedPort | null;
  timestamp: number;
}

export interface RelatedPort {
  port: number;
  protocol: Protocol;
}

export interface RunLogEvent {
  profileId: string;
  stream: "stdout" | "stderr" | "system";
  line: string;
  timestamp: number;
}

/**
 * One run's archive reached its final row.
 *
 * The exit event's own history reload runs while the row is still `writing`, because
 * the writer closes the file outside the lock that event is emitted under. This is
 * what turns a `finalizing` badge into the row it settled on.
 */
export interface ArchiveClosedEvent {
  sessionId: string;
}

/**
 * What the run log archive can do right now.
 *
 * `enabled` is what the user asked for and `available` is what this run can
 * actually do. They are separate because an archive that failed to initialize must
 * not render as on: that would promise output is being captured when none is.
 */
export interface RunLogArchiveState {
  enabled: boolean;
  available: boolean;
  unavailableReason?: string | null;
}

/** The statuses this build knows how to describe. */
export type RunLogArchiveStatus = "writing" | "complete" | "partial" | "removed";

/** The reasons this build knows how to describe. */
export type RunLogArchiveReason =
  | "write-error"
  | "quota-exceeded"
  | "queue-overflow"
  | "interrupted"
  | "user-disabled"
  | "quota-evicted"
  | "user-deleted"
  | "file-missing";

/**
 * One run log archive as the history surfaces see it.
 *
 * `status` and `reason` are plain strings rather than the unions above for the
 * same reason the Rust type keeps them as strings: a database written by a newer
 * build may carry a value this one does not know, and passing it through is better
 * than failing to read the whole history. Anything rendering them must have a
 * fallback for a value it does not recognise.
 */
export interface RunLogArchiveSummary {
  status: string;
  reason?: string | null;
  lineCount: number;
  byteSize: number;
  droppedLines: number;
  droppedBytes: number;
  startedAt: number;
  endedAt?: number | null;
}

/** One archived record: the same three fields the live drawer shows. */
export interface RunLogArchiveRecord {
  stream: RunLogEvent["stream"];
  line: string;
  timestamp: number;
}

/**
 * One page of an archive, oldest record first, plus what the viewer needs to ask
 * for the page before it.
 *
 * `fileLength` is measured at read time and is exact; the row counters are as
 * fresh as the writer's last refresh, so a page of a session still being written
 * can hold more lines than `lineCount` claims.
 */
export interface RunLogArchivePage {
  sessionId: string;
  status: string;
  reason?: string | null;
  lineCount: number;
  byteSize: number;
  droppedLines: number;
  droppedBytes: number;
  startedAt: number;
  endedAt?: number | null;
  records: RunLogArchiveRecord[];
  fileLength: number;
  /** Feed this back as `beforeOffset` to page towards the start. */
  pageStartOffset: number;
  hasMoreBefore: boolean;
  /** Which bound ended the page: `lines`, `bytes`, or `start`. */
  stoppedBy: string;
  incompleteTailSkipped: boolean;
  malformedLines: number;
}

export type RunSessionStatus =
  | "starting"
  | "running"
  | "exited"
  | "interrupted"
  | "unknown";

export interface RunSession {
  id: string;
  profileId?: string | null;
  profileName: string;
  pid?: number | null;
  startedAt: number;
  endedAt?: number | null;
  exitCode?: number | null;
  status: RunSessionStatus;
  /** Absent when the session has no archive: archiving was off, or it predates the feature. */
  archive?: RunLogArchiveSummary | null;
}

export interface RestoreResult {
  startedProfileIds: string[];
  failedProfileId?: string | null;
  error?: string | null;
  relatedPort?: RelatedPort | null;
}

export interface ExternalProcessRequest {
  port: number;
  protocol: Protocol;
  pid: number;
  startedAt: number;
  executablePath: string;
}

export interface ConfirmAssociationRequest {
  port: number;
  protocol: Protocol;
  projectId: string;
  profileId?: string | null;
  pid: number;
  startedAt: number;
  executablePath: string;
}

export type Unlisten = () => void;

export interface RunCoveApi {
  getDashboardSnapshot(): Promise<DashboardSnapshot>;
  discoverProject(directory: string): Promise<DiscoveredProject>;
  scanDevelopmentRoot(directory: string): Promise<DiscoveredProject[]>;
  scanSavedDevelopmentRoot(): Promise<DiscoveredProject[]>;
  saveProject(project: ProjectInput): Promise<Project>;
  deleteProject(projectId: string): Promise<void>;
  startProfile(profileId: string): Promise<RunStatusEvent>;
  stopProfile(profileId: string): Promise<RunStatusEvent>;
  restartProfile(profileId: string): Promise<RunStatusEvent>;
  restoreLastRunSet(): Promise<RestoreResult>;
  terminateExternalProcess(request: ExternalProcessRequest): Promise<void>;
  confirmPortAssociation(request: ConfirmAssociationRequest): Promise<void>;
  clearLogs(profileId: string): Promise<void>;
  getLogs(profileId: string): Promise<RunLogEvent[]>;
  getRunHistory(): Promise<RunSession[]>;
  setRunLogArchiving(enabled: boolean): Promise<RunLogArchiveState>;
  readRunLogArchive(
    sessionId: string,
    beforeOffset?: number | null,
    maxLines?: number | null,
  ): Promise<RunLogArchivePage>;
  deleteRunLogArchive(sessionId: string): Promise<void>;
  openPort(port: number, protocol: Protocol): Promise<void>;
  openProjectDirectory(projectId: string): Promise<void>;
  shutdownApp(): Promise<void>;
  hideToTray(): Promise<void>;
  setCloseBehavior(behavior: CloseBehavior): Promise<AppSettings>;
  setLanguagePreference(preference: AppSettings["languagePreference"]): Promise<AppSettings>;
  requestElevatedMonitoring(): Promise<void>;
  pickProjectDirectory(): Promise<string | null>;
  onRunStatus(handler: (event: RunStatusEvent) => void): Promise<Unlisten>;
  onRunLog(handler: (event: RunLogEvent) => void): Promise<Unlisten>;
  onArchiveClosed(handler: (event: ArchiveClosedEvent) => void): Promise<Unlisten>;
  onPortSnapshot(handler: (snapshot: DashboardSnapshot) => void): Promise<Unlisten>;
  onLifecycleError(handler: (message: string) => void): Promise<Unlisten>;
  onTrayRestoreRequested(handler: () => void): Promise<Unlisten>;
  onTrayQuitRequested(handler: () => void): Promise<Unlisten>;
  onWindowCloseChoiceRequested(handler: () => void): Promise<Unlisten>;
}
