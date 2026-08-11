import {
  ArrowRight,
  FolderKanban,
  Network,
  Rocket,
  ShieldAlert,
  ShieldCheck,
  X,
  type LucideIcon,
} from "lucide-react";
import { type KeyboardEvent, useRef, useState } from "react";

import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";
import type { CloseBehavior } from "../types";
import { IconButton } from "./IconButton";
import { useDialogFocus } from "./useDialogFocus";

export type HelpTopic = "quickStart" | "ports" | "projects" | "access" | "safety";

interface HelpDrawerProps {
  initialTopic: HelpTopic;
  closeBehavior: CloseBehavior;
  closeBehaviorBusy?: boolean;
  onClose: () => void;
  onNavigate: (view: "ports" | "projects") => void;
  onResetCloseBehavior: () => void;
}

const topics = [
  { id: "quickStart", icon: Rocket },
  { id: "ports", icon: Network },
  { id: "projects", icon: FolderKanban },
  { id: "access", icon: ShieldCheck },
  { id: "safety", icon: ShieldAlert },
] as const satisfies ReadonlyArray<{ id: HelpTopic; icon: LucideIcon }>;

const topicCopy = {
  quickStart: {
    label: "help.topic.quickStart",
    title: "help.quickStart.title",
    intro: "help.quickStart.intro",
    items: [
      ["help.quickStart.item1Title", "help.quickStart.item1Detail"],
      ["help.quickStart.item2Title", "help.quickStart.item2Detail"],
      ["help.quickStart.item3Title", "help.quickStart.item3Detail"],
      ["help.quickStart.item4Title", "help.quickStart.item4Detail"],
    ],
    action: { label: "help.openProjects", view: "projects" },
  },
  ports: {
    label: "help.topic.ports",
    title: "help.ports.title",
    intro: "help.ports.intro",
    items: [
      ["help.ports.item1Title", "help.ports.item1Detail"],
      ["help.ports.item2Title", "help.ports.item2Detail"],
      ["help.ports.item3Title", "help.ports.item3Detail"],
      ["help.ports.item4Title", "help.ports.item4Detail"],
    ],
    action: { label: "help.openPorts", view: "ports" },
  },
  projects: {
    label: "help.topic.projects",
    title: "help.projects.title",
    intro: "help.projects.intro",
    items: [
      ["help.projects.item1Title", "help.projects.item1Detail"],
      ["help.projects.item2Title", "help.projects.item2Detail"],
      ["help.projects.item3Title", "help.projects.item3Detail"],
      ["help.projects.item4Title", "help.projects.item4Detail"],
    ],
    action: { label: "help.openProjects", view: "projects" },
  },
  access: {
    label: "help.topic.access",
    title: "help.access.title",
    intro: "help.access.intro",
    items: [
      ["help.access.item1Title", "help.access.item1Detail"],
      ["help.access.item2Title", "help.access.item2Detail"],
      ["help.access.item3Title", "help.access.item3Detail"],
      ["help.access.item4Title", "help.access.item4Detail"],
    ],
  },
  safety: {
    label: "help.topic.safety",
    title: "help.safety.title",
    intro: "help.safety.intro",
    items: [
      ["help.safety.item1Title", "help.safety.item1Detail"],
      ["help.safety.item2Title", "help.safety.item2Detail"],
      ["help.safety.item3Title", "help.safety.item3Detail"],
      ["help.safety.item4Title", "help.safety.item4Detail"],
    ],
  },
} as const;

const closeBehaviorMessage: Record<CloseBehavior, MessageKey> = {
  ask: "help.closeBehaviorAsk",
  hideToTray: "help.closeBehaviorHide",
  quit: "help.closeBehaviorQuit",
};

export function HelpDrawer({
  initialTopic,
  closeBehavior,
  closeBehaviorBusy,
  onClose,
  onNavigate,
  onResetCloseBehavior,
}: HelpDrawerProps) {
  const { t } = useI18n();
  const [activeTopic, setActiveTopic] = useState<HelpTopic>(initialTopic);
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>(onClose);
  const copy = topicCopy[activeTopic];

  const selectTopic = (topic: HelpTopic, index: number) => {
    setActiveTopic(topic);
    tabRefs.current[index]?.focus();
  };

  const handleTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let nextIndex: number | undefined;
    if (event.key === "ArrowRight") nextIndex = (index + 1) % topics.length;
    if (event.key === "ArrowLeft") nextIndex = (index - 1 + topics.length) % topics.length;
    if (event.key === "Home") nextIndex = 0;
    if (event.key === "End") nextIndex = topics.length - 1;
    if (nextIndex === undefined) return;

    event.preventDefault();
    selectTopic(topics[nextIndex].id, nextIndex);
  };

  return (
    <div className="help-backdrop" role="presentation" onMouseDown={onClose}>
      <aside
        ref={dialogRef}
        className="help-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="help-drawer-title"
        aria-describedby="help-drawer-subtitle"
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="help-header">
          <div className="help-heading">
            <h2 id="help-drawer-title">{t("help.title")}</h2>
            <p id="help-drawer-subtitle">{t("help.subtitle")}</p>
          </div>
          <IconButton className="help-close" label={t("action.close")} onClick={onClose}>
            <X size={16} />
          </IconButton>
        </header>

        <div className="help-layout">
          <nav className="help-topics" role="tablist" aria-label={t("help.title")}>
            {topics.map((topic, index) => {
              const Icon = topic.icon;
              const selected = topic.id === activeTopic;
              return (
                <button
                  key={topic.id}
                  ref={(element) => { tabRefs.current[index] = element; }}
                  type="button"
                  className={`help-topic${selected ? " help-topic--active" : ""}`}
                  id={`help-tab-${topic.id}`}
                  role="tab"
                  aria-selected={selected}
                  aria-controls="help-panel"
                  tabIndex={selected ? 0 : -1}
                  autoFocus={selected}
                  onClick={() => setActiveTopic(topic.id)}
                  onKeyDown={(event) => handleTabKeyDown(event, index)}
                >
                  <Icon size={17} aria-hidden="true" />
                  <span>{t(topicCopy[topic.id].label)}</span>
                </button>
              );
            })}
          </nav>

          <section
            className="help-content"
            id="help-panel"
            role="tabpanel"
            aria-labelledby={`help-tab-${activeTopic}`}
            tabIndex={0}
          >
            <div className="help-introduction">
              <h3>{t(copy.title)}</h3>
              <p>{t(copy.intro)}</p>
            </div>
            <ol className="help-steps">
              {copy.items.map(([title, detail], index) => (
                <li className="help-step" key={title}>
                  <span className="help-step-number" aria-hidden="true">{index + 1}</span>
                  <div className="help-step-copy">
                    <h4>{t(title)}</h4>
                    <p>{t(detail)}</p>
                  </div>
                </li>
              ))}
            </ol>
            {"action" in copy && (
              <div className="help-footer">
                <button
                  type="button"
                  className="help-navigate"
                  onClick={() => onNavigate(copy.action.view)}
                >
                  <span>{t(copy.action.label)}</span>
                  <ArrowRight size={16} aria-hidden="true" />
                </button>
              </div>
            )}
            {activeTopic === "safety" && (
              <div className="help-close-behavior">
                <div>
                  <h4>{t("help.closeBehaviorTitle")}</h4>
                  <p className="help-close-behavior-current">{t(closeBehaviorMessage[closeBehavior])}</p>
                  <p>{t("help.closeBehaviorDetail")}</p>
                </div>
                {closeBehavior !== "ask" && (
                  <button
                    type="button"
                    className="button button--secondary button--compact"
                    disabled={closeBehaviorBusy}
                    onClick={onResetCloseBehavior}
                  >
                    {closeBehaviorBusy
                      ? t("help.closeBehaviorResetting")
                      : t("help.closeBehaviorReset")}
                  </button>
                )}
              </div>
            )}
          </section>
        </div>
      </aside>
    </div>
  );
}
