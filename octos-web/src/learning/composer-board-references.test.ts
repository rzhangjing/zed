import { describe, expect, it } from "vitest";
import {
  INK_SELECTION_FORMAT,
  INK_SELECTION_FORMAT_VERSION,
  type InkSelectionSnapshot,
} from "octos-lesson-language/ink-runtime";
import {
  buildComposerBoardReferenceContext,
  type ComposerBoardReference,
} from "./composer-board-references";

function reference(id: string, targetId: string): ComposerBoardReference {
  const snapshot: InkSelectionSnapshot = {
    format: INK_SELECTION_FORMAT,
    format_version: INK_SELECTION_FORMAT_VERSION,
    source_id: `source-${id}`,
    document_id: "ink-1",
    document_version: 3,
    created_at: "2026-08-15T10:00:00.000Z",
    bounds: { x: 20, y: 30, width: 100, height: 60 },
    region: {
      kind: "rectangle",
      closed: true,
      points: [
        { x: 20, y: 30 },
        { x: 120, y: 30 },
        { x: 120, y: 90 },
        { x: 20, y: 90 },
      ],
    },
    component_ids: [`stroke:${id}`],
    checksum: { algorithm: "sha-256", value: "a".repeat(64) },
    svg: `<svg><path data-private-source="${id}" /></svg>`,
  };
  return {
    id,
    label: `选区 ${id}`,
    snapshot,
    contentKind: "math",
    contextImage: new File(["png"], `${id}.png`, { type: "image/png" }),
    boardContext: {
      boardId: "board-1",
      boardRevision: 12,
      targets: [{
        target_id: `${targetId}:fragment:formula`,
        node_id: targetId,
        element_id: `${targetId}:fragment:formula`,
        kind: "math-fragment",
        label: "x²+y²=k",
        world_bounds: { x: 0, y: 0, width: 120, height: 80 },
        overlap: 1,
        distance: 0,
        z_index: 1,
      }],
    },
  };
}

describe("composer board references", () => {
  it("sends only explicitly attached stable references with unique aliases", () => {
    const first = reference("one", "lesson-a:node:formula");
    const second = reference("two", "lesson-b:node:formula");
    const context = buildComposerBoardReferenceContext([
      { reference: first, mediaPath: "uploads/one.png" },
      { reference: second, mediaPath: "uploads/two.png" },
    ]);

    expect(context).toContain("request_source: explicit_board_follow_up");
    expect(context).toContain('"as":"board-ref-1-1"');
    expect(context).toContain('"as":"board-ref-2-1"');
    expect(context).toContain("lesson-a:node:formula:fragment:formula");
    expect(context).toContain("lesson-b:node:formula:fragment:formula");
    expect(context).toContain("uploads/one.png");
    expect(context).not.toContain(first.snapshot.svg);
    expect(context).not.toContain("world_bounds");
  });

  it("does not claim board context when the learner attached nothing", () => {
    expect(buildComposerBoardReferenceContext([])).toBe("");
  });
});
