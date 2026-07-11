export type PointerGestureSemanticQueue = {
  enqueue(task: () => Promise<void>): Promise<void>;
};

export function createPointerGestureSemanticQueue(): PointerGestureSemanticQueue {
  let tail = Promise.resolve();

  return {
    enqueue(task) {
      const completion = tail.then(task);
      tail = completion.catch(() => undefined);
      return completion;
    }
  };
}
