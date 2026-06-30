import type {
  ActorHitZoneDefinition,
  AvatarGesturePokeInput
} from "./yuukeiClient";

export const AVATAR_GESTURE_POKE = "avatar.gesture.poke";
export const AVATAR_GESTURE_PAT = "avatar.gesture.pat";

export type ResolvedActorHitZone = {
  id: string;
  label?: string;
  source: "humanoidBone" | "nodeName";
  bones: string[];
  nodes: string[];
  shape: "auto" | "mesh";
  events: string[];
  priority: number;
};

export type HitZoneCandidate = {
  zone: ResolvedActorHitZone;
  distance: number;
};

type HitZoneOrigin = "auto" | "pack";

const AUTO_HIT_ZONE_DEFINITIONS: Array<
  ActorHitZoneDefinition & { requiredBones: string[]; priority: number }
> = [
  {
    id: "head",
    label: "頭",
    source: "humanoidBone",
    bones: ["head"],
    requiredBones: ["head"],
    shape: "auto",
    events: [AVATAR_GESTURE_POKE, AVATAR_GESTURE_PAT],
    priority: 30
  },
  {
    id: "leftHand",
    label: "左手",
    source: "humanoidBone",
    bones: ["leftHand"],
    requiredBones: ["leftHand"],
    shape: "auto",
    events: [AVATAR_GESTURE_POKE],
    priority: 20
  },
  {
    id: "rightHand",
    label: "右手",
    source: "humanoidBone",
    bones: ["rightHand"],
    requiredBones: ["rightHand"],
    shape: "auto",
    events: [AVATAR_GESTURE_POKE],
    priority: 20
  },
  {
    id: "body",
    label: "からだ",
    source: "humanoidBone",
    bones: ["chest", "spine", "hips"],
    requiredBones: ["chest", "spine", "hips"],
    shape: "auto",
    events: [AVATAR_GESTURE_POKE],
    priority: 5
  }
];

export function autoHitZoneDefinitions(
  availableBones: ReadonlySet<string>
): ActorHitZoneDefinition[] {
  return AUTO_HIT_ZONE_DEFINITIONS.flatMap((definition) => {
    const bones = definition.bones?.filter((bone) => availableBones.has(bone)) ?? [];
    const hasRequiredBone = definition.requiredBones.some((bone) =>
      availableBones.has(bone)
    );
    if (!hasRequiredBone || bones.length === 0) return [];
    return [
      {
        id: definition.id,
        label: definition.label,
        source: definition.source,
        bones,
        shape: definition.shape,
        events: definition.events,
        priority: definition.priority
      }
    ];
  });
}

export function mergeHitZoneDefinitions(
  autoDefinitions: ActorHitZoneDefinition[],
  packDefinitions: ActorHitZoneDefinition[]
): ResolvedActorHitZone[] {
  const merged = new Map<string, ResolvedActorHitZone>();

  for (const definition of autoDefinitions) {
    const normalized = normalizeHitZoneDefinition(definition, undefined, "auto");
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

function normalizeHitZoneDefinition(
  definition: ActorHitZoneDefinition,
  fallback: ResolvedActorHitZone | undefined,
  origin: HitZoneOrigin
): ResolvedActorHitZone | null {
  const id = definition.id.trim();
  if (!id) return null;

  const source = definition.source;
  const bones = nonEmptyList(definition.bones);
  const nodes = nonEmptyList(definition.nodes);
  const fallbackCompatible =
    fallback && fallback.source === source ? fallback : undefined;
  const nextBones =
    bones.length > 0 ? bones : fallbackCompatible?.bones.slice() ?? [];
  const nextNodes =
    nodes.length > 0 ? nodes : fallbackCompatible?.nodes.slice() ?? [];

  if (source === "humanoidBone" && nextBones.length === 0) return null;
  if (source === "nodeName" && nextNodes.length === 0) return null;

  const events = nonEmptyList(definition.events);
  const label = optionalTrim(definition.label) ?? fallback?.label;
  const shape =
    definition.shape ?? fallbackCompatible?.shape ?? (source === "nodeName" ? "mesh" : "auto");

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
        : fallback?.events.slice() ?? [AVATAR_GESTURE_POKE],
    priority:
      definition.priority ??
      fallback?.priority ??
      (origin === "pack" ? 100 : 0)
  };
}

export function chooseHitZoneCandidate<T extends HitZoneCandidate>(
  candidates: T[]
): T | null {
  return candidates
    .map((candidate, index) => ({ candidate, index }))
    .filter(({ candidate }) => candidate.zone.events.includes(AVATAR_GESTURE_POKE))
    .sort((left, right) => {
      const priority = right.candidate.zone.priority - left.candidate.zone.priority;
      if (priority !== 0) return priority;
      const distance = left.candidate.distance - right.candidate.distance;
      if (distance !== 0) return distance;
      return left.index - right.index;
    })[0]?.candidate ?? null;
}

export type PointerLike = {
  button: number;
  screenX: number;
  screenY: number;
};

export function buildAvatarGesturePokePayload(
  actorId: string,
  zone: ResolvedActorHitZone,
  pointer: PointerLike
): AvatarGesturePokeInput {
  return {
    actorId,
    hitZoneId: zone.id,
    hitZoneLabel: zone.label,
    input: {
      kind: "pointer",
      button: pointerButtonName(pointer.button)
    },
    screen: {
      x: pointer.screenX,
      y: pointer.screenY
    }
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

function nonEmptyList(values: string[] | undefined): string[] {
  return values?.map((value) => value.trim()).filter(Boolean) ?? [];
}

function optionalTrim(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}
