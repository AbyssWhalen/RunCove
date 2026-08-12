import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CloseChoiceModal } from "./CloseChoiceModal";

vi.mock("../i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

describe("CloseChoiceModal", () => {
  it("defaults focus to the non-destructive tray action", () => {
    render(
      <CloseChoiceModal
        remember={false}
        onRememberChange={vi.fn()}
        onCancel={vi.fn()}
        onChoose={vi.fn()}
      />,
    );

    expect(screen.getByRole("dialog")).toHaveAccessibleName("dialog.closeTitle");
    expect(screen.getByRole("button", { name: /dialog.closeHide/ })).toHaveFocus();
    expect(screen.getByRole("checkbox", { name: "dialog.closeRemember" })).not.toBeChecked();
  });

  it("returns the selected behavior and remember state changes", async () => {
    const user = userEvent.setup();
    const onChoose = vi.fn();
    const onRememberChange = vi.fn();
    render(
      <CloseChoiceModal
        remember={false}
        onRememberChange={onRememberChange}
        onCancel={vi.fn()}
        onChoose={onChoose}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "dialog.closeRemember" }));
    await user.click(screen.getByRole("button", { name: /dialog.closeQuit/ }));

    expect(onRememberChange).toHaveBeenCalledWith(true);
    expect(onChoose).toHaveBeenCalledWith("quit");
  });

  it("cancels with Escape and blocks every control while busy", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const { rerender } = render(
      <CloseChoiceModal
        remember={false}
        onRememberChange={vi.fn()}
        onCancel={onCancel}
        onChoose={vi.fn()}
      />,
    );

    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledOnce();

    rerender(
      <CloseChoiceModal
        remember
        busyAction="quit"
        onRememberChange={vi.fn()}
        onCancel={onCancel}
        onChoose={vi.fn()}
      />,
    );
    expect(screen.getAllByRole("button")).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ disabled: true }),
      ]),
    );
    screen.getAllByRole("button").forEach((button) => expect(button).toBeDisabled());
    expect(screen.getByRole("checkbox")).toBeDisabled();
  });
});
