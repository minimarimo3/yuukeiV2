import type { AvatarGesturePokeInput } from "./yuukeiClient";

export type Point = { x: number; y: number };
export type ActorHit = { actorId: string };
export type SemanticActorHit = { actorId: string; poke: AvatarGesturePokeInput };

export type PointerGestureState =
  | { type: "idle" }
  | { type: "pressing"; pointerId: number; actorHit: ActorHit; semanticHit: SemanticActorHit | null; startClient: Point; startScreen: Point; maxDistancePx: number }
  | { type: "startingDrag"; pointerId: number; actorId: string; startScreen: Point; release: "none" | "finish" | "cancel" }
  | { type: "dragging"; pointerId: number; actorId: string; sessionId: string; startScreen: Point }
  | { type: "endingDrag"; actorId: string; sessionId: string; ending: "finish" | "cancel" };

export type PointerGestureEffect =
  | { type: "poke"; poke: AvatarGesturePokeInput }
  | { type: "beginWindowDrag"; actorId: string }
  | { type: "finishWindowDrag"; actorId: string; sessionId: string }
  | { type: "cancelWindowDrag"; actorId: string; sessionId: string };

export type PointerGestureTransition = { state: PointerGestureState; effects: PointerGestureEffect[] };

export const idlePointerGesture = (): PointerGestureState => ({ type: "idle" });

export function releasePointerGesture(state: PointerGestureState, cancelled: boolean): PointerGestureTransition {
  if (state.type === "pressing") {
    return { state: idlePointerGesture(), effects: cancelled || !state.semanticHit ? [] : [{ type: "poke", poke: state.semanticHit.poke }] };
  }
  if (state.type === "startingDrag") {
    return { state: { ...state, release: cancelled ? "cancel" : "finish" }, effects: [] };
  }
  if (state.type === "dragging") {
    const ending = cancelled ? "cancel" : "finish";
    return { state: { type: "endingDrag", actorId: state.actorId, sessionId: state.sessionId, ending }, effects: [{ type: cancelled ? "cancelWindowDrag" : "finishWindowDrag", actorId: state.actorId, sessionId: state.sessionId }] };
  }
  return { state, effects: [] };
}

export function beginDragRequested(state: PointerGestureState): PointerGestureTransition {
  if (state.type !== "pressing") return { state, effects: [] };
  return { state: { type: "startingDrag", pointerId: state.pointerId, actorId: state.actorHit.actorId, startScreen: state.startScreen, release: "none" }, effects: [{ type: "beginWindowDrag", actorId: state.actorHit.actorId }] };
}

export function windowDragBegan(state: PointerGestureState, sessionId: string): PointerGestureTransition {
  if (state.type !== "startingDrag") return { state, effects: [] };
  if (state.release !== "none") {
    const ending = state.release;
    return { state: { type: "endingDrag", actorId: state.actorId, sessionId, ending }, effects: [{ type: ending === "cancel" ? "cancelWindowDrag" : "finishWindowDrag", actorId: state.actorId, sessionId }] };
  }
  return { state: { type: "dragging", pointerId: state.pointerId, actorId: state.actorId, sessionId, startScreen: state.startScreen }, effects: [] };
}
