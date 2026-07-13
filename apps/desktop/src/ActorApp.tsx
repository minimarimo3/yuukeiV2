import {
  type VRM,
  VRMHumanBoneName,
  VRMLoaderPlugin,
  VRMUtils,
} from "@pixiv/three-vrm";
import {
  createVRMAnimationClip,
  type VRMAnimation,
  VRMAnimationLoaderPlugin,
} from "@pixiv/three-vrm-animation";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import {
  autoHitZoneDefinitions,
  buildAvatarGesturePokePayload,
  dominantSkinBoneForIntersection,
  type HitSurface,
  hitSurfaceForIntersection,
  hitZoneForLineageOrHumanoidBone,
  humanoidBoneNameForObject,
  mergeHitZoneDefinitions,
  type ResolvedActorHitZone,
} from "./actorHitZones";
import {
  type ActorHit,
  acceptsNewPointerInput,
  idlePointerGesture,
  POINTER_GESTURE_HOLD_MS,
  POINTER_GESTURE_MOVE_THRESHOLD_PX,
  type PointerGestureEffect,
  type PointerGestureEvent,
  type PointerGestureState,
  reducePointerGesture,
  type SemanticActorHit,
  shouldStartPointerGestureDrag,
} from "./pointerGesture";
import { createPointerGestureSemanticQueue } from "./pointerGestureSemanticQueue";
import {
  type ActorSurfaceAsset,
  type ActorSurfaceRendererAsset,
  type AvatarGesturePokeInput,
  type StageAnchor,
  tauriYuukeiClient,
  type YuukeiClient,
} from "./yuukeiClient";

type ActorAppProps = {
  actorId?: string | null;
  client?: YuukeiClient;
};

type LoadedActor = {
  actorId: string;
  vrm: VRM;
  mixer: THREE.AnimationMixer;
  actions: Map<string, THREE.AnimationAction>;
  currentMotionId: string | null;
  baseRotationY: number;
  hitZones: ResolvedActorHitZone[];
  boneNodes: Map<string, THREE.Object3D>;
  humanoidBoneByObject: Map<THREE.Object3D, string>;
  mouthOffsetY: number;
};

type VrmStageProps = {
  assets: ActorSurfaceAsset[];
  snapshot: ResidentSnapshot | null;
  onStageAnchorReport(actorId: string, anchor: StageAnchor): Promise<void>;
  onHitTestChange(passthrough: boolean): Promise<void>;
  onAvatarGesturePoke(gesture: AvatarGesturePokeInput): Promise<void>;
  onConversationOpen(actorId: string): Promise<void>;
  client: Pick<
    YuukeiClient,
    | "beginActorWindowDrag"
    | "moveActorWindowDrag"
    | "finishActorWindowDrag"
    | "cancelActorWindowDrag"
    | "notifyAvatarGestureGrab"
    | "notifyAvatarGestureDrop"
  >;
};

export const AVATAR_GRAB_HOLD_MS = POINTER_GESTURE_HOLD_MS;
export const AVATAR_GRAB_MOVE_THRESHOLD_PX = POINTER_GESTURE_MOVE_THRESHOLD_PX;
export const shouldStartAvatarGrab = shouldStartPointerGestureDrag;

export function ActorApp({
  actorId,
  client = tauriYuukeiClient,
}: ActorAppProps) {
  const activeActorId = useMemo(
    () => actorId ?? actorIdFromLocation(),
    [actorId],
  );
  const [snapshot, setSnapshot] = useState<ResidentSnapshot | null>(null);
  const [assets, setAssets] = useState<ActorSurfaceAsset[]>([]);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      try {
        unlisteners.push(
          await client.onSnapshot((nextSnapshot) => {
            setSnapshot(nextSnapshot);
          }),
        );
        unlisteners.push(
          await client.onCommand((command) => {
            setSnapshot((current) => applyCommandHint(current, command));
          }),
        );
        unlisteners.push(
          await client.onAssetsChanged((catalog) => {
            setAssets(catalog.actors);
          }),
        );
        const { snapshot: initialSnapshot, assets: initialAssets } =
          await loadInitialActorSurfaceState(client);
        if (!disposed) {
          setSnapshot(initialSnapshot);
          setAssets(initialAssets);
          setStatus(null);
        }
      } catch (error) {
        if (!disposed) {
          setStatus(error instanceof Error ? error.message : String(error));
        }
      }
    }

    void connect();
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
      void client.setActorWindowClickThrough(false);
    };
  }, [client]);

  const actorAssets = useMemo(
    () => actorSurfaceAssetsForActor(assets, activeActorId),
    [assets, activeActorId],
  );
  const visibleStatus = status ?? (activeActorId ? null : "actorId is missing");
  const setClickThrough = useCallback(
    (passthrough: boolean) => client.setActorWindowClickThrough(passthrough),
    [client],
  );
  const sendAvatarGesturePoke = useCallback(
    async (gesture: AvatarGesturePokeInput) => {
      await client.sendAvatarGesturePoke(gesture);
    },
    [client],
  );
  const reportStageAnchor = useCallback(
    async (reportedActorId: string, anchor: StageAnchor) => {
      await client.reportActorStageAnchor(reportedActorId, anchor);
    },
    [client],
  );
  const openConversation = useCallback(
    async (reportedActorId: string) => {
      await client.openConversationComposer(reportedActorId);
    },
    [client],
  );

  return (
    <main className="actor-shell" aria-label="Yuukei actor surface">
      <VrmStage
        assets={actorAssets}
        snapshot={snapshot}
        onStageAnchorReport={reportStageAnchor}
        onHitTestChange={setClickThrough}
        onAvatarGesturePoke={sendAvatarGesturePoke}
        onConversationOpen={openConversation}
        client={client}
      />
      {visibleStatus ? (
        <p className="actor-status" data-actor-solid="true" role="alert">
          {visibleStatus}
        </p>
      ) : null}
    </main>
  );
}

export async function loadInitialActorSurfaceState(
  client: YuukeiClient,
): Promise<{
  snapshot: ResidentSnapshot;
  assets: ActorSurfaceAsset[];
}> {
  const [snapshot, catalog] = await Promise.all([
    client.attachSurface(),
    client.getActorSurfaceAssets(),
  ]);
  return {
    snapshot,
    assets: catalog.actors,
  };
}

function VrmStage({
  assets,
  snapshot,
  onStageAnchorReport,
  onHitTestChange,
  onAvatarGesturePoke,
  onConversationOpen,
  client,
}: VrmStageProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const snapshotRef = useRef<ResidentSnapshot | null>(snapshot);
  const rendererRef = useRef<THREE.WebGLRenderer | null>(null);

  useEffect(() => {
    snapshotRef.current = snapshot;
  }, [snapshot]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const stageElement = containerRef.current;
    if (!canvas || !stageElement) return;
    const checkedCanvas = canvas;
    const checkedStageElement = stageElement;

    let disposed = false;
    let animationFrame = 0;
    let hitTestTimer = 0;
    let lastPassthrough: boolean | null = null;
    let lastAnchorSignature = "";
    let gesture: PointerGestureState = idlePointerGesture();
    let nextGestureId = 0;
    const holdTimers = new Map<number, number>();
    let moveFrame = 0;
    let moveQueue = Promise.resolve();
    // Meaning notifications stay ordered without delaying physical drag state.
    const semanticNotificationQueue = createPointerGestureSemanticQueue();
    let pendingMoveEffect: Extract<
      PointerGestureEffect,
      { type: "moveWindowDrag" }
    > | null = null;

    const actorRoot = new THREE.Group();
    const scene = new THREE.Scene();
    scene.add(actorRoot);
    scene.add(new THREE.AmbientLight(0xffffff, 1.8));

    const keyLight = new THREE.DirectionalLight(0xffffff, 2.3);
    keyLight.position.set(2.5, 4, 3);
    scene.add(keyLight);

    const fillLight = new THREE.DirectionalLight(0xdde7ff, 1.2);
    fillLight.position.set(-3, 2, 2);
    scene.add(fillLight);

    const camera = new THREE.PerspectiveCamera(28, 1, 0.05, 100);
    camera.position.set(0, 1.2, 4);

    const renderer = new THREE.WebGLRenderer({
      alpha: true,
      antialias: true,
      canvas,
      preserveDrawingBuffer: true,
    });
    rendererRef.current = renderer;
    renderer.setClearColor(0x000000, 0);
    renderer.outputColorSpace = THREE.SRGBColorSpace;

    const loadedActors = new Map<string, LoadedActor>();
    const clock = new THREE.Clock();
    const semanticRaycaster = new THREE.Raycaster();

    function resize() {
      const width = Math.max(checkedStageElement.clientWidth, 1);
      const height = Math.max(checkedStageElement.clientHeight, 1);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      frameCamera(actorRoot, camera);
    }

    async function loadActors() {
      const vrmAssets = assets.filter(hasVrmRenderer);
      const modelLoader = new GLTFLoader();
      modelLoader.register((parser) => new VRMLoaderPlugin(parser));

      for (const asset of vrmAssets) {
        if (disposed) return;
        const gltf = await modelLoader.loadAsync(asset.renderer.modelUrl);
        const vrm = gltf.userData.vrm as VRM | undefined;
        if (!vrm) continue;

        VRMUtils.rotateVRM0(vrm);
        vrm.scene.name = `actor-${asset.actorId}`;
        vrm.scene.position.set(0, 0, 0);
        actorRoot.add(vrm.scene);
        const boneNodes = humanoidBoneNodes(vrm);
        const humanoidBoneByObject = reverseHumanoidBoneNodes(boneNodes);
        const hitZones = mergeHitZoneDefinitions(
          autoHitZoneDefinitions(new Set(boneNodes.keys())),
          asset.renderer.hitZones ?? [],
        );

        const loaded: LoadedActor = {
          actorId: asset.actorId,
          vrm,
          mixer: new THREE.AnimationMixer(vrm.scene),
          actions: new Map(),
          currentMotionId: null,
          baseRotationY: vrm.scene.rotation.y,
          hitZones,
          boneNodes,
          humanoidBoneByObject,
          mouthOffsetY: estimateMouthOffsetY(vrm.scene),
        };
        loadedActors.set(asset.actorId, loaded);
        await loadMotionActions(asset.renderer, loaded);
      }
      resize();
    }

    function animate() {
      if (disposed) return;
      const delta = clock.getDelta();
      const currentSnapshot = snapshotRef.current;
      for (const loaded of loadedActors.values()) {
        const actor = currentSnapshot?.actors[loaded.actorId];
        applyExpression(loaded.vrm, actor?.expression);
        applyMotion(loaded, actor?.motion);
        loaded.vrm.scene.rotation.y = headingRotationY(
          loaded.baseRotationY,
          actor?.heading,
        );
        loaded.mixer.update(delta);
        loaded.vrm.update(delta);
      }
      publishStageAnchors();
      renderer.render(scene, camera);
      animationFrame = window.requestAnimationFrame(animate);
    }

    function publishStageAnchors() {
      const anchors = projectActorMouthAnchors(
        renderer.domElement,
        camera,
        loadedActors,
      );
      const signature = anchorSignature(anchors);
      if (signature === lastAnchorSignature) return;
      lastAnchorSignature = signature;
      for (const [reportedActorId, anchor] of Object.entries(anchors)) {
        void onStageAnchorReport(reportedActorId, anchor).catch((error) => {
          console.warn("Failed to report actor stage anchor", error);
        });
      }
    }

    async function updateClickThrough() {
      if (!isTauriRuntime()) return;
      const solid = await pointerHitsVisibleSurface(renderer);
      const passthrough = !solid;
      if (lastPassthrough !== passthrough) {
        lastPassthrough = passthrough;
        await onHitTestChange(passthrough);
      }
    }

    function handlePointerDown(event: PointerEvent) {
      if (
        !shouldBeginActorPointerGesture(event) ||
        !acceptsNewPointerInput(gesture)
      )
        return;
      const actorHit = actorAtPointer(
        event,
        renderer.domElement,
        camera,
        loadedActors,
        semanticRaycaster,
      );
      if (!actorHit) return;
      const hit = semanticHitAtPointer(
        event,
        renderer.domElement,
        camera,
        loadedActors,
        semanticRaycaster,
      );
      event.preventDefault();
      const semanticHit: SemanticActorHit | null = hit
        ? {
            actorId: hit.actorId,
            poke: buildAvatarGesturePokePayload(hit.actorId, hit.zone, event, {
              hitBone: hit.hitBone,
              hitSurface: hit.hitSurface,
            }),
          }
        : null;
      nextGestureId += 1;
      dispatchPointerGesture({
        type: "pointerPressed",
        gestureId: nextGestureId,
        pointerId: event.pointerId,
        actorHit,
        semanticHit,
        client: { x: event.clientX, y: event.clientY },
        screen: { x: event.screenX, y: event.screenY },
      });
    }

    function handleContextMenu(event: MouseEvent) {
      const actorHit = actorAtPointer(
        event as PointerEvent,
        renderer.domElement,
        camera,
        loadedActors,
        semanticRaycaster,
      );
      if (!actorHit) return;
      void openConversationFromContextMenu(
        event,
        actorHit.actorId,
        onConversationOpen,
      ).catch((error) => {
        console.warn("Failed to open conversation composer", error);
      });
    }

    function schedulePointerDragMove(
      effect: Extract<PointerGestureEffect, { type: "moveWindowDrag" }>,
    ) {
      pendingMoveEffect = effect;
      if (moveFrame) return;
      moveFrame = window.requestAnimationFrame(() => {
        moveFrame = 0;
        flushPendingPointerDragMove();
      });
    }

    function flushPendingPointerDragMove() {
      if (moveFrame) {
        window.cancelAnimationFrame(moveFrame);
        moveFrame = 0;
      }
      const effect = pendingMoveEffect;
      pendingMoveEffect = null;
      if (!effect) return;
      moveQueue = moveQueue.then(async () => {
        try {
          await client.moveActorWindowDrag(
            effect.actorId,
            effect.sessionId,
            effect.dx,
            effect.dy,
          );
          dispatchPointerGesture({
            type: "windowDragMoved",
            gestureId: effect.gestureId,
            actorId: effect.actorId,
            sessionId: effect.sessionId,
          });
        } catch (error) {
          console.warn("Failed to move avatar window; continuing drag", error);
          dispatchPointerGesture({
            type: "windowDragMoveFailed",
            gestureId: effect.gestureId,
            actorId: effect.actorId,
            sessionId: effect.sessionId,
            error,
          });
        }
      });
    }

    function dispatchPointerGesture(event: PointerGestureEvent) {
      if (disposed) return;
      const transition = reducePointerGesture(gesture, event);
      gesture = transition.state;
      for (const effect of transition.effects) {
        executePointerGestureEffect(effect);
      }
    }

    function executePointerGestureEffect(effect: PointerGestureEffect) {
      if (effect.type === "capturePointer") {
        try {
          checkedCanvas.setPointerCapture(effect.pointerId);
        } catch (error) {
          console.warn("Failed to capture avatar pointer", error);
        }
        return;
      }
      if (effect.type === "releasePointerCapture") {
        try {
          if (checkedCanvas.hasPointerCapture(effect.pointerId)) {
            checkedCanvas.releasePointerCapture(effect.pointerId);
          }
        } catch (error) {
          console.warn("Failed to release avatar pointer capture", error);
        }
        return;
      }
      if (effect.type === "scheduleHold") {
        const existing = holdTimers.get(effect.gestureId);
        if (existing !== undefined) window.clearTimeout(existing);
        const timer = window.setTimeout(() => {
          holdTimers.delete(effect.gestureId);
          dispatchPointerGesture({
            type: "holdElapsed",
            gestureId: effect.gestureId,
            pointerId: effect.pointerId,
          });
        }, effect.delayMs);
        holdTimers.set(effect.gestureId, timer);
        return;
      }
      if (effect.type === "cancelHold") {
        const timer = holdTimers.get(effect.gestureId);
        if (timer !== undefined) window.clearTimeout(timer);
        holdTimers.delete(effect.gestureId);
        return;
      }
      if (effect.type === "beginWindowDrag") {
        void client.beginActorWindowDrag(effect.actorId).then(
          (started) => {
            dispatchPointerGesture({
              type: "windowDragStarted",
              gestureId: effect.gestureId,
              pointerId: effect.pointerId,
              actorId: effect.actorId,
              sessionId: started.sessionId,
            });
          },
          (error: unknown) => {
            console.warn("Failed to begin avatar window drag", error);
            dispatchPointerGesture({
              type: "windowDragStartFailed",
              gestureId: effect.gestureId,
              pointerId: effect.pointerId,
              actorId: effect.actorId,
              error,
            });
          },
        );
        return;
      }
      if (effect.type === "moveWindowDrag") {
        schedulePointerDragMove(effect);
        return;
      }
      if (effect.type === "finishWindowDrag") {
        flushPendingPointerDragMove();
        void (async () => {
          try {
            await moveQueue;
            const finished = await client.finishActorWindowDrag(
              effect.actorId,
              effect.sessionId,
            );
            dispatchPointerGesture({
              type: "windowDragFinished",
              gestureId: effect.gestureId,
              actorId: effect.actorId,
              sessionId: effect.sessionId,
              movedDistance: finished.movedDistance,
            });
          } catch (error) {
            console.warn("Failed to finish avatar window drag", error);
            dispatchPointerGesture({
              type: "windowDragFinishFailed",
              gestureId: effect.gestureId,
              actorId: effect.actorId,
              sessionId: effect.sessionId,
              error,
            });
          }
        })();
        return;
      }
      if (effect.type === "cancelWindowDrag") {
        flushPendingPointerDragMove();
        void (async () => {
          try {
            await moveQueue;
            await client.cancelActorWindowDrag(
              effect.actorId,
              effect.sessionId,
            );
            dispatchPointerGesture({
              type: "windowDragCancelled",
              gestureId: effect.gestureId,
              actorId: effect.actorId,
              sessionId: effect.sessionId,
            });
          } catch (error) {
            console.warn("Failed to cancel avatar window drag", error);
            dispatchPointerGesture({
              type: "windowDragCancelFailed",
              gestureId: effect.gestureId,
              actorId: effect.actorId,
              sessionId: effect.sessionId,
              error,
            });
          }
        })();
        return;
      }
      if (effect.type === "notifyPoke") {
        const completion = semanticNotificationQueue.enqueue(() =>
          onAvatarGesturePoke(effect.poke),
        );
        void completion.then(
          () =>
            dispatchPointerGesture({
              type: "avatarPokeNotified",
              gestureId: effect.gestureId,
            }),
          (error: unknown) => {
            console.warn("Failed to notify avatar poke", error);
            dispatchPointerGesture({
              type: "avatarPokeNotifyFailed",
              gestureId: effect.gestureId,
              error,
            });
          },
        );
        return;
      }
      if (effect.type === "notifyGrab") {
        const completion = semanticNotificationQueue.enqueue(async () => {
          await client.notifyAvatarGestureGrab(effect.actorId);
        });
        void completion.then(
          () =>
            dispatchPointerGesture({
              type: "avatarGrabNotified",
              gestureId: effect.gestureId,
            }),
          (error: unknown) => {
            console.warn("Failed to notify avatar grab", error);
            dispatchPointerGesture({
              type: "avatarGrabNotifyFailed",
              gestureId: effect.gestureId,
              error,
            });
          },
        );
        return;
      }
      if (effect.type === "notifyDrop") {
        const completion = semanticNotificationQueue.enqueue(async () => {
          await client.notifyAvatarGestureDrop(
            effect.actorId,
            effect.movedDistance,
          );
        });
        void completion.then(
          () =>
            dispatchPointerGesture({
              type: "avatarDropNotified",
              gestureId: effect.gestureId,
            }),
          (error: unknown) => {
            console.warn("Failed to notify avatar drop", error);
            dispatchPointerGesture({
              type: "avatarDropNotifyFailed",
              gestureId: effect.gestureId,
              error,
            });
          },
        );
        return;
      }
      const unhandledEffect: never = effect;
      void unhandledEffect;
    }

    function handlePointerMove(event: PointerEvent) {
      dispatchPointerGesture({
        type: "pointerMoved",
        pointerId: event.pointerId,
        client: { x: event.clientX, y: event.clientY },
        screen: { x: event.screenX, y: event.screenY },
      });
    }

    function dispatchPointerEnd(
      event: PointerEvent,
      type: "pointerReleased" | "pointerCancelled",
    ) {
      dispatchPointerGesture({
        type: "pointerMoved",
        pointerId: event.pointerId,
        client: { x: event.clientX, y: event.clientY },
        screen: { x: event.screenX, y: event.screenY },
      });
      dispatchPointerGesture({ type, pointerId: event.pointerId });
    }

    const handlePointerUp = (event: PointerEvent) =>
      dispatchPointerEnd(event, "pointerReleased");
    const handlePointerCancel = (event: PointerEvent) =>
      dispatchPointerEnd(event, "pointerCancelled");

    window.addEventListener("resize", resize);
    canvas.addEventListener("pointerdown", handlePointerDown);
    canvas.addEventListener("pointermove", handlePointerMove);
    canvas.addEventListener("pointerup", handlePointerUp);
    canvas.addEventListener("pointercancel", handlePointerCancel);
    canvas.addEventListener("contextmenu", handleContextMenu);
    resize();
    void loadActors().catch((error) => {
      console.error("Failed to load VRM actors", error);
    });
    animationFrame = window.requestAnimationFrame(animate);
    hitTestTimer = window.setInterval(() => {
      void updateClickThrough().catch(() => undefined);
    }, 70);

    return () => {
      disposed = true;
      window.removeEventListener("resize", resize);
      canvas.removeEventListener("pointerdown", handlePointerDown);
      canvas.removeEventListener("pointermove", handlePointerMove);
      canvas.removeEventListener("pointerup", handlePointerUp);
      canvas.removeEventListener("pointercancel", handlePointerCancel);
      canvas.removeEventListener("contextmenu", handleContextMenu);
      for (const timer of holdTimers.values()) window.clearTimeout(timer);
      holdTimers.clear();
      if (moveFrame) window.cancelAnimationFrame(moveFrame);
      window.cancelAnimationFrame(animationFrame);
      window.clearInterval(hitTestTimer);
      rendererRef.current = null;
      for (const loaded of loadedActors.values()) {
        loaded.mixer.stopAllAction();
        VRMUtils.deepDispose(loaded.vrm.scene);
      }
      renderer.dispose();
    };
  }, [
    assets,
    client,
    onAvatarGesturePoke,
    onHitTestChange,
    onConversationOpen,
    onStageAnchorReport,
  ]);

  return (
    <div className="actor-stage" ref={containerRef}>
      <svg
        aria-hidden="true"
        height="0"
        style={{ pointerEvents: "none", position: "absolute" }}
        width="0"
      >
        <filter
          colorInterpolationFilters="sRGB"
          height="110%"
          id="actor-silhouette-outline"
          width="110%"
          x="-5%"
          y="-5%"
        >
          <feMorphology
            in="SourceAlpha"
            operator="dilate"
            radius="2"
            result="outline-shape"
          />
          <feFlood floodColor="#ffffff" result="outline-color" />
          <feComposite
            in="outline-color"
            in2="outline-shape"
            operator="in"
            result="outline"
          />
          <feMerge>
            <feMergeNode in="outline" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </svg>
      <canvas className="actor-canvas" ref={canvasRef} />
    </div>
  );
}

export async function openConversationFromContextMenu(
  event: Pick<MouseEvent, "preventDefault">,
  actorId: string,
  open: (actorId: string) => Promise<void>,
): Promise<void> {
  event.preventDefault();
  await open(actorId);
}

export function shouldBeginActorPointerGesture(
  event: Pick<PointerEvent, "button" | "ctrlKey">,
): boolean {
  return event.button === 0 && !event.ctrlKey;
}

async function loadMotionActions(
  renderer: ActorSurfaceRendererAsset,
  loaded: LoadedActor,
) {
  const motionLoader = new GLTFLoader();
  motionLoader.register((parser) => new VRMAnimationLoaderPlugin(parser));
  const loadedByUrl = new Map<string, THREE.AnimationAction>();

  for (const [motionId, url] of Object.entries(renderer.motions)) {
    try {
      let action = loadedByUrl.get(url);
      if (!action) {
        const gltf = await motionLoader.loadAsync(url);
        const animation = (
          gltf.userData.vrmAnimations as VRMAnimation[] | undefined
        )?.[0];
        if (!animation) continue;
        const clip = createVRMAnimationClip(animation, loaded.vrm);
        action = loaded.mixer.clipAction(clip);
        action.loop = THREE.LoopRepeat;
        action.clampWhenFinished = false;
        loadedByUrl.set(url, action);
      }
      const normalizedMotionId = normalizeMotionId(motionId);
      if (normalizedMotionId) {
        loaded.actions.set(normalizedMotionId, action);
      }
      loaded.actions.set(motionId, action);
    } catch (error) {
      console.warn(`Failed to load motion ${motionId}`, error);
    }
  }
}

function applyExpression(vrm: VRM, expression: string | undefined) {
  const manager = vrm.expressionManager;
  if (!manager) return;
  manager.resetValues();
  const preset = expressionPresetFor(expression);
  if (preset) {
    manager.setValue(preset, 1);
  }
}

function applyMotion(loaded: LoadedActor, motion: string | undefined) {
  const motionId = normalizeMotionId(motion);
  if (motionId === loaded.currentMotionId) return;

  const previous = loaded.currentMotionId
    ? loaded.actions.get(loaded.currentMotionId)
    : undefined;
  previous?.fadeOut(0.18);

  const next = motionId
    ? (loaded.actions.get(motionId) ?? loaded.actions.get(motion ?? ""))
    : undefined;
  if (next) {
    next.reset().fadeIn(0.18).play();
    loaded.currentMotionId = motionId;
  } else {
    loaded.currentMotionId = null;
  }
}

export function headingRotationY(
  baseRotationY: number,
  heading: string | undefined,
): number {
  if (heading === "right") return baseRotationY + Math.PI / 2;
  if (heading === "left") return baseRotationY - Math.PI / 2;
  return baseRotationY;
}

function frameCamera(root: THREE.Object3D, camera: THREE.PerspectiveCamera) {
  const box = new THREE.Box3().setFromObject(root);
  if (box.isEmpty()) {
    camera.position.set(0, 1.25, 4);
    camera.lookAt(0, 1.15, 0);
    camera.updateProjectionMatrix();
    return;
  }
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  const maxSize = Math.max(size.x, size.y, size.z, 1);
  const fov = THREE.MathUtils.degToRad(camera.fov);
  const distance = (maxSize / (2 * Math.tan(fov / 2))) * 1.28;
  camera.position.set(center.x, center.y + size.y * 0.08, center.z + distance);
  camera.near = Math.max(distance / 100, 0.01);
  camera.far = Math.max(distance * 100, 100);
  camera.lookAt(center.x, center.y + size.y * 0.08, center.z);
  camera.updateProjectionMatrix();
}

async function pointerHitsVisibleSurface(
  renderer: THREE.WebGLRenderer,
): Promise<boolean> {
  const windowHandle = getCurrentWindow();
  const [cursor, outerPosition, innerSize] = await Promise.all([
    cursorPosition(),
    windowHandle.outerPosition(),
    windowHandle.innerSize(),
  ]);
  const scaleX = innerSize.width / Math.max(window.innerWidth, 1);
  const scaleY = innerSize.height / Math.max(window.innerHeight, 1);
  const clientX = (cursor.x - outerPosition.x) / scaleX;
  const clientY = (cursor.y - outerPosition.y) / scaleY;
  if (
    clientX < 0 ||
    clientY < 0 ||
    clientX > window.innerWidth ||
    clientY > window.innerHeight
  ) {
    return false;
  }

  const solidElement = document
    .elementFromPoint(clientX, clientY)
    ?.closest("[data-actor-solid='true']");
  if (solidElement) return true;

  const canvas = renderer.domElement;
  const rect = canvas.getBoundingClientRect();
  if (
    clientX < rect.left ||
    clientX > rect.right ||
    clientY < rect.top ||
    clientY > rect.bottom
  ) {
    return false;
  }

  const canvasX = ((clientX - rect.left) / rect.width) * canvas.width;
  const canvasY = (1 - (clientY - rect.top) / rect.height) * canvas.height;
  const pixel = new Uint8Array(4);
  const gl = renderer.getContext();
  try {
    gl.readPixels(
      Math.floor(canvasX),
      Math.floor(canvasY),
      1,
      1,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      pixel,
    );
    return pixel[3] > 18;
  } catch {
    return true;
  }
}

type SemanticHitZoneResult = {
  actorId: string;
  zone: ResolvedActorHitZone;
  hitBone?: string;
  hitSurface: HitSurface;
};

const HUMANOID_BONE_NAMES = Object.values(VRMHumanBoneName);

function humanoidBoneNodes(vrm: VRM): Map<string, THREE.Object3D> {
  const nodes = new Map<string, THREE.Object3D>();
  for (const boneName of HUMANOID_BONE_NAMES) {
    const node = vrm.humanoid.getRawBoneNode(boneName);
    if (node) {
      nodes.set(boneName, node);
    }
  }
  return nodes;
}

function reverseHumanoidBoneNodes(
  boneNodes: ReadonlyMap<string, THREE.Object3D>,
): Map<THREE.Object3D, string> {
  const byObject = new Map<THREE.Object3D, string>();
  for (const [boneName, node] of boneNodes) {
    byObject.set(node, boneName);
  }
  return byObject;
}

function intersectionsAtPointer(
  event: PointerEvent,
  canvas: HTMLCanvasElement,
  camera: THREE.PerspectiveCamera,
  loadedActors: Map<string, LoadedActor>,
  raycaster: THREE.Raycaster,
): THREE.Intersection[] {
  const rect = canvas.getBoundingClientRect();
  if (
    event.clientX < rect.left ||
    event.clientX > rect.right ||
    event.clientY < rect.top ||
    event.clientY > rect.bottom
  ) {
    return [];
  }

  const pointer = new THREE.Vector2(
    ((event.clientX - rect.left) / Math.max(rect.width, 1)) * 2 - 1,
    -(((event.clientY - rect.top) / Math.max(rect.height, 1)) * 2 - 1),
  );
  raycaster.setFromCamera(pointer, camera);

  const actorScenes = [...loadedActors.values()].map(
    (loaded) => loaded.vrm.scene,
  );
  return raycaster.intersectObjects(actorScenes, true);
}

function actorAtPointer(
  event: PointerEvent,
  canvas: HTMLCanvasElement,
  camera: THREE.PerspectiveCamera,
  loadedActors: Map<string, LoadedActor>,
  raycaster: THREE.Raycaster,
): ActorHit | null {
  for (const intersection of intersectionsAtPointer(
    event,
    canvas,
    camera,
    loadedActors,
    raycaster,
  )) {
    const loaded = loadedActorForObject(intersection.object, loadedActors);
    if (loaded) return { actorId: loaded.actorId };
  }
  return null;
}

function semanticHitAtPointer(
  event: PointerEvent,
  canvas: HTMLCanvasElement,
  camera: THREE.PerspectiveCamera,
  loadedActors: Map<string, LoadedActor>,
  raycaster: THREE.Raycaster,
): SemanticHitZoneResult | null {
  for (const intersection of intersectionsAtPointer(
    event,
    canvas,
    camera,
    loadedActors,
    raycaster,
  )) {
    const loaded = loadedActorForObject(intersection.object, loadedActors);
    if (!loaded) continue;
    const hit = semanticHitZoneForIntersection(loaded, intersection);
    if (hit) return { ...hit, actorId: loaded.actorId };
  }
  return null;
}

function loadedActorForObject(
  object: THREE.Object3D,
  loadedActors: Map<string, LoadedActor>,
): LoadedActor | null {
  for (const loaded of loadedActors.values()) {
    if (isDescendantOf(object, loaded.vrm.scene)) {
      return loaded;
    }
  }
  return null;
}

function semanticHitZoneForIntersection(
  loaded: LoadedActor,
  intersection: THREE.Intersection,
): Omit<SemanticHitZoneResult, "actorId"> | null {
  const names = objectLineageNames(intersection.object, loaded.vrm.scene);
  const hitSurface = hitSurfaceForIntersection(intersection);
  const hitBone = humanoidBoneNameForIntersection(intersection, loaded);
  const zone = hitZoneForLineageOrHumanoidBone(loaded.hitZones, names, hitBone);
  return zone ? { zone, hitBone: hitBone ?? undefined, hitSurface } : null;
}

function objectLineageNames(
  object: THREE.Object3D,
  root: THREE.Object3D,
): string[] {
  const names: string[] = [];
  let current: THREE.Object3D | null = object;
  while (current) {
    if (current.name) {
      names.push(current.name);
    }
    if (current === root) break;
    current = current.parent;
  }
  return names;
}

function humanoidBoneNameForIntersection(
  intersection: THREE.Intersection,
  loaded: LoadedActor,
): string | null {
  const dominantBone = dominantSkinBoneForIntersection(intersection);
  if (dominantBone) {
    const humanoidName = humanoidBoneNameForObject(
      dominantBone,
      loaded.humanoidBoneByObject,
    );
    if (humanoidName) return humanoidName;
  }

  return humanoidBoneNameForObject(
    intersection.object,
    loaded.humanoidBoneByObject,
  );
}

function estimateMouthOffsetY(root: THREE.Object3D): number {
  const box = new THREE.Box3().setFromObject(root);
  if (box.isEmpty()) return 0.1;
  return Math.max(box.getSize(new THREE.Vector3()).y * 0.075, 0.08);
}

function projectActorMouthAnchors(
  canvas: HTMLCanvasElement,
  camera: THREE.PerspectiveCamera,
  loadedActors: Map<string, LoadedActor>,
): Record<string, StageAnchor> {
  const rect = canvas.getBoundingClientRect();
  const anchors: Record<string, StageAnchor> = {};
  camera.updateMatrixWorld();

  for (const loaded of loadedActors.values()) {
    loaded.vrm.scene.updateWorldMatrix(true, true);
    const worldPosition = mouthWorldPosition(loaded);
    anchors[loaded.actorId] = worldPosition
      ? projectWorldPosition(worldPosition, camera, rect)
      : { x: 0, y: 0, visible: false };
  }

  return anchors;
}

function mouthWorldPosition(loaded: LoadedActor): THREE.Vector3 | null {
  const node =
    loaded.boneNodes.get(VRMHumanBoneName.Head) ??
    loaded.boneNodes.get(VRMHumanBoneName.Neck) ??
    loaded.boneNodes.get(VRMHumanBoneName.UpperChest) ??
    loaded.boneNodes.get(VRMHumanBoneName.Chest);
  if (!node) return null;

  const position = new THREE.Vector3();
  node.getWorldPosition(position);
  position.y -= loaded.mouthOffsetY;
  return position;
}

function projectWorldPosition(
  worldPosition: THREE.Vector3,
  camera: THREE.PerspectiveCamera,
  rect: DOMRect,
): StageAnchor {
  if (rect.width <= 0 || rect.height <= 0) {
    return { x: 0, y: 0, visible: false };
  }

  const projected = worldPosition.clone().project(camera);
  if (
    !Number.isFinite(projected.x) ||
    !Number.isFinite(projected.y) ||
    !Number.isFinite(projected.z)
  ) {
    return { x: 0, y: 0, visible: false };
  }

  const x = rect.left + (projected.x * 0.5 + 0.5) * rect.width;
  const y = rect.top + (-projected.y * 0.5 + 0.5) * rect.height;
  const visible =
    projected.z >= -1 &&
    projected.z <= 1 &&
    x >= rect.left &&
    x <= rect.right &&
    y >= rect.top &&
    y <= rect.bottom;

  return { x, y, visible };
}

function anchorSignature(anchors: Record<string, StageAnchor>): string {
  return Object.entries(anchors)
    .map(([actorId, anchor]) => {
      return `${actorId}:${Math.round(anchor.x)}:${Math.round(anchor.y)}:${
        anchor.visible ? 1 : 0
      }`;
    })
    .join("|");
}

function isDescendantOf(object: THREE.Object3D, root: THREE.Object3D): boolean {
  let current: THREE.Object3D | null = object;
  while (current) {
    if (current === root) return true;
    current = current.parent;
  }
  return false;
}

function hasVrmRenderer(
  asset: ActorSurfaceAsset,
): asset is ActorSurfaceAsset & { renderer: ActorSurfaceRendererAsset } {
  return asset.renderer?.kind === "vrm";
}

export function actorIdFromLocation(
  search = window.location.search,
): string | null {
  const actorId = new URLSearchParams(search).get("actorId");
  return actorId && actorId.length > 0 ? actorId : null;
}

export function actorSurfaceAssetsForActor(
  assets: ActorSurfaceAsset[],
  actorId: string | null,
): ActorSurfaceAsset[] {
  if (!actorId) return [];
  return assets.filter(
    (asset) => asset.actorId === actorId && hasVrmRenderer(asset),
  );
}

export function normalizeMotionId(motion: string | undefined): string | null {
  if (!motion) return null;
  const trimmed = motion.trim();
  if (!trimmed || trimmed === "idle" || trimmed === "待機") return null;
  if (trimmed === "walk" || trimmed === "歩く") return "walk";
  return trimmed;
}

export function expressionPresetFor(
  expression: string | undefined,
): string | null {
  if (!expression) return null;
  switch (expression.trim()) {
    case "happy":
    case "smile":
    case "笑顔":
      return "happy";
    case "angry":
    case "怒り":
      return "angry";
    case "sad":
    case "悲しい":
      return "sad";
    case "relaxed":
    case "照れ":
      return "relaxed";
    case "surprised":
    case "驚き":
      return "surprised";
    default:
      return null;
  }
}

export function applyCommandHint(
  snapshot: ResidentSnapshot | null,
  command: RuntimeCommand,
): ResidentSnapshot | null {
  if (!snapshot) return snapshot;
  const actorId =
    command.target?.actorId ??
    (typeof command.payload.speakerId === "string"
      ? command.payload.speakerId
      : undefined);
  if (!actorId || !snapshot.actors[actorId]) return snapshot;

  const actor = snapshot.actors[actorId];
  if (!actor) return snapshot;

  if (
    command.type === "avatar.motion" &&
    typeof command.payload.motion === "string"
  ) {
    return {
      ...snapshot,
      actors: {
        ...snapshot.actors,
        [actorId]: {
          ...actor,
          motion: command.payload.motion,
        },
      },
    };
  }
  if (
    command.type === "stage.walk" &&
    typeof command.payload.motion === "string" &&
    typeof command.payload.destination === "string"
  ) {
    const heading =
      command.payload.destination === "right-edge"
        ? "right"
        : command.payload.destination === "left-edge"
          ? "left"
          : "";
    return {
      ...snapshot,
      actors: {
        ...snapshot.actors,
        [actorId]: {
          ...actor,
          motion: command.payload.motion,
          heading,
        },
      },
    };
  }
  if (
    command.type === "dialogue.say" &&
    typeof command.payload.text === "string"
  ) {
    return {
      ...snapshot,
      actors: {
        ...snapshot.actors,
        [actorId]: {
          ...actor,
          bubble: command.payload.text,
          speaking: true,
        },
      },
    };
  }
  if (
    command.type === "avatar.expression" &&
    typeof command.payload.expression === "string"
  ) {
    return {
      ...snapshot,
      actors: {
        ...snapshot.actors,
        [actorId]: {
          ...actor,
          expression: command.payload.expression,
        },
      },
    };
  }
  return snapshot;
}

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}
