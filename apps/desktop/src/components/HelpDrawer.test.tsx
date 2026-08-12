import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { HelpDrawer } from "./HelpDrawer";

vi.mock("../i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

describe("HelpDrawer", () => {
  it("opens on the requested initial topic", () => {
    render(<HelpDrawer initialTopic="projects" closeBehavior="ask" onClose={vi.fn()} onNavigate={vi.fn()} onResetCloseBehavior={vi.fn()} />);

    expect(screen.getByRole("dialog")).toHaveAccessibleName("help.title");
    expect(screen.getByRole("tab", { name: "help.topic.projects" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent("help.projects.title");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("help.projects.item4Detail");
  });

  it("switches topics with tabs and the keyboard", async () => {
    const user = userEvent.setup();
    render(<HelpDrawer initialTopic="quickStart" closeBehavior="ask" onClose={vi.fn()} onNavigate={vi.fn()} onResetCloseBehavior={vi.fn()} />);

    const portsTab = screen.getByRole("tab", { name: "help.topic.ports" });
    await user.click(portsTab);
    expect(portsTab).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("help.ports.intro");

    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "help.topic.projects" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent("help.projects.title");
  });

  it("navigates to the view linked from the active topic", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    render(<HelpDrawer initialTopic="ports" closeBehavior="ask" onClose={vi.fn()} onNavigate={onNavigate} onResetCloseBehavior={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "help.openPorts" }));
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(onNavigate).toHaveBeenCalledWith("ports");
  });

  it("closes on Escape", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<HelpDrawer initialTopic="safety" closeBehavior="ask" onClose={onClose} onNavigate={vi.fn()} onResetCloseBehavior={vi.fn()} />);

    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("shows and resets a remembered title-bar close behavior", async () => {
    const user = userEvent.setup();
    const onResetCloseBehavior = vi.fn();
    render(
      <HelpDrawer
        initialTopic="safety"
        closeBehavior="quit"
        onClose={vi.fn()}
        onNavigate={vi.fn()}
        onResetCloseBehavior={onResetCloseBehavior}
      />,
    );

    expect(screen.getByRole("tabpanel")).toHaveTextContent("help.closeBehaviorQuit");
    await user.click(screen.getByRole("button", { name: "help.closeBehaviorReset" }));
    expect(onResetCloseBehavior).toHaveBeenCalledOnce();
  });
});
