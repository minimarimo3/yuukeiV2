import { describe, expect, it, vi } from "vitest";
import { createPointerGestureSemanticQueue } from "./pointerGestureSemanticQueue";

describe("pointer gesture semantic notification queue", () => {
  it("does not start drop before grab completes", async () => {
    let resolveGrab: (() => void) | undefined;
    const grab = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveGrab = resolve;
        })
    );
    const drop = vi.fn(async () => undefined);
    const queue = createPointerGestureSemanticQueue();

    const grabCompletion = queue.enqueue(grab);
    const dropCompletion = queue.enqueue(drop);
    await Promise.resolve();

    expect(grab).toHaveBeenCalledOnce();
    expect(drop).not.toHaveBeenCalled();

    resolveGrab?.();
    await grabCompletion;
    await dropCompletion;

    expect(drop).toHaveBeenCalledOnce();
  });

  it("continues with drop after grab fails", async () => {
    const calls: string[] = [];
    const queue = createPointerGestureSemanticQueue();

    const grabCompletion = queue.enqueue(async () => {
      calls.push("grab");
      throw new Error("grab failed");
    });
    const dropCompletion = queue.enqueue(async () => {
      calls.push("drop");
    });

    await expect(grabCompletion).rejects.toThrow("grab failed");
    await expect(dropCompletion).resolves.toBeUndefined();
    expect(calls).toEqual(["grab", "drop"]);
  });
});
