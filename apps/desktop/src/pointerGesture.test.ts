import { describe, expect, it } from "vitest";
import { beginDragRequested, idlePointerGesture, releasePointerGesture, windowDragBegan, type PointerGestureState } from "./pointerGesture";

const pressing = (semantic = true): PointerGestureState => ({
  type: "pressing", pointerId: 1, actorHit: { actorId: "yuukei" },
  semanticHit: semantic ? { actorId: "yuukei", poke: { actorId: "yuukei", hitZoneId: "head", input: { kind: "pointer", button: "primary" }, screen: { x: 1, y: 2 } } } : null,
  startClient: { x: 1, y: 2 }, startScreen: { x: 10, y: 20 }, maxDistancePx: 0
});

describe("pointer gesture state machine", () => {
  it("turns a normal short release into poke only with a semantic hit", () => {
    expect(releasePointerGesture(pressing(), false).effects.map((e) => e.type)).toEqual(["poke"]);
    expect(releasePointerGesture(pressing(false), false).effects).toEqual([]);
  });
  it("starts a long press from actor hit without semantic hit", () => {
    const result = beginDragRequested(pressing(false));
    expect(result.state.type).toBe("startingDrag");
    expect(result.effects).toEqual([{ type: "beginWindowDrag", actorId: "yuukei" }]);
  });
  it("finishes a released drag but cancels an interrupted drag", () => {
    const dragging = windowDragBegan(beginDragRequested(pressing()).state, "session-1").state;
    expect(releasePointerGesture(dragging, false).effects[0]?.type).toBe("finishWindowDrag");
    expect(releasePointerGesture(dragging, true).effects[0]?.type).toBe("cancelWindowDrag");
  });
  it("never emits poke or finish when a press is cancelled", () => {
    expect(releasePointerGesture(pressing(), true)).toEqual({ state: idlePointerGesture(), effects: [] });
  });
  it("carries release intent while begin is pending and returns idle on begin failure", () => {
    const starting = beginDragRequested(pressing()).state;
    const pendingRelease = releasePointerGesture(starting, true).state;
    expect(windowDragBegan(pendingRelease, "session-1").effects[0]?.type).toBe("cancelWindowDrag");
    expect(idlePointerGesture()).toEqual({ type: "idle" });
  });
});
