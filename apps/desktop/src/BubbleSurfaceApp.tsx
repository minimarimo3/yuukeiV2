import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import {
  type CSSProperties,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  computeStageBubblePlacement,
  intersectsViewport,
  localRect,
  rectOverlapArea,
  type StageBubblePlacement,
  type StageBubbleSize,
  type StageRect,
} from "./stageBubbleLayout";
import {
  type StageRect as ClientStageRect,
  type DesktopStageState,
  type StageActor,
  type StageBubble,
  type StageMonitor,
  tauriYuukeiClient,
  type YuukeiClient,
} from "./yuukeiClient";

type BubbleSurfaceAppProps = {
  actorId?: string | null;
  client?: YuukeiClient;
};

type BubbleRenderItem = {
  bubble: StageBubble;
  actor: StageActor;
  monitor: StageMonitor;
  placement: StageBubblePlacement;
};

type MeasuredBubble = StageBubbleSize & {
  bubbleId: string;
};

const DEFAULT_BUBBLE_SIZE: StageBubbleSize = {
  width: 260,
  height: 72,
};
const SURFACE_PADDING = 16;
const SPEECH_FALLBACK_GRACE_MS = 5_000;
const READING_MS_PER_CODE_POINT = 90;

export function bubbleTypingProgress(bubble: StageBubble, now: number): number {
  const characterCount = [...bubble.text].length;
  if (characterCount === 0) return 1;

  const fallbackStartMs =
    bubble.createdAtMs + (bubble.speechPending ? SPEECH_FALLBACK_GRACE_MS : 0);
  const fallbackDurationMs = Math.min(
    characterCount * READING_MS_PER_CODE_POINT,
    Math.max(
      bubble.durationMs - (bubble.speechPending ? SPEECH_FALLBACK_GRACE_MS : 0),
      1,
    ) * 0.8,
  );
  const fallbackProgress =
    now < fallbackStartMs
      ? 0
      : clampUnit((now - fallbackStartMs) / Math.max(fallbackDurationMs, 1));
  if (!bubble.speechPending) return fallbackProgress;

  const audioProgress =
    typeof bubble.audioStartedAtMs === "number" &&
    typeof bubble.audioDurationMs === "number" &&
    bubble.audioDurationMs > 0
      ? clampUnit((now - bubble.audioStartedAtMs) / bubble.audioDurationMs)
      : 0;
  return Math.max(fallbackProgress, audioProgress);
}

function clampUnit(value: number): number {
  return Math.min(Math.max(value, 0), 1);
}

export function BubbleSurfaceApp({
  actorId,
  client = tauriYuukeiClient,
}: BubbleSurfaceAppProps) {
  const [stageState, setStageState] = useState<DesktopStageState | null>(null);
  const [measuredBubble, setMeasuredBubble] = useState<MeasuredBubble | null>(
    null,
  );
  const [interactingBubbleIds, setInteractingBubbleIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [hiddenChoiceIds, setHiddenChoiceIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [deferUntil, setDeferUntil] = useState<Record<string, number>>({});
  const [, setTimerTick] = useState(0);
  const [surfaceConnected, setSurfaceConnected] = useState(false);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      unlisteners.push(
        await client.onStageState((nextState) => {
          setStageState(nextState);
        }),
      );
      const initialState = await client.getDesktopStageState();
      if (!disposed) {
        setStageState(initialState);
        setSurfaceConnected(true);
      }
    }

    void connect().catch((error) => {
      console.warn("Failed to connect bubble surface", error);
    });
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
      void client.setBubbleSurfaceClickThrough(true);
    };
  }, [client]);

  useEffect(() => {
    if (!surfaceConnected) return;
    void client.surfaceReady?.().catch((error) => {
      console.warn("Failed to mark bubble surface ready", error);
    });
  }, [client, surfaceConnected]);

  const renderItem = useMemo(
    () => computeRenderItem(stageState, actorId, measuredBubble),
    [actorId, measuredBubble, stageState],
  );

  useEffect(() => {
    if (
      !renderItem ||
      measuredBubble?.bubbleId !== renderItem.bubble.bubbleId
    ) {
      return;
    }
    const bounds = bubbleSurfaceBounds(
      renderItem.monitor.bounds,
      renderItem.placement,
    );
    void client
      .placeBubbleSurface(renderItem.bubble.bubbleId, bounds)
      .catch((error) => {
        console.warn("Failed to place bubble surface", error);
      });
  }, [client, measuredBubble?.bubbleId, renderItem]);

  useBubbleExpiry({
    bubbles: renderItem ? [renderItem.bubble] : [],
    client,
    deferUntil,
    interactingBubbleIds,
    onTick: () => setTimerTick((tick) => tick + 1),
  });
  useBubbleSurfaceHitTesting(client, renderItem ? 1 : 0);

  const updateBubbleSize = useCallback(
    (bubbleId: string, size: StageBubbleSize) => {
      setMeasuredBubble((current) => {
        if (
          current?.bubbleId === bubbleId &&
          Math.abs(current.width - size.width) < 0.5 &&
          Math.abs(current.height - size.height) < 0.5
        ) {
          return current;
        }
        return { bubbleId, ...size };
      });
    },
    [],
  );

  const setBubbleInteracting = useCallback(
    (bubbleId: string, active: boolean) => {
      setInteractingBubbleIds((current) => {
        const next = new Set(current);
        if (active) next.add(bubbleId);
        else next.delete(bubbleId);
        return next;
      });
    },
    [],
  );

  const deferBubble = useCallback((bubbleId: string, durationMs = 2500) => {
    setDeferUntil((current) => ({
      ...current,
      [bubbleId]: Date.now() + durationMs,
    }));
  }, []);

  return (
    <main className="bubble-surface-shell" aria-label="Yuukei speech bubble">
      {renderItem ? (
        <BubbleView
          item={renderItem}
          hiddenChoiceIds={hiddenChoiceIds}
          onChoiceSelect={(choiceId, choice, index) => {
            setHiddenChoiceIds((current) => new Set(current).add(choiceId));
            void client
              .sendConversationChoice(choiceId, choice, index)
              .catch((error) => {
                console.warn("Failed to send conversation choice", error);
              });
          }}
          onBlur={() => {
            setBubbleInteracting(renderItem.bubble.bubbleId, false);
            deferBubble(renderItem.bubble.bubbleId, 1200);
          }}
          onFocus={() => setBubbleInteracting(renderItem.bubble.bubbleId, true)}
          onMouseEnter={() =>
            setBubbleInteracting(renderItem.bubble.bubbleId, true)
          }
          onMouseLeave={() => {
            setBubbleInteracting(renderItem.bubble.bubbleId, false);
            deferBubble(renderItem.bubble.bubbleId, 1200);
          }}
          onScroll={() => deferBubble(renderItem.bubble.bubbleId)}
          onSizeChange={updateBubbleSize}
          onWheel={() => deferBubble(renderItem.bubble.bubbleId)}
        />
      ) : null}
    </main>
  );
}

export function bubbleActorIdFromLocation(
  search = window.location.search,
): string | null {
  const actorId = new URLSearchParams(search).get("bubbleActorId");
  return actorId && actorId.length > 0 ? actorId : null;
}

export function bubbleSurfaceBounds(
  monitorBounds: ClientStageRect,
  placement: StageBubblePlacement,
): ClientStageRect {
  return {
    x: monitorBounds.x + placement.left - SURFACE_PADDING,
    y: monitorBounds.y + placement.top - SURFACE_PADDING,
    width: placement.rect.width + SURFACE_PADDING * 2,
    height: placement.rect.height + SURFACE_PADDING * 2,
  };
}

function BubbleView({
  hiddenChoiceIds,
  item,
  onBlur,
  onChoiceSelect,
  onFocus,
  onMouseEnter,
  onMouseLeave,
  onScroll,
  onSizeChange,
  onWheel,
}: {
  hiddenChoiceIds: Set<string>;
  item: BubbleRenderItem;
  onBlur(): void;
  onChoiceSelect(choiceId: string, choice: string, index: number): void;
  onFocus(): void;
  onMouseEnter(): void;
  onMouseLeave(): void;
  onScroll(): void;
  onSizeChange(bubbleId: string, size: StageBubbleSize): void;
  onWheel(): void;
}) {
  const { ref } = useMeasuredBubbleSize(item.bubble.bubbleId, onSizeChange);
  const choice = item.bubble.choice;
  const typing = useBubbleTypingProgress(item.bubble);
  const characters = [...item.bubble.text];
  const visibleCharacterCount = Math.floor(typing.progress * characters.length);
  const waitingForSpeech =
    item.bubble.speechPending &&
    item.bubble.audioStartedAtMs === undefined &&
    typing.now < item.bubble.createdAtMs + SPEECH_FALLBACK_GRACE_MS;
  const visibleChoices =
    choice && !hiddenChoiceIds.has(choice.choiceId) ? choice.choices : [];
  const sideClass =
    item.placement.side === "left"
      ? "actor-bubble--left"
      : item.placement.side === "right"
        ? "actor-bubble--right"
        : "";
  const className = [
    "actor-bubble",
    "stage-bubble",
    `stage-bubble--${item.placement.side}`,
    sideClass,
  ]
    .filter(Boolean)
    .join(" ");
  const style = {
    left: `${SURFACE_PADDING}px`,
    top: `${SURFACE_PADDING}px`,
    "--actor-bubble-max-width": `${item.placement.maxWidth}px`,
    "--actor-bubble-tail-top": `${item.placement.tailTop}px`,
    "--actor-bubble-tail-left": `${item.placement.tailLeft}px`,
  } as CSSProperties;

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: hover/focusで吹き出しの自動消滅を止めるための意図的なフォーカス可能要素
    <div
      className={className}
      data-actor-id={item.actor.actorId}
      data-bubble-solid="true"
      onBlur={onBlur}
      onFocus={onFocus}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onScroll={onScroll}
      onWheel={onWheel}
      ref={ref}
      style={style}
      /* biome-ignore lint/a11y/noNoninteractiveTabindex: hover/focusで吹き出しの自動消滅を止めるため */
      tabIndex={0}
    >
      <span className="actor-bubble-tail" aria-hidden="true" />
      {item.bubble.text ? (
        <span
          className="actor-bubble-content"
          data-typing-progress={typing.progress}
        >
          {characters.map((character, index) => (
            <span
              className="actor-bubble-character"
              data-typing-visible={index < visibleCharacterCount}
              /* biome-ignore lint/suspicious/noArrayIndexKey: 並び替えがなく、文字は重複しうる */
              key={`${index}:${character}`}
              style={{
                visibility:
                  index < visibleCharacterCount ? "visible" : "hidden",
              }}
            >
              {character}
            </span>
          ))}
          {waitingForSpeech ? (
            <span
              className="actor-bubble-placeholder"
              aria-label="読み上げを待っています"
              role="status"
            >
              …
            </span>
          ) : null}
        </span>
      ) : null}
      {choice && visibleChoices.length > 0 ? (
        <span className="actor-bubble-choices">
          {visibleChoices.map((label, index) => (
            <button
              className="actor-bubble-choice"
              /* biome-ignore lint/suspicious/noArrayIndexKey: 並び替えがなく、ラベルは重複しうる */
              key={`${choice.choiceId}:${index}`}
              onClick={(event) => {
                event.stopPropagation();
                onChoiceSelect(choice.choiceId, label, index);
              }}
              type="button"
            >
              {label}
            </button>
          ))}
        </span>
      ) : null}
    </div>
  );
}

function useBubbleTypingProgress(bubble: StageBubble) {
  const [typing, setTyping] = useState(() => {
    const now = Date.now();
    return {
      bubbleId: bubble.bubbleId,
      now,
      progress: bubbleTypingProgress(bubble, now),
    };
  });

  useEffect(() => {
    let timer: number | undefined;
    const tick = () => {
      const now = Date.now();
      const nextProgress = bubbleTypingProgress(bubble, now);
      setTyping((current) => ({
        bubbleId: bubble.bubbleId,
        now,
        progress:
          current.bubbleId === bubble.bubbleId
            ? Math.max(current.progress, nextProgress)
            : nextProgress,
      }));
      if (nextProgress >= 1 && timer !== undefined) {
        window.clearInterval(timer);
        timer = undefined;
      }
    };

    tick();
    if (bubbleTypingProgress(bubble, Date.now()) < 1) {
      timer = window.setInterval(tick, 50);
    }
    return () => {
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [bubble]);

  if (typing.bubbleId !== bubble.bubbleId) {
    const now = Date.now();
    return { now, progress: bubbleTypingProgress(bubble, now) };
  }
  return typing;
}

function useMeasuredBubbleSize(
  bubbleId: string,
  onSizeChange: (bubbleId: string, size: StageBubbleSize) => void,
) {
  const ref = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;

    const update = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      onSizeChange(bubbleId, {
        width: Math.max(rect.width, 1),
        height: Math.max(rect.height, 1),
      });
    };

    update();
    if (!("ResizeObserver" in window)) return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [bubbleId, onSizeChange]);

  return { ref };
}

function useBubbleExpiry({
  bubbles,
  client,
  deferUntil,
  interactingBubbleIds,
  onTick,
}: {
  bubbles: StageBubble[];
  client: YuukeiClient;
  deferUntil: Record<string, number>;
  interactingBubbleIds: Set<string>;
  onTick(): void;
}) {
  useEffect(() => {
    const timers: number[] = [];
    const now = Date.now();

    for (const bubble of bubbles) {
      if (interactingBubbleIds.has(bubble.bubbleId)) continue;
      const expiry = Math.max(
        bubble.createdAtMs + bubble.durationMs,
        deferUntil[bubble.bubbleId] ?? 0,
      );
      const delay = expiry - now;
      if (delay <= 0) {
        void client.dismissStageBubble(bubble.bubbleId);
        continue;
      }
      timers.push(
        window.setTimeout(() => {
          void client.dismissStageBubble(bubble.bubbleId);
          onTick();
        }, delay),
      );
    }

    return () => {
      for (const timer of timers) window.clearTimeout(timer);
    };
  }, [bubbles, client, deferUntil, interactingBubbleIds, onTick]);
}

function useBubbleSurfaceHitTesting(
  client: YuukeiClient,
  activeInteractiveCount: number,
) {
  useEffect(() => {
    if (activeInteractiveCount === 0) {
      void client.setBubbleSurfaceClickThrough(true);
      return;
    }
    let disposed = false;
    let lastPassthrough: boolean | null = null;
    const update = async () => {
      const passthrough = !(await pointerHitsBubbleSolid());
      if (disposed || passthrough === lastPassthrough) return;
      lastPassthrough = passthrough;
      await client.setBubbleSurfaceClickThrough(passthrough);
    };
    void update();
    const interval = window.setInterval(() => void update(), 50);
    return () => {
      disposed = true;
      window.clearInterval(interval);
      void client.setBubbleSurfaceClickThrough(true);
    };
  }, [activeInteractiveCount, client]);
}

async function pointerHitsBubbleSolid(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  const windowHandle = getCurrentWindow();
  const [cursor, outerPosition, innerSize] = await Promise.all([
    cursorPosition(),
    windowHandle.outerPosition(),
    windowHandle.innerSize(),
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
  return Boolean(
    document
      .elementFromPoint(clientX, clientY)
      ?.closest("[data-bubble-solid='true']"),
  );
}

function computeRenderItem(
  stageState: DesktopStageState | null,
  actorId: string | null | undefined,
  measuredBubble: MeasuredBubble | null,
): BubbleRenderItem | null {
  if (!stageState || !actorId) return null;
  const actor = stageState.actors.find(
    (candidate) => candidate.actorId === actorId && candidate.visible,
  );
  const bubble = stageState.bubbles.find(
    (candidate) => candidate.actorId === actorId,
  );
  if (!actor || !bubble) return null;
  const monitor = selectMonitor(stageState.monitors, actor.bounds);
  if (!monitor) return null;

  const monitorBounds = toLayoutRect(monitor.bounds);
  const anchor = localAnchorForActor(actor, monitor.bounds);
  const actorObstacles = stageState.actors
    .filter(
      (candidate) =>
        candidate.visible &&
        intersectsViewport(toLayoutRect(candidate.bounds), monitorBounds),
    )
    .map((candidate) =>
      localRect(toLayoutRect(candidate.bounds), monitorBounds),
    );
  const size =
    measuredBubble?.bubbleId === bubble.bubbleId
      ? measuredBubble
      : DEFAULT_BUBBLE_SIZE;
  const placement = computeStageBubblePlacement(
    anchor,
    {
      width: Math.max(monitor.bounds.width, 1),
      height: Math.max(monitor.bounds.height, 1),
    },
    size,
    actorObstacles,
  );
  return { actor, bubble, monitor, placement };
}

function selectMonitor(
  monitors: StageMonitor[],
  actorBounds: ClientStageRect,
): StageMonitor | null {
  const actorRect = toLayoutRect(actorBounds);
  return (
    [...monitors].sort(
      (a, b) =>
        rectOverlapArea(actorRect, toLayoutRect(b.bounds)) -
        rectOverlapArea(actorRect, toLayoutRect(a.bounds)),
    )[0] ?? null
  );
}

function localAnchorForActor(actor: StageActor, origin: ClientStageRect) {
  if (actor.anchor.visible) {
    return {
      x: actor.anchor.x - origin.x,
      y: actor.anchor.y - origin.y,
      visible: true,
    };
  }
  return {
    x: actor.bounds.x - origin.x + actor.bounds.width * 0.5,
    y: actor.bounds.y - origin.y + actor.bounds.height * 0.28,
    visible: true,
  };
}

function toLayoutRect(rect: ClientStageRect): StageRect {
  return {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
  };
}

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}
