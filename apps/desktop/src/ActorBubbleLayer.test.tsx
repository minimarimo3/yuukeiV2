import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ActorSnapshot } from "@yuukei/protocol";
import { ActorBubbleLayer } from "./ActorBubbleLayer";

describe("ActorBubbleLayer", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders a scrollable comic bubble near the projected actor anchor", () => {
    render(
      <ActorBubbleLayer
        actors={[
          [
            "yuukei",
            actorSnapshot(
              "とても長いセリフです。\n画面端でも折り返して、長い単語aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaをこぼさない。"
            )
          ]
        ]}
        anchors={{
          yuukei: {
            x: 64,
            y: 120,
            visible: true
          }
        }}
        viewport={{
          width: 640,
          height: 360
        }}
      />
    );

    const content = screen.getByText(/とても長いセリフです/);
    expect(content).toHaveClass("actor-bubble-content");

    const bubble = content.closest(".actor-bubble");
    expect(bubble).toHaveAttribute("data-actor-solid", "true");
    expect(bubble).toHaveClass("actor-bubble--right");
    expect(bubble).toHaveStyle({ left: "90px" });
    expect(bubble?.getAttribute("style")).toContain("--actor-bubble-max-width");
    expect(bubble?.querySelector(".actor-bubble-tail")).not.toBeNull();
  });

  it("falls back to safe separate anchors when actor anchors are unavailable", () => {
    render(
      <ActorBubbleLayer
        actors={[
          ["yuukei", actorSnapshot("ひとつめ")],
          ["another", actorSnapshot("ふたつめ")]
        ]}
        anchors={{}}
        viewport={{
          width: 500,
          height: 260
        }}
      />
    );

    const first = screen.getByText("ひとつめ").closest(".actor-bubble");
    const second = screen.getByText("ふたつめ").closest(".actor-bubble");

    expect(first).toHaveAttribute("data-actor-solid", "true");
    expect(second).toHaveAttribute("data-actor-solid", "true");
    expect(first?.getAttribute("style")).not.toEqual(second?.getAttribute("style"));
  });
});

function actorSnapshot(bubble: string): ActorSnapshot {
  return {
    displayName: "Yuukei",
    expression: "neutral",
    motion: "idle",
    location: "desktop",
    speaking: true,
    bubble
  };
}
