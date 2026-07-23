import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StartupErrorApp } from "./StartupErrorApp";

afterEach(cleanup);

describe("StartupErrorApp", () => {
  it("shows the failed pack root and runtime error", async () => {
    render(
      <StartupErrorApp
        loadError={async () => ({
          packRoot: "/tmp/packs/default-yuukei",
          detail: "world error: world pack io error: No such file or directory",
        })}
        quit={async () => {}}
      />,
    );

    expect(
      await screen.findByRole("heading", {
        name: "Yuukeiを安全に起動できませんでした",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("/tmp/packs/default-yuukei")).toBeInTheDocument();
    expect(
      screen.getByText(
        "world error: world pack io error: No such file or directory",
      ),
    ).toBeInTheDocument();
  });

  it("lets the user exit without exposing the normal settings UI", async () => {
    const user = userEvent.setup();
    const quit = vi.fn(async () => {});
    render(
      <StartupErrorApp
        loadError={async () => ({
          packRoot: "/tmp/packs/default-yuukei",
          detail: "pack.json is missing",
        })}
        quit={quit}
      />,
    );

    await user.click(
      await screen.findByRole("button", { name: "Yuukeiを終了" }),
    );
    expect(quit).toHaveBeenCalledOnce();
    expect(screen.queryByText("World Pack設定")).not.toBeInTheDocument();
  });
});
