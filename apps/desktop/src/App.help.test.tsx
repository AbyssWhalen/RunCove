import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";

describe("RunCove in-app help", () => {
  beforeEach(async () => {
    resetMockApi();
    window.localStorage.setItem("runcove.language", "en");
    await api.setLanguagePreference("en");
    vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([]);
  });

  afterEach(() => vi.restoreAllMocks());

  it("opens the guide and navigates from a help topic to Projects", async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Help and usage guide" }));

    const help = screen.getByRole("dialog", { name: "RunCove Help" });
    expect(help).toBeVisible();
    expect(screen.getByRole("tab", { name: "Quick start" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    await user.click(screen.getByRole("tab", { name: "Projects" }));
    expect(screen.getByRole("heading", { name: "Manage projects and launch profiles" }))
      .toBeVisible();
    await user.click(screen.getByRole("button", { name: "Open Projects" }));

    expect(screen.queryByRole("dialog", { name: "RunCove Help" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Projects", level: 1 })).toBeVisible();
  });

  it("opens the contextual Ports topic from the Ports view", async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Ports" }));
    await user.click(screen.getByRole("button", { name: "Help and usage guide" }));

    expect(screen.getByRole("tab", { name: "Ports" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("heading", { name: "Understand the Ports view" })).toBeVisible();
  });

  it("resets a remembered title-bar close choice from Help Safety", async () => {
    await api.setCloseBehavior("quit");
    const save = vi.spyOn(api, "setCloseBehavior");
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Help and usage guide" }));
    await user.click(screen.getByRole("tab", { name: "Safety" }));

    expect(screen.getByText("Current: Quit RunCove")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Ask every time" }));

    await waitFor(() => expect(save).toHaveBeenCalledWith("ask"));
    expect(screen.getByText("Current: Ask every time")).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent(
      "RunCove will ask again the next time the window is closed.",
    );
  });
});
