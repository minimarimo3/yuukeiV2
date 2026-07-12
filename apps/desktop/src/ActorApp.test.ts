import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import {
  actorIdFromLocation,
  actorSurfaceAssetsForActor,
  applyCommandHint,
  expressionPresetFor,
  headingRotationY,
  loadInitialActorSurfaceState,
  normalizeMotionId,
  shouldStartAvatarGrab
} from "./ActorApp";
import {
  autoHitZoneDefinitions,
  buildAvatarGesturePokePayload,
  dominantSkinBoneForIntersection,
  dominantSkinBoneIndexForFace,
  hitSurfaceForIntersection,
  hitSurfaceFromNames,
  hitZoneForHumanoidBone,
  hitZoneForLineageOrHumanoidBone,
  humanoidBoneNameForObject,
  mergeHitZoneDefinitions,
  type ResolvedActorHitZone
} from "./actorHitZones";
import {
  chooseBubbleSide,
  computeBubblePlacement
} from "./actorBubbleLayout";
import type { ActorSurfaceAsset, YuukeiClient } from "./yuukeiClient";

describe("ActorApp renderer helpers", () => {
  it("normalizes authored motion aliases to renderer motion ids", () => {
    expect(normalizeMotionId("walk")).toBe("walk");
    expect(normalizeMotionId("歩く")).toBe("walk");
    expect(normalizeMotionId("idle")).toBeNull();
    expect(normalizeMotionId("待機")).toBeNull();
  });

  it("maps authored expressions to VRM preset names", () => {
    expect(expressionPresetFor("笑顔")).toBe("happy");
    expect(expressionPresetFor("angry")).toBe("angry");
    expect(expressionPresetFor("neutral")).toBeNull();
  });

  it("rotates headings relative to the VRM authored baseline", () => {
    expect(headingRotationY(0.25, "right")).toBeCloseTo(0.25 + Math.PI / 2);
    expect(headingRotationY(0.25, "left")).toBeCloseTo(0.25 - Math.PI / 2);
    expect(headingRotationY(0.25, "")).toBeCloseTo(0.25);
  });

  it("reads the actorId query parameter as the actor window contract", () => {
    expect(actorIdFromLocation("?actorId=yuukei")).toBe("yuukei");
    expect(actorIdFromLocation("?actorId=actor%2Fwith%2Fslash")).toBe(
      "actor/with/slash"
    );
    expect(actorIdFromLocation("?view=settings")).toBeNull();
  });

  it("selects only the requested renderable actor asset", () => {
    const selected = actorSurfaceAssetsForActor(
      [
        actorAsset("yuukei", true),
        actorAsset("headless", false),
        actorAsset("another", true)
      ],
      "another"
    );

    expect(selected.map((asset) => asset.actorId)).toEqual(["another"]);
  });

  it("loads the initial actor state by attaching the surface", async () => {
    const initialSnapshot = snapshotFixture();
    const client = {
      attachSurface: vi.fn(async () => initialSnapshot),
      getSnapshot: vi.fn(async () => {
        throw new Error("getSnapshot should not be used for actor bootstrap");
      }),
      getActorSurfaceAssets: vi.fn(async () => ({
        worldPackId: "default-yuukei",
        actors: [actorAsset("yuukei", true), actorAsset("headless", false)]
      }))
    } as unknown as YuukeiClient;

    const initial = await loadInitialActorSurfaceState(client);

    expect(client.attachSurface).toHaveBeenCalledTimes(1);
    expect(client.getSnapshot).not.toHaveBeenCalled();
    expect(initial.snapshot).toBe(initialSnapshot);
    expect(initial.assets.map((asset) => asset.actorId)).toEqual([
      "yuukei",
      "headless"
    ]);
  });

  it("surfaces actor bootstrap attach failures to the caller", async () => {
    const client = {
      attachSurface: vi.fn(async () => {
        throw new Error("attach failed");
      }),
      getSnapshot: vi.fn(),
      getActorSurfaceAssets: vi.fn(async () => ({
        worldPackId: "default-yuukei",
        actors: []
      }))
    } as unknown as YuukeiClient;

    await expect(loadInitialActorSurfaceState(client)).rejects.toThrow(
      "attach failed"
    );
    expect(client.getSnapshot).not.toHaveBeenCalled();
  });

  it("builds baseline humanoid hit zones from available VRM bones", () => {
    const zones = autoHitZoneDefinitions(
      new Set(["head", "rightHand", "hips", "spine", "chest"])
    );

    expect(zones.map((zone) => zone.id)).toEqual([
      "head",
      "chest",
      "belly",
      "hips",
      "rightHand"
    ]);
    expect(zones.find((zone) => zone.id === "chest")?.label).toBe("胸");
    expect(zones.find((zone) => zone.id === "belly")?.bones).toEqual(["spine"]);
  });

  it("merges pack hit zones over auto generated zones", () => {
    const autoZones = autoHitZoneDefinitions(new Set(["head", "hips"]));
    const zones = mergeHitZoneDefinitions(autoZones, [
      {
        id: "head",
        label: "おでこ",
        source: "humanoidBone",
        bones: ["head"],
        events: ["avatar.gesture.poke"]
      },
      {
        id: "tail",
        label: "しっぽ",
        source: "nodeName",
        nodes: ["Tail", "Tail_001"],
        shape: "mesh"
      }
    ]);

    expect(zones.find((zone) => zone.id === "head")).toMatchObject({
      label: "おでこ",
      events: ["avatar.gesture.poke"]
    });
    expect(zones.find((zone) => zone.id === "tail")).toMatchObject({
      label: "しっぽ",
      source: "nodeName",
      nodes: ["Tail", "Tail_001"],
      events: ["avatar.gesture.poke"]
    });
  });

  it("maps humanoid bones to fine grained zones without head priority bleed", () => {
    const zones = mergeHitZoneDefinitions(
      autoHitZoneDefinitions(new Set(["head", "chest", "spine", "leftUpperLeg"])),
      []
    );

    expect(hitZoneForHumanoidBone(zones, "head")?.id).toBe("head");
    expect(hitZoneForHumanoidBone(zones, "chest")?.id).toBe("chest");
    expect(hitZoneForHumanoidBone(zones, "spine")?.id).toBe("belly");
    expect(hitZoneForHumanoidBone(zones, "leftUpperLeg")?.id).toBe("leftThigh");
  });

  it("lets nodeName pack zones override skin weight classification", () => {
    const zones = mergeHitZoneDefinitions(
      autoHitZoneDefinitions(new Set(["head", "hips"])),
      [
        {
          id: "ribbon",
          label: "リボン",
          source: "nodeName",
          nodes: ["Ribbon"]
        }
      ]
    );

    expect(
      hitZoneForLineageOrHumanoidBone(zones, ["Ribbon", "HairMesh"], "head")?.id
    ).toBe("ribbon");
  });

  it("classifies VRoid material and mesh names as hit surfaces", () => {
    expect(hitSurfaceFromNames(["N00_000_00_Body_00_SKIN"])).toBe("skin");
    expect(hitSurfaceFromNames(["N00_000_00_Tops_01_CLOTH"])).toBe("cloth");
    expect(hitSurfaceFromNames(["N00_000_Hair_00_HAIR"])).toBe("hair");
    expect(hitSurfaceFromNames(["Face_EyeExtra"])).toBe("face");
    expect(hitSurfaceFromNames(["Accessory"])).toBe("unknown");
    expect(hitSurfaceFromNames(["HairBack"])).toBe("hair");
  });

  it("reads dominant skin weights from indexed and non-indexed faces", () => {
    expect(dominantSkinBoneIndexForFace(indexedSkinGeometry(), 0)).toBe(1);
    expect(dominantSkinBoneIndexForFace(nonIndexedSkinGeometry(), 0)).toBe(2);
  });

  it("resolves dominant skinned mesh bones to humanoid zones", () => {
    const headBone = new THREE.Bone();
    headBone.name = "Head";
    const chestBone = new THREE.Bone();
    chestBone.name = "Chest";
    const mesh = new THREE.SkinnedMesh(
      indexedSkinGeometry(),
      new THREE.MeshBasicMaterial({ name: "N00_000_00_Body_00_SKIN" })
    );
    mesh.add(headBone);
    mesh.add(chestBone);
    mesh.bind(new THREE.Skeleton([headBone, chestBone]));
    const intersection = {
      object: mesh,
      faceIndex: 0,
      distance: 1,
      point: new THREE.Vector3()
    } as THREE.Intersection;
    const zones = mergeHitZoneDefinitions(
      autoHitZoneDefinitions(new Set(["head", "chest"])),
      []
    );
    const dominantBone = dominantSkinBoneForIntersection(intersection);
    const humanoidBone = dominantBone
      ? humanoidBoneNameForObject(dominantBone, new Map([[chestBone, "chest"]]))
      : null;

    expect(dominantBone).toBe(chestBone);
    expect(humanoidBone).toBe("chest");
    expect(hitZoneForHumanoidBone(zones, humanoidBone ?? "")?.id).toBe("chest");
    expect(hitSurfaceForIntersection(intersection)).toBe("skin");
  });

  it("walks from non-humanoid hair bones to the nearest humanoid parent", () => {
    const head = new THREE.Bone();
    const hair = new THREE.Bone();
    head.add(hair);
    const boneName = humanoidBoneNameForObject(hair, new Map([[head, "head"]]));
    const zones = mergeHitZoneDefinitions(autoHitZoneDefinitions(new Set(["head"])), []);

    expect(boneName).toBe("head");
    expect(hitZoneForHumanoidBone(zones, boneName ?? "")?.id).toBe("head");
  });

  it("builds avatar gesture poke payloads from pointer input", () => {
    const payload = buildAvatarGesturePokePayload(
      "yuukei",
      hitZone("head"),
      {
        button: 0,
        screenX: 123,
        screenY: 456
      },
      {
        hitBone: "head",
        hitSurface: "face"
      }
    );

    expect(payload).toEqual({
      actorId: "yuukei",
      hitZoneId: "head",
      hitZoneLabel: "頭",
      hitBone: "head",
      hitSurface: "face",
      input: {
        kind: "pointer",
        button: "primary"
      },
      screen: {
        x: 123,
        y: 456
      }
    });
  });

  it("keeps short presses as pokes and requires a steady 500ms hold to grab", () => {
    expect(shouldStartAvatarGrab(499, 0)).toBe(false);
    expect(shouldStartAvatarGrab(500, 6)).toBe(true);
    expect(shouldStartAvatarGrab(500, 6.01)).toBe(false);
  });

  it("places actor bubbles on the side with more space", () => {
    expect(chooseBubbleSide(120, 800, 16)).toBe("right");
    expect(chooseBubbleSide(680, 800, 16)).toBe("left");

    expect(
      computeBubblePlacement(
        { x: 120, y: 160, visible: true },
        { width: 800, height: 420 },
        { width: 240, height: 80 }
      ).left
    ).toBeGreaterThan(120);

    expect(
      computeBubblePlacement(
        { x: 680, y: 160, visible: true },
        { width: 800, height: 420 },
        { width: 240, height: 80 }
      ).left
    ).toBeLessThan(680);
  });

  it("clamps actor bubbles inside the viewport near edges", () => {
    const placement = computeBubblePlacement(
      { x: 790, y: 4, visible: true },
      { width: 800, height: 280 },
      { width: 260, height: 96 }
    );

    expect(placement.left).toBeGreaterThanOrEqual(16);
    expect(placement.left + placement.width).toBeLessThanOrEqual(800 - 16);
    expect(placement.top).toBeGreaterThanOrEqual(16);
  });

  it("applies dialogue.say hints to the targeted actor bubble immediately", () => {
    const next = applyCommandHint(
      snapshotFixture(),
      commandFixture("dialogue.say", {
        targetActorId: "yuukei",
        speakerId: "another",
        payload: { text: "今ここで話します" }
      })
    );

    expect(next?.actors.yuukei?.bubble).toBe("今ここで話します");
    expect(next?.actors.yuukei?.speaking).toBe(true);
    expect(next?.actors.another?.bubble).toBeUndefined();
  });

  it("uses payload speakerId for dialogue.say when target actorId is missing", () => {
    const next = applyCommandHint(
      snapshotFixture(),
      commandFixture("dialogue.say", {
        speakerId: "another",
        payload: { text: "こちらからです" }
      })
    );

    expect(next?.actors.another?.bubble).toBe("こちらからです");
    expect(next?.actors.another?.speaking).toBe(true);
  });

  it("applies stage.walk motion and heading hints immediately", () => {
    const next = applyCommandHint(
      snapshotFixture(),
      commandFixture("stage.walk", {
        targetActorId: "yuukei",
        payload: { destination: "left-edge", motion: "歩く" }
      })
    );

    expect(next?.actors.yuukei?.motion).toBe("歩く");
    expect(next?.actors.yuukei?.heading).toBe("left");
  });
});

function hitZone(
  id: string,
  events = ["avatar.gesture.poke"]
): ResolvedActorHitZone {
  return {
    id,
    label: id === "head" ? "頭" : undefined,
    source: "humanoidBone",
    bones: [id],
    nodes: [],
    shape: "auto",
    events
  };
}

function indexedSkinGeometry(): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(
      [0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0],
      3
    )
  );
  geometry.setIndex([0, 1, 2]);
  geometry.setAttribute(
    "skinIndex",
    new THREE.Uint16BufferAttribute(
      [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3],
      4
    )
  );
  geometry.setAttribute(
    "skinWeight",
    new THREE.Float32BufferAttribute(
      [
        0.1, 0.8, 0.1, 0,
        0.2, 0.7, 0.1, 0,
        0.1, 0.9, 0, 0,
        0.5, 0.5, 0, 0
      ],
      4
    )
  );
  return geometry;
}

function nonIndexedSkinGeometry(): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute([0, 0, 0, 1, 0, 0, 0, 1, 0], 3)
  );
  geometry.setAttribute(
    "skinIndex",
    new THREE.Uint16BufferAttribute(
      [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3],
      4
    )
  );
  geometry.setAttribute(
    "skinWeight",
    new THREE.Float32BufferAttribute(
      [0.1, 0.1, 0.8, 0, 0, 0.2, 0.8, 0, 0, 0.1, 0.9, 0],
      4
    )
  );
  return geometry;
}

function actorAsset(actorId: string, renderable: boolean): ActorSurfaceAsset {
  return {
    actorId,
    displayName: actorId,
    renderer: renderable
      ? {
          kind: "vrm",
          modelUrl: `yuukei-pack://localhost/actors/${actorId}/model`,
          motions: {},
          hitZones: []
        }
      : undefined
  };
}

function snapshotFixture(): ResidentSnapshot {
  return {
    residentId: "resident-default",
    worldPackId: "default-yuukei",
    activeSurfaceId: "surface-actor",
    actors: {
      yuukei: {
        displayName: "Yuukei",
        expression: "neutral",
        motion: "idle",
        heading: "",
        location: "desktop"
      },
      another: {
        displayName: "Another",
        expression: "neutral",
        motion: "idle",
        heading: "",
        location: "desktop"
      }
    },
    surfaces: {},
    capabilities: {},
    extensions: {},
    recentEventCursor: "cursor-1"
  };
}

function commandFixture(
  type: string,
  options: {
    targetActorId?: string;
    speakerId?: string;
    payload?: Record<string, unknown>;
  } = {}
): RuntimeCommand {
  return {
    id: "cmd-test",
    type,
    timestamp: "2026-06-30T00:00:00.000Z",
    source: "daihon",
    residentId: "resident-default",
    payload: {
      ...(options.speakerId ? { speakerId: options.speakerId } : {}),
      ...(options.payload ?? {})
    },
    target: options.targetActorId
      ? {
          actorId: options.targetActorId,
          surfaceId: "surface-actor"
        }
      : undefined
  };
}
