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
  timestamp: number;
}

export interface RunLogEvent {
  profileId: string;
  stream: "stdout" | "stderr" | "system";
  line: string;
  timestamp: number;
}

export interface RunSession {
  id: string;
  profileId?: string | null;
  profileName: string;
  pid?: number | null;
  startedAt: number;
  endedAt?: number | null;
  exitCode?: number | null;
  status: string;
}

export interface RestoreResult {
  startedProfileIds: string[];
  failedProfileId?: string | null;
  error?: string | null;
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
  onPortSnapshot(handler: (snapshot: DashboardSnapshot) => void): Promise<Unlisten>;
  onLifecycleError(handler: (message: string) => void): Promise<Unlisten>;
  onTrayRestoreRequested(handler: () => void): Promise<Unlisten>;
  onTrayQuitRequested(handler: () => void): Promise<Unlisten>;
  onWindowCloseChoiceRequested(handler: () => void): Promise<Unlisten>;
}
