export type StageBubbleSide = "left" | "right" | "above" | "below";

export type StageRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type StageAnchor = {
  x: number;
  y: number;
  visible: boolean;
};

export type StageBubbleViewport = {
  width: number;
  height: number;
};

export type StageBubbleSize = {
  width: number;
  height: number;
};

export type StageBubblePlacement = {
  side: StageBubbleSide;
  left: number;
  top: number;
  width: number;
  maxWidth: number;
  tailTop: number;
  tailLeft: number;
  rect: StageRect;
};

export type StageBubblePlacementOptions = {
  margin?: number;
  gap?: number;
  minWidth?: number;
  maxWidth?: number;
};

const DEFAULT_MARGIN = 16;
const DEFAULT_GAP = 26;
const DEFAULT_MIN_WIDTH = 180;
const DEFAULT_MAX_WIDTH = 360;
const DEFAULT_SIZE: StageBubbleSize = {
  width: 260,
  height: 72
};

export function computeStageBubblePlacement(
  anchor: StageAnchor,
  viewport: StageBubbleViewport,
  measuredSize: Partial<StageBubbleSize> = DEFAULT_SIZE,
  obstacles: StageRect[] = [],
  options: StageBubblePlacementOptions = {}
): StageBubblePlacement {
  const margin = options.margin ?? DEFAULT_MARGIN;
  const gap = options.gap ?? DEFAULT_GAP;
  const minWidth = options.minWidth ?? DEFAULT_MIN_WIDTH;
  const maxWidth = options.maxWidth ?? DEFAULT_MAX_WIDTH;
  const viewportWidth = Math.max(viewport.width, margin * 2 + 1);
  const viewportHeight = Math.max(viewport.height, margin * 2 + 1);
  const interiorWidth = Math.max(viewportWidth - margin * 2, 1);
  const width = clamp(
    measuredSize.width ?? DEFAULT_SIZE.width,
    Math.min(minWidth, interiorWidth),
    Math.min(maxWidth, interiorWidth)
  );
  const height = Math.max(measuredSize.height ?? DEFAULT_SIZE.height, 40);
  const candidates = [
    ...candidatePlacements(
      anchor,
      { width, height },
      { width: viewportWidth, height: viewportHeight },
      margin,
      gap
    ),
    ...obstacleAvoidancePlacements(
      anchor,
      { width, height },
      { width: viewportWidth, height: viewportHeight },
      obstacles,
      margin,
      gap
    )
  ];
  const ranked = candidates
    .map((candidate) => ({
      candidate,
      overlapArea: obstacles.reduce(
        (total, obstacle) => total + rectOverlapArea(candidate.rect, obstacle),
        0
      ),
      distance:
        Math.abs(candidate.left + width / 2 - anchor.x) +
        Math.abs(candidate.top + height / 2 - anchor.y)
    }))
    .sort((a, b) => a.overlapArea - b.overlapArea || a.distance - b.distance);
  const selected = ranked[0]?.candidate ?? candidates[0];

  return {
    ...selected,
    width,
    maxWidth: width,
    tailTop: clamp(anchor.y - selected.top, 20, Math.max(height - 20, 20)),
    tailLeft: clamp(anchor.x - selected.left, 20, Math.max(width - 20, 20))
  };
}

export function rectsOverlap(a: StageRect, b: StageRect): boolean {
  return (
    a.x < b.x + b.width &&
    a.x + a.width > b.x &&
    a.y < b.y + b.height &&
    a.y + a.height > b.y
  );
}

export function rectOverlapArea(a: StageRect, b: StageRect): number {
  const width = Math.max(
    Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x),
    0
  );
  const height = Math.max(
    Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y),
    0
  );
  return width * height;
}

export function localRect(globalRect: StageRect, origin: StageRect): StageRect {
  return {
    x: globalRect.x - origin.x,
    y: globalRect.y - origin.y,
    width: globalRect.width,
    height: globalRect.height
  };
}

export function intersectsViewport(rect: StageRect, viewport: StageRect): boolean {
  return rectsOverlap(rect, viewport);
}

function candidatePlacements(
  anchor: StageAnchor,
  size: StageBubbleSize,
  viewport: StageBubbleViewport,
  margin: number,
  gap: number
): StageBubblePlacement[] {
  const rawCandidates: Array<{
    side: StageBubbleSide;
    left: number;
    top: number;
  }> = [
    {
      side: "right",
      left: anchor.x + gap,
      top: anchor.y - size.height * 0.52
    },
    {
      side: "left",
      left: anchor.x - gap - size.width,
      top: anchor.y - size.height * 0.52
    },
    {
      side: "above",
      left: anchor.x - size.width / 2,
      top: anchor.y - gap - size.height
    },
    {
      side: "below",
      left: anchor.x - size.width / 2,
      top: anchor.y + gap
    }
  ];

  return rawCandidates.map((candidate) =>
    buildPlacement(candidate, anchor, size, viewport, margin)
  );
}

function obstacleAvoidancePlacements(
  anchor: StageAnchor,
  size: StageBubbleSize,
  viewport: StageBubbleViewport,
  obstacles: StageRect[],
  margin: number,
  gap: number
): StageBubblePlacement[] {
  return obstacles.flatMap((obstacle) => {
    const rawCandidates: Array<{
      side: StageBubbleSide;
      left: number;
      top: number;
    }> = [
      {
        side: "right",
        left: obstacle.x + obstacle.width + gap,
        top: anchor.y - size.height * 0.52
      },
      {
        side: "left",
        left: obstacle.x - gap - size.width,
        top: anchor.y - size.height * 0.52
      },
      {
        side: "above",
        left: anchor.x - size.width / 2,
        top: obstacle.y - gap - size.height
      },
      {
        side: "below",
        left: anchor.x - size.width / 2,
        top: obstacle.y + obstacle.height + gap
      }
    ];
    return rawCandidates.map((candidate) =>
      buildPlacement(candidate, anchor, size, viewport, margin)
    );
  });
}

function buildPlacement(
  candidate: {
    side: StageBubbleSide;
    left: number;
    top: number;
  },
  anchor: StageAnchor,
  size: StageBubbleSize,
  viewport: StageBubbleViewport,
  margin: number
): StageBubblePlacement {
  const left = clamp(candidate.left, margin, viewport.width - margin - size.width);
  const top = clamp(candidate.top, margin, viewport.height - margin - size.height);
  return {
    side: candidate.side,
    left,
    top,
    width: size.width,
    maxWidth: size.width,
    tailTop: clamp(anchor.y - top, 20, Math.max(size.height - 20, 20)),
    tailLeft: clamp(anchor.x - left, 20, Math.max(size.width - 20, 20)),
    rect: {
      x: left,
      y: top,
      width: size.width,
      height: size.height
    }
  };
}

function clamp(value: number, min: number, max: number): number {
  if (max < min) return min;
  return Math.min(Math.max(value, min), max);
}
