export interface WhiteboardRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface OpenPositionOptions {
  preferred: { x: number; y: number };
  width: number;
  height: number;
  occupied: WhiteboardRect[];
  gap?: number;
}

interface NewTopicPositionOptions {
  width: number;
  height: number;
  occupied: WhiteboardRect[];
  gutter?: number;
  top?: number;
}

function overlaps(
  candidate: WhiteboardRect,
  occupied: WhiteboardRect,
  gap: number,
): boolean {
  return candidate.x < occupied.x + occupied.width + gap
    && candidate.x + candidate.width + gap > occupied.x
    && candidate.y < occupied.y + occupied.height + gap
    && candidate.y + candidate.height + gap > occupied.y;
}

/**
 * Place new board UI near its preferred location without covering existing
 * lesson nodes, student ink, questions, or auxiliary cards. Candidates come
 * from the edges of real occupied rectangles first; a bounded grid is only a
 * fallback for dense boards.
 */
export function findOpenWhiteboardPosition({
  preferred,
  width,
  height,
  occupied,
  gap = 24,
}: OpenPositionOptions): { x: number; y: number } {
  const finiteOccupied = occupied.filter((rect) =>
    Number.isFinite(rect.x)
    && Number.isFinite(rect.y)
    && Number.isFinite(rect.width)
    && Number.isFinite(rect.height)
    && rect.width > 0
    && rect.height > 0);
  const candidates = [{ ...preferred }];
  for (const rect of finiteOccupied) {
    candidates.push(
      { x: rect.x + rect.width + gap, y: rect.y },
      { x: rect.x - width - gap, y: rect.y },
      { x: rect.x, y: rect.y + rect.height + gap },
      { x: rect.x, y: rect.y - height - gap },
    );
  }
  const stepX = Math.max(120, width / 2 + gap);
  const stepY = Math.max(100, height / 2 + gap);
  for (let ring = 1; ring <= 8; ring += 1) {
    for (let x = -ring; x <= ring; x += 1) {
      candidates.push(
        { x: preferred.x + x * stepX, y: preferred.y - ring * stepY },
        { x: preferred.x + x * stepX, y: preferred.y + ring * stepY },
      );
    }
    for (let y = -ring + 1; y < ring; y += 1) {
      candidates.push(
        { x: preferred.x - ring * stepX, y: preferred.y + y * stepY },
        { x: preferred.x + ring * stepX, y: preferred.y + y * stepY },
      );
    }
  }
  const ordered = candidates
    .map((candidate, index) => ({
      ...candidate,
      index,
      distance: (candidate.x - preferred.x) ** 2
        + (candidate.y - preferred.y) ** 2,
    }))
    .sort((left, right) => left.distance - right.distance || left.index - right.index);
  const open = ordered.find((candidate) => {
    const bounds = { x: candidate.x, y: candidate.y, width, height };
    return finiteOccupied.every((rect) => !overlaps(bounds, rect, gap));
  });
  if (open) return { x: open.x, y: open.y };
  const rightEdge = finiteOccupied.reduce(
    (maximum, rect) => Math.max(maximum, rect.x + rect.width),
    preferred.x,
  );
  return { x: rightEdge + gap, y: preferred.y };
}

/**
 * A new composer lesson is a new whiteboard topic, not another loose card.
 * Start its question/loading footprint after everything already on the board
 * so it cannot be packed into a hole inside an older lesson or handwriting.
 */
export function findNewTopicWhiteboardPosition({
  occupied,
  gutter = 180,
  top = 90,
}: NewTopicPositionOptions): { x: number; y: number } {
  const finiteOccupied = occupied.filter((rect) =>
    Number.isFinite(rect.x)
    && Number.isFinite(rect.y)
    && Number.isFinite(rect.width)
    && Number.isFinite(rect.height)
    && rect.width > 0
    && rect.height > 0);
  if (finiteOccupied.length === 0) return { x: 100, y: top };
  const rightEdge = Math.max(...finiteOccupied.map((rect) => rect.x + rect.width));
  return { x: rightEdge + gutter, y: top };
}
