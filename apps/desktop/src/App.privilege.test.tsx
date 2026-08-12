import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";

describe("RunCove enhanced monitoring", () => {
  beforeEach(async () => {
    resetMockApi();
    window.localStorage.setItem("runcove.language", "en");
    await api.setLanguagePreference("en");
    vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([]);
  });

  afterEach(() => vi.restoreAllMocks());

  it("requires confirmation before requesting Windows elevation", async () => {
    const requestElevation = vi.spyOn(api, "requestElevatedMonitoring");
    const user = userEvent.setup();
    render(<App />);

    const privilegeButton = await screen.findByRole("button", {
      name: "Standard monitoring. Request enhanced access",
    });
    await user.click(privilegeButton);

    expect(screen.getByRole("alertdialog", { name: "Restart with enhanced monitoring?" }))
      .toHaveTextContent("Windows UAC will ask for administrator access");
    expect(requestElevation).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog", { name: "Restart with enhanced monitoring?" }))
      .not.toBeInTheDocument();
    expect(requestElevation).not.toHaveBeenCalled();

    await user.click(privilegeButton);
    await user.click(screen.getByRole("button", { name: "Continue to Windows UAC" }));
    await waitFor(() => expect(requestElevation).toHaveBeenCalledTimes(1));
  });

  it("makes the elevated instance visibly monitor-only and disables process actions", async () => {
    await api.requestElevatedMonitoring();
    const requestElevation = vi.spyOn(api, "requestElevatedMonitoring");
    const startProfile = vi.spyOn(api, "startProfile");
    render(<App />);

    const privilegeButton = await screen.findByRole("button", {
      name: "Enhanced monitoring is active (read-only)",
    });
    expect(privilegeButton).toBeDisabled();
    expect(screen.getByText("Administrator monitor-only mode")).toBeInTheDocument();
    expect(screen.getByText(/development commands never run as administrator/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restore previous run" })).toBeDisabled();
    const disabledStart = screen.getAllByRole("button", {
      name: /Unavailable in administrator monitor-only mode/,
    }).find((button) => button.getAttribute("aria-label")?.startsWith("Start "));
    expect(disabledStart).toBeDisabled();
    expect(requestElevation).not.toHaveBeenCalled();
    expect(startProfile).not.toHaveBeenCalled();
  });
});
