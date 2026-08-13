import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n";
import type { Project } from "../types";
import { ProjectsView } from "./ProjectsView";

const project: Project = {
  id: "project-docs",
  name: "Docs Lab",
  path: "D:\\projects\\docs-lab",
  createdAt: 1,
  updatedAt: 1,
  profiles: [],
};

describe("ProjectsView history focus", () => {
  it("finishes the focus highlight even when a polling snapshot rerenders the view", () => {
    vi.useFakeTimers();
    const handled = vi.fn();
    const props: React.ComponentProps<typeof ProjectsView> = {
      projects: [project],
      ports: [],
      busyProfileIds: new Set(),
      monitorOnly: false,
      onImport: vi.fn(),
      onAutoDiscover: vi.fn(),
      hasSavedDiscoveryRoot: true,
      onEdit: vi.fn(),
      onDelete: vi.fn(),
      onStart: vi.fn(),
      onStop: vi.fn(),
      onRestart: vi.fn(),
      onOpenPort: vi.fn(),
      onOpenDirectory: vi.fn(),
      onOpenLogs: vi.fn(),
      focusedProjectId: project.id,
      onFocusedProjectHandled: handled,
    };
    const view = render(<I18nProvider><ProjectsView {...props} /></I18nProvider>);

    expect(document.querySelector(".project-section--focused")).not.toBeNull();
    act(() => vi.advanceTimersByTime(2_000));
    const handledAfterRerender = vi.fn();
    view.rerender(
      <I18nProvider>
        <ProjectsView
          {...props}
          projects={[{ ...project, updatedAt: 2 }]}
          onFocusedProjectHandled={handledAfterRerender}
        />
      </I18nProvider>,
    );
    act(() => vi.advanceTimersByTime(500));

    expect(handled).not.toHaveBeenCalled();
    expect(handledAfterRerender).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
});
