import { describe, expect, it } from "vitest";
import {
  computeStageBubblePlacement,
  rectsOverlap,
  type StageRect
} from "./stageBubbleLayout";

describe("stageBubbleLayout", () => {
  it("keeps stage bubbles inside the overlay viewport", () => {
    const placement = computeStageBubblePlacement(
      { x: 790, y: 8, visible: true },
      { width: 800, height: 260 },
      { width: 260, height: 92 }
    );

    expect(placement.left).toBeGreaterThanOrEqual(16);
    expect(placement.left + placement.rect.width).toBeLessThanOrEqual(800 - 16);
    expect(placement.top).toBeGreaterThanOrEqual(16);
    expect(placement.tailLeft).toBeLessThanOrEqual(placement.width - 20);
  });

  it("chooses a side that avoids an actor obstacle", () => {
    const actorObstacle: StageRect = {
      x: 80,
      y: 40,
      width: 420,
      height: 560
    };
    const placement = computeStageBubblePlacement(
      { x: 260, y: 160, visible: true },
      { width: 980, height: 680 },
      { width: 240, height: 80 },
      [actorObstacle]
    );

    expect(rectsOverlap(placement.rect, actorObstacle)).toBe(false);
  });

  it("uses prior bubble placements as obstacles", () => {
    const first = computeStageBubblePlacement(
      { x: 180, y: 120, visible: true },
      { width: 800, height: 420 },
      { width: 240, height: 80 }
    );
    const second = computeStageBubblePlacement(
      { x: 180, y: 120, visible: true },
      { width: 800, height: 420 },
      { width: 240, height: 80 },
      [first.rect]
    );

    expect(rectsOverlap(first.rect, second.rect)).toBe(false);
  });

  it("tracks horizontal tail position for vertical placements", () => {
    const placement = computeStageBubblePlacement(
      { x: 450, y: 360, visible: true },
      { width: 900, height: 640 },
      { width: 260, height: 80 },
      [
        {
          x: 240,
          y: 200,
          width: 420,
          height: 420
        }
      ]
    );

    expect(placement.side).toBe("above");
    expect(placement.tailLeft).toBeGreaterThanOrEqual(20);
    expect(placement.tailLeft).toBeLessThanOrEqual(placement.width - 20);
  });
});
