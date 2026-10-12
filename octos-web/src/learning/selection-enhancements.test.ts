import { describe, expect, it } from "vitest";
import {
  INK_SELECTION_FORMAT,
  INK_SELECTION_FORMAT_VERSION,
  inkSvgChecksum,
  type InkSelectionSnapshot,
} from "octos-lesson-language/ink-runtime";
import {
  addSelectionSource,
  buildSelectionClassificationActionArguments,
  buildSelectionEnhancementActionArguments,
  buildSelectionEnhancementTurnContext,
  loadSelectionEnhancementState,
  parseSelectionClassificationMetadata,
  removeSelectionSources,
  saveSelectionEnhancementState,
  selectionArtifactMatchesSource,
  selectionArtifactTargetsExist,
  selectionBoardContextTargetsExist,
  validateSelectionEnhancementArtifact,
} from "./selection-enhancements";

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.values.set(key, value); }
}

async function source(): Promise<InkSelectionSnapshot> {
  const svg = '<svg data-oll-ink-selection="1"><path d="M0 0L10 10"/></svg>';
  const region = {
    kind: "rectangle" as const,
    closed: true,
    points: [
      { x: 20, y: 30 },
      { x: 120, y: 30 },
      { x: 120, y: 90 },
      { x: 20, y: 90 },
    ],
  };
  const componentIds = ["stroke:source-1"];
  return {
    format: INK_SELECTION_FORMAT,
    format_version: INK_SELECTION_FORMAT_VERSION,
    source_id: "source-1",
    document_id: "ink-1",
    document_version: 2,
    created_at: "2026-08-14T10:00:00.000Z",
    bounds: { x: 20, y: 30, width: 100, height: 60 },
    region,
    component_ids: componentIds,
    checksum: {
      algorithm: "sha-256",
      value: await inkSvgChecksum(JSON.stringify({
        svg,
        region,
        component_ids: componentIds,
      })),
    },
    svg,
  };
}

describe("selection enhancement persistence", () => {
  it("removes erased selection sources and hides every linked result", async () => {
    const selection = await source();
    const state = {
      profile: "octos.selection-enhancement-state" as const,
      version: "0.1" as const,
      session_id: "selection-source-removal",
      sources: [selection],
      hidden_enhancement_turn_ids: ["already-hidden"],
    };
    expect(removeSelectionSources(
      state,
      [selection.source_id],
      ["explanation-turn", "plot-turn"],
    )).toEqual({
      ...state,
      sources: [],
      hidden_enhancement_turn_ids: [
        "already-hidden",
        "explanation-turn",
        "plot-turn",
      ],
    });
  });

  it("builds a bounded selection-classification action and validates its metadata", async () => {
    const selection = await source();
    const argumentsValue = buildSelectionClassificationActionArguments({
      turnId: "classification-1",
      mediaPath: "turn_media/selection.png",
      source: selection,
      boardContext: {
        boardId: "board-1",
        boardRevision: 3,
        targets: [],
      },
    });
    expect(argumentsValue).toEqual({
      paths: ["turn_media/selection.png"],
      turn_id: "classification-1",
      source: {
        source_id: selection.source_id,
        document_id: selection.document_id,
        document_version: selection.document_version,
        bounds: selection.bounds,
        checksum: selection.checksum,
      },
      board: { board_id: "board-1", revision: 3, targets: [] },
    });
    expect(parseSelectionClassificationMetadata({
      selection_classification: {
        kind: "math",
        content: " y=x^2 ",
        confidence: "high",
      },
    })).toEqual({ kind: "math", content: "y=x^2", confidence: "high" });
    expect(() => parseSelectionClassificationMetadata({
      selection_classification: {
        kind: "plot-tool",
        content: "y=x^2",
        confidence: "high",
      },
    })).toThrow(/类型无效/);
  });

  it("passes exact selection identity to the direct skill action", async () => {
    const selection = await source();
    selection.bounds.x = 12.6666666667;
    const argumentsValue = buildSelectionEnhancementActionArguments({
      sessionId: "session-1",
      turnId: "turn-1",
      mediaPath: "turn_media/selection.png",
      source: selection,
      contentKind: "math",
      recognizedContent: "y = sin(x)",
      recognitionConfidence: "high",
      learnerRequest: "解释这部分",
      toolId: "explain",
      boardContext: {
        boardId: "board-1",
        boardRevision: 8,
        targets: [{
          target_id: "plot:curve",
          node_id: "plot",
          element_id: "curve",
          kind: "plot-curve",
          label: "y = sin(x)",
          value: { expression: "sin(x)" },
          world_bounds: { x: 200, y: 100, width: 300, height: 180 },
          overlap: .75,
          distance: 0,
          z_index: 2,
        }],
      },
    });
    expect(argumentsValue).toMatchObject({
      paths: ["turn_media/selection.png"],
      recognized_content: "y = sin(x)",
      recognition_confidence: "high",
      source: { bounds: { x: 12.6666666667 } },
      board: {
        board_id: "board-1",
        revision: 8,
        targets: [{ value_json: '{"expression":"sin(x)"}' }],
      },
    });
  });

  it("restores a checksummed source snapshot without editor component IDs", async () => {
    const storage = new MemoryStorage();
    const selection = await source();
    const state = addSelectionSource({
      profile: "octos.selection-enhancement-state",
      version: "0.1",
      session_id: "session-1",
      sources: [],
      hidden_enhancement_turn_ids: [],
    }, selection);
    saveSelectionEnhancementState(state, storage);

    await expect(loadSelectionEnhancementState("session-1", storage))
      .resolves.toEqual(state);
  });

  it("drops a corrupted local snapshot but leaves validation deterministic", async () => {
    const storage = new MemoryStorage();
    const selection = await source();
    selection.svg = selection.svg.replace("10 10", "9 9");
    saveSelectionEnhancementState({
      profile: "octos.selection-enhancement-state",
      version: "0.1",
      session_id: "session-1",
      sources: [selection],
      hidden_enhancement_turn_ids: [],
    }, storage);

    await expect(loadSelectionEnhancementState("session-1", storage))
      .resolves.toMatchObject({ sources: [] });
  });

  it("accepts source-linked plot and explicit unsupported artifacts", () => {
    const artifact = validateSelectionEnhancementArtifact({
      profile: "octos.selection-enhancement",
      version: "0.1",
      turn_id: "turn-1",
      created_at: "2026-08-14T10:00:01.000Z",
      source: {
        source_id: "source-1",
        document_id: "ink-1",
        document_version: 2,
        bounds: { x: 20, y: 30, width: 100, height: 60 },
        checksum: { algorithm: "sha-256", value: "a".repeat(64) },
      },
      interpretation: {
        kind: "math",
        content: "y=sin(x)",
        confidence: "high",
      },
      response: {
        kind: "plot",
        title: "正弦函数图像",
        text: "这是所选函数对应的图像。",
        expression: "sin(x)",
        x_range: { min: -6.28, max: 6.28 },
        y_range: { min: -1.2, max: 1.2 },
      },
    });
    expect(artifact.response.kind).toBe("plot");

    const unsupported = validateSelectionEnhancementArtifact({
      ...artifact,
      turn_id: "turn-unsupported",
      response: {
        kind: "unsupported",
        title: "暂时无法绘制",
        text: "这个表达式包含四个独立变量，不能直接显示为三维图像。",
        reason_code: "unsupported_variables",
        alternatives: ["固定其中一个变量后绘制三维切片"],
      },
    });
    expect(unsupported.response.kind).toBe("unsupported");

    const scene3d = validateSelectionEnhancementArtifact({
      ...artifact,
      turn_id: "turn-scene3d",
      response: {
        kind: "scene3d",
        title: "四次曲面",
        text: "拖动旋转查看这个三维曲面。",
        content: {
          title: "四次曲面",
          fallback: "x⁴+y⁴+z⁴=1",
          axes: true,
          camera: { yaw: .65, pitch: .45, zoom: 1 },
          objects: [{
            as: "selected-function",
            kind: "implicit_surface",
            expression: "x^4+y^4+z^4-1",
            level: 0,
            x_range: { min: -1.2, max: 1.2 },
            y_range: { min: -1.2, max: 1.2 },
            z_range: { min: -1.2, max: 1.2 },
          }],
        },
      },
    });
    expect(scene3d.response.kind).toBe("scene3d");

    const matchingSource = {
      format: INK_SELECTION_FORMAT,
      format_version: INK_SELECTION_FORMAT_VERSION,
      source_id: artifact.source.source_id,
      document_id: artifact.source.document_id,
      document_version: artifact.source.document_version,
      created_at: "2026-08-14T10:00:00.000Z",
      bounds: artifact.source.bounds,
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
      component_ids: ["stroke:matching-source"],
      checksum: artifact.source.checksum,
      svg: "<svg />",
    } satisfies InkSelectionSnapshot;
    expect(selectionArtifactMatchesSource(artifact, matchingSource)).toBe(true);
    expect(selectionArtifactMatchesSource({
      ...artifact,
      source: {
        ...artifact.source,
        bounds: {
          ...artifact.source.bounds,
          x: artifact.source.bounds.x - 1e-12,
          height: artifact.source.bounds.height + 1e-12,
        },
      },
    }, matchingSource)).toBe(true);
    expect(selectionArtifactMatchesSource(artifact, {
      ...matchingSource,
      checksum: { algorithm: "sha-256", value: "b".repeat(64) },
    })).toBe(false);
    expect(selectionArtifactMatchesSource(artifact, {
      ...matchingSource,
      document_version: matchingSource.document_version + 1,
    })).toBe(false);

    expect(() => validateSelectionEnhancementArtifact({
      ...artifact,
      source: { ...artifact.source, source_id: "" },
    })).toThrow(/来源或说明/);
  });

  it("routes a turn to selection enhancement without embedding the source SVG", async () => {
    const selection = await source();
    const context = buildSelectionEnhancementTurnContext({
      sessionId: "session-1",
      turnId: "turn-selection-1",
      mediaPath: "uploads/selection.png",
      source: selection,
      contentKind: "math",
      learnerRequest: "请解释这条公式",
      lessonTitle: "二次函数",
    });

    expect(context).toContain("selection_artifact_tool: oll_enhance_selection");
    expect(context).toContain("lesson_artifact_policy: forbidden");
    expect(context).toContain("preserve_source_ink: required");
    expect(context).toContain(selection.checksum.value);
    expect(context).not.toContain(selection.svg);
  });

  it("records an explicit local tool and exact board targets without serializing the board", async () => {
    const selection = await source();
    const context = buildSelectionEnhancementTurnContext({
      sessionId: "session-1",
      turnId: "turn-selection-2",
      mediaPath: "uploads/selection-context.png",
      source: selection,
      contentKind: "math",
      toolId: "generate-plot",
      boardContext: {
        boardId: "board-1",
        boardRevision: 8,
        targets: [{
          target_id: "node-1:curve:sin",
          node_id: "node-1",
          element_id: "sin",
          kind: "plot-curve",
          label: "y = sin x",
          value: { expression: "sin(x)" },
          world_bounds: { x: 0, y: 0, width: 100, height: 60 },
          overlap: 0.9,
          distance: 0,
          z_index: 2,
        }],
      },
    });

    expect(context).toContain("selection_tool_id: generate-plot");
    expect(context).toContain("board_id: board-1");
    expect(context).toContain("node-1:curve:sin");
    expect(context).toContain('\\"expression\\":\\"sin(x)\\"');
    expect(context).toContain('"overlap":0.9');
    expect(context).not.toContain(selection.svg);
    expect(context).toContain("world_bounds");
  });

  it("accepts a v0.2 source-linked artifact and invalidates it when its exact board target disappears", () => {
    const artifact = validateSelectionEnhancementArtifact({
      profile: "octos.selection-enhancement",
      version: "0.2",
      turn_id: "turn-2",
      created_at: "2026-08-14T10:00:01.000Z",
      source: {
        source_id: "source-1",
        document_id: "ink-1",
        document_version: 2,
        bounds: { x: 20, y: 30, width: 100, height: 60 },
        checksum: { algorithm: "sha-256", value: "a".repeat(64) },
      },
      board: {
        board_id: "board-1",
        revision: 8,
        targets: [{
          target_id: "node-1:fragment:formula",
          node_id: "node-1",
          element_id: "node-1:fragment:formula",
          kind: "math-fragment",
          label: "x²+y²=k",
          value: { latex: "x^2+y^2=k" },
          world_bounds: { x: 100, y: 120, width: 160, height: 60 },
          overlap: 0.82,
          distance: 0,
          z_index: 3,
        }],
      },
      tool_id: "explain",
      interpretation: {
        kind: "math",
        content: "x²+y²=k",
        confidence: "high",
      },
      response: {
        kind: "explanation",
        title: "截线为什么是圆",
        text: "固定 k 后，半径为根号 k。",
      },
    });
    const matchingBoard = {
      board_id: "board-1",
      revision: 8,
      nodes: {
        "node-1": {
          id: "node-1",
          kind: "math",
          content: { fragments: [{ id: "node-1:fragment:formula", latex: "x^2+y^2=k" }] },
        },
      },
    };

    expect(selectionArtifactTargetsExist(artifact, matchingBoard as never)).toBe(true);
    expect(selectionArtifactTargetsExist(artifact, {
      ...matchingBoard,
      revision: 9,
    } as never)).toBe(false);
    expect(selectionBoardContextTargetsExist({
      boardId: "board-1",
      boardRevision: 8,
      targets: artifact.board!.targets as never,
    }, matchingBoard as never)).toBe(true);
    expect(selectionBoardContextTargetsExist({
      boardId: "board-1",
      boardRevision: 7,
      targets: artifact.board!.targets as never,
    }, matchingBoard as never)).toBe(false);
    expect(selectionArtifactTargetsExist(artifact, {
      ...matchingBoard,
      nodes: { "node-1": { ...matchingBoard.nodes["node-1"], content: { fragments: [] } } },
    } as never)).toBe(false);
    expect(() => validateSelectionEnhancementArtifact({
      ...artifact,
      tool_id: "model-invented-tool",
    })).toThrow(/来源或说明/);

    const tableArtifact = validateSelectionEnhancementArtifact({
      ...artifact,
      board: {
        board_id: "board-1",
        revision: 9,
        targets: [{
          target_id: "table-1:table:row:0:column:1",
          node_id: "table-1",
          element_id: "table-1:table:row:0:column:1",
          kind: "table-cell",
          label: "第二列",
          value: "42",
          world_bounds: { x: 40, y: 50, width: 80, height: 32 },
          overlap: 1,
          distance: 0,
          z_index: 2,
        }],
      },
    });
    const tableBoard = {
      board_id: "board-1",
      revision: 9,
      nodes: {
        "table-1": {
          id: "table-1",
          kind: "table",
          content: { columns: ["a", "b"], rows: [[1, 42]] },
        },
      },
    };
    expect(selectionArtifactTargetsExist(tableArtifact, tableBoard as never)).toBe(true);
    expect(selectionArtifactTargetsExist(tableArtifact, {
      ...tableBoard,
      nodes: {
        "table-1": {
          ...tableBoard.nodes["table-1"],
          content: { columns: ["a"], rows: [[1]] },
        },
      },
    } as never)).toBe(false);
  });
});
