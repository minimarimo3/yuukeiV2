import { describe, expect, it } from "vitest";
import {
  idlePointerGesture,
  reducePointerGesture,
  type PointerGestureEffect,
  type PointerGestureEvent,
  type PointerGestureState,
} from "./pointerGesture";

const gestureId = 11;
const pointerId = 7;
const actorId = "yuukei";
const sessionId = "session-1";

const semanticHit = {
  actorId,
  poke: {
    actorId,
    hitZoneId: "head",
    input: { kind: "pointer" as const, button: "primary" },
    screen: { x: 10, y: 20 },
  },
};

function pointerPressed(
  semantic = true,
  overrides: Partial<
    Extract<PointerGestureEvent, { type: "pointerPressed" }>
  > = {},
): Extract<PointerGestureEvent, { type: "pointerPressed" }> {
  return {
    type: "pointerPressed",
    gestureId,
    pointerId,
    actorHit: { actorId },
    semanticHit: semantic ? semanticHit : null,
    client: { x: 1, y: 2 },
    screen: { x: 10, y: 20 },
    ...overrides,
  };
}

function transition(state: PointerGestureState, event: PointerGestureEvent) {
  return reducePointerGesture(state, event);
}

function pressing(semantic = true) {
  return transition(idlePointerGesture(), pointerPressed(semantic)).state;
}

function startingDrag() {
  return transition(pressing(), {
    type: "holdElapsed",
    gestureId,
    pointerId,
  }).state;
}

function dragging() {
  return transition(startingDrag(), {
    type: "windowDragStarted",
    gestureId,
    pointerId,
    actorId,
    sessionId,
  }).state;
}

function effectsOfType<T extends PointerGestureEffect["type"]>(
  effects: PointerGestureEffect[],
  type: T,
) {
  return effects.filter(
    (effect): effect is Extract<PointerGestureEffect, { type: T }> =>
      effect.type === type,
  );
}

describe("pointer gesture state machine", () => {
  describe("normal input", () => {
    it("moves from idle to pressing and schedules one hold on pointerPressed", () => {
      const result = transition(idlePointerGesture(), pointerPressed());

      expect(result.state).toMatchObject({
        type: "pressing",
        gestureId,
        pointerId,
        holdStatus: "scheduled",
      });
      expect(effectsOfType(result.effects, "scheduleHold")).toHaveLength(1);
    });

    it("returns from pressing to idle on pointerReleased", () => {
      const result = transition(pressing(), {
        type: "pointerReleased",
        pointerId,
      });

      expect(result.state).toEqual(idlePointerGesture());
    });

    it("emits notifyPoke for a semantic short press", () => {
      const result = transition(pressing(true), {
        type: "pointerReleased",
        pointerId,
      });

      expect(effectsOfType(result.effects, "notifyPoke")).toEqual([
        { type: "notifyPoke", gestureId, poke: semanticHit.poke },
      ]);
    });

    it("does not emit notifyPoke without a semantic hit", () => {
      const result = transition(pressing(false), {
        type: "pointerReleased",
        pointerId,
      });

      expect(effectsOfType(result.effects, "notifyPoke")).toEqual([]);
    });

    it("moves from pressing to startingDrag and begins once on holdElapsed", () => {
      const first = transition(pressing(false), {
        type: "holdElapsed",
        gestureId,
        pointerId,
      });
      const repeated = transition(first.state, {
        type: "holdElapsed",
        gestureId,
        pointerId,
      });

      expect(first.state).toMatchObject({
        type: "startingDrag",
        releaseIntent: "none",
      });
      expect(effectsOfType(first.effects, "beginWindowDrag")).toHaveLength(1);
      expect(repeated.effects).toEqual([]);
    });

    it("cancels the pending hold once movement crosses the threshold", () => {
      const firstMove = transition(pressing(), {
        type: "pointerMoved",
        pointerId,
        client: { x: 8, y: 2 },
        screen: { x: 17, y: 20 },
      });
      const laterMove = transition(firstMove.state, {
        type: "pointerMoved",
        pointerId,
        client: { x: 9, y: 2 },
        screen: { x: 18, y: 20 },
      });
      const elapsed = transition(laterMove.state, {
        type: "holdElapsed",
        gestureId,
        pointerId,
      });

      expect(firstMove.state).toMatchObject({
        type: "pressing",
        holdStatus: "cancelledByMovement",
      });
      expect(effectsOfType(firstMove.effects, "cancelHold")).toHaveLength(1);
      expect(effectsOfType(laterMove.effects, "cancelHold")).toHaveLength(0);
      expect(elapsed.state).toEqual(laterMove.state);
      expect(elapsed.effects).toEqual([]);
    });

    it("moves to dragging with the Device Host session on windowDragStarted", () => {
      const result = transition(startingDrag(), {
        type: "windowDragStarted",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });

      expect(result.state).toMatchObject({
        type: "dragging",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });
      expect(effectsOfType(result.effects, "notifyGrab")).toHaveLength(1);
    });

    it("emits moveWindowDrag with displacement on pointerMoved while dragging", () => {
      const result = transition(dragging(), {
        type: "pointerMoved",
        pointerId,
        client: { x: 4, y: 5 },
        screen: { x: 25, y: 45 },
      });

      expect(effectsOfType(result.effects, "moveWindowDrag")).toEqual([
        {
          type: "moveWindowDrag",
          gestureId,
          actorId,
          sessionId,
          dx: 15,
          dy: 25,
        },
      ]);
    });

    it("moves from dragging to endingDrag on pointerReleased", () => {
      const result = transition(dragging(), {
        type: "pointerReleased",
        pointerId,
      });

      expect(result.state).toEqual({
        type: "endingDrag",
        gestureId,
        actorId,
        sessionId,
      });
      expect(effectsOfType(result.effects, "finishWindowDrag")).toHaveLength(1);
    });

    it("returns to idle before emitting notifyDrop on windowDragFinished", () => {
      const ending = transition(dragging(), {
        type: "pointerReleased",
        pointerId,
      }).state;
      const result = transition(ending, {
        type: "windowDragFinished",
        gestureId,
        actorId,
        sessionId,
        movedDistance: 42,
      });

      expect(result.state).toEqual(idlePointerGesture());
      expect(effectsOfType(result.effects, "notifyDrop")).toEqual([
        { type: "notifyDrop", gestureId, actorId, movedDistance: 42 },
      ]);
    });
  });

  describe("cancellation", () => {
    it("returns pressing to idle without poke on pointerCancelled", () => {
      const result = transition(pressing(), {
        type: "pointerCancelled",
        pointerId,
      });

      expect(result.state).toEqual(idlePointerGesture());
      expect(effectsOfType(result.effects, "notifyPoke")).toEqual([]);
    });

    it("moves dragging to cancellingDrag and requests cancel", () => {
      const result = transition(dragging(), {
        type: "pointerCancelled",
        pointerId,
      });

      expect(result.state).toEqual({
        type: "cancellingDrag",
        gestureId,
        actorId,
        sessionId,
      });
      expect(effectsOfType(result.effects, "cancelWindowDrag")).toHaveLength(1);
      expect(effectsOfType(result.effects, "notifyDrop")).toEqual([]);
    });

    it("returns cancellingDrag to idle when Device Host cancellation succeeds", () => {
      const cancelling = transition(dragging(), {
        type: "pointerCancelled",
        pointerId,
      }).state;
      const result = transition(cancelling, {
        type: "windowDragCancelled",
        gestureId,
        actorId,
        sessionId,
      });

      expect(result.state).toEqual(idlePointerGesture());
      expect(result.effects).toEqual([]);
    });

    it("returns cancellingDrag to idle when Device Host cancellation fails", () => {
      const cancelling = transition(dragging(), {
        type: "pointerCancelled",
        pointerId,
      }).state;
      const result = transition(cancelling, {
        type: "windowDragCancelFailed",
        gestureId,
        actorId,
        sessionId,
        error: new Error("cancel failed"),
      });

      expect(result.state).toEqual(idlePointerGesture());
    });
  });

  describe("asynchronous races and failures", () => {
    it("catches up pointer movement received while drag start is pending", () => {
      const moved = transition(startingDrag(), {
        type: "pointerMoved",
        pointerId,
        client: { x: 30, y: 35 },
        screen: { x: 40, y: 55 },
      });
      const started = transition(moved.state, {
        type: "windowDragStarted",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });

      expect(started.state).toMatchObject({
        type: "dragging",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });
      expect(effectsOfType(started.effects, "moveWindowDrag")).toEqual([
        {
          type: "moveWindowDrag",
          gestureId,
          actorId,
          sessionId,
          dx: 30,
          dy: 35,
        },
      ]);
    });

    it("applies pending movement before finish when released during drag start", () => {
      const moved = transition(startingDrag(), {
        type: "pointerMoved",
        pointerId,
        client: { x: 70, y: 15 },
        screen: { x: 80, y: 35 },
      });
      const released = transition(moved.state, {
        type: "pointerReleased",
        pointerId,
      });
      const started = transition(released.state, {
        type: "windowDragStarted",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });

      expect(started.state.type).toBe("endingDrag");
      expect(effectsOfType(started.effects, "moveWindowDrag")).toEqual([
        {
          type: "moveWindowDrag",
          gestureId,
          actorId,
          sessionId,
          dx: 70,
          dy: 15,
        },
      ]);
      const moveIndex = started.effects.findIndex(
        (effect) => effect.type === "moveWindowDrag",
      );
      const finishIndex = started.effects.findIndex(
        (effect) => effect.type === "finishWindowDrag",
      );
      expect(moveIndex).toBeGreaterThanOrEqual(0);
      expect(finishIndex).toBeGreaterThan(moveIndex);
    });

    it("records finish intent while start is pending and finishes after start", () => {
      const released = transition(startingDrag(), {
        type: "pointerReleased",
        pointerId,
      });
      const started = transition(released.state, {
        type: "windowDragStarted",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });

      expect(released.state).toMatchObject({
        type: "startingDrag",
        releaseIntent: "finish",
      });
      expect(started.state.type).toBe("endingDrag");
      expect(effectsOfType(started.effects, "finishWindowDrag")).toHaveLength(
        1,
      );
    });

    it("records cancel intent while start is pending and cancels after start", () => {
      const cancelled = transition(startingDrag(), {
        type: "pointerCancelled",
        pointerId,
      });
      const started = transition(cancelled.state, {
        type: "windowDragStarted",
        gestureId,
        pointerId,
        actorId,
        sessionId,
      });

      expect(cancelled.state).toMatchObject({
        type: "startingDrag",
        releaseIntent: "cancel",
      });
      expect(started.state.type).toBe("cancellingDrag");
      expect(effectsOfType(started.effects, "cancelWindowDrag")).toHaveLength(
        1,
      );
      expect(effectsOfType(started.effects, "notifyGrab")).toHaveLength(0);
    });

    it("returns startingDrag to idle on windowDragStartFailed", () => {
      const result = transition(startingDrag(), {
        type: "windowDragStartFailed",
        gestureId,
        pointerId,
        actorId,
        error: new Error("begin failed"),
      });

      expect(result.state).toEqual(idlePointerGesture());
    });

    it("returns endingDrag to idle on windowDragFinishFailed", () => {
      const ending = transition(dragging(), {
        type: "pointerReleased",
        pointerId,
      }).state;
      const result = transition(ending, {
        type: "windowDragFinishFailed",
        gestureId,
        actorId,
        sessionId,
        error: new Error("finish failed"),
      });

      expect(result.state).toEqual(idlePointerGesture());
    });

    it("keeps dragging after a move failure", () => {
      const active = dragging();
      const result = transition(active, {
        type: "windowDragMoveFailed",
        gestureId,
        actorId,
        sessionId,
        error: new Error("move failed"),
      });

      expect(result.state).toEqual(active);
      expect(result.effects).toEqual([]);
    });

    it("keeps dragging when grab notification fails", () => {
      const active = dragging();
      const result = transition(active, {
        type: "avatarGrabNotifyFailed",
        gestureId,
        error: new Error("grab notification failed"),
      });

      expect(result.state).toEqual(active);
    });

    it("keeps idle when drop notification fails", () => {
      const result = transition(idlePointerGesture(), {
        type: "avatarDropNotifyFailed",
        gestureId,
        error: new Error("drop notification failed"),
      });

      expect(result.state).toEqual(idlePointerGesture());
    });

    it("keeps idle when poke notification fails", () => {
      const result = transition(idlePointerGesture(), {
        type: "avatarPokeNotifyFailed",
        gestureId,
        error: new Error("poke notification failed"),
      });

      expect(result.state).toEqual(idlePointerGesture());
    });
  });

  describe("invalid and stale events", () => {
    it("ignores pointerReleased while idle", () => {
      const result = transition(idlePointerGesture(), {
        type: "pointerReleased",
        pointerId,
      });

      expect(result).toEqual({ state: idlePointerGesture(), effects: [] });
    });

    it("ignores stale windowDragStarted from another gesture", () => {
      const active = startingDrag();
      const result = transition(active, {
        type: "windowDragStarted",
        gestureId: gestureId + 1,
        pointerId,
        actorId,
        sessionId: "stale-session",
      });

      expect(result).toEqual({ state: active, effects: [] });
    });

    it("ignores pointerMoved from another pointer while dragging", () => {
      const active = dragging();
      const result = transition(active, {
        type: "pointerMoved",
        pointerId: pointerId + 1,
        client: { x: 50, y: 50 },
        screen: { x: 50, y: 50 },
      });

      expect(result).toEqual({ state: active, effects: [] });
    });

    it("does not accept a new pointerPressed while endingDrag", () => {
      const ending = transition(dragging(), {
        type: "pointerReleased",
        pointerId,
      }).state;
      const result = transition(
        ending,
        pointerPressed(true, {
          gestureId: gestureId + 1,
          pointerId: pointerId + 1,
        }),
      );

      expect(result).toEqual({ state: ending, effects: [] });
    });
  });
});
