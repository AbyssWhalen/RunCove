import { FolderOpen, FolderSearch, FolderTree, Plus, Trash2, X } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useRef, useState } from "react";

import { useI18n } from "../i18n";
import type {
  DiscoveredProject,
  LaunchProfileInput,
  Project,
  ProjectInput,
} from "../types";
import { IconButton } from "./IconButton";
import { useDialogFocus } from "./useDialogFocus";

interface ProjectModalProps {
  project?: Project | null;
  initialImportMode?: ImportMode;
  initialRoot?: string;
  initialRootProjects?: DiscoveredProject[];
  onDiscover: (directory: string) => Promise<DiscoveredProject>;
  onScanDevelopmentRoot: (directory: string) => Promise<DiscoveredProject[]>;
  onPickDirectory: () => Promise<string | null>;
  onSave: (input: ProjectInput) => Promise<void>;
  onSaveMany: (inputs: ProjectInput[]) => Promise<void>;
  registeredPaths: string[];
  onClose: () => void;
}

type ImportMode = "project" | "root";

function projectToInput(project: Project): ProjectInput {
  return {
    id: project.id,
    name: project.name,
    path: project.path,
    profiles: project.profiles.map((profile) => ({
      id: profile.id,
      name: profile.name,
      program: profile.program,
      args: [...profile.args],
      cwd: profile.cwd,
      expectedPorts: profile.expectedPorts.map((port) => ({
        id: port.id,
        port: port.port,
        protocol: port.protocol,
      })),
    })),
  };
}

function discoveryToInput(discovery: DiscoveredProject): ProjectInput {
  return {
    name: discovery.name,
    path: discovery.path,
    profiles: discovery.profiles.map((profile) => ({
      name: profile.name,
      program: profile.program,
      args: [...profile.args],
      cwd: profile.cwd,
      expectedPorts: profile.expectedPorts.map((port) => ({ ...port })),
      observedRuntime: profile.observedRuntime,
    })),
  };
}

function emptyProfile(path: string): LaunchProfileInput {
  return {
    name: "Dev",
    program: "npm.cmd",
    args: ["run", "dev"],
    cwd: path,
    expectedPorts: [],
  };
}

function normalizedPath(path: string): string {
  return path.replace(/[\\/]+$/, "").toLowerCase();
}

function isImportableDiscovery(
  project: DiscoveredProject,
  registeredPaths: Set<string>,
): boolean {
  return project.profiles.length > 0 && !registeredPaths.has(normalizedPath(project.path));
}

export function ProjectModal({
  project,
  initialImportMode,
  initialRoot,
  initialRootProjects,
  onDiscover,
  onScanDevelopmentRoot,
  onPickDirectory,
  onSave,
  onSaveMany,
  registeredPaths,
  onClose,
}: ProjectModalProps) {
  const { t } = useI18n();
  const [importMode, setImportMode] = useState<ImportMode>(initialImportMode ?? (initialRootProjects ? "root" : "project"));
  const [directory, setDirectory] = useState(initialRoot ?? project?.path ?? "");
  const [form, setForm] = useState<ProjectInput | null>(project ? projectToInput(project) : null);
  const [rootProjects, setRootProjects] = useState<DiscoveredProject[] | null>(initialRootProjects ?? null);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [discovering, setDiscovering] = useState(false);
  const [pickingDirectory, setPickingDirectory] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const discoveryInFlight = useRef(false);
  const pickerInFlight = useRef(false);
  const modalBusy = discovering || pickingDirectory || saving;
  const { dialogRef, onDialogKeyDown } = useDialogFocus(onClose, modalBusy);
  const registeredPathSet = useMemo(
    () => new Set(registeredPaths.map(normalizedPath)),
    [registeredPaths],
  );

  useEffect(() => {
    setSelectedPaths((current) => new Set(
      [...current].filter((path) => {
        const discovered = rootProjects?.find((item) => item.path === path);
        return discovered && isImportableDiscovery(discovered, registeredPathSet);
      }),
    ));
  }, [registeredPathSet, rootProjects]);

  useEffect(() => {
    if (!initialRootProjects) return;
    setSelectedPaths(new Set(
      initialRootProjects
        .filter((item) => isImportableDiscovery(item, registeredPathSet))
        .map((item) => item.path),
    ));
  }, [initialRootProjects, registeredPathSet]);

  const valid = useMemo(() => {
    if (!form?.name.trim() || !form.path.trim() || form.profiles.length === 0) return false;
    return form.profiles.every((profile) =>
      profile.name.trim() &&
      profile.program.trim() &&
      profile.cwd.trim() &&
      profile.args.every((arg) => arg.trim()) &&
      profile.expectedPorts.every((port) => Number.isInteger(port.port) && port.port > 0 && port.port <= 65_535),
    );
  }, [form]);

  const discover = async () => {
    const target = directory.trim();
    if (!target || discoveryInFlight.current) return;
    discoveryInFlight.current = true;
    setDiscovering(true);
    setError(null);
    try {
      const result = await onDiscover(target);
      setForm(discoveryToInput(result));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      discoveryInFlight.current = false;
      setDiscovering(false);
    }
  };

  const scanRoot = async () => {
    const target = directory.trim();
    if (!target || discoveryInFlight.current) return;
    discoveryInFlight.current = true;
    setDiscovering(true);
    setError(null);
    try {
      const results = await onScanDevelopmentRoot(target);
      setRootProjects(results);
      setSelectedPaths(new Set(
        results
          .filter((item) => isImportableDiscovery(item, registeredPathSet))
          .map((item) => item.path),
      ));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      discoveryInFlight.current = false;
      setDiscovering(false);
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!form || !valid) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(form);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setSaving(false);
    }
  };

  const submitMany = async (event: FormEvent) => {
    event.preventDefault();
    if (!rootProjects || selectedPaths.size === 0) return;
    setSaving(true);
    setError(null);
    try {
      await onSaveMany(
        rootProjects
          .filter((item) => selectedPaths.has(item.path) && item.profiles.length > 0)
          .map(discoveryToInput),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setSaving(false);
    }
  };

  const switchImportMode = (mode: ImportMode) => {
    setImportMode(mode);
    setDirectory("");
    setForm(null);
    setRootProjects(null);
    setSelectedPaths(new Set());
    setError(null);
  };

  const chooseDirectory = async () => {
    if (pickerInFlight.current) return;
    pickerInFlight.current = true;
    setPickingDirectory(true);
    setError(null);
    try {
      const selected = await onPickDirectory();
      if (selected) setDirectory(selected);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      pickerInFlight.current = false;
      setPickingDirectory(false);
    }
  };

  const updateProfile = (index: number, update: Partial<LaunchProfileInput>) => {
    setForm((current) => current && ({
      ...current,
      profiles: current.profiles.map((profile, profileIndex) =>
        profileIndex === index ? { ...profile, ...update } : profile,
      ),
    }));
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={modalBusy ? undefined : onClose}>
      <section
        ref={dialogRef}
        className="modal project-modal"
        role="dialog"
        aria-modal="true"
        aria-busy={modalBusy}
        aria-labelledby="project-modal-title"
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-header">
          <div>
            <h2 id="project-modal-title">{project ? t("project.edit") : t("project.import")}</h2>
            {project && <span className="modal-subtitle">{project.path}</span>}
          </div>
          <IconButton label={t("action.close")} onClick={onClose} disabled={modalBusy}>
            <X size={16} />
          </IconButton>
        </div>

        {!project && (
          <div className="import-mode-bar">
            <div className="segmented-control" aria-label={t("project.importSource")}>
              <button
                type="button"
                className={importMode === "project" ? "is-selected" : ""}
                aria-pressed={importMode === "project"}
                disabled={modalBusy}
                onClick={() => switchImportMode("project")}
              >
                {t("project.single")}
              </button>
              <button
                type="button"
                className={importMode === "root" ? "is-selected" : ""}
                aria-pressed={importMode === "root"}
                disabled={modalBusy}
                onClick={() => switchImportMode("root")}
              >
                {t("project.developmentRoot")}
              </button>
            </div>
          </div>
        )}

        {!project && importMode === "root" ? (
          rootProjects ? (
            <form className="root-import-form" onSubmit={(event) => void submitMany(event)}>
              <div className="root-scan-heading">
                <div>
                  <span className="root-scan-icon" aria-hidden="true"><FolderTree size={16} /></span>
                  <div>
                    <strong>{t("count.projectsFound", { count: rootProjects.length })}</strong>
                    <span title={directory}>{directory}</span>
                  </div>
                </div>
                <button
                  className="button button--secondary button--compact"
                  type="button"
                  onClick={() => {
                    setRootProjects(null);
                    setSelectedPaths(new Set());
                    setError(null);
                  }}
                >
                  {t("action.changeRoot")}
                </button>
              </div>

              <div className="root-selection-toolbar">
                <span>{t("count.selected", { count: selectedPaths.size })}</span>
                <div>
                  <button
                    type="button"
                    className="text-command"
                    onClick={() => setSelectedPaths(new Set(
                      rootProjects
                        .filter((item) => isImportableDiscovery(item, registeredPathSet))
                        .map((item) => item.path),
                    ))}
                  >
                    {t("action.selectAll")}
                  </button>
                  <button type="button" className="text-command" onClick={() => setSelectedPaths(new Set())}>
                    {t("action.clear")}
                  </button>
                </div>
              </div>

              <div className="root-project-list">
                {rootProjects.map((item) => {
                  const registered = registeredPathSet.has(normalizedPath(item.path));
                  const hasServiceProfiles = item.profiles.length > 0;
                  const selectable = hasServiceProfiles && !registered;
                  return (
                    <label className={`root-project-row ${selectable ? "" : "is-disabled"}`} key={item.path}>
                      <input
                        type="checkbox"
                        checked={selectedPaths.has(item.path)}
                        disabled={!selectable}
                        aria-label={t("project.select", { project: item.name })}
                        onChange={(event) => setSelectedPaths((current) => {
                          const next = new Set(current);
                          if (event.target.checked) next.add(item.path);
                          else next.delete(item.path);
                          return next;
                        })}
                      />
                      <span className="project-monogram" aria-hidden="true">{item.name.slice(0, 1).toUpperCase()}</span>
                      <span className="root-project-identity">
                        <strong>{item.name}</strong>
                        <span title={item.path}>{item.path}</span>
                      </span>
                      <span className="root-project-meta">
                        <span>{item.packageManager ?? t("project.unknownPackageManager")}</span>
                        <span>{t("count.serviceProfiles", { count: item.profiles.length })}</span>
                        {!hasServiceProfiles && <span className="registered-label">{t("project.noServiceScripts")}</span>}
                        {registered && <span className="registered-label">{t("project.registered")}</span>}
                      </span>
                    </label>
                  );
                })}
                {rootProjects.length === 0 && <div className="empty-state">{t("project.noProjectsFound")}</div>}
              </div>

              {error && <div className="inline-error" role="alert">{error}</div>}
              <div className="modal-actions root-import-actions">
                <button className="button button--secondary" type="button" onClick={onClose} disabled={modalBusy}>{t("action.cancel")}</button>
                <button className="button button--primary" type="submit" disabled={selectedPaths.size === 0 || saving}>
                  {saving ? t("project.importing") : t("count.importProjects", { count: selectedPaths.size })}
                </button>
              </div>
            </form>
          ) : (
            <div className="discover-panel">
              <div className="field field--grow">
                <label htmlFor="development-root-directory">{t("project.developmentRoot")}</label>
                <div className="path-picker-row">
                  <input
                    id="development-root-directory"
                    autoFocus
                    value={directory}
                    disabled={modalBusy}
                    onChange={(event) => setDirectory(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void scanRoot();
                      }
                    }}
                    placeholder="D:\\CodexProject\\personal-projects"
                  />
                  <IconButton label={t("project.chooseRoot")} onClick={() => void chooseDirectory()} disabled={modalBusy}>
                    <FolderOpen size={15} />
                  </IconButton>
                </div>
              </div>
              <button className="button button--primary" onClick={() => void scanRoot()} disabled={!directory.trim() || discovering}>
                <FolderSearch size={16} />
                {discovering ? t("project.scanning") : t("project.scanRoot")}
              </button>
              {error && <div className="inline-error" role="alert">{error}</div>}
            </div>
          )
        ) : !form ? (
          <div className="discover-panel">
            <div className="field field--grow">
              <label htmlFor="import-project-directory">{t("project.directory")}</label>
              <div className="path-picker-row">
                <input
                  id="import-project-directory"
                  autoFocus
                  value={directory}
                  disabled={modalBusy}
                  onChange={(event) => setDirectory(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void discover();
                    }
                  }}
                  placeholder="D:\\CodexProject\\personal-projects\\my-project"
                />
                <IconButton
                  label={t("project.chooseDirectory")}
                  onClick={() => void chooseDirectory()}
                  disabled={modalBusy}
                >
                  <FolderOpen size={15} />
                </IconButton>
              </div>
            </div>
            <button className="button button--primary" onClick={() => void discover()} disabled={!directory.trim() || discovering}>
              <FolderSearch size={16} />
              {discovering ? t("project.inspecting") : t("project.inspectDirectory")}
            </button>
            {error && <div className="inline-error" role="alert">{error}</div>}
          </div>
        ) : (
          <form className="project-form" onSubmit={(event) => void submit(event)}>
            <div className="form-grid form-grid--project">
              <label className="field">
                <span>{t("project.name")}</span>
                <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
              </label>
              <label className="field field--path">
                <span>{t("project.directory")}</span>
                <input value={form.path} onChange={(event) => setForm({ ...form, path: event.target.value })} />
              </label>
            </div>

            <div className="profile-editor-heading">
              <div>
                <h3>{t("project.launchProfiles")}</h3>
                <span>{t("count.configured", { count: form.profiles.length })}</span>
              </div>
              <button
                className="button button--secondary button--compact"
                type="button"
                onClick={() => setForm({ ...form, profiles: [...form.profiles, emptyProfile(form.path)] })}
              >
                <Plus size={14} />
                {t("action.addProfile")}
              </button>
            </div>

            {!project && (
              <div className="inline-warning" role="status">
                <span>{t("project.safeScripts")}</span>
                {form.profiles.length === 0 && <span>{t("project.addReviewedProfile")}</span>}
              </div>
            )}

            <div className="profile-editors">
              {form.profiles.map((profile, index) => (
                <section className="profile-editor" key={profile.id ?? `new-${index}`}>
                  <div className="profile-editor-title">
                    <div className="profile-editor-title-label">
                      <span>{t("project.profileNumber", { number: index + 1 })}</span>
                      {profile.observedRuntime && (
                        <span className="association-badge association-badge--suggested">{t("project.observed")}</span>
                      )}
                    </div>
                    <IconButton
                      label={t("project.removeProfile", { number: index + 1 })}
                      tone="danger"
                      disabled={form.profiles.length === 1}
                      onClick={() => setForm({ ...form, profiles: form.profiles.filter((_, itemIndex) => itemIndex !== index) })}
                    >
                      <Trash2 size={14} />
                    </IconButton>
                  </div>
                  <div className="form-grid form-grid--profile">
                    <label className="field">
                      <span>{t("project.profileName")}</span>
                      <input value={profile.name} onChange={(event) => updateProfile(index, { name: event.target.value })} />
                    </label>
                    <label className="field">
                      <span>{t("project.program")}</span>
                      <input value={profile.program} onChange={(event) => updateProfile(index, { program: event.target.value })} />
                    </label>
                    <label className="field field--wide">
                      <span>{t("project.workingDirectory")}</span>
                      <input value={profile.cwd} onChange={(event) => updateProfile(index, { cwd: event.target.value })} />
                    </label>
                    <div className="field field--args">
                      <span>{t("project.arguments")}</span>
                      <div className="argument-input-list">
                        {profile.args.map((argument, argumentIndex) => (
                          <div className="argument-input-row" key={`argument-${argumentIndex}`}>
                            <input
                              aria-label={t("project.argumentLabel", { profile: index + 1, argument: argumentIndex + 1 })}
                              value={argument}
                              onChange={(event) => updateProfile(index, {
                                args: profile.args.map((item, itemIndex) => itemIndex === argumentIndex ? event.target.value : item),
                              })}
                            />
                            <IconButton
                              label={t("project.removeArgument", { number: argumentIndex + 1 })}
                              tone="danger"
                              onClick={() => updateProfile(index, {
                                args: profile.args.filter((_, itemIndex) => itemIndex !== argumentIndex),
                              })}
                            >
                              <X size={14} />
                            </IconButton>
                          </div>
                        ))}
                        <button className="text-command" type="button" onClick={() => updateProfile(index, { args: [...profile.args, ""] })}>
                          <Plus size={13} /> {t("action.addArgument")}
                        </button>
                      </div>
                    </div>
                    <div className="field field--ports">
                      <span>{t("table.expectedPorts")}</span>
                      <div className="port-input-list">
                        {profile.expectedPorts.map((port, portIndex) => (
                          <div className="port-input-row" key={port.id ?? `port-${portIndex}`}>
                            <input
                              type="number"
                              min="1"
                              max="65535"
                              aria-label={t("project.expectedPortLabel", { profile: index + 1, port: portIndex + 1 })}
                              value={port.port || ""}
                              onChange={(event) => updateProfile(index, {
                                expectedPorts: profile.expectedPorts.map((item, itemIndex) =>
                                  itemIndex === portIndex ? { ...item, port: Number(event.target.value) } : item,
                                ),
                              })}
                            />
                            <select
                              aria-label={t("project.protocolLabel", { profile: index + 1, port: portIndex + 1 })}
                              value={port.protocol}
                              onChange={(event) => updateProfile(index, {
                                expectedPorts: profile.expectedPorts.map((item, itemIndex) =>
                                  itemIndex === portIndex ? { ...item, protocol: event.target.value as "tcp" | "udp" } : item,
                                ),
                              })}
                            >
                              <option value="tcp">TCP</option>
                              <option value="udp">UDP</option>
                            </select>
                            <IconButton
                              label={t("project.removeExpectedPort", { port: port.port })}
                              tone="danger"
                              onClick={() => updateProfile(index, {
                                expectedPorts: profile.expectedPorts.filter((_, itemIndex) => itemIndex !== portIndex),
                              })}
                            >
                              <X size={14} />
                            </IconButton>
                          </div>
                        ))}
                        <button
                          className="text-command"
                          type="button"
                          onClick={() => updateProfile(index, {
                            expectedPorts: [...profile.expectedPorts, { port: 3000, protocol: "tcp" }],
                          })}
                        >
                          <Plus size={13} /> {t("action.addPort")}
                        </button>
                      </div>
                    </div>
                  </div>
                </section>
              ))}
            </div>

            {error && <div className="inline-error" role="alert">{error}</div>}
            <div className="modal-actions">
              <div className="modal-action-group">
                <button className="button button--secondary" type="button" onClick={onClose} disabled={modalBusy}>{t("action.cancel")}</button>
                <button className="button button--primary" type="submit" disabled={!valid || saving}>
                  {saving ? t("project.saving") : t("project.save")}
                </button>
              </div>
            </div>
          </form>
        )}
      </section>
    </div>
  );
}
