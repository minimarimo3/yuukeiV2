import * as THREE from "three";
import type {
  ActorHitZoneDefinition,
  AvatarGesturePokeInput,
} from "./yuukeiClient";

export const AVATAR_GESTURE_POKE = "avatar.gesture.poke";
export const AVATAR_GESTURE_PAT = "avatar.gesture.pat";

export type HitSurface = "skin" | "cloth" | "hair" | "face" | "unknown";

export type ResolvedActorHitZone = {
  id: string;
  label?: string;
  source: "humanoidBone" | "nodeName";
  bones: string[];
  nodes: string[];
  shape: "auto" | "mesh";
  events: string[];
};

type HitZoneOrigin = "auto" | "pack";

export const HUMANOID_HIT_ZONE_BONES = [
  {
    id: "head",
    label: "頭",
    bones: ["head", "neck", "leftEye", "rightEye", "jaw"],
    events: [AVATAR_GESTURE_POKE, AVATAR_GESTURE_PAT],
  },
  {
    id: "chest",
    label: "胸",
    bones: ["chest", "upperChest"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "belly",
    label: "おなか",
    bones: ["spine"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "hips",
    label: "腰",
    bones: ["hips"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "leftArm",
    label: "左腕",
    bones: ["leftShoulder", "leftUpperArm", "leftLowerArm"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "rightArm",
    label: "右腕",
    bones: ["rightShoulder", "rightUpperArm", "rightLowerArm"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "leftHand",
    label: "左手",
    bones: ["leftHand", ...fingerBones("left")],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "rightHand",
    label: "右手",
    bones: ["rightHand", ...fingerBones("right")],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "leftThigh",
    label: "左もも",
    bones: ["leftUpperLeg"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "rightThigh",
    label: "右もも",
    bones: ["rightUpperLeg"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "leftLeg",
    label: "左すね",
    bones: ["leftLowerLeg"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "rightLeg",
    label: "右すね",
    bones: ["rightLowerLeg"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "leftFoot",
    label: "左足",
    bones: ["leftFoot", "leftToes"],
    events: [AVATAR_GESTURE_POKE],
  },
  {
    id: "rightFoot",
    label: "右足",
    bones: ["rightFoot", "rightToes"],
    events: [AVATAR_GESTURE_POKE],
  },
] satisfies Array<{
  id: string;
  label: string;
  bones: string[];
  events: string[];
}>;

export function autoHitZoneDefinitions(
  availableBones: ReadonlySet<string>,
): ActorHitZoneDefinition[] {
  return HUMANOID_HIT_ZONE_BONES.flatMap((definition) => {
    const bones = definition.bones.filter((bone) => availableBones.has(bone));
    if (bones.length === 0) return [];
    return [
      {
        id: definition.id,
        label: definition.label,
        source: "humanoidBone",
        bones,
        shape: "auto",
        events: definition.events,
      },
    ];
  });
}

export function mergeHitZoneDefinitions(
  autoDefinitions: ActorHitZoneDefinition[],
  packDefinitions: ActorHitZoneDefinition[],
): ResolvedActorHitZone[] {
  const merged = new Map<string, ResolvedActorHitZone>();

  for (const definition of autoDefinitions) {
    const normalized = normalizeHitZoneDefinition(
      definition,
      undefined,
      "auto",
    );
    if (normalized) {
      merged.set(normalized.id, normalized);
    }
  }

  for (const definition of packDefinitions) {
    const fallback = merged.get(definition.id.trim());
    const normalized = normalizeHitZoneDefinition(definition, fallback, "pack");
    if (normalized) {
      merged.set(normalized.id, normalized);
    }
  }

  return [...merged.values()];
}

export function nodeNameHitZoneForLineage(
  zones: ResolvedActorHitZone[],
  lineageNames: string[],
): ResolvedActorHitZone | null {
  return (
    zones.find(
      (zone) =>
        zone.source === "nodeName" &&
        zone.events.includes(AVATAR_GESTURE_POKE) &&
        zone.nodes.some((nodeName) => lineageNames.includes(nodeName)),
    ) ?? null
  );
}

export function hitZoneForHumanoidBone(
  zones: ResolvedActorHitZone[],
  boneName: string,
): ResolvedActorHitZone | null {
  return (
    zones.find(
      (zone) =>
        zone.source === "humanoidBone" &&
        zone.events.includes(AVATAR_GESTURE_POKE) &&
        zone.bones.includes(boneName),
    ) ?? null
  );
}

export function hitZoneForLineageOrHumanoidBone(
  zones: ResolvedActorHitZone[],
  lineageNames: string[],
  boneName: string | null,
): ResolvedActorHitZone | null {
  return (
    nodeNameHitZoneForLineage(zones, lineageNames) ??
    (boneName ? hitZoneForHumanoidBone(zones, boneName) : null)
  );
}

export function dominantSkinBoneForIntersection(
  intersection: THREE.Intersection,
): THREE.Bone | null {
  const object = intersection.object;
  if (!isSkinnedMesh(object) || intersection.faceIndex == null) {
    return null;
  }
  const boneIndex = dominantSkinBoneIndexForFace(
    object.geometry,
    intersection.faceIndex,
  );
  if (boneIndex === null) return null;
  return object.skeleton.bones[boneIndex] ?? null;
}

export function dominantSkinBoneIndexForFace(
  geometry: THREE.BufferGeometry,
  faceIndex: number,
): number | null {
  const skinIndex = geometry.getAttribute("skinIndex");
  const skinWeight = geometry.getAttribute("skinWeight");
  if (!skinIndex || !skinWeight) return null;

  const vertexIndices = faceVertexIndices(geometry, faceIndex);
  if (!vertexIndices) return null;

  const totals = new Map<number, number>();
  for (const vertexIndex of vertexIndices) {
    for (let component = 0; component < 4; component += 1) {
      const boneIndex = Math.trunc(
        attributeComponent(skinIndex, vertexIndex, component),
      );
      const weight = attributeComponent(skinWeight, vertexIndex, component);
      if (
        !Number.isFinite(boneIndex) ||
        !Number.isFinite(weight) ||
        weight <= 0
      ) {
        continue;
      }
      totals.set(boneIndex, (totals.get(boneIndex) ?? 0) + weight);
    }
  }

  let bestBone: number | null = null;
  let bestWeight = 0;
  for (const [boneIndex, weight] of totals) {
    if (weight > bestWeight) {
      bestBone = boneIndex;
      bestWeight = weight;
    }
  }
  return bestBone;
}

export function humanoidBoneNameForObject(
  object: THREE.Object3D,
  humanoidBoneByObject: ReadonlyMap<THREE.Object3D, string>,
): string | null {
  let current: THREE.Object3D | null = object;
  while (current) {
    const boneName = humanoidBoneByObject.get(current);
    if (boneName) return boneName;
    current = current.parent;
  }
  return null;
}

export function hitSurfaceForIntersection(
  intersection: THREE.Intersection,
): HitSurface {
  const materialNames = materialNamesForIntersection(intersection);
  const materialSurface = hitSurfaceFromNames(materialNames);
  if (materialSurface !== "unknown") return materialSurface;
  return hitSurfaceFromNames(objectLineageNames(intersection.object));
}

export function hitSurfaceFromNames(names: string[]): HitSurface {
  for (const name of names) {
    const normalized = name.toLowerCase();
    if (
      normalized.includes("_face") ||
      normalized.includes("_eyeextra") ||
      normalized.includes("_eye")
    ) {
      return "face";
    }
    if (normalized.includes("_skin")) return "skin";
    if (normalized.includes("_cloth")) return "cloth";
    if (normalized.includes("_hair")) return "hair";
  }

  for (const name of names) {
    const normalized = name.toLowerCase();
    if (normalized.includes("face") || normalized.includes("eye"))
      return "face";
    if (normalized.includes("hair")) return "hair";
    if (
      normalized.includes("cloth") ||
      normalized.includes("skirt") ||
      normalized.includes("dress")
    ) {
      return "cloth";
    }
    if (normalized.includes("body") || normalized.includes("skin"))
      return "skin";
  }
  return "unknown";
}

export type PointerLike = {
  button: number;
  screenX: number;
  screenY: number;
};

export function buildAvatarGesturePokePayload(
  actorId: string,
  zone: ResolvedActorHitZone,
  pointer: PointerLike,
  options: {
    hitSurface?: HitSurface;
    hitBone?: string;
  } = {},
): AvatarGesturePokeInput {
  return {
    actorId,
    hitZoneId: zone.id,
    hitZoneLabel: zone.label,
    hitSurface: options.hitSurface,
    hitBone: options.hitBone,
    input: {
      kind: "pointer",
      button: pointerButtonName(pointer.button),
    },
    screen: {
      x: pointer.screenX,
      y: pointer.screenY,
    },
  };
}

export function pointerButtonName(button: number): string {
  switch (button) {
    case 0:
      return "primary";
    case 1:
      return "auxiliary";
    case 2:
      return "secondary";
    default:
      return `button-${button}`;
  }
}

function normalizeHitZoneDefinition(
  definition: ActorHitZoneDefinition,
  fallback: ResolvedActorHitZone | undefined,
  origin: HitZoneOrigin,
): ResolvedActorHitZone | null {
  const id = definition.id.trim();
  if (!id) return null;

  const source = definition.source;
  const bones = nonEmptyList(definition.bones);
  const nodes = nonEmptyList(definition.nodes);
  const fallbackCompatible =
    fallback && fallback.source === source ? fallback : undefined;
  const nextBones =
    bones.length > 0 ? bones : (fallbackCompatible?.bones.slice() ?? []);
  const nextNodes =
    nodes.length > 0 ? nodes : (fallbackCompatible?.nodes.slice() ?? []);

  if (source === "humanoidBone" && nextBones.length === 0) return null;
  if (source === "nodeName" && nextNodes.length === 0) return null;

  const events = nonEmptyList(definition.events);
  const label = optionalTrim(definition.label) ?? fallback?.label;
  const shape =
    definition.shape ??
    fallbackCompatible?.shape ??
    (source === "nodeName" ? "mesh" : "auto");

  return {
    id,
    label,
    source,
    bones: nextBones,
    nodes: nextNodes,
    shape,
    events:
      events.length > 0
        ? events
        : (fallback?.events.slice() ?? [AVATAR_GESTURE_POKE]),
  };
}

function fingerBones(side: "left" | "right"): string[] {
  const prefix = side === "left" ? "left" : "right";
  return [
    `${prefix}ThumbMetacarpal`,
    `${prefix}ThumbProximal`,
    `${prefix}ThumbDistal`,
    `${prefix}IndexProximal`,
    `${prefix}IndexIntermediate`,
    `${prefix}IndexDistal`,
    `${prefix}MiddleProximal`,
    `${prefix}MiddleIntermediate`,
    `${prefix}MiddleDistal`,
    `${prefix}RingProximal`,
    `${prefix}RingIntermediate`,
    `${prefix}RingDistal`,
    `${prefix}LittleProximal`,
    `${prefix}LittleIntermediate`,
    `${prefix}LittleDistal`,
  ];
}

function faceVertexIndices(
  geometry: THREE.BufferGeometry,
  faceIndex: number,
): [number, number, number] | null {
  const base = faceIndex * 3;
  const index = geometry.getIndex();
  if (index) {
    if (base + 2 >= index.count) return null;
    return [index.getX(base), index.getX(base + 1), index.getX(base + 2)];
  }
  const position = geometry.getAttribute("position");
  if (!position || base + 2 >= position.count) return null;
  return [base, base + 1, base + 2];
}

function attributeComponent(
  attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute,
  index: number,
  component: number,
): number {
  switch (component) {
    case 0:
      return attribute.getX(index);
    case 1:
      return attribute.getY(index);
    case 2:
      return attribute.getZ(index);
    default:
      return attribute.getW(index);
  }
}

function materialNamesForIntersection(
  intersection: THREE.Intersection,
): string[] {
  const mesh = intersection.object as THREE.Mesh;
  const material = mesh.material;
  if (Array.isArray(material)) {
    const materialIndex = intersection.face?.materialIndex ?? 0;
    return [material[materialIndex]?.name ?? ""].filter(Boolean);
  }
  return material?.name ? [material.name] : [];
}

function objectLineageNames(object: THREE.Object3D): string[] {
  const names: string[] = [];
  let current: THREE.Object3D | null = object;
  while (current) {
    if (current.name) names.push(current.name);
    current = current.parent;
  }
  return names;
}

function isSkinnedMesh(object: THREE.Object3D): object is THREE.SkinnedMesh {
  return (object as THREE.SkinnedMesh).isSkinnedMesh === true;
}

function nonEmptyList(values: string[] | undefined): string[] {
  return values?.map((value) => value.trim()).filter(Boolean) ?? [];
}

function optionalTrim(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}
