import type { AvatarGesturePokeInput } from "./yuukeiClient";

export const POINTER_GESTURE_HOLD_MS = 500;
export const POINTER_GESTURE_MOVE_THRESHOLD_PX = 6;

export function shouldStartPointerGestureDrag(
  elapsedMs: number,
  maxDistancePx: number
): boolean {
  return (
    elapsedMs >= POINTER_GESTURE_HOLD_MS &&
    maxDistancePx <= POINTER_GESTURE_MOVE_THRESHOLD_PX
  );
}

export type Point = { x: number; y: number };
export type ActorHit = { actorId: string };
export type SemanticActorHit = {
  actorId: string;
  poke: AvatarGesturePokeInput;
};

type ActivePointerGesture = {
  gestureId: number;
  pointerId: number;
};

export type PointerGestureState =
  // The only state that accepts pointerPressed.
  | { type: "idle" }
  // The actor is pressed, but short press versus long press is undecided.
  | (ActivePointerGesture & {
      type: "pressing";
      actorHit: ActorHit;
      semanticHit: SemanticActorHit | null;
      startClient: Point;
      startScreen: Point;
      maxDistancePx: number;
      holdStatus: "scheduled" | "cancelledByMovement";
    })
  // The long press won; Device Host has not returned a session yet.
  | (ActivePointerGesture & {
      type: "startingDrag";
      actorId: string;
      startScreen: Point;
      releaseIntent: "none" | "finish" | "cancel";
    })
  // Device Host returned an active session that accepts pointerMoved.
  | (ActivePointerGesture & {
      type: "dragging";
      actorId: string;
      sessionId: string;
      startScreen: Point;
    })
  // Normal pointer release occurred; Device Host finish is pending.
  | {
      type: "endingDrag";
      gestureId: number;
      actorId: string;
      sessionId: string;
    }
  // Pointer tracking was interrupted; Device Host cancel is pending.
  | {
      type: "cancellingDrag";
      gestureId: number;
      actorId: string;
      sessionId: string;
    };

export type PointerGestureEvent =
  | (ActivePointerGesture & {
      type: "pointerPressed";
      actorHit: ActorHit;
      semanticHit: SemanticActorHit | null;
      client: Point;
      screen: Point;
    })
  | {
      type: "pointerMoved";
      pointerId: number;
      client: Point;
      screen: Point;
    }
  | (ActivePointerGesture & { type: "holdElapsed" })
  | { type: "pointerReleased"; pointerId: number }
  | { type: "pointerCancelled"; pointerId: number }
  | (ActivePointerGesture & {
      type: "windowDragStarted";
      actorId: string;
      sessionId: string;
    })
  | (ActivePointerGesture & {
      type: "windowDragStartFailed";
      actorId: string;
      error: unknown;
    })
  | {
      type: "windowDragMoved";
      gestureId: number;
      actorId: string;
      sessionId: string;
    }
  | {
      type: "windowDragMoveFailed";
      gestureId: number;
      actorId: string;
      sessionId: string;
      error: unknown;
    }
  | {
      type: "windowDragFinished";
      gestureId: number;
      actorId: string;
      sessionId: string;
      movedDistance: number;
    }
  | {
      type: "windowDragFinishFailed";
      gestureId: number;
      actorId: string;
      sessionId: string;
      error: unknown;
    }
  | {
      type: "windowDragCancelled";
      gestureId: number;
      actorId: string;
      sessionId: string;
    }
  | {
      type: "windowDragCancelFailed";
      gestureId: number;
      actorId: string;
      sessionId: string;
      error: unknown;
    }
  | { type: "avatarPokeNotified"; gestureId: number }
  | { type: "avatarPokeNotifyFailed"; gestureId: number; error: unknown }
  | { type: "avatarGrabNotified"; gestureId: number }
  | { type: "avatarGrabNotifyFailed"; gestureId: number; error: unknown }
  | { type: "avatarDropNotified"; gestureId: number }
  | { type: "avatarDropNotifyFailed"; gestureId: number; error: unknown };

export type PointerGestureEffect =
  | (ActivePointerGesture & {
      type: "capturePointer";
    })
  | (ActivePointerGesture & {
      type: "releasePointerCapture";
    })
  | (ActivePointerGesture & {
      type: "scheduleHold";
      delayMs: number;
    })
  | (ActivePointerGesture & { type: "cancelHold" })
  | {
      type: "notifyPoke";
      gestureId: number;
      poke: AvatarGesturePokeInput;
    }
  | {
      type: "beginWindowDrag";
      gestureId: number;
      pointerId: number;
      actorId: string;
    }
  | {
      type: "moveWindowDrag";
      gestureId: number;
      actorId: string;
      sessionId: string;
      dx: number;
      dy: number;
    }
  | {
      type: "finishWindowDrag";
      gestureId: number;
      actorId: string;
      sessionId: string;
    }
  | {
      type: "cancelWindowDrag";
      gestureId: number;
      actorId: string;
      sessionId: string;
    }
  | { type: "notifyGrab"; gestureId: number; actorId: string }
  | {
      type: "notifyDrop";
      gestureId: number;
      actorId: string;
      movedDistance: number;
    };

export type PointerGestureTransition = {
  state: PointerGestureState;
  effects: PointerGestureEffect[];
};

export const idlePointerGesture = (): PointerGestureState => ({ type: "idle" });

export function acceptsNewPointerInput(state: PointerGestureState): boolean {
  return state.type === "idle";
}

export function reducePointerGesture(
  state: PointerGestureState,
  event: PointerGestureEvent
): PointerGestureTransition {
  if (event.type === "pointerPressed") {
    if (!acceptsNewPointerInput(state)) return unchanged(state);
    return {
      state: {
        type: "pressing",
        gestureId: event.gestureId,
        pointerId: event.pointerId,
        actorHit: event.actorHit,
        semanticHit: event.semanticHit,
        startClient: event.client,
        startScreen: event.screen,
        maxDistancePx: 0,
        holdStatus: "scheduled"
      },
      effects: [
        {
          type: "capturePointer",
          gestureId: event.gestureId,
          pointerId: event.pointerId
        },
        {
          type: "scheduleHold",
          gestureId: event.gestureId,
          pointerId: event.pointerId,
          delayMs: POINTER_GESTURE_HOLD_MS
        }
      ]
    };
  }

  if (event.type === "pointerMoved") {
    if (!matchesPointer(state, event)) return unchanged(state);
    if (state.type === "pressing") {
      const distance = Math.hypot(
        event.client.x - state.startClient.x,
        event.client.y - state.startClient.y
      );
      const maxDistancePx = Math.max(state.maxDistancePx, distance);
      const crossedThreshold =
        state.holdStatus === "scheduled" &&
        maxDistancePx > POINTER_GESTURE_MOVE_THRESHOLD_PX;
      return {
        state: {
          ...state,
          maxDistancePx,
          holdStatus: crossedThreshold
            ? "cancelledByMovement"
            : state.holdStatus
        },
        effects: crossedThreshold
          ? [
              {
                type: "cancelHold",
                gestureId: state.gestureId,
                pointerId: state.pointerId
              }
            ]
          : []
      };
    }
    if (state.type === "dragging") {
      return {
        state,
        effects: [
          {
            type: "moveWindowDrag",
            gestureId: state.gestureId,
            actorId: state.actorId,
            sessionId: state.sessionId,
            dx: event.screen.x - state.startScreen.x,
            dy: event.screen.y - state.startScreen.y
          }
        ]
      };
    }
    return unchanged(state);
  }

  if (event.type === "holdElapsed") {
    if (
      state.type !== "pressing" ||
      !matchesActiveGesture(state, event) ||
      state.holdStatus !== "scheduled" ||
      !shouldStartPointerGestureDrag(
        POINTER_GESTURE_HOLD_MS,
        state.maxDistancePx
      )
    ) {
      return unchanged(state);
    }
    return {
      state: {
        type: "startingDrag",
        gestureId: state.gestureId,
        pointerId: state.pointerId,
        actorId: state.actorHit.actorId,
        startScreen: state.startScreen,
        releaseIntent: "none"
      },
      effects: [
        {
          type: "beginWindowDrag",
          gestureId: state.gestureId,
          pointerId: state.pointerId,
          actorId: state.actorHit.actorId
        }
      ]
    };
  }

  if (event.type === "pointerReleased") {
    return releasePointer(state, event, "finish");
  }

  if (event.type === "pointerCancelled") {
    return releasePointer(state, event, "cancel");
  }

  if (event.type === "windowDragStarted") {
    if (
      state.type !== "startingDrag" ||
      !matchesDragStart(state, event)
    ) {
      return unchanged(state);
    }
    const notifyGrab: PointerGestureEffect = {
      type: "notifyGrab",
      gestureId: state.gestureId,
      actorId: state.actorId
    };
    if (state.releaseIntent === "finish") {
      return {
        state: {
          type: "endingDrag",
          gestureId: state.gestureId,
          actorId: state.actorId,
          sessionId: event.sessionId
        },
        effects: [
          notifyGrab,
          {
            type: "finishWindowDrag",
            gestureId: state.gestureId,
            actorId: state.actorId,
            sessionId: event.sessionId
          }
        ]
      };
    }
    if (state.releaseIntent === "cancel") {
      return {
        state: {
          type: "cancellingDrag",
          gestureId: state.gestureId,
          actorId: state.actorId,
          sessionId: event.sessionId
        },
        effects: [
          notifyGrab,
          {
            type: "cancelWindowDrag",
            gestureId: state.gestureId,
            actorId: state.actorId,
            sessionId: event.sessionId
          }
        ]
      };
    }
    return {
      state: {
        type: "dragging",
        gestureId: state.gestureId,
        pointerId: state.pointerId,
        actorId: state.actorId,
        sessionId: event.sessionId,
        startScreen: state.startScreen
      },
      effects: [notifyGrab]
    };
  }

  if (event.type === "windowDragStartFailed") {
    if (
      state.type !== "startingDrag" ||
      !matchesDragStart(state, event)
    ) {
      return unchanged(state);
    }
    return {
      state: idlePointerGesture(),
      effects: [
        {
          type: "releasePointerCapture",
          gestureId: state.gestureId,
          pointerId: state.pointerId
        }
      ]
    };
  }

  if (
    event.type === "windowDragMoved" ||
    event.type === "windowDragMoveFailed"
  ) {
    if (state.type !== "dragging" || !matchesDragSession(state, event)) {
      return unchanged(state);
    }
    // Move failures are treated as transient. The session stays active so a
    // later move or the final finish/cancel can still restore physical state.
    return unchanged(state);
  }

  if (event.type === "windowDragFinished") {
    if (state.type !== "endingDrag" || !matchesDragSession(state, event)) {
      return unchanged(state);
    }
    return {
      state: idlePointerGesture(),
      effects: [
        {
          type: "notifyDrop",
          gestureId: state.gestureId,
          actorId: event.actorId,
          movedDistance: event.movedDistance
        }
      ]
    };
  }

  if (event.type === "windowDragFinishFailed") {
    if (state.type !== "endingDrag" || !matchesDragSession(state, event)) {
      return unchanged(state);
    }
    return { state: idlePointerGesture(), effects: [] };
  }

  if (
    event.type === "windowDragCancelled" ||
    event.type === "windowDragCancelFailed"
  ) {
    if (
      state.type !== "cancellingDrag" ||
      !matchesDragSession(state, event)
    ) {
      return unchanged(state);
    }
    return { state: idlePointerGesture(), effects: [] };
  }

  if (
    event.type === "avatarPokeNotified" ||
    event.type === "avatarPokeNotifyFailed" ||
    event.type === "avatarGrabNotified" ||
    event.type === "avatarGrabNotifyFailed" ||
    event.type === "avatarDropNotified" ||
    event.type === "avatarDropNotifyFailed"
  ) {
    // Semantic notifications never hold the physical input state open.
    return unchanged(state);
  }

  const unhandledEvent: never = event;
  void unhandledEvent;
  return unchanged(state);
}

function releasePointer(
  state: PointerGestureState,
  event: Extract<
    PointerGestureEvent,
    { type: "pointerReleased" | "pointerCancelled" }
  >,
  intent: "finish" | "cancel"
): PointerGestureTransition {
  if (!matchesPointer(state, event)) return unchanged(state);
  const releaseCapture: PointerGestureEffect = {
    type: "releasePointerCapture",
    gestureId: state.gestureId,
    pointerId: state.pointerId
  };
  if (state.type === "pressing") {
    const effects: PointerGestureEffect[] = [
      {
        type: "cancelHold",
        gestureId: state.gestureId,
        pointerId: state.pointerId
      },
      releaseCapture
    ];
    if (intent === "finish" && state.semanticHit) {
      effects.push({
        type: "notifyPoke",
        gestureId: state.gestureId,
        poke: state.semanticHit.poke
      });
    }
    return { state: idlePointerGesture(), effects };
  }
  if (state.type === "startingDrag") {
    return {
      state: { ...state, releaseIntent: intent },
      effects: [releaseCapture]
    };
  }
  if (state.type === "dragging") {
    const common = {
      gestureId: state.gestureId,
      actorId: state.actorId,
      sessionId: state.sessionId
    };
    if (intent === "finish") {
      return {
        state: { type: "endingDrag", ...common },
        effects: [
          releaseCapture,
          { type: "finishWindowDrag", ...common }
        ]
      };
    }
    return {
      state: { type: "cancellingDrag", ...common },
      effects: [releaseCapture, { type: "cancelWindowDrag", ...common }]
    };
  }
  return unchanged(state);
}

function matchesPointer(
  state: PointerGestureState,
  event: { pointerId: number }
): state is Extract<
  PointerGestureState,
  { type: "pressing" | "startingDrag" | "dragging" }
> {
  return "pointerId" in state && state.pointerId === event.pointerId;
}

function matchesActiveGesture(
  state: PointerGestureState,
  event: ActivePointerGesture
): state is Extract<
  PointerGestureState,
  { type: "pressing" | "startingDrag" | "dragging" }
> {
  return (
    matchesPointer(state, event) && state.gestureId === event.gestureId
  );
}

function matchesDragStart(
  state: Extract<PointerGestureState, { type: "startingDrag" }>,
  event: { gestureId: number; pointerId: number; actorId: string }
): boolean {
  return (
    state.gestureId === event.gestureId &&
    state.pointerId === event.pointerId &&
    state.actorId === event.actorId
  );
}

function matchesDragSession(
  state: Extract<
    PointerGestureState,
    { type: "dragging" | "endingDrag" | "cancellingDrag" }
  >,
  event: { gestureId: number; actorId: string; sessionId: string }
): boolean {
  return (
    state.gestureId === event.gestureId &&
    state.actorId === event.actorId &&
    state.sessionId === event.sessionId
  );
}

function unchanged(state: PointerGestureState): PointerGestureTransition {
  return { state, effects: [] };
}
