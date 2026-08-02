import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ExtensionConsentDialog } from "./ExtensionConsentDialog";
import type { ExtensionInstallInspection } from "./yuukeiClient";

const inspection: ExtensionInstallInspection = {
  extensionId: "desktop-friend",
  displayName: "Desktop Friend",
  runtime: "process",
  permissions: {
    broadEventSubscription: true,
    eventLogRead: {
      eventTypes: ["observation.*"],
      privacyCategories: ["behavioral"],
      allowPayloads: false,
      allowReferences: false,
      maxRecords: 20,
      purpose: "デスクトップ上の出来事に反応するため",
    },
  },
  hooks: [],
  eventSubscriptions: [{ eventTypes: ["*"] }],
  emittedEvents: [],
  capabilities: [],
  manifestDigest: "sha256-desktop-friend",
  trustedCodeNotice:
    "このExtensionはローカルで信頼済みコードとして実行されます。",
};

describe("ExtensionConsentDialog", () => {
  afterEach(cleanup);

  it("shows the fixed permission contract and starts from the safe choice", () => {
    render(
      <ExtensionConsentDialog
        busy={false}
        error={null}
        inspection={inspection}
        onApprove={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("dialog", { name: "Desktop Friend" }),
    ).toHaveTextContent("入手元が信頼できる場合だけ追加してください");
    expect(
      screen.getByText("住人の生活で起きたすべてのできごとを受け取ります"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "許可しない" })).toHaveFocus();
  });

  it("cancels with Escape and approves only from the explicit action", async () => {
    const onApprove = vi.fn();
    const onCancel = vi.fn();
    const user = userEvent.setup();
    const { rerender } = render(
      <ExtensionConsentDialog
        busy={false}
        error={null}
        inspection={inspection}
        onApprove={onApprove}
        onCancel={onCancel}
      />,
    );

    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onApprove).not.toHaveBeenCalled();

    rerender(
      <ExtensionConsentDialog
        busy={false}
        error={null}
        inspection={inspection}
        onApprove={onApprove}
        onCancel={onCancel}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "この内容で許可して追加" }),
    );
    expect(onApprove).toHaveBeenCalledTimes(1);
  });
});
