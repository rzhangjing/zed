import { describe, expect, it } from "vitest";
import { availableSelectionTools, selectionToolRegistry } from "./selection-tools";

describe("selection tool registry", () => {
  it("is finite, auditable, and never claims it can change the learner source", () => {
    expect(selectionToolRegistry.map((tool) => tool.id)).toEqual([
      "explain",
      "check-and-suggest",
      "generate-plot",
      "teach-lesson",
    ]);
    expect(selectionToolRegistry.filter((tool) =>
      tool.action === "local-enhancement",
    ).every((tool) => tool.changesSource === false)).toBe(true);
  });

  it("offers plotting only for recognized math or an explicitly selected math fragment", () => {
    expect(availableSelectionTools("unknown").map((tool) => tool.id))
      .not.toContain("generate-plot");
    expect(availableSelectionTools("math").map((tool) => tool.id))
      .toContain("generate-plot");
    expect(availableSelectionTools("unknown", ["math-fragment"]).map((tool) => tool.id))
      .toContain("generate-plot");
    expect(availableSelectionTools("geometry", ["plot", "plot-point"]).map((tool) => tool.id))
      .not.toContain("generate-plot");
    expect(availableSelectionTools("unknown", ["geometry-arc"]).map((tool) => tool.id))
      .not.toContain("generate-plot");
    expect(selectionToolRegistry.find((tool) => tool.id === "generate-plot"))
      .toMatchObject({
        requestContentKind: "math",
      });
  });
});
