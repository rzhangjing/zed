import type { WhiteboardRect } from "./whiteboard-placement";

export const COURSE_REGION_RESERVED_WIDTH = 1_180;
export const COURSE_REGION_GUTTER = 180;
export const COURSE_RUNTIME_OFFSET_X = 294;
export const COURSE_PENDING_FOOTPRINT_WIDTH = 654;
export const COURSE_PENDING_FOOTPRINT_HEIGHT = 220;

export interface CourseRegionRecord {
  id: string;
  sessionId: string;
  questionId: string;
  runtimeRegionId?: string;
  origin: { x: number; y: number };
  bounds: WhiteboardRect;
  reservedWidth: number;
  createdAt: string;
}

const STORAGE_PREFIX = "octos-learning-course-regions:v1:";

export function courseRegionsStorageKey(sessionId: string): string {
  return `${STORAGE_PREFIX}${sessionId}`;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function validPoint(value: unknown): value is { x: number; y: number } {
  if (!value || typeof value !== "object") return false;
  const point = value as Record<string, unknown>;
  return finiteNumber(point.x) && finiteNumber(point.y);
}

function validRect(value: unknown): value is WhiteboardRect {
  if (!value || typeof value !== "object") return false;
  const rect = value as Record<string, unknown>;
  return finiteNumber(rect.x)
    && finiteNumber(rect.y)
    && finiteNumber(rect.width)
    && rect.width > 0
    && finiteNumber(rect.height)
    && rect.height > 0;
}

function validCourseRegion(
  value: unknown,
  sessionId: string,
): value is CourseRegionRecord {
  if (!value || typeof value !== "object") return false;
  const region = value as Record<string, unknown>;
  return typeof region.id === "string"
    && region.id.length > 0
    && region.sessionId === sessionId
    && typeof region.questionId === "string"
    && region.questionId.length > 0
    && (region.runtimeRegionId === undefined
      || typeof region.runtimeRegionId === "string")
    && validPoint(region.origin)
    && validRect(region.bounds)
    && finiteNumber(region.reservedWidth)
    && region.reservedWidth > 0
    && typeof region.createdAt === "string";
}

export function createCourseRegion(
  sessionId: string,
  questionId: string,
  origin: { x: number; y: number },
  initialSize: { width: number; height: number },
  createdAt = new Date().toISOString(),
): CourseRegionRecord {
  return {
    id: questionId,
    sessionId,
    questionId,
    origin: { ...origin },
    bounds: { ...origin, ...initialSize },
    reservedWidth: Math.max(COURSE_REGION_RESERVED_WIDTH, initialSize.width),
    createdAt,
  };
}

/** The reserved width participates in placement without drawing any boundary. */
export function courseRegionOccupiedRect(
  region: CourseRegionRecord,
): WhiteboardRect {
  const x = Math.min(region.origin.x, region.bounds.x);
  const y = Math.min(region.origin.y, region.bounds.y);
  const right = Math.max(
    region.bounds.x + region.bounds.width,
    region.origin.x + region.reservedWidth,
  );
  const bottom = Math.max(
    region.bounds.y + region.bounds.height,
    region.origin.y + COURSE_PENDING_FOOTPRINT_HEIGHT,
  );
  return {
    x,
    y,
    width: right - x,
    height: bottom - y,
  };
}

/**
 * A course's invisible occupied area may grow as later sections, controls, or
 * tasks arrive, but it must never shrink and make a later course overlap it.
 */
export function expandCourseRegionBounds(
  current: WhiteboardRect,
  measured: WhiteboardRect,
): WhiteboardRect {
  const x = Math.min(current.x, measured.x);
  const y = Math.min(current.y, measured.y);
  const right = Math.max(
    current.x + current.width,
    measured.x + measured.width,
  );
  const bottom = Math.max(
    current.y + current.height,
    measured.y + measured.height,
  );
  return { x, y, width: right - x, height: bottom - y };
}

export function loadCourseRegions(
  sessionId: string,
  storage: Storage = localStorage,
): CourseRegionRecord[] {
  try {
    const parsed = JSON.parse(
      storage.getItem(courseRegionsStorageKey(sessionId)) ?? "[]",
    );
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((value): value is CourseRegionRecord =>
        validCourseRegion(value, sessionId))
      .sort((left, right) => left.createdAt.localeCompare(right.createdAt));
  } catch {
    return [];
  }
}

export function saveCourseRegions(
  sessionId: string,
  regions: CourseRegionRecord[],
  storage: Storage = localStorage,
): void {
  try {
    storage.setItem(
      courseRegionsStorageKey(sessionId),
      JSON.stringify(regions.filter((region) =>
        validCourseRegion(region, sessionId))),
    );
  } catch {
    // In-memory layout remains usable when browser storage is unavailable.
  }
}
