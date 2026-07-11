import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import {
  VRMHumanBoneName,
  VRMLoaderPlugin,
  VRMUtils,
  type VRM
} from "@pixiv/three-vrm";
import {
  createVRMAnimationClip,
  VRMAnimationLoaderPlugin,
  type VRMAnimation
} from "@pixiv/three-vrm-animation";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import {
  tauriYuukeiClient,
  type AvatarGesturePokeInput,
  type ActorSurfaceAsset,
  type ActorSurfaceRendererAsset,
  type StageAnchor,
  type YuukeiClient
} from "./yuukeiClient";
import {
  autoHitZoneDefinitions,
  buildAvatarGesturePokePayload,
  dominantSkinBoneForIntersection,
  hitSurfaceForIntersection,
  hitZoneForLineageOrHumanoidBone,
  humanoidBoneNameForObject,
  mergeHitZoneDefinitions,
  type HitSurface,
  type ResolvedActorHitZone
} from "./actorHitZones";
import { beginDragRequested, idlePointerGesture, releasePointerGesture, windowDragBegan, type ActorHit, type PointerGestureState, type SemanticActorHit } from "./pointerGesture";

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
  client: Pick<YuukeiClient, "beginActorWindowDrag" | "moveActorWindowDrag" | "finishActorWindowDrag" | "cancelActorWindowDrag" | "notifyAvatarGestureGrab" | "notifyAvatarGestureDrop">;
};

export const AVATAR_GRAB_HOLD_MS = 500;
export const AVATAR_GRAB_MOVE_THRESHOLD_PX = 6;

export function shouldStartAvatarGrab(elapsedMs: number, maxDistancePx: number): boolean {
  return elapsedMs >= AVATAR_GRAB_HOLD_MS && maxDistancePx <= AVATAR_GRAB_MOVE_THRESHOLD_PX;
}

export function ActorApp({
  actorId,
  client = tauriYuukeiClient
}: ActorAppProps) {
  const activeActorId = useMemo(() => actorId ?? actorIdFromLocation(), [actorId]);
  const [snapshot, setSnapshot] = useState<ResidentSnapshot | null>(null);
  const [assets, setAssets] = useState<ActorSurfaceAsset[]>([]);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      try {
        unlisteners.push(await client.onSnapshot((nextSnapshot) => {
          setSnapshot(nextSnapshot);
        }));
        unlisteners.push(await client.onCommand((command) => {
          setSnapshot((current) => applyCommandHint(current, command));
        }));
        unlisteners.push(await client.onAssetsChanged((catalog) => {
          setAssets(catalog.actors);
        }));
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
    [assets, activeActorId]
  );
  const visibleStatus = status ?? (activeActorId ? null : "actorId is missing");
  const setClickThrough = useCallback(
    (passthrough: boolean) => client.setActorWindowClickThrough(passthrough),
    [client]
  );
  const sendAvatarGesturePoke = useCallback(
    async (gesture: AvatarGesturePokeInput) => {
      await client.sendAvatarGesturePoke(gesture);
    },
    [client]
  );
  const reportStageAnchor = useCallback(
    async (reportedActorId: string, anchor: StageAnchor) => {
      await client.reportActorStageAnchor(reportedActorId, anchor);
    },
    [client]
  );

  return (
    <main className="actor-shell" aria-label="Yuukei actor surface">
      <VrmStage
        assets={actorAssets}
        snapshot={snapshot}
        onStageAnchorReport={reportStageAnchor}
        onHitTestChange={setClickThrough}
        onAvatarGesturePoke={sendAvatarGesturePoke}
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

export async function loadInitialActorSurfaceState(client: YuukeiClient): Promise<{
  snapshot: ResidentSnapshot;
  assets: ActorSurfaceAsset[];
}> {
  const [snapshot, catalog] = await Promise.all([
    client.attachSurface(),
    client.getActorSurfaceAssets()
  ]);
  return {
    snapshot,
    assets: catalog.actors
  };
}

function VrmStage({
  assets,
  snapshot,
  onStageAnchorReport,
  onHitTestChange,
  onAvatarGesturePoke,
  client
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
    let holdTimer = 0;
    let moveFrame = 0;
    let moveQueue = Promise.resolve();
    let latestScreen: { x: number; y: number } | null = null;

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
      preserveDrawingBuffer: true
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
          asset.renderer.hitZones ?? []
        );

        const loaded: LoadedActor = {
          actorId: asset.actorId,
          vrm,
          mixer: new THREE.AnimationMixer(vrm.scene),
          actions: new Map(),
          currentMotionId: null,
          hitZones,
          boneNodes,
          humanoidBoneByObject,
          mouthOffsetY: estimateMouthOffsetY(vrm.scene)
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
        loadedActors
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
      if (event.button !== 0 || gesture.type !== "idle") return;
      const actorHit = actorAtPointer(
        event,
        renderer.domElement,
        camera,
        loadedActors,
        semanticRaycaster
      );
      if (!actorHit) return;
      const hit = semanticHitAtPointer(event, renderer.domElement, camera, loadedActors, semanticRaycaster);
      event.preventDefault();
      const semanticHit: SemanticActorHit | null = hit ? { actorId: hit.actorId, poke: buildAvatarGesturePokePayload(hit.actorId, hit.zone, event, {
          hitBone: hit.hitBone,
          hitSurface: hit.hitSurface
        }) } : null;
      gesture = { type: "pressing", pointerId: event.pointerId, actorHit, semanticHit, startClient: { x: event.clientX, y: event.clientY }, startScreen: { x: event.screenX, y: event.screenY }, maxDistancePx: 0 };
      latestScreen = { x: event.screenX, y: event.screenY };
      checkedCanvas.setPointerCapture(event.pointerId);
      holdTimer = window.setTimeout(() => {
        if (gesture.type !== "pressing" || !shouldStartAvatarGrab(AVATAR_GRAB_HOLD_MS, gesture.maxDistancePx)) return;
        const transition = beginDragRequested(gesture);
        gesture = transition.state;
        const actorId = gesture.type === "startingDrag" ? gesture.actorId : actorHit.actorId;
        void (async () => {
          try {
            const started = await client.beginActorWindowDrag(actorId);
            void client.notifyAvatarGestureGrab(actorId).catch((error) => console.warn("Failed to notify avatar grab", error));
            const began = windowDragBegan(gesture, started.sessionId);
            gesture = began.state;
            await runEffects(began.effects);
          } catch (error) {
            console.warn("Failed to drag avatar window", error);
            gesture = idlePointerGesture();
          }
        })();
      }, AVATAR_GRAB_HOLD_MS);
    }

    function schedulePointerDragMove() {
      if (gesture.type !== "dragging" || moveFrame) return;
      moveFrame = window.requestAnimationFrame(() => {
        moveFrame = 0;
        if (gesture.type === "dragging") enqueuePointerDragMove(gesture);
      });
    }

    function enqueuePointerDragMove(state: Extract<PointerGestureState, { type: "dragging" }>) {
      if (!latestScreen) return;
      const dx = latestScreen.x - state.startScreen.x;
      const dy = latestScreen.y - state.startScreen.y;
      moveQueue = moveQueue.then(async () => {
        await client.moveActorWindowDrag(state.actorId, state.sessionId, dx, dy);
      }).catch((error) => {
        console.warn("Failed to move avatar window", error);
      });
    }

    async function runEffects(effects: ReturnType<typeof releasePointerGesture>["effects"]) {
      for (const effect of effects) {
        if (effect.type === "poke") await onAvatarGesturePoke(effect.poke);
        if (effect.type === "finishWindowDrag") {
          const finished = await moveQueue.then(() => client.finishActorWindowDrag(effect.actorId, effect.sessionId));
          await client.notifyAvatarGestureDrop(finished.actorId, finished.movedDistance);
          gesture = idlePointerGesture();
        }
        if (effect.type === "cancelWindowDrag") {
          await moveQueue.then(() => client.cancelActorWindowDrag(effect.actorId, effect.sessionId));
          gesture = idlePointerGesture();
        }
      }
    }

    function handlePointerMove(event: PointerEvent) {
      if (gesture.type === "idle" || !("pointerId" in gesture) || gesture.pointerId !== event.pointerId) return;
      latestScreen = { x: event.screenX, y: event.screenY };
      if (gesture.type === "dragging") {
        schedulePointerDragMove();
        return;
      }
      if (gesture.type === "pressing") {
        const distance = Math.hypot(event.clientX - gesture.startClient.x, event.clientY - gesture.startClient.y);
        gesture = { ...gesture, maxDistancePx: Math.max(gesture.maxDistancePx, distance) };
        if (gesture.maxDistancePx > AVATAR_GRAB_MOVE_THRESHOLD_PX) window.clearTimeout(holdTimer);
      }
    }

    function endPointer(event: PointerEvent, cancelled: boolean) {
      if (gesture.type === "idle" || !("pointerId" in gesture) || gesture.pointerId !== event.pointerId) return;
      window.clearTimeout(holdTimer);
      latestScreen = { x: event.screenX, y: event.screenY };
      if (gesture.type === "dragging") enqueuePointerDragMove(gesture);
      const transition = releasePointerGesture(gesture, cancelled);
      gesture = transition.state;
      if (!cancelled && checkedCanvas.hasPointerCapture(event.pointerId)) checkedCanvas.releasePointerCapture(event.pointerId);
      void runEffects(transition.effects).catch((error) => console.warn("Failed to end pointer gesture", error));
    }

    const handlePointerUp = (event: PointerEvent) => endPointer(event, false);
    const handlePointerCancel = (event: PointerEvent) => endPointer(event, true);

    window.addEventListener("resize", resize);
    canvas.addEventListener("pointerdown", handlePointerDown);
    canvas.addEventListener("pointermove", handlePointerMove);
    canvas.addEventListener("pointerup", handlePointerUp);
    canvas.addEventListener("pointercancel", handlePointerCancel);
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
      window.clearTimeout(holdTimer);
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
    onStageAnchorReport
  ]);

  return (
    <div className="actor-stage" ref={containerRef}>
      <canvas className="actor-canvas" ref={canvasRef} />
    </div>
  );
}

async function loadMotionActions(
  renderer: ActorSurfaceRendererAsset,
  loaded: LoadedActor
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
    ? loaded.actions.get(motionId) ?? loaded.actions.get(motion ?? "")
    : undefined;
  if (next) {
    next.reset().fadeIn(0.18).play();
    loaded.currentMotionId = motionId;
  } else {
    loaded.currentMotionId = null;
  }
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
  renderer: THREE.WebGLRenderer
): Promise<boolean> {
  const windowHandle = getCurrentWindow();
  const [cursor, outerPosition, innerSize] = await Promise.all([
    cursorPosition(),
    windowHandle.outerPosition(),
    windowHandle.innerSize()
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
      pixel
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
  boneNodes: ReadonlyMap<string, THREE.Object3D>
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
  raycaster: THREE.Raycaster
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
    -(((event.clientY - rect.top) / Math.max(rect.height, 1)) * 2 - 1)
  );
  raycaster.setFromCamera(pointer, camera);

  const actorScenes = [...loadedActors.values()].map((loaded) => loaded.vrm.scene);
  return raycaster.intersectObjects(actorScenes, true);
}

function actorAtPointer(event: PointerEvent, canvas: HTMLCanvasElement, camera: THREE.PerspectiveCamera, loadedActors: Map<string, LoadedActor>, raycaster: THREE.Raycaster): ActorHit | null {
  for (const intersection of intersectionsAtPointer(event, canvas, camera, loadedActors, raycaster)) {
    const loaded = loadedActorForObject(intersection.object, loadedActors);
    if (loaded) return { actorId: loaded.actorId };
  }
  return null;
}

function semanticHitAtPointer(event: PointerEvent, canvas: HTMLCanvasElement, camera: THREE.PerspectiveCamera, loadedActors: Map<string, LoadedActor>, raycaster: THREE.Raycaster): SemanticHitZoneResult | null {
  for (const intersection of intersectionsAtPointer(event, canvas, camera, loadedActors, raycaster)) {
    const loaded = loadedActorForObject(intersection.object, loadedActors);
    if (!loaded) continue;
    const hit = semanticHitZoneForIntersection(loaded, intersection);
    if (hit) return { ...hit, actorId: loaded.actorId };
  }
  return null;
}

function loadedActorForObject(
  object: THREE.Object3D,
  loadedActors: Map<string, LoadedActor>
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
  intersection: THREE.Intersection
): Omit<SemanticHitZoneResult, "actorId"> | null {
  const names = objectLineageNames(intersection.object, loaded.vrm.scene);
  const hitSurface = hitSurfaceForIntersection(intersection);
  const hitBone = humanoidBoneNameForIntersection(intersection, loaded);
  const zone = hitZoneForLineageOrHumanoidBone(loaded.hitZones, names, hitBone);
  return zone ? { zone, hitBone: hitBone ?? undefined, hitSurface } : null;
}

function objectLineageNames(object: THREE.Object3D, root: THREE.Object3D): string[] {
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
  loaded: LoadedActor
): string | null {
  const dominantBone = dominantSkinBoneForIntersection(intersection);
  if (dominantBone) {
    const humanoidName = humanoidBoneNameForObject(
      dominantBone,
      loaded.humanoidBoneByObject
    );
    if (humanoidName) return humanoidName;
  }

  return humanoidBoneNameForObject(
    intersection.object,
    loaded.humanoidBoneByObject
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
  loadedActors: Map<string, LoadedActor>
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
  rect: DOMRect
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
  asset: ActorSurfaceAsset
): asset is ActorSurfaceAsset & { renderer: ActorSurfaceRendererAsset } {
  return asset.renderer?.kind === "vrm";
}

export function actorIdFromLocation(search = window.location.search): string | null {
  const actorId = new URLSearchParams(search).get("actorId");
  return actorId && actorId.length > 0 ? actorId : null;
}

export function actorSurfaceAssetsForActor(
  assets: ActorSurfaceAsset[],
  actorId: string | null
): ActorSurfaceAsset[] {
  if (!actorId) return [];
  return assets.filter((asset) => asset.actorId === actorId && hasVrmRenderer(asset));
}

export function normalizeMotionId(motion: string | undefined): string | null {
  if (!motion) return null;
  const trimmed = motion.trim();
  if (!trimmed || trimmed === "idle" || trimmed === "待機") return null;
  if (trimmed === "walk" || trimmed === "歩く") return "walk";
  return trimmed;
}

export function expressionPresetFor(expression: string | undefined): string | null {
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
  command: RuntimeCommand
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

  if (command.type === "avatar.motion" && typeof command.payload.motion === "string") {
    return {
      ...snapshot,
      actors: {
        ...snapshot.actors,
        [actorId]: {
          ...actor,
          motion: command.payload.motion
        }
      }
    };
  }
  if (command.type === "dialogue.say" && typeof command.payload.text === "string") {
    return {
      ...snapshot,
      actors: {
        ...snapshot.actors,
        [actorId]: {
          ...actor,
          bubble: command.payload.text,
          speaking: true
        }
      }
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
          expression: command.payload.expression
        }
      }
    };
  }
  return snapshot;
}

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}
