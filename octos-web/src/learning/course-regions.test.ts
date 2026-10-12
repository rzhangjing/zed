import { describe, expect, it } from "vitest";
import {
  COURSE_REGION_RESERVED_WIDTH,
  courseRegionOccupiedRect,
  createCourseRegion,
  expandCourseRegionBounds,
  loadCourseRegions,
  saveCourseRegions,
} from "./course-regions";

describe("course region records", () => {
  it("only expands course bounds as later content arrives", () => {
    expect(expandCourseRegionBounds(
      { x: 100, y: 90, width: 1_180, height: 500 },
      { x: 120, y: 110, width: 600, height: 240 },
    )).toEqual({ x: 100, y: 90, width: 1_180, height: 500 });
    expect(expandCourseRegionBounds(
      { x: 100, y: 90, width: 1_180, height: 500 },
      { x: 80, y: 70, width: 1_400, height: 620 },
    )).toEqual({ x: 80, y: 70, width: 1_400, height: 620 });
  });

  it("reserves future lesson width without creating a visible object", () => {
    const region = createCourseRegion(
      "session",
      "turn",
      { x: 100, y: 90 },
      { width: 654, height: 220 },
      "2026-08-19T00:00:00.000Z",
    );
    expect(courseRegionOccupiedRect(region)).toEqual({
      x: 100,
      y: 90,
      width: COURSE_REGION_RESERVED_WIDTH,
      height: 220,
    });
  });

  it("persists only valid records for the current whiteboard", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    } as Storage;
    const region = createCourseRegion(
      "session",
      "turn",
      { x: 120, y: 80 },
      { width: 654, height: 220 },
      "2026-08-19T00:00:00.000Z",
    );
    saveCourseRegions("session", [region], storage);
    expect(loadCourseRegions("session", storage)).toEqual([region]);
    expect(loadCourseRegions("another-session", storage)).toEqual([]);
  });
});
