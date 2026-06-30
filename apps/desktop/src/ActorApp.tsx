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
import type { ActorSnapshot, ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import {
  tauriYuukeiClient,
  type AvatarGesturePokeInput,
  type ActorSurfaceAsset,
  type ActorSurfaceRendererAsset,
  type YuukeiClient
} from "./yuukeiClient";
import {
  autoHitZoneDefinitions,
  buildAvatarGesturePokePayload,
  chooseHitZoneCandidate,
  mergeHitZoneDefinitions,
  type HitZoneCandidate,
  type ResolvedActorHitZone
} from "./actorHitZones";

type ActorAppProps = {
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
  hitTestRadius: number;
};

type VrmStageProps = {
  assets: ActorSurfaceAsset[];
  snapshot: ResidentSnapshot | null;
  onHitTestChange(passthrough: boolean): Promise<void>;
  onAvatarGesturePoke(gesture: AvatarGesturePokeInput): Promise<void>;
};

export function ActorApp({ client = tauriYuukeiClient }: ActorAppProps) {
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
        const [attached, catalog] = await Promise.all([
          client.attachSurface(),
          client.getActorSurfaceAssets()
        ]);
        if (!disposed) {
          setSnapshot(attached);
          setAssets(catalog.actors);
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

  const bubbleActors = useMemo(() => {
    return Object.entries(snapshot?.actors ?? {}).filter((entry): entry is [string, ActorSnapshot] => {
      return Boolean(entry[1]?.bubble);
    });
  }, [snapshot]);
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

  return (
    <main className="actor-shell" aria-label="Yuukei actor surface">
      <VrmStage
        assets={assets}
        snapshot={snapshot}
        onHitTestChange={setClickThrough}
        onAvatarGesturePoke={sendAvatarGesturePoke}
      />
      <div className="actor-bubbles" aria-live="polite">
        {bubbleActors.map(([actorId, actor]) => (
          <p className="actor-bubble" data-actor-solid="true" key={actorId}>
            {actor.bubble}
          </p>
        ))}
      </div>
      {status ? (
        <p className="actor-status" data-actor-solid="true" role="alert">
          {status}
        </p>
      ) : null}
    </main>
  );
}

function VrmStage({
  assets,
  snapshot,
  onHitTestChange,
  onAvatarGesturePoke
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
    const checkedStageElement = stageElement;

    let disposed = false;
    let animationFrame = 0;
    let hitTestTimer = 0;
    let lastPassthrough: boolean | null = null;

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

      for (const [index, asset] of vrmAssets.entries()) {
        if (disposed) return;
        const gltf = await modelLoader.loadAsync(asset.renderer.modelUrl);
        const vrm = gltf.userData.vrm as VRM | undefined;
        if (!vrm) continue;

        VRMUtils.rotateVRM0(vrm);
        vrm.scene.name = `actor-${asset.actorId}`;
        vrm.scene.position.x = (index - (vrmAssets.length - 1) / 2) * 1.05;
        actorRoot.add(vrm.scene);
        const boneNodes = humanoidBoneNodes(vrm);
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
          hitTestRadius: semanticHitTestRadius(vrm.scene)
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
      renderer.render(scene, camera);
      animationFrame = window.requestAnimationFrame(animate);
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
      if (event.button !== 0) return;
      const hit = semanticHitZoneAtPointer(
        event,
        renderer.domElement,
        camera,
        loadedActors,
        semanticRaycaster
      );
      if (!hit) return;
      event.preventDefault();
      void onAvatarGesturePoke(
        buildAvatarGesturePokePayload(hit.actorId, hit.zone, event)
      ).catch((error) => {
        console.warn("Failed to send avatar gesture", error);
      });
    }

    window.addEventListener("resize", resize);
    canvas.addEventListener("pointerdown", handlePointerDown);
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
      window.cancelAnimationFrame(animationFrame);
      window.clearInterval(hitTestTimer);
      rendererRef.current = null;
      for (const loaded of loadedActors.values()) {
        loaded.mixer.stopAllAction();
        VRMUtils.deepDispose(loaded.vrm.scene);
      }
      renderer.dispose();
    };
  }, [assets, onAvatarGesturePoke, onHitTestChange]);

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

function semanticHitZoneAtPointer(
  event: PointerEvent,
  canvas: HTMLCanvasElement,
  camera: THREE.PerspectiveCamera,
  loadedActors: Map<string, LoadedActor>,
  raycaster: THREE.Raycaster
): SemanticHitZoneResult | null {
  const rect = canvas.getBoundingClientRect();
  if (
    event.clientX < rect.left ||
    event.clientX > rect.right ||
    event.clientY < rect.top ||
    event.clientY > rect.bottom
  ) {
    return null;
  }

  const pointer = new THREE.Vector2(
    ((event.clientX - rect.left) / Math.max(rect.width, 1)) * 2 - 1,
    -(((event.clientY - rect.top) / Math.max(rect.height, 1)) * 2 - 1)
  );
  raycaster.setFromCamera(pointer, camera);

  const actorScenes = [...loadedActors.values()].map((loaded) => loaded.vrm.scene);
  const candidates: Array<HitZoneCandidate & { actorId: string }> = [];
  for (const intersection of raycaster.intersectObjects(actorScenes, true)) {
    const loaded = loadedActorForObject(intersection.object, loadedActors);
    if (!loaded) continue;
    for (const candidate of hitZoneCandidatesForIntersection(loaded, intersection)) {
      candidates.push({ ...candidate, actorId: loaded.actorId });
    }
  }

  const selected = chooseHitZoneCandidate(candidates);
  if (!selected) return null;
  return {
    actorId: selected.actorId,
    zone: selected.zone
  };
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

function hitZoneCandidatesForIntersection(
  loaded: LoadedActor,
  intersection: THREE.Intersection
): HitZoneCandidate[] {
  const names = objectLineageNames(intersection.object, loaded.vrm.scene);
  const candidates: HitZoneCandidate[] = [];

  for (const zone of loaded.hitZones) {
    if (
      zone.source === "nodeName" &&
      zone.nodes.some((nodeName) => names.includes(nodeName))
    ) {
      candidates.push({ zone, distance: intersection.distance });
      continue;
    }

    if (zone.source !== "humanoidBone") continue;
    const nearestBone = nearestBoneDistance(zone.bones, loaded.boneNodes, intersection.point);
    if (nearestBone === null) continue;
    const threshold = semanticBoneThreshold(zone, loaded.hitTestRadius);
    if (nearestBone <= threshold) {
      candidates.push({
        zone,
        distance: intersection.distance + nearestBone * 0.05
      });
    }
  }

  return candidates;
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

function nearestBoneDistance(
  bones: string[],
  boneNodes: Map<string, THREE.Object3D>,
  point: THREE.Vector3
): number | null {
  let nearest: number | null = null;
  const bonePosition = new THREE.Vector3();
  for (const bone of bones) {
    const node = boneNodes.get(bone);
    if (!node) continue;
    node.getWorldPosition(bonePosition);
    const distance = bonePosition.distanceTo(point);
    nearest = nearest === null ? distance : Math.min(nearest, distance);
  }
  return nearest;
}

function semanticBoneThreshold(
  zone: ResolvedActorHitZone,
  baseRadius: number
): number {
  if (zone.id === "body") return baseRadius * 1.8;
  if (zone.id === "head") return baseRadius * 1.2;
  if (zone.id === "leftHand" || zone.id === "rightHand") return baseRadius;
  return baseRadius;
}

function semanticHitTestRadius(root: THREE.Object3D): number {
  const box = new THREE.Box3().setFromObject(root);
  if (box.isEmpty()) return 0.18;
  const size = box.getSize(new THREE.Vector3());
  return Math.max(size.y * 0.16, 0.12);
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

function applyCommandHint(
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
