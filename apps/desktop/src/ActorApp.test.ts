import { describe, expect, it } from "vitest";
import { expressionPresetFor, normalizeMotionId } from "./ActorApp";

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
});
