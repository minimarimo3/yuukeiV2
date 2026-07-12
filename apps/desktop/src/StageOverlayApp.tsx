import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties
} from "react";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import {
  tauriYuukeiClient,
  type AppSettingsState,
  type DesktopStageState,
  type StageActor,
  type StageBubble,
  type StageMonitor,
  type StageRect as ClientStageRect,
  type YuukeiClient
} from "./yuukeiClient";
import {
  computeStageBubblePlacement,
  intersectsViewport,
  localRect,
  type StageBubblePlacement,
  type StageBubbleSize,
  type StageRect
} from "./stageBubbleLayout";
import { ConversationComposer } from "./ConversationComposer";

type StageOverlayAppProps = {
  monitorId?: string | null;
  client?: YuukeiClient;
};

type StageBubbleRenderItem = {
  bubble: StageBubble;
  actor: StageActor;
  placement: StageBubblePlacement;
};

const DEFAULT_BUBBLE_SIZE: StageBubbleSize = {
  width: 260,
  height: 72
};
const SPEECH_FALLBACK_GRACE_MS = 5_000;
const READING_MS_PER_CODE_POINT = 90;

export function bubbleTypingProgress(bubble: StageBubble, now: number): number {
  const characterCount = [...bubble.text].length;
  if (characterCount === 0) return 1;

  const fallbackStartMs = bubble.createdAtMs +
    (bubble.speechPending ? SPEECH_FALLBACK_GRACE_MS : 0);
  const fallbackDurationMs = Math.min(
    characterCount * READING_MS_PER_CODE_POINT,
    Math.max(
      bubble.durationMs - (bubble.speechPending ? SPEECH_FALLBACK_GRACE_MS : 0),
      1
    ) * 0.8
  );
  const fallbackProgress = now < fallbackStartMs
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

export function StageOverlayApp({
  monitorId,
  client = tauriYuukeiClient
}: StageOverlayAppProps) {
  const [stageState, setStageState] = useState<DesktopStageState | null>(null);
  const [appSettings, setAppSettings] = useState<AppSettingsState | null>(null);
  const [bubbleSizes, setBubbleSizes] = useState<Record<string, StageBubbleSize>>({});
  const [interactingBubbleIds, setInteractingBubbleIds] = useState<Set<string>>(
    () => new Set()
  );
  const [hiddenChoiceIds, setHiddenChoiceIds] = useState<Set<string>>(
    () => new Set()
  );
  const [deferUntil, setDeferUntil] = useState<Record<string, number>>({});
  const [, setTimerTick] = useState(0);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      unlisteners.push(await client.onStageState((nextState) => {
        setStageState(nextState);
      }));
      unlisteners.push(await client.onAppSettings((nextSettings) => {
        setAppSettings(nextSettings);
      }));
      const [initialState, initialSettings] = await Promise.all([
        client.getDesktopStageState(),
        client.getAppSettings()
      ]);
      if (!disposed) {
        setStageState(initialState);
        setAppSettings(initialSettings);
      }
    }

    void connect().catch((error) => {
      console.warn("Failed to connect stage overlay", error);
    });
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
      void client.setStageOverlayClickThrough(true);
    };
  }, [client]);

  const activeMonitor = useMemo(
    () => selectMonitor(stageState, monitorId),
    [stageState, monitorId]
  );
  const renderItems = useMemo(
    () => computeRenderItems(stageState, activeMonitor, bubbleSizes),
    [activeMonitor, bubbleSizes, stageState]
  );
  const composer = useMemo(
    () => composerForMonitor(stageState, activeMonitor),
    [activeMonitor, stageState]
  );

  useBubbleExpiry({
    bubbles: stageState?.bubbles ?? [],
    client,
    deferUntil,
    interactingBubbleIds,
    onTick: () => setTimerTick((tick) => tick + 1)
  });
  useStageOverlayHitTesting(client, renderItems.length + (composer ? 1 : 0));

  const updateBubbleSize = useCallback(
    (bubbleId: string, size: StageBubbleSize) => {
      setBubbleSizes((current) => {
        const previous = current[bubbleId];
        if (
          previous &&
          Math.abs(previous.width - size.width) < 0.5 &&
          Math.abs(previous.height - size.height) < 0.5
        ) {
          return current;
        }
        return { ...current, [bubbleId]: size };
      });
    },
    []
  );

  const setBubbleInteracting = useCallback((bubbleId: string, active: boolean) => {
    setInteractingBubbleIds((current) => {
      const next = new Set(current);
      if (active) {
        next.add(bubbleId);
      } else {
        next.delete(bubbleId);
      }
      return next;
    });
  }, []);

  const deferBubble = useCallback((bubbleId: string, durationMs = 2500) => {
    setDeferUntil((current) => ({
      ...current,
      [bubbleId]: Date.now() + durationMs
    }));
  }, []);

  return (
    <main
      className="stage-overlay-shell"
      aria-label="Yuukei desktop stage"
      onPointerDownCapture={(event) => {
        const target = event.target;
        if (
          composer &&
          target instanceof Element &&
          !target.closest(".conversation-composer-position")
        ) {
          void client.closeConversationComposer();
        }
      }}
    >
      <div className="stage-overlay-layer" aria-live="polite">
        {renderItems.map((item) => (
          <StageBubbleView
            item={item}
            key={item.bubble.bubbleId}
            hiddenChoiceIds={hiddenChoiceIds}
            onChoiceSelect={(choiceId, choice, index) => {
              setHiddenChoiceIds((current) => new Set(current).add(choiceId));
              void client.sendConversationChoice(choiceId, choice, index).catch((error) => {
                console.warn("Failed to send conversation choice", error);
              });
            }}
            onBlur={() => {
              setBubbleInteracting(item.bubble.bubbleId, false);
              deferBubble(item.bubble.bubbleId, 1200);
            }}
            onFocus={() => setBubbleInteracting(item.bubble.bubbleId, true)}
            onMouseEnter={() => setBubbleInteracting(item.bubble.bubbleId, true)}
            onMouseLeave={() => {
              setBubbleInteracting(item.bubble.bubbleId, false);
              deferBubble(item.bubble.bubbleId, 1200);
            }}
            onScroll={() => deferBubble(item.bubble.bubbleId)}
            onSizeChange={updateBubbleSize}
            onWheel={() => deferBubble(item.bubble.bubbleId)}
          />
        ))}
        {composer ? (
          <div
            className="conversation-composer-position"
            data-stage-solid="true"
            style={{ left: composer.left, top: composer.top }}
            onPointerDown={(event) => event.stopPropagation()}
          >
            <ConversationComposer
              shortcut={appSettings?.conversationSendShortcut ?? "ctrlEnter"}
              onSubmit={async (text) => {
                await client.sendConversationText(text);
              }}
              onDismiss={() => {
                void client.closeConversationComposer();
              }}
            />
          </div>
        ) : null}
      </div>
    </main>
  );
}

export function stageOverlayIdFromLocation(
  search = window.location.search
): string | null {
  const monitorId = new URLSearchParams(search).get("stageOverlayId");
  return monitorId && monitorId.length > 0 ? monitorId : null;
}

function StageBubbleView({
  hiddenChoiceIds,
  item,
  onBlur,
  onChoiceSelect,
  onFocus,
  onMouseEnter,
  onMouseLeave,
  onScroll,
  onSizeChange,
  onWheel
}: {
  hiddenChoiceIds: Set<string>;
  item: StageBubbleRenderItem;
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
    sideClass
  ]
    .filter(Boolean)
    .join(" ");
  const style = {
    left: `${item.placement.left}px`,
    top: `${item.placement.top}px`,
    "--actor-bubble-max-width": `${item.placement.maxWidth}px`,
    "--actor-bubble-tail-top": `${item.placement.tailTop}px`,
    "--actor-bubble-tail-left": `${item.placement.tailLeft}px`
  } as CSSProperties;

  return (
    <div
      className={className}
      data-actor-id={item.actor.actorId}
      data-stage-solid="true"
      onBlur={onBlur}
      onFocus={onFocus}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onScroll={onScroll}
      onWheel={onWheel}
      ref={ref}
      style={style}
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
              key={`${index}:${character}`}
              style={{ visibility: index < visibleCharacterCount ? "visible" : "hidden" }}
            >
              {character}
            </span>
          ))}
          {waitingForSpeech ? (
            <span className="actor-bubble-placeholder" aria-label="読み上げを待っています">
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
      progress: bubbleTypingProgress(bubble, now)
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
            : nextProgress
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
  onSizeChange: (bubbleId: string, size: StageBubbleSize) => void
) {
  const ref = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;

    const update = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 && rect.height <= 0) return;
      onSizeChange(bubbleId, {
        width: Math.max(rect.width, 1),
        height: Math.max(rect.height, 1)
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
  onTick
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
      if (interactingBubbleIds.has(bubble.bubbleId)) {
        timers.push(window.setTimeout(onTick, 500));
        continue;
      }
      const expiry = Math.max(
        bubble.createdAtMs + bubble.durationMs,
        deferUntil[bubble.bubbleId] ?? 0
      );
      const delay = expiry - now;
      if (delay <= 0) {
        void client.dismissStageBubble(bubble.bubbleId).catch((error) => {
          console.warn("Failed to dismiss stage bubble", error);
        });
      } else {
        timers.push(window.setTimeout(onTick, Math.min(delay, 1000)));
      }
    }
    return () => {
      for (const timer of timers) {
        window.clearTimeout(timer);
      }
    };
  }, [bubbles, client, deferUntil, interactingBubbleIds, onTick]);
}

function useStageOverlayHitTesting(
  client: YuukeiClient,
  activeInteractiveCount: number
) {
  useEffect(() => {
    let disposed = false;
    let lastPassthrough: boolean | null = null;

    async function update() {
      const solid = activeInteractiveCount > 0 && (await pointerHitsStageSolid());
      const passthrough = stageOverlayPassthrough(solid);
      if (!disposed && lastPassthrough !== passthrough) {
        lastPassthrough = passthrough;
        await client.setStageOverlayClickThrough(passthrough);
      }
    }

    void update().catch(() => undefined);
    const interval = window.setInterval(() => {
      void update().catch(() => undefined);
    }, 80);
    return () => {
      disposed = true;
      window.clearInterval(interval);
      void client.setStageOverlayClickThrough(true);
    };
  }, [activeInteractiveCount, client]);
}

export function stageOverlayPassthrough(pointerHitsInteractiveContent: boolean): boolean {
  return !pointerHitsInteractiveContent;
}

function composerForMonitor(
  stageState: DesktopStageState | null,
  monitor: StageMonitor | null
): { left: number; top: number } | null {
  const composer = stageState?.conversationComposer;
  if (
    !composer ||
    !composer.anchor.visible ||
    !monitor ||
    (composer.monitorId && composer.monitorId !== monitor.id)
  ) {
    return null;
  }
  const { bounds } = monitor;
  if (
    composer.anchor.x < bounds.x ||
    composer.anchor.x > bounds.x + bounds.width ||
    composer.anchor.y < bounds.y ||
    composer.anchor.y > bounds.y + bounds.height
  ) {
    return null;
  }
  const width = Math.min(340, Math.max(bounds.width - 24, 1));
  return {
    left: Math.max(12, Math.min(composer.anchor.x - bounds.x - width / 2, bounds.width - width - 12)),
    top: Math.max(12, Math.min(composer.anchor.y - bounds.y + 28, bounds.height - 180))
  };
}

async function pointerHitsStageSolid(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
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

  return Boolean(
    document
      .elementFromPoint(clientX, clientY)
      ?.closest("[data-stage-solid='true']")
  );
}

function computeRenderItems(
  stageState: DesktopStageState | null,
  monitor: StageMonitor | null,
  bubbleSizes: Record<string, StageBubbleSize>
): StageBubbleRenderItem[] {
  if (!stageState || !monitor) return [];
  const viewport = {
    width: Math.max(monitor.bounds.width, 1),
    height: Math.max(monitor.bounds.height, 1)
  };
  const actorsById = new Map(
    stageState.actors.map((actor) => [actor.actorId, actor])
  );
  const monitorBounds = toLayoutRect(monitor.bounds);
  const actorObstacles = stageState.actors
    .filter(
      (actor) =>
        actor.visible && intersectsViewport(toLayoutRect(actor.bounds), monitorBounds)
    )
    .map((actor) => localRect(toLayoutRect(actor.bounds), monitorBounds));
  const occupied: StageRect[] = [...actorObstacles];
  const items: StageBubbleRenderItem[] = [];

  for (const bubble of [...stageState.bubbles].sort(
    (a, b) => a.createdAtMs - b.createdAtMs
  )) {
    const actor = actorsById.get(bubble.actorId);
    if (
      !actor ||
      !actor.visible ||
      !intersectsViewport(toLayoutRect(actor.bounds), monitorBounds)
    ) {
      continue;
    }
    const anchor = localAnchorForActor(actor, monitor.bounds);
    const placement = computeStageBubblePlacement(
      anchor,
      viewport,
      bubbleSizes[bubble.bubbleId] ?? DEFAULT_BUBBLE_SIZE,
      occupied
    );
    occupied.push(placement.rect);
    items.push({ actor, bubble, placement });
  }

  return items;
}

function localAnchorForActor(actor: StageActor, origin: ClientStageRect) {
  if (actor.anchor.visible) {
    return {
      x: actor.anchor.x - origin.x,
      y: actor.anchor.y - origin.y,
      visible: true
    };
  }
  return {
    x: actor.bounds.x - origin.x + actor.bounds.width * 0.5,
    y: actor.bounds.y - origin.y + actor.bounds.height * 0.28,
    visible: true
  };
}

function selectMonitor(
  stageState: DesktopStageState | null,
  monitorId: string | null | undefined
): StageMonitor | null {
  if (!stageState) return null;
  return (
    stageState.monitors.find((monitor) => monitor.id === monitorId) ??
    stageState.monitors[0] ??
    null
  );
}

function toLayoutRect(rect: ClientStageRect): StageRect {
  return {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height
  };
}

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}
