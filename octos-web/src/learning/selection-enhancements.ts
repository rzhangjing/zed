import { buildApiHeaders } from "@/api/client";
import { buildFileUrl } from "@/api/files";
import type { SessionFileInfo } from "@/api/sessions";
import type { Thread } from "@/store/thread-store";
import {
  assertInkSelectionIntegrity,
  validateInkSelectionSnapshot,
  type InkSelectionBounds,
  type InkSelectionSnapshot,
} from "octos-lesson-language/ink-runtime";
import type {
  BoardTargetCandidate,
  MountedInfiniteBoard,
} from "octos-lesson-language/web-runtime";
import type { SemanticBoardState } from "octos-lesson-language";
import type { SelectionToolId } from "./selection-tools";

const ARTIFACT_SUFFIX = ".octos-selection-enhancement.json";
const STATE_PROFILE = "octos.selection-enhancement-state";
const STATE_VERSION = "0.1";

export type SelectionContentKind =
  | "text"
  | "math"
  | "geometry"
  | "data"
  | "unknown";

export interface SelectionClassification {
  kind: SelectionContentKind;
  content: string;
  confidence: "high" | "medium" | "low";
}

export interface SelectionEnhancementSourceRef {
  source_id: string;
  document_id: string;
  document_version: number;
  bounds: InkSelectionBounds;
  checksum: { algorithm: "sha-256"; value: string };
}

export interface SelectionEnhancementBoardTargetRef {
  target_id: string;
  node_id: string;
  element_id?: string;
  kind: string;
  label?: string;
  value?: unknown;
  world_bounds: InkSelectionBounds;
  overlap: number;
  distance: number;
  z_index: number;
}

export interface SelectionEnhancementBoardRef {
  board_id: string;
  revision: number;
  targets: SelectionEnhancementBoardTargetRef[];
}

export interface SelectionEnhancementArtifact {
  profile: "octos.selection-enhancement";
  version: "0.1" | "0.2";
  turn_id: string;
  created_at: string;
  source: SelectionEnhancementSourceRef;
  board?: SelectionEnhancementBoardRef;
  tool_id?: SelectionToolId;
  interpretation: {
    kind: SelectionContentKind;
    content: string;
    confidence: "high" | "medium" | "low";
  };
  response:
    | {
        kind: "explanation";
        title: string;
        text: string;
        items?: string[];
      }
    | {
        kind: "plot";
        title: string;
        text: string;
        expression: string;
        plot_kind?: "explicit" | "implicit";
        level?: number;
        samples?: number;
        x_range: { min: number; max: number };
        y_range: { min: number; max: number };
      }
    | {
        kind: "scene3d";
        title: string;
        text: string;
        content: {
          title: string;
          fallback: string;
          axes: boolean;
          camera: { yaw: number; pitch: number; zoom: number };
          objects: Array<Record<string, unknown>>;
        };
      }
    | {
        kind: "unsupported";
        title: string;
        text: string;
        reason_code:
          | "unreadable_expression"
          | "unsupported_variables"
          | "unsupported_representation"
          | "unsafe_complexity";
        alternatives?: string[];
      };
}

export interface SelectionEnhancementArtifactRef {
  id: string;
  filename: string;
  path: string;
  turnId: string;
}

export interface SelectionEnhancementState {
  profile: typeof STATE_PROFILE;
  version: typeof STATE_VERSION;
  session_id: string;
  sources: InkSelectionSnapshot[];
  hidden_enhancement_turn_ids: string[];
}

export interface SelectionEnhancementTurnContext {
  sessionId: string;
  turnId: string;
  mediaPath: string;
  source: InkSelectionSnapshot;
  contentKind: SelectionContentKind;
  recognizedContent?: string;
  recognitionConfidence?: "high" | "medium" | "low";
  learnerRequest?: string;
  lessonTitle?: string;
  boardSummary?: string;
  boardContext?: SelectionBoardContext;
  toolId?: SelectionToolId;
}

export interface SelectionClassificationTurnContext {
  turnId: string;
  mediaPath: string;
  source: InkSelectionSnapshot;
  boardContext: SelectionBoardContext;
}

function selectionSourceArgument(source: InkSelectionSnapshot) {
  return {
    source_id: source.source_id,
    document_id: source.document_id,
    document_version: source.document_version,
    bounds: { ...source.bounds },
    checksum: { ...source.checksum },
  };
}

function selectionBoardArgument(context: SelectionBoardContext) {
  return {
    board_id: context.boardId,
    revision: context.boardRevision,
    targets: context.targets.map((target) => {
      const valueJson = compactTargetValue(target.value);
      return {
        target_id: target.target_id,
        node_id: target.node_id,
        ...(target.element_id ? { element_id: target.element_id } : {}),
        kind: target.kind,
        ...(target.label ? { label: target.label } : {}),
        ...(valueJson ? { value_json: valueJson } : {}),
        world_bounds: { ...target.world_bounds },
        overlap: target.overlap,
        distance: target.distance,
        z_index: target.z_index,
      };
    }),
  };
}

export function buildSelectionClassificationActionArguments(
  context: SelectionClassificationTurnContext,
): Record<string, unknown> {
  return {
    paths: [context.mediaPath],
    turn_id: context.turnId,
    source: selectionSourceArgument(context.source),
    board: selectionBoardArgument(context.boardContext),
  };
}

export function parseSelectionClassificationMetadata(
  value: unknown,
): SelectionClassification {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("选区识别结果无效");
  }
  const classification = (value as Record<string, unknown>).selection_classification;
  if (!classification || typeof classification !== "object" || Array.isArray(classification)) {
    throw new Error("选区识别结果缺少分类信息");
  }
  const result = classification as Record<string, unknown>;
  if (
    result.kind !== "text"
    && result.kind !== "math"
    && result.kind !== "geometry"
    && result.kind !== "data"
    && result.kind !== "unknown"
  ) {
    throw new Error("选区识别类型无效");
  }
  if (
    result.confidence !== "high"
    && result.confidence !== "medium"
    && result.confidence !== "low"
  ) {
    throw new Error("选区识别可信度无效");
  }
  if (typeof result.content !== "string") {
    throw new Error("选区识别内容无效");
  }
  return {
    kind: result.kind,
    content: result.content.trim(),
    confidence: result.confidence,
  };
}

export function buildSelectionEnhancementActionArguments(
  context: SelectionEnhancementTurnContext,
): Record<string, unknown> {
  return {
    paths: [context.mediaPath],
    turn_id: context.turnId,
    learner_request: context.learnerRequest?.trim() || "请解释我选中的内容",
    source: selectionSourceArgument(context.source),
    content_hint: context.contentKind,
    ...(context.recognizedContent?.trim()
      ? { recognized_content: context.recognizedContent.trim() }
      : {}),
    ...(context.recognitionConfidence
      ? { recognition_confidence: context.recognitionConfidence }
      : {}),
    tool_id: context.toolId ?? "custom-question",
    board: selectionBoardArgument(context.boardContext ?? {
      boardId: context.sessionId,
      boardRevision: 0,
      targets: [],
    }),
    ...(context.lessonTitle?.trim() ? { lesson_title: context.lessonTitle.trim() } : {}),
    ...(context.boardSummary?.trim() ? { board_summary: context.boardSummary.trim() } : {}),
  };
}

export interface SelectionBoardContext {
  boardId: string;
  boardRevision: number;
  targets: BoardTargetCandidate[];
}

function contextLine(name: string, value: string | number): string {
  return `${name}: ${String(value).replace(/[\r\n]+/g, " ")}`;
}

function compactTargetValue(value: unknown): string | undefined {
  if (value === undefined) return undefined;
  try {
    return JSON.stringify(value)?.slice(0, 2_000);
  } catch {
    return undefined;
  }
}

/** Explicitly routes one turn to the selection enhancer instead of asking the
 * lesson generator to replace the current board. */
export function buildSelectionEnhancementTurnContext(
  context: SelectionEnhancementTurnContext,
): string {
  const { bounds } = context.source;
  const lines = [
    "[[LEARNING_SELECTION]]",
    "lesson_artifact_policy: forbidden",
    "selection_artifact_tool: oll_enhance_selection",
    "selection_artifact_policy: tool_only",
    contextLine("session_id", context.sessionId),
    contextLine("turn_id", context.turnId),
    contextLine("selection_media", context.mediaPath),
    contextLine("source_id", context.source.source_id),
    contextLine("source_document_id", context.source.document_id),
    contextLine("source_document_version", context.source.document_version),
    contextLine("source_bounds", [bounds.x, bounds.y, bounds.width, bounds.height].join(",")),
    contextLine("source_checksum", context.source.checksum.value),
    contextLine("content_hint", context.contentKind),
  ];
  if (context.toolId) lines.push(contextLine("selection_tool_id", context.toolId));
  if (context.learnerRequest?.trim()) {
    lines.push(contextLine("learner_request", context.learnerRequest.trim()));
  }
  if (context.lessonTitle?.trim()) {
    lines.push(contextLine("lesson_title", context.lessonTitle.trim()));
  }
  if (context.boardSummary?.trim()) {
    lines.push(contextLine("board_summary", context.boardSummary.trim()));
  }
  if (context.boardContext) {
    lines.push(
      contextLine("board_id", context.boardContext.boardId),
      contextLine("board_revision", context.boardContext.boardRevision),
      contextLine(
        "board_targets",
        JSON.stringify(context.boardContext.targets.map((target) => {
          const valueJson = compactTargetValue(target.value);
          return {
            target_id: target.target_id,
            node_id: target.node_id,
            element_id: target.element_id,
            kind: target.kind,
            label: target.label,
            ...(valueJson ? { value_json: valueJson } : {}),
            world_bounds: target.world_bounds,
            overlap: target.overlap,
            distance: target.distance,
            z_index: target.z_index,
          };
        })),
      ),
    );
  }
  lines.push("preserve_source_ink: required", "[[/LEARNING_SELECTION]]");
  return lines.join("\n");
}

/** Rasterize only the immutable selection snapshot. The rest of the board is
 * deliberately unavailable to this helper. */
export async function selectionSnapshotToPngFile(
  snapshot: InkSelectionSnapshot,
): Promise<File> {
  const source = validateInkSelectionSnapshot(snapshot);
  await assertInkSelectionIntegrity(source);
  const scale = Math.min(2, 1600 / Math.max(source.bounds.width, source.bounds.height));
  const width = Math.max(1, Math.ceil(source.bounds.width * scale));
  const height = Math.max(1, Math.ceil(source.bounds.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("当前浏览器无法生成选区图片");
  context.fillStyle = "#fbfaf5";
  context.fillRect(0, 0, width, height);
  const image = new Image();
  const blob = new Blob([source.svg], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  try {
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("选区图片解析失败"));
      image.src = url;
    });
    context.drawImage(image, 0, 0, width, height);
  } finally {
    URL.revokeObjectURL(url);
  }
  const png = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (value) => value ? resolve(value) : reject(new Error("选区图片生成失败")),
      "image/png",
    );
  });
  return new File([png], `${source.source_id}.png`, { type: "image/png" });
}

const capturedStyleProperties = [
  "background", "background-color", "border", "border-radius", "box-shadow",
  "color", "display", "font", "font-family", "font-size", "font-style",
  "font-weight", "height", "letter-spacing", "line-height", "margin", "opacity",
  "padding", "position", "text-align", "text-decoration", "text-transform",
  "transform", "transform-origin", "white-space", "width",
] as const;

function inlineComputedStyles(source: Element, clone: Element): void {
  const style = getComputedStyle(source);
  const targetStyle = (clone as HTMLElement | SVGElement).style;
  for (const property of capturedStyleProperties) {
    targetStyle.setProperty(property, style.getPropertyValue(property));
  }
  if (source instanceof SVGElement && clone instanceof SVGElement) {
    for (const property of ["fill", "stroke", "stroke-width", "stroke-dasharray"]) {
      clone.style.setProperty(property, style.getPropertyValue(property));
    }
  }
  const sourceChildren = Array.from(source.children);
  const cloneChildren = Array.from(clone.children);
  sourceChildren.forEach((child, index) => {
    const clonedChild = cloneChildren[index];
    if (clonedChild) inlineComputedStyles(child, clonedChild);
  });
}

function boardNodeElement(
  mounted: MountedInfiniteBoard,
  nodeId: string,
): HTMLElement | undefined {
  return Array.from(
    mounted.elements.nodes.querySelectorAll<HTMLElement>(".board-node"),
  ).find((element) => element.dataset.id === nodeId);
}

function unionBounds(
  bounds: InkSelectionBounds[],
): InkSelectionBounds {
  const left = Math.min(...bounds.map((value) => value.x));
  const top = Math.min(...bounds.map((value) => value.y));
  const right = Math.max(...bounds.map((value) => value.x + value.width));
  const bottom = Math.max(...bounds.map((value) => value.y + value.height));
  const padding = 24;
  return {
    x: left - padding,
    y: top - padding,
    width: right - left + padding * 2,
    height: bottom - top + padding * 2,
  };
}

async function foreignObjectToPngFile(
  svg: string,
  width: number,
  height: number,
  filename: string,
): Promise<File> {
  const scale = Math.min(2, 1800 / Math.max(width, height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.ceil(width * scale));
  canvas.height = Math.max(1, Math.ceil(height * scale));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("当前浏览器无法生成选区图片");
  const image = new Image();
  const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  try {
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("局部白板图片解析失败"));
      image.src = url;
    });
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
  } finally {
    URL.revokeObjectURL(url);
  }
  const png = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (value) => value ? resolve(value) : reject(new Error("局部白板图片生成失败")),
      "image/png",
    );
  });
  return new File([png], filename, { type: "image/png" });
}

/** Builds the image that the selection enhancer sees. It is deliberately
 * invoked only after a learner asks about a selection. Normal lesson render
 * and generation never traverse or rasterize the board. */
export async function selectionContextToPngFile(
  snapshot: InkSelectionSnapshot,
  mounted: MountedInfiniteBoard,
  targets: BoardTargetCandidate[],
): Promise<File> {
  const source = validateInkSelectionSnapshot(snapshot);
  await assertInkSelectionIntegrity(source);
  if (targets.length === 0) return selectionSnapshotToPngFile(source);

  const uniqueTargets = [...new Map(
    targets.map((target) => [target.target_id, target]),
  ).values()];
  const crop = unionBounds([
    source.bounds,
    ...uniqueTargets.map((target) => target.world_bounds),
  ]);
  const document = mounted.elements.viewport.ownerDocument;
  const content = document.createElement("div");
  Object.assign(content.style, {
    position: "relative",
    width: `${crop.width}px`,
    height: `${crop.height}px`,
    overflow: "hidden",
    background: "#fbfaf5",
    color: "#202b2a",
  });

  const nodeIds = [...new Set(uniqueTargets.map((target) => target.node_id))];
  for (const nodeId of nodeIds) {
    const sourceNode = boardNodeElement(mounted, nodeId);
    const target = uniqueTargets.find((candidate) => candidate.node_id === nodeId);
    if (!sourceNode || !target) continue;
    const clone = sourceNode.cloneNode(true) as HTMLElement;
    inlineComputedStyles(sourceNode, clone);
    const nodeBounds = uniqueTargets.find(
      (candidate) => candidate.node_id === nodeId && candidate.target_id === nodeId,
    )?.world_bounds ?? {
      x: Number.parseFloat(sourceNode.style.left),
      y: Number.parseFloat(sourceNode.style.top),
      width: Number.parseFloat(sourceNode.style.width),
      height: Number.parseFloat(sourceNode.style.height),
    };
    Object.assign(clone.style, {
      position: "absolute",
      left: `${nodeBounds.x - crop.x}px`,
      top: `${nodeBounds.y - crop.y}px`,
      width: `${nodeBounds.width}px`,
      height: `${nodeBounds.height}px`,
      margin: "0",
      transform: "none",
    });
    content.append(clone);
  }

  const inkHost = document.createElement("div");
  Object.assign(inkHost.style, {
    position: "absolute",
    left: `${source.bounds.x - crop.x}px`,
    top: `${source.bounds.y - crop.y}px`,
    width: `${source.bounds.width}px`,
    height: `${source.bounds.height}px`,
  });
  inkHost.innerHTML = source.svg;
  const inkSvg = inkHost.querySelector("svg");
  if (inkSvg) {
    inkSvg.setAttribute("width", String(source.bounds.width));
    inkSvg.setAttribute("height", String(source.bounds.height));
  }
  content.append(inkHost);

  const serialized = new XMLSerializer().serializeToString(content);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${crop.width}" height="${crop.height}" viewBox="0 0 ${crop.width} ${crop.height}"><foreignObject width="100%" height="100%"><div xmlns="http://www.w3.org/1999/xhtml">${serialized}</div></foreignObject></svg>`;
  try {
    return await foreignObjectToPngFile(
      svg,
      crop.width,
      crop.height,
      `${source.source_id}-context.png`,
    );
  } catch {
    return selectionSnapshotToPngFile(source);
  }
}

function finiteRange(value: unknown): value is { min: number; max: number } {
  if (!value || typeof value !== "object") return false;
  const range = value as { min?: unknown; max?: unknown };
  return typeof range.min === "number"
    && Number.isFinite(range.min)
    && typeof range.max === "number"
    && Number.isFinite(range.max)
    && range.max > range.min;
}

function validSelectionScene3d(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  const content = value as Record<string, unknown>;
  const camera = content.camera as Record<string, unknown> | undefined;
  if (
    typeof content.title !== "string"
    || !content.title.trim()
    || typeof content.fallback !== "string"
    || !content.fallback.trim()
    || typeof content.axes !== "boolean"
    || !camera
    || ![camera.yaw, camera.pitch, camera.zoom].every((number) =>
      typeof number === "number" && Number.isFinite(number)
    )
    || !Array.isArray(content.objects)
    || content.objects.length !== 1
  ) return false;
  const object = content.objects[0];
  if (!object || typeof object !== "object") return false;
  const surface = object as Record<string, unknown>;
  if (
    (surface.kind !== "surface" && surface.kind !== "implicit_surface")
    || typeof surface.as !== "string"
    || !surface.as
    || typeof surface.expression !== "string"
    || !surface.expression.trim()
    || !finiteRange(surface.x_range)
    || !finiteRange(surface.y_range)
  ) return false;
  if (surface.kind === "implicit_surface") {
    return finiteRange(surface.z_range)
      && typeof surface.level === "number"
      && Number.isFinite(surface.level);
  }
  return true;
}

function validSource(value: unknown): value is SelectionEnhancementSourceRef {
  if (!value || typeof value !== "object") return false;
  const source = value as Partial<SelectionEnhancementSourceRef>;
  const bounds = source.bounds;
  return typeof source.source_id === "string"
    && source.source_id.length > 0
    && typeof source.document_id === "string"
    && source.document_id.length > 0
    && Number.isInteger(source.document_version)
    && (source.document_version ?? -1) >= 0
    && Boolean(bounds)
    && Number.isFinite(bounds?.x)
    && Number.isFinite(bounds?.y)
    && Number.isFinite(bounds?.width)
    && (bounds?.width ?? 0) > 0
    && Number.isFinite(bounds?.height)
    && (bounds?.height ?? 0) > 0
    && source.checksum?.algorithm === "sha-256"
    && /^[a-f0-9]{64}$/.test(source.checksum.value ?? "");
}

function validBoardRef(value: unknown): value is SelectionEnhancementBoardRef {
  if (!value || typeof value !== "object") return false;
  const board = value as Partial<SelectionEnhancementBoardRef>;
  if (
    typeof board.board_id !== "string"
    || !board.board_id.trim()
    || !Number.isSafeInteger(board.revision)
    || (board.revision ?? -1) < 0
    || !Array.isArray(board.targets)
    || board.targets.length > 6
  ) return false;
  const ids = new Set<string>();
  return board.targets.every((value) => {
    if (!value || typeof value !== "object") return false;
    const target = value as Partial<SelectionEnhancementBoardTargetRef>;
    const overlap = target.overlap;
    const distance = target.distance;
    if (
      typeof target.target_id !== "string"
      || !target.target_id.trim()
      || ids.has(target.target_id)
      || typeof target.node_id !== "string"
      || !target.node_id.trim()
      || typeof target.kind !== "string"
      || !target.kind.trim()
      || (target.element_id !== undefined && typeof target.element_id !== "string")
      || (target.element_id !== undefined && target.element_id !== target.target_id)
      || (target.label !== undefined && typeof target.label !== "string")
      || !target.world_bounds
      || !Number.isFinite(target.world_bounds.x)
      || !Number.isFinite(target.world_bounds.y)
      || !Number.isFinite(target.world_bounds.width)
      || target.world_bounds.width <= 0
      || !Number.isFinite(target.world_bounds.height)
      || target.world_bounds.height <= 0
      || typeof overlap !== "number"
      || !Number.isFinite(overlap)
      || overlap < 0
      || overlap > 1
      || typeof distance !== "number"
      || !Number.isFinite(distance)
      || distance < 0
      || !Number.isSafeInteger(target.z_index)
    ) return false;
    ids.add(target.target_id);
    return true;
  });
}

export function validateSelectionEnhancementArtifact(
  value: unknown,
): SelectionEnhancementArtifact {
  if (!value || typeof value !== "object") {
    throw new Error("选区辅助内容必须是对象");
  }
  const artifact = value as Partial<SelectionEnhancementArtifact>;
  const interpretation = artifact.interpretation;
  const response = artifact.response;
  const validVersion = artifact.version === "0.1" || artifact.version === "0.2";
  const validV2Context = artifact.version !== "0.2"
    || (validBoardRef(artifact.board)
      && typeof artifact.tool_id === "string"
      && ["explain", "check-and-suggest", "generate-plot", "custom-question"]
        .includes(artifact.tool_id));
  if (
    artifact.profile !== "octos.selection-enhancement"
    || !validVersion
    || !validV2Context
    || typeof artifact.turn_id !== "string"
    || !artifact.turn_id
    || typeof artifact.created_at !== "string"
    || !validSource(artifact.source)
    || !interpretation
    || !["text", "math", "geometry", "data", "unknown"].includes(
      String(interpretation.kind),
    )
    || typeof interpretation.content !== "string"
    || !["high", "medium", "low"].includes(String(interpretation.confidence))
    || !response
    || typeof response.title !== "string"
    || !response.title.trim()
    || typeof response.text !== "string"
    || !response.text.trim()
  ) {
    throw new Error("选区辅助内容的来源或说明字段无效");
  }
  if (response.kind === "unsupported") {
    if (
      ![
        "unreadable_expression",
        "unsupported_variables",
        "unsupported_representation",
        "unsafe_complexity",
      ].includes(response.reason_code)
      || (response.alternatives !== undefined && (
        !Array.isArray(response.alternatives)
        || response.alternatives.some((item) =>
          typeof item !== "string" || !item.trim()
        )
      ))
    ) {
      throw new Error("选区函数图的不支持说明无效");
    }
  } else if (response.kind === "scene3d") {
    if (!validSelectionScene3d(response.content)) {
      throw new Error("选区三维函数图内容无效");
    }
  } else if (response.kind === "explanation") {
    if (
      response.items !== undefined
      && (
        !Array.isArray(response.items)
        || response.items.some((item) => typeof item !== "string" || !item.trim())
      )
    ) {
      throw new Error("选区辅助内容的条目无效");
    }
  } else if (
    response.kind !== "plot"
    || typeof response.expression !== "string"
    || !response.expression.trim()
    || (response.plot_kind !== undefined
      && response.plot_kind !== "explicit"
      && response.plot_kind !== "implicit")
    || (response.plot_kind === "implicit" && (
      typeof response.level !== "number"
      || !Number.isFinite(response.level)
    ))
    || !finiteRange(response.x_range)
    || !finiteRange(response.y_range)
  ) {
    throw new Error("选区函数图内容无效");
  }
  return structuredClone(artifact as SelectionEnhancementArtifact);
}

export function selectionArtifactMatchesSource(
  artifact: SelectionEnhancementArtifact,
  source: InkSelectionSnapshot,
): boolean {
  // The immutable selection is identified by its stable source/document IDs,
  // document version, and checksum. Bounds are layout metadata, not identity:
  // JSON tool calls can round an otherwise identical floating-point coordinate
  // by a few ulps. Rendering also deliberately uses the locally saved source
  // bounds, never the coordinates echoed by the generated artifact.
  return artifact.source.source_id === source.source_id
    && artifact.source.document_id === source.document_id
    && artifact.source.document_version === source.document_version
    && artifact.source.checksum.algorithm === source.checksum.algorithm
    && artifact.source.checksum.value === source.checksum.value;
}

export function selectionArtifactTargetsExist(
  artifact: SelectionEnhancementArtifact,
  board: SemanticBoardState | null,
): boolean {
  if (artifact.version === "0.1" || !artifact.board) return true;
  if (
    !board
    || artifact.board.board_id !== board.board_id
    || artifact.board.revision !== board.revision
  ) return false;
  return artifact.board.targets.every((target) => boardTargetExists(target, board));
}

export function selectionBoardContextTargetsExist(
  context: SelectionBoardContext,
  board: SemanticBoardState | null,
): boolean {
  if (
    !board
    || context.boardId !== board.board_id
    || context.boardRevision !== board.revision
  ) return false;
  return context.targets.every((target) => boardTargetExists(target, board));
}

function boardTargetExists(
  target: Pick<
    SelectionEnhancementBoardTargetRef,
    "target_id" | "node_id" | "element_id" | "kind"
  >,
  board: SemanticBoardState,
): boolean {
  const node = board.nodes[target.node_id];
  if (!node) return false;
  if (!target.element_id) return target.target_id === target.node_id;
  const content = node.content as Record<string, unknown>;
  if (target.kind === "table-cell") {
    const header = new RegExp(`^${escapeRegExp(target.node_id)}:table:header:(\\d+)$`)
      .exec(target.element_id);
    if (header) {
      const columns = Array.isArray(content.columns) ? content.columns : [];
      return Number(header[1]) < columns.length;
    }
    const cell = new RegExp(
      `^${escapeRegExp(target.node_id)}:table:row:(\\d+):column:(\\d+)$`,
    ).exec(target.element_id);
    if (!cell) return false;
    const rows = Array.isArray(content.rows) ? content.rows : [];
    const columns = Array.isArray(content.columns) ? content.columns : [];
    return Number(cell[1]) < rows.length && Number(cell[2]) < columns.length;
  }
  return [
    "fragments", "curves", "points", "guides", "regions", "elements",
    "edges", "circles", "segments", "arcs", "objects", "sections",
    "highlights",
  ].some((field) => {
    const values = content[field];
    return Array.isArray(values) && values.some((value) =>
      value && typeof value === "object"
      && (value as Record<string, unknown>).id === target.element_id,
    );
  });
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function selectionEnhancementStorageKey(sessionId: string): string {
  return "learn:selection-enhancements:" + sessionId + ":v1";
}

export async function loadSelectionEnhancementState(
  sessionId: string,
  storage: Storage = localStorage,
): Promise<SelectionEnhancementState> {
  const empty: SelectionEnhancementState = {
    profile: STATE_PROFILE,
    version: STATE_VERSION,
    session_id: sessionId,
    sources: [],
    hidden_enhancement_turn_ids: [],
  };
  const raw = storage.getItem(selectionEnhancementStorageKey(sessionId));
  if (!raw) return empty;
  try {
    const parsed = JSON.parse(raw) as Partial<SelectionEnhancementState>;
    if (
      parsed.profile !== STATE_PROFILE
      || parsed.version !== STATE_VERSION
      || parsed.session_id !== sessionId
      || !Array.isArray(parsed.sources)
      || !Array.isArray(parsed.hidden_enhancement_turn_ids)
      || parsed.hidden_enhancement_turn_ids.some(
        (id) => typeof id !== "string" || !id,
      )
    ) {
      throw new Error("invalid selection enhancement state");
    }
    const sources: InkSelectionSnapshot[] = [];
    const ids = new Set<string>();
    for (const candidate of parsed.sources) {
      const source = validateInkSelectionSnapshot(candidate);
      await assertInkSelectionIntegrity(source);
      if (ids.has(source.source_id)) throw new Error("duplicate selection source");
      ids.add(source.source_id);
      sources.push(source);
    }
    return {
      ...empty,
      sources,
      hidden_enhancement_turn_ids: [
        ...new Set(parsed.hidden_enhancement_turn_ids),
      ],
    };
  } catch {
    storage.removeItem(selectionEnhancementStorageKey(sessionId));
    return empty;
  }
}

export function saveSelectionEnhancementState(
  state: SelectionEnhancementState,
  storage: Storage = localStorage,
): void {
  storage.setItem(
    selectionEnhancementStorageKey(state.session_id),
    JSON.stringify(state),
  );
}

export function addSelectionSource(
  state: SelectionEnhancementState,
  source: InkSelectionSnapshot,
): SelectionEnhancementState {
  const validated = validateInkSelectionSnapshot(source);
  const existing = state.sources.find(
    (candidate) => candidate.source_id === validated.source_id,
  );
  if (existing) {
    if (JSON.stringify(existing) !== JSON.stringify(validated)) {
      throw new Error("同一选区来源编号不能指向不同原稿");
    }
    return state;
  }
  return { ...state, sources: [...state.sources, validated] };
}

export function hideSelectionEnhancement(
  state: SelectionEnhancementState,
  turnId: string,
): SelectionEnhancementState {
  if (state.hidden_enhancement_turn_ids.includes(turnId)) return state;
  return {
    ...state,
    hidden_enhancement_turn_ids: [
      ...state.hidden_enhancement_turn_ids,
      turnId,
    ],
  };
}

export function removeSelectionSources(
  state: SelectionEnhancementState,
  sourceIds: Iterable<string>,
  matchingTurnIds: Iterable<string> = [],
): SelectionEnhancementState {
  const ids = new Set(sourceIds);
  if (ids.size === 0) return state;
  const sources = state.sources.filter((source) => !ids.has(source.source_id));
  if (sources.length === state.sources.length) return state;
  return {
    ...state,
    sources,
    hidden_enhancement_turn_ids: [
      ...new Set([
        ...state.hidden_enhancement_turn_ids,
        ...matchingTurnIds,
      ]),
    ],
  };
}

export function isSelectionEnhancementArtifact(
  file: { filename?: string; path?: string },
): boolean {
  return [file.filename, file.path].some(
    (value) =>
      typeof value === "string"
      && value.toLowerCase().endsWith(ARTIFACT_SUFFIX),
  );
}

function artifactIdentity(
  artifact: Pick<SelectionEnhancementArtifactRef, "filename">,
): string {
  const filename = artifact.filename.replaceAll("\\", "/").split("/").at(-1);
  return encodeURIComponent(filename ?? artifact.filename);
}

export function collectSelectionEnhancementArtifacts(
  threads: Thread[],
): SelectionEnhancementArtifactRef[] {
  const artifacts: SelectionEnhancementArtifactRef[] = [];
  const seen = new Set<string>();
  for (const thread of threads) {
    const messages = [
      ...thread.responses,
      ...(thread.pendingAssistant ? [thread.pendingAssistant] : []),
    ];
    for (const message of messages) {
      for (const file of message.files) {
        if (!isSelectionEnhancementArtifact(file)) continue;
        const artifact = {
          id: [message.id, file.path].join(":"),
          filename: file.filename,
          path: file.path,
          turnId: thread.turnId ?? thread.id,
        };
        const identity = artifactIdentity(artifact);
        if (seen.has(identity)) continue;
        seen.add(identity);
        artifacts.push(artifact);
      }
    }
  }
  return artifacts;
}

export function collectPersistedSelectionEnhancementArtifacts(
  files: SessionFileInfo[],
): SelectionEnhancementArtifactRef[] {
  return files
    .filter((file) => isSelectionEnhancementArtifact(file))
    .sort((left, right) => Date.parse(left.modified_at) - Date.parse(right.modified_at))
    .map((file) => ({
      id: "persisted:" + file.path,
      filename: file.filename,
      path: file.path,
      turnId: file.filename.slice(0, -ARTIFACT_SUFFIX.length),
    }));
}

export function mergeSelectionEnhancementArtifacts(
  ...groups: SelectionEnhancementArtifactRef[][]
): SelectionEnhancementArtifactRef[] {
  const result: SelectionEnhancementArtifactRef[] = [];
  const seen = new Set<string>();
  for (const group of groups) {
    for (const artifact of group) {
      const identity = artifactIdentity(artifact);
      if (seen.has(identity)) continue;
      seen.add(identity);
      result.push(artifact);
    }
  }
  return result;
}

export async function loadSelectionEnhancementArtifact(
  artifact: SelectionEnhancementArtifactRef,
  sessionId: string,
  signal?: AbortSignal,
): Promise<SelectionEnhancementArtifact> {
  const response = await fetch(buildFileUrl(artifact.path, { sessionId }), {
    headers: buildApiHeaders(),
    signal,
  });
  if (!response.ok) {
    throw new Error("选区辅助内容读取失败 (" + response.status + ")");
  }
  return validateSelectionEnhancementArtifact(await response.json());
}
