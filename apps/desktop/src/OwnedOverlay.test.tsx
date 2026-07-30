import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OwnedOverlay } from "./OwnedOverlay";
import type { StageOwnedOverlay } from "./yuukeiClient";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("OwnedOverlay", () => {
  it("renders supplied markup as plain text and dismisses from its close button", () => {
    const onDismiss = vi.fn();
    const { container } = render(
      <OwnedOverlay
        overlay={overlay({
          title: "<img src=x>",
          message: "<script>alert('x')</script>",
        })}
        onDismiss={onDismiss}
      />,
    );

    expect(screen.getByText("<img src=x>")).toBeInTheDocument();
    expect(screen.getByText("<script>alert('x')</script>")).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("script")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: "ごまかし画面を閉じる" }),
    );
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledWith("user-dismissed");
  });

  it("auto-dismisses at the bounded presentation deadline only once", () => {
    vi.useFakeTimers();
    vi.setSystemTime(5_000);
    const onDismiss = vi.fn();
    render(
      <OwnedOverlay
        overlay={overlay({ createdAtMs: 4_000, durationMs: 4_000 })}
        onDismiss={onDismiss}
      />,
    );

    act(() => vi.advanceTimersByTime(2_999));
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(1));
    expect(onDismiss).toHaveBeenCalledOnce();
    expect(onDismiss).toHaveBeenCalledWith("expired");

    fireEvent.click(
      screen.getByRole("button", { name: "ごまかし画面を閉じる" }),
    );
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("focuses the close button, dismisses with Escape, and restores prior focus", () => {
    const previouslyFocused = document.createElement("button");
    previouslyFocused.textContent = "元の操作";
    document.body.append(previouslyFocused);
    previouslyFocused.focus();
    const onDismiss = vi.fn();
    const { unmount } = render(
      <OwnedOverlay overlay={overlay()} onDismiss={onDismiss} />,
    );

    expect(
      screen.getByRole("button", { name: "ごまかし画面を閉じる" }),
    ).toHaveFocus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onDismiss).toHaveBeenCalledOnce();
    expect(onDismiss).toHaveBeenCalledWith("user-dismissed");

    unmount();
    expect(previouslyFocused).toHaveFocus();
    previouslyFocused.remove();
  });

  it("allows another close attempt after the host dismissal rejects", async () => {
    const onDismiss = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary runtime failure"))
      .mockResolvedValueOnce(undefined);
    render(<OwnedOverlay overlay={overlay()} onDismiss={onDismiss} />);
    const close = screen.getByRole("button", {
      name: "ごまかし画面を閉じる",
    });

    fireEvent.click(close);
    await waitFor(() => expect(onDismiss).toHaveBeenCalledTimes(1));
    await act(async () => {
      await Promise.resolve();
    });

    fireEvent.click(close);
    await waitFor(() => expect(onDismiss).toHaveBeenCalledTimes(2));
    expect(onDismiss).toHaveBeenNthCalledWith(2, "user-dismissed");
  });
});

function overlay(
  overrides: Partial<StageOwnedOverlay> = {},
): StageOwnedOverlay {
  return {
    overlayId: "overlay-1",
    actorId: "yuukei",
    style: "error",
    title: "エラー",
    message: "ここは見られません",
    createdAtMs: Date.now(),
    durationMs: 8_000,
    ...overrides,
  };
}
