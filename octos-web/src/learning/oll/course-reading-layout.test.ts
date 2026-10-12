import { describe, expect, it } from "vitest";
import { computeBoardLayout } from "octos-lesson-language/web-runtime";

type BoardState = Parameters<typeof computeBoardLayout>[0];

function boardWithNodes(nodes: Record<string, unknown>): BoardState {
  return {
    board_id: "saved-board",
    revision: 8,
    nodes,
    groups: {},
    connections: {},
    variables: {},
    focus: [],
  } as unknown as BoardState;
}

describe("course reading layout", () => {
  it("reserves an interaction row below its visual without overlapping cards", () => {
    const state = boardWithNodes({
      plot: {
        id: "plot",
        kind: "plot",
        region_id: "course",
        content: {},
        placement: { relation: "new_region" },
      },
      formula: {
        id: "formula",
        kind: "math",
        region_id: "course",
        content: { latex: "e=\\lim(1+1/n)^n" },
        placement: { relation: "new_region" },
      },
      explanation: {
        id: "explanation",
        kind: "note",
        region_id: "course",
        content: { text: "自然对数的直觉含义" },
        placement: { relation: "below", anchor: "formula" },
      },
      conclusion: {
        id: "conclusion",
        kind: "math",
        region_id: "course",
        content: { latex: "e\\approx2.71828" },
        placement: { relation: "below", anchor: "explanation" },
      },
    });
    const options = {
      regions: {
        course: {
          x: 394,
          y: 120,
          reservedWidth: 886,
          flow: "reading" as const,
          attachments: [{
            id: "course:interaction:1",
            anchorNodeId: "plot",
            width: 718,
            height: 260,
            gap: 42,
          }],
        },
      },
    };
    const sizes = {
      plot: { width: 340, height: 230 },
      formula: { width: 620, height: 96 },
      explanation: { width: 420, height: 130 },
      conclusion: { width: 440, height: 96 },
    };
    const layout = computeBoardLayout(state, sizes, options);
    const interaction = layout.attachments["course:interaction:1"]!;

    expect(interaction.x).toBe(layout.nodes.plot!.x);
    expect(interaction.y).toBe(layout.nodes.plot!.y + layout.nodes.plot!.height + 42);
    for (const node of Object.values(layout.nodes)) {
      expect(
        interaction.x < node.x + node.width
        && interaction.x + interaction.width > node.x
        && interaction.y < node.y + node.height
        && interaction.y + interaction.height > node.y,
      ).toBe(false);
    }

    const withLaterNarrative = computeBoardLayout(boardWithNodes({
      ...state.nodes,
      later: {
        id: "later",
        kind: "note",
        region_id: "course",
        content: { text: "稍后出现的课程说明" },
        placement: { relation: "below", anchor: "conclusion" },
      },
    }), {
      ...sizes,
      later: { width: 420, height: 112 },
    }, options);
    expect(withLaterNarrative.attachments["course:interaction:1"])
      .toEqual(interaction);
  });

  it("reflows an existing visual and derivation chain into two stable lanes", () => {
    const board = boardWithNodes({
      circle: {
        id: "circle",
        kind: "geometry",
        region_id: "course-one",
        content: { title: "完整圆周" },
        placement: { relation: "new_region" },
      },
      arcFormula: {
        id: "arcFormula",
        kind: "math",
        region_id: "course-one",
        content: { latex: "l_1=2\\pi r/360=\\pi r/180" },
        placement: { relation: "new_region" },
      },
      circumference: {
        id: "circumference",
        kind: "math",
        region_id: "course-one",
        content: { latex: "C=2\\pi r" },
        placement: {
          relation: "below",
          anchor: "circle",
          align: "center",
        },
      },
      result: {
        id: "result",
        kind: "math",
        region_id: "course-one",
        content: { latex: "l=n\\pi r/180" },
        placement: {
          relation: "below",
          anchor: "circumference",
          align: "center",
        },
      },
    });
    const layout = computeBoardLayout(board, {
      circle: { width: 380, height: 300 },
      arcFormula: { width: 620, height: 96 },
      circumference: { width: 360, height: 96 },
      result: { width: 440, height: 96 },
    }, {
      regions: {
        "course-one": {
          x: 394,
          y: 90,
          reservedWidth: 886,
          flow: "reading",
        },
      },
    });

    expect(layout.nodes.circle).toEqual({
      x: 394,
      y: 120,
      width: 380,
      height: 300,
    });
    expect([
      layout.nodes.arcFormula?.x,
      layout.nodes.circumference?.x,
      layout.nodes.result?.x,
    ]).toEqual([828, 828, 828]);
    expect([
      layout.nodes.arcFormula?.y,
      layout.nodes.circumference?.y,
      layout.nodes.result?.y,
    ]).toEqual([120, 244, 368]);
    expect(layout.nodes.arcFormula?.width).toBe(452);
    expect(
      Math.max(...Object.values(layout.nodes).map((node) => node.x + node.width)),
    ).toBeLessThanOrEqual(394 + 886);
  });

  it("keeps independently persisted course regions separated after reflow", () => {
    const board = boardWithNodes({
      oldVisual: {
        id: "oldVisual",
        kind: "scene3d",
        region_id: "old-course",
        content: {},
        placement: { relation: "new_region" },
      },
      oldExplanation: {
        id: "oldExplanation",
        kind: "math",
        region_id: "old-course",
        content: { latex: "x" },
        placement: { relation: "below", anchor: "oldVisual" },
      },
      nextVisual: {
        id: "nextVisual",
        kind: "geometry",
        region_id: "next-course",
        content: {},
        placement: { relation: "new_region" },
      },
      nextExplanation: {
        id: "nextExplanation",
        kind: "text",
        region_id: "next-course",
        content: { text: "下一节课" },
        placement: { relation: "right_of", anchor: "nextVisual" },
      },
    });
    const layout = computeBoardLayout(board, {
      oldVisual: { width: 460, height: 360 },
      oldExplanation: { width: 680, height: 136 },
      nextVisual: { width: 380, height: 300 },
      nextExplanation: { width: 440, height: 106 },
    }, {
      regions: {
        "old-course": {
          x: 394,
          y: 90,
          reservedWidth: 886,
          flow: "reading",
          attachments: [{
            id: "old-controls",
            anchorNodeId: "oldVisual",
            width: 718,
            height: 260,
          }],
        },
        "next-course": {
          x: 1_754,
          y: 90,
          reservedWidth: 886,
          flow: "reading",
          attachments: [{
            id: "next-controls",
            anchorNodeId: "nextVisual",
            width: 360,
            height: 96,
          }],
        },
      },
    });

    const oldRight = Math.max(
      layout.nodes.oldVisual!.x + layout.nodes.oldVisual!.width,
      layout.nodes.oldExplanation!.x + layout.nodes.oldExplanation!.width,
    );
    const nextLeft = Math.min(
      layout.nodes.nextVisual!.x,
      layout.nodes.nextExplanation!.x,
    );
    expect(oldRight).toBeLessThanOrEqual(394 + 886);
    expect(nextLeft).toBe(1_754);
    expect(oldRight).toBeLessThan(nextLeft);
    expect(layout.attachments["old-controls"]!.x + 718)
      .toBeLessThanOrEqual(394 + 886);
    expect(layout.attachments["next-controls"]!.x).toBe(1_754);
  });

  it("preserves intentional overlays inside an interactive visual", () => {
    const board = boardWithNodes({
      visual: {
        id: "visual",
        kind: "geometry",
        region_id: "course",
        content: {},
        placement: { relation: "new_region" },
      },
      annotation: {
        id: "annotation",
        kind: "note",
        region_id: "course",
        content: { text: "圆心" },
        placement: { relation: "overlay", anchor: "visual" },
      },
    });
    const layout = computeBoardLayout(board, {
      visual: { width: 380, height: 300 },
      annotation: { width: 120, height: 72 },
    }, {
      regions: {
        course: {
          x: 394,
          y: 90,
          reservedWidth: 886,
          flow: "reading",
        },
      },
    });

    expect(layout.nodes.annotation).toEqual({
      x: 418,
      y: 144,
      width: 120,
      height: 72,
    });
  });

  it("preserves authored relationships within and around a visual group", () => {
    const board = {
      ...boardWithNodes({
        circle: {
          id: "circle",
          kind: "geometry",
          region_id: "course",
          content: {},
          placement: { relation: "new_region" },
        },
        sinePlot: {
          id: "sinePlot",
          kind: "plot",
          region_id: "course",
          content: {},
          placement: {
            relation: "right_of",
            anchor: "circle",
            gap: "normal",
          },
        },
        sideFormula: {
          id: "sideFormula",
          kind: "math",
          region_id: "course",
          content: { latex: "y=\\sin(\\theta)" },
          placement: {
            relation: "right_of",
            anchor: "visualPair",
            gap: "normal",
          },
        },
        lowerFormula: {
          id: "lowerFormula",
          kind: "math",
          region_id: "course",
          content: { latex: "y=\\sin x" },
          placement: {
            relation: "below",
            anchor: "visualPair",
            align: "center",
            gap: "normal",
          },
        },
      }),
      groups: {
        visualPair: {
          id: "visualPair",
          members: ["circle", "sinePlot"],
        },
      },
    } as BoardState;
    const layout = computeBoardLayout(board, {
      circle: { width: 380, height: 300 },
      sinePlot: { width: 340, height: 230 },
      sideFormula: { width: 360, height: 96 },
      lowerFormula: { width: 360, height: 96 },
    }, {
      regions: {
        course: {
          x: 394,
          y: 90,
          reservedWidth: 1_300,
          flow: "reading",
        },
      },
    });

    const circle = layout.nodes.circle!;
    const sinePlot = layout.nodes.sinePlot!;
    const visualPair = layout.groups.visualPair!;
    const sideFormula = layout.nodes.sideFormula!;
    const lowerFormula = layout.nodes.lowerFormula!;

    expect(sinePlot.x).toBe(circle.x + circle.width + 54);
    expect(sinePlot.y + sinePlot.height / 2)
      .toBe(circle.y + circle.height / 2);
    expect(sideFormula.x).toBe(visualPair.x + visualPair.width + 54);
    expect(lowerFormula.x + lowerFormula.width / 2)
      .toBe(visualPair.x + visualPair.width / 2);
    expect(lowerFormula.y).toBe(visualPair.y + visualPair.height + 54);
  });
});
