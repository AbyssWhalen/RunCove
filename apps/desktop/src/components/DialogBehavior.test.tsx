import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { ConfirmModal } from "./ConfirmModal";

function DialogHost() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>Open dialog</button>
      {open && (
        <ConfirmModal
          title="Confirm action"
          detail="This action needs confirmation."
          confirmLabel="Continue"
          onCancel={() => setOpen(false)}
          onConfirm={() => undefined}
        />
      )}
    </>
  );
}

describe("dialog behavior", () => {
  it("traps focus, closes on Escape, and restores the trigger", async () => {
    const user = userEvent.setup();
    render(<DialogHost />);

    const trigger = screen.getByRole("button", { name: "Open dialog" });
    await user.click(trigger);
    const dialog = screen.getByRole("alertdialog", { name: "Confirm action" });
    const close = screen.getByRole("button", { name: "Close" });
    const confirm = screen.getByRole("button", { name: "Continue" });
    expect(close).toHaveFocus();

    confirm.focus();
    await user.tab();
    expect(close).toHaveFocus();

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("cannot be dismissed while a confirmed operation is busy", () => {
    const onCancel = vi.fn();
    render(
      <ConfirmModal
        title="Working"
        detail="The operation is running."
        confirmLabel="Continue"
        busy
        onCancel={onCancel}
        onConfirm={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("alertdialog", { name: "Working" });
    fireEvent.mouseDown(dialog.parentElement!);
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });
});
