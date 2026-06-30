import { describe, expect, it, vi } from "vitest";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import {
  actorIdFromLocation,
  actorSurfaceAssetsForActor,
  applyCommandHint,
  expressionPresetFor,
  loadInitialActorSurfaceState,
  normalizeMotionId
} from "./ActorApp";
import {
  autoHitZoneDefinitions,
  buildAvatarGesturePokePayload,
  chooseHitZoneCandidate,
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
      new Set(["head", "rightHand", "hips", "spine"])
    );

    expect(zones.map((zone) => zone.id)).toEqual(["head", "rightHand", "body"]);
    expect(zones.find((zone) => zone.id === "body")?.bones).toEqual([
      "spine",
      "hips"
    ]);
  });

  it("merges pack hit zones over auto generated zones", () => {
    const autoZones = autoHitZoneDefinitions(new Set(["head", "hips"]));
    const zones = mergeHitZoneDefinitions(autoZones, [
      {
        id: "head",
        label: "おでこ",
        source: "humanoidBone",
        bones: ["head"],
        events: ["avatar.gesture.poke"],
        priority: 60
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
      events: ["avatar.gesture.poke"],
      priority: 60
    });
    expect(zones.find((zone) => zone.id === "tail")).toMatchObject({
      label: "しっぽ",
      source: "nodeName",
      nodes: ["Tail", "Tail_001"],
      events: ["avatar.gesture.poke"],
      priority: 100
    });
  });

  it("chooses pokeable hit zones by priority then distance", () => {
    const lowPriority = hitZone("body", 5);
    const highPriority = hitZone("head", 30);
    const patOnly = hitZone("hat", 100, ["avatar.gesture.pat"]);

    expect(
      chooseHitZoneCandidate([
        { zone: lowPriority, distance: 1 },
        { zone: highPriority, distance: 3 },
        { zone: patOnly, distance: 0.1 }
      ])?.zone.id
    ).toBe("head");

    expect(
      chooseHitZoneCandidate([
        { zone: hitZone("leftHand", 20), distance: 4 },
        { zone: hitZone("rightHand", 20), distance: 2 }
      ])?.zone.id
    ).toBe("rightHand");
  });

  it("builds avatar gesture poke payloads from pointer input", () => {
    const payload = buildAvatarGesturePokePayload("yuukei", hitZone("head", 30), {
      button: 0,
      screenX: 123,
      screenY: 456
    });

    expect(payload).toEqual({
      actorId: "yuukei",
      hitZoneId: "head",
      hitZoneLabel: "頭",
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
});

function hitZone(
  id: string,
  priority: number,
  events = ["avatar.gesture.poke"]
): ResolvedActorHitZone {
  return {
    id,
    label: id === "head" ? "頭" : undefined,
    source: "humanoidBone",
    bones: [id],
    nodes: [],
    shape: "auto",
    events,
    priority
  };
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
        location: "desktop"
      },
      another: {
        displayName: "Another",
        expression: "neutral",
        motion: "idle",
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
