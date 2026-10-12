import { describe, expect, it } from "vitest";
import { isCurrentInkMergeCompletion } from "./ink-replay";

describe("ink replay", () => {
  it("does not let an older merge complete a newer replay", () => {
    expect(isCurrentInkMergeCompletion(
      "lesson",
      "lesson:replay:1",
      "lesson:replay:1",
      "lesson:replay:2",
    )).toBe(false);
    expect(isCurrentInkMergeCompletion(
      "lesson:replay:1",
      "lesson:replay:2",
      "lesson:replay:1",
      "lesson:replay:2",
    )).toBe(true);
  });
});
