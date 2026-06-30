export type ActorBubbleSide = "left" | "right";

export type ActorBubbleAnchor = {
  x: number;
  y: number;
  visible: boolean;
};

export type ActorBubbleViewport = {
  width: number;
  height: number;
};

export type ActorBubbleSize = {
  width: number;
  height: number;
};

export type ActorBubblePlacementOptions = {
  margin?: number;
  gap?: number;
  minWidth?: number;
  maxWidth?: number;
  stackOffsetY?: number;
};

export type ActorBubblePlacement = {
  side: ActorBubbleSide;
  left: number;
  top: number;
  width: number;
  maxWidth: number;
  tailTop: number;
};

const DEFAULT_MARGIN = 16;
const DEFAULT_GAP = 26;
const DEFAULT_MIN_WIDTH = 180;
const DEFAULT_MAX_WIDTH = 360;
const DEFAULT_SIZE: ActorBubbleSize = {
  width: 260,
  height: 72
};

export function chooseBubbleSide(
  anchorX: number,
  viewportWidth: number,
  margin = DEFAULT_MARGIN
): ActorBubbleSide {
  const leftSpace = Math.max(anchorX - margin, 0);
  const rightSpace = Math.max(viewportWidth - anchorX - margin, 0);
  return rightSpace >= leftSpace ? "right" : "left";
}

export function computeBubblePlacement(
  anchor: ActorBubbleAnchor,
  viewport: ActorBubbleViewport,
  measuredSize: Partial<ActorBubbleSize> = DEFAULT_SIZE,
  options: ActorBubblePlacementOptions = {}
): ActorBubblePlacement {
  const margin = options.margin ?? DEFAULT_MARGIN;
  const gap = options.gap ?? DEFAULT_GAP;
  const minWidth = options.minWidth ?? DEFAULT_MIN_WIDTH;
  const maxWidth = options.maxWidth ?? DEFAULT_MAX_WIDTH;
  const stackOffsetY = options.stackOffsetY ?? 0;
  const viewportWidth = Math.max(viewport.width, margin * 2 + 1);
  const viewportHeight = Math.max(viewport.height, margin * 2 + 1);
  const side = chooseBubbleSide(anchor.x, viewportWidth, margin);
  const availableSideWidth =
    side === "right" ? viewportWidth - anchor.x - margin : anchor.x - margin;
  const interiorWidth = Math.max(viewportWidth - margin * 2, 1);
  const minUsableWidth = Math.min(minWidth, interiorWidth);
  const sideWidthLimit = Math.max(availableSideWidth - gap, minUsableWidth);
  const maxUsableWidth = Math.max(
    Math.min(maxWidth, interiorWidth, sideWidthLimit),
    minUsableWidth
  );
  const width = clamp(
    measuredSize.width ?? DEFAULT_SIZE.width,
    minUsableWidth,
    maxUsableWidth
  );
  const height = Math.max(measuredSize.height ?? DEFAULT_SIZE.height, 40);
  const preferredLeft =
    side === "right" ? anchor.x + gap : anchor.x - gap - width;
  const left = clamp(preferredLeft, margin, viewportWidth - margin - width);
  const preferredTop = anchor.y - height * 0.52 + stackOffsetY;
  const top = clamp(preferredTop, margin, viewportHeight - margin - height);
  const tailTop = clamp(anchor.y - top, 20, Math.max(height - 20, 20));

  return {
    side,
    left,
    top,
    width,
    maxWidth: maxUsableWidth,
    tailTop
  };
}

export function fallbackActorBubbleAnchor(
  viewport: ActorBubbleViewport,
  index = 0,
  total = 1,
  options: Pick<ActorBubblePlacementOptions, "margin"> = {}
): ActorBubbleAnchor {
  const margin = options.margin ?? DEFAULT_MARGIN;
  const safeWidth = Math.max(viewport.width, margin * 2 + 1);
  const safeHeight = Math.max(viewport.height, margin * 2 + 1);
  const centeredIndex = index - (Math.max(total, 1) - 1) / 2;

  return {
    x: safeWidth / 2,
    y: clamp(safeHeight - 96 + centeredIndex * 24, margin + 48, safeHeight - margin),
    visible: true
  };
}

export function clampBubbleToViewport(
  value: number,
  min: number,
  max: number
): number {
  return clamp(value, min, max);
}

function clamp(value: number, min: number, max: number): number {
  if (max < min) return min;
  return Math.min(Math.max(value, min), max);
}
