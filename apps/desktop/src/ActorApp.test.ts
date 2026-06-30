import { describe, expect, it } from "vitest";
import { expressionPresetFor, normalizeMotionId } from "./ActorApp";
import {
  autoHitZoneDefinitions,
  buildAvatarGesturePokePayload,
  chooseHitZoneCandidate,
  mergeHitZoneDefinitions,
  type ResolvedActorHitZone
} from "./actorHitZones";

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
