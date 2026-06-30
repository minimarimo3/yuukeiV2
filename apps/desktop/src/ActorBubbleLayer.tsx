import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties
} from "react";
import type { ActorSnapshot } from "@yuukei/protocol";
import {
  computeBubblePlacement,
  fallbackActorBubbleAnchor,
  type ActorBubbleAnchor,
  type ActorBubbleSize,
  type ActorBubbleViewport
} from "./actorBubbleLayout";

type ActorBubbleLayerProps = {
  actors: Array<[string, ActorSnapshot]>;
  anchors: Record<string, ActorBubbleAnchor>;
  viewport?: ActorBubbleViewport;
};

type ActorBubbleProps = {
  actorId: string;
  text: string;
  anchor: ActorBubbleAnchor;
  viewport: ActorBubbleViewport;
  stackOffsetY: number;
};

const DEFAULT_BUBBLE_SIZE: ActorBubbleSize = {
  width: 260,
  height: 72
};

export function ActorBubbleLayer({
  actors,
  anchors,
  viewport
}: ActorBubbleLayerProps) {
  const measuredViewport = useViewportSize();
  const activeViewport = viewport ?? measuredViewport;
  const anchorBuckets = new Map<string, number>();

  return (
    <div className="actor-bubble-layer" aria-live="polite">
      {actors.map(([actorId, actor], index) => {
        const anchor = usableAnchor(anchors[actorId])
          ? anchors[actorId]
          : fallbackActorBubbleAnchor(activeViewport, index, actors.length);
        const bucketKey = `${Math.round(anchor.x / 12)}:${Math.round(anchor.y / 12)}`;
        const overlapIndex = anchorBuckets.get(bucketKey) ?? 0;
        anchorBuckets.set(bucketKey, overlapIndex + 1);

        return (
          <ActorBubble
            actorId={actorId}
            anchor={anchor}
            key={actorId}
            stackOffsetY={overlapIndex * 20}
            text={actor.bubble ?? ""}
            viewport={activeViewport}
          />
        );
      })}
    </div>
  );
}

export function ActorBubble({
  actorId,
  text,
  anchor,
  viewport,
  stackOffsetY
}: ActorBubbleProps) {
  const { ref, size } = useMeasuredBubbleSize(text);
  const placement = computeBubblePlacement(anchor, viewport, size, {
    stackOffsetY
  });
  const style = {
    left: `${placement.left}px`,
    top: `${placement.top}px`,
    "--actor-bubble-max-width": `${placement.maxWidth}px`,
    "--actor-bubble-tail-top": `${placement.tailTop}px`
  } as CSSProperties;

  return (
    <p
      className={`actor-bubble actor-bubble--${placement.side}`}
      data-actor-id={actorId}
      data-actor-solid="true"
      ref={ref}
      style={style}
    >
      <span className="actor-bubble-tail" aria-hidden="true" />
      <span className="actor-bubble-content">{text}</span>
    </p>
  );
}

function useMeasuredBubbleSize(text: string) {
  const ref = useRef<HTMLParagraphElement | null>(null);
  const [size, setSize] = useState<ActorBubbleSize>(DEFAULT_BUBBLE_SIZE);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;

    const update = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 && rect.height <= 0) return;
      setSize((current) => {
        const next = {
          width: Math.max(rect.width, 1),
          height: Math.max(rect.height, 1)
        };
        if (
          Math.abs(current.width - next.width) < 0.5 &&
          Math.abs(current.height - next.height) < 0.5
        ) {
          return current;
        }
        return next;
      });
    };

    update();
    if (!("ResizeObserver" in window)) return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [text]);

  return { ref, size };
}

function useViewportSize(): ActorBubbleViewport {
  const [viewport, setViewport] = useState<ActorBubbleViewport>(() =>
    readViewportSize()
  );

  useEffect(() => {
    const update = () => setViewport(readViewportSize());
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  return viewport;
}

function readViewportSize(): ActorBubbleViewport {
  return {
    width: Math.max(window.innerWidth || document.documentElement.clientWidth || 1, 1),
    height: Math.max(window.innerHeight || document.documentElement.clientHeight || 1, 1)
  };
}

function usableAnchor(
  anchor: ActorBubbleAnchor | undefined
): anchor is ActorBubbleAnchor {
  return Boolean(
    anchor &&
      anchor.visible &&
      Number.isFinite(anchor.x) &&
      Number.isFinite(anchor.y)
  );
}
