import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { ConversationComposer } from "./ConversationComposer";
import type { ConversationSendShortcut } from "./yuukeiClient";

describe("ConversationComposer", () => {
  afterEach(cleanup);

  it.each([
    ["ctrlEnter", { ctrlKey: true }],
    ["enter", {}],
    ["shiftEnter", { shiftKey: true }],
  ] as const)("submits with %s", async (shortcut, modifiers) => {
    const submit = vi.fn(async () => undefined);
    const dismiss = vi.fn();
    const user = userEvent.setup();
    renderComposer(shortcut, submit, dismiss);
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });

    await user.type(input, "こんにちは");
    fireEvent.keyDown(input, { key: "Enter", ...modifiers });

    await waitFor(() => expect(submit).toHaveBeenCalledWith("こんにちは"));
    expect(dismiss).toHaveBeenCalledOnce();
  });

  it("does not submit with an unassigned Enter combination", async () => {
    const submit = vi.fn(async () => undefined);
    renderComposer("ctrlEnter", submit, vi.fn());
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });

    fireEvent.change(input, { target: { value: "一行目" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(submit).not.toHaveBeenCalled();
  });

  it("inserts a newline when plain Enter is not the configured shortcut", async () => {
    const user = userEvent.setup();
    renderComposer(
      "ctrlEnter",
      vi.fn(async () => undefined),
      vi.fn(),
    );
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });

    await user.type(input, "一行目{Enter}二行目");

    expect(input).toHaveValue("一行目\n二行目");
  });

  it("does not submit while the IME is composing", () => {
    const submit = vi.fn(async () => undefined);
    renderComposer("enter", submit, vi.fn());
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });

    fireEvent.change(input, { target: { value: "変換中" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });

    expect(submit).not.toHaveBeenCalled();
  });

  it("ignores whitespace-only submissions", () => {
    const submit = vi.fn(async () => undefined);
    renderComposer("ctrlEnter", submit, vi.fn());
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });

    fireEvent.change(input, { target: { value: "   \n" } });
    fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });

    expect(submit).not.toHaveBeenCalled();
  });

  it("dismisses with Escape", () => {
    const dismiss = vi.fn();
    renderComposer(
      "ctrlEnter",
      vi.fn(async () => undefined),
      dismiss,
    );

    fireEvent.keyDown(
      screen.getByRole("textbox", { name: "住人に話しかける" }),
      { key: "Escape" },
    );

    expect(dismiss).toHaveBeenCalledOnce();
  });

  it("keeps the text and displays an error when sending fails", async () => {
    const submit = vi.fn(async () => {
      throw new Error("送信できませんでした");
    });
    const dismiss = vi.fn();
    renderComposer("ctrlEnter", submit, dismiss);
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });

    fireEvent.change(input, { target: { value: "あとでもう一度" } });
    fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "送信できませんでした",
    );
    expect(input).toHaveValue("あとでもう一度");
    expect(dismiss).not.toHaveBeenCalled();
  });

  it("prevents duplicate submissions while the first send is pending", async () => {
    let finish: (() => void) | undefined;
    const submit = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    renderComposer("ctrlEnter", submit, vi.fn());
    const input = screen.getByRole("textbox", { name: "住人に話しかける" });
    fireEvent.change(input, { target: { value: "一度だけ" } });

    fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });
    fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });

    expect(submit).toHaveBeenCalledTimes(1);
    finish?.();
  });
});

function renderComposer(
  shortcut: ConversationSendShortcut,
  onSubmit: (text: string) => Promise<void>,
  onDismiss: () => void,
) {
  return render(
    <ConversationComposer
      shortcut={shortcut}
      onSubmit={onSubmit}
      onDismiss={onDismiss}
    />,
  );
}
