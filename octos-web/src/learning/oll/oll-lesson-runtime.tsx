import {
  BoxSelect,
  CheckCircle2,
  Eraser,
  Hand,
  Lightbulb,
  MessageCircle,
  Mic,
  Palette,
  PenLine,
  RotateCcw,
  Redo2,
  Undo2,
} from "lucide-react";
import { createPortal } from "react-dom";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  formatVariableValue,
  mountInfiniteBoard,
  studentInputMethod,
  type BoardTargetCandidate,
  type MountedInfiniteBoard,
  type StudentInputMethod,
  type RegionLayoutConstraint,
  type ViewportInsets,
  variableControlModels,
} from "octos-lesson-language/web-runtime";
import {
  mountInkRuntime,
  type InkSelectionSnapshot,
  type InkMode,
  type InkRuntime,
  type InkRuntimeState,
} from "./oll-ink-runtime";
import { SelectionEnhancementLayer } from "../selection-enhancement-layer";
import {
  WhiteboardQuestionCard,
  WHITEBOARD_QUESTION_CARD_WIDTH,
} from "../whiteboard-question-card";
import type { WhiteboardQuestionRecord } from "../whiteboard-questions";
import type {
  SelectionBoardContext,
  SelectionClassification,
  SelectionContentKind,
  SelectionEnhancementArtifact,
} from "../selection-enhancements";
import {
  selectionArtifactTargetsExist,
  selectionContextToPngFile,
  selectionSnapshotToPngFile,
} from "../selection-enhancements";
import {
  availableSelectionTools,
  selectionLessonTool,
  type SelectionToolId,
} from "../selection-tools";
import type {
  OllLessonRuntimeController,
} from "./use-oll-lesson-runtime";
import { buildInteractionClusters } from "./interaction-clusters";
import {
  WhiteboardLoadingBlock,
  type WhiteboardLoadingState,
} from "../whiteboard-loading-block";
import {
  findNewTopicWhiteboardPosition,
  findOpenWhiteboardPosition,
  type WhiteboardRect,
} from "../whiteboard-placement";
import {
  WhiteboardCameraController,
  type WhiteboardCameraDecision,
} from "../whiteboard-camera-controller";
import {
  COURSE_PENDING_FOOTPRINT_HEIGHT,
  COURSE_PENDING_FOOTPRINT_WIDTH,
  COURSE_REGION_GUTTER,
  COURSE_RUNTIME_OFFSET_X,
  courseRegionOccupiedRect,
  type CourseRegionRecord,
} from "../course-regions";
import "octos-lesson-language/web-runtime/styles.css";

type LearningInkState = InkRuntimeState & {
  pen_color: string;
  selection_color: string | null;
  selection_mode: "rectangle" | "lasso";
  content_bounds: InkSelectionSnapshot["bounds"] | null;
  content_bounds_list: InkSelectionSnapshot["bounds"][];
};

type LearningInkRuntime = InkRuntime & {
  setPenColor?: (color: string) => void;
  setSelectionColor?: (color: string) => void | Promise<void>;
  setSelectionMode?: (mode: "rectangle" | "lasso") => void;
};

interface PreparedSelectionContext {
  snapshot: InkSelectionSnapshot;
  candidates: BoardTargetCandidate[];
  boardId: string;
  boardRevision: number;
  recorded: boolean;
}

export interface DegradedVisualRetryRequest {
  boardId: string;
  boardRevision: number;
  nodeId: string;
  visualId: string;
  surface: string;
  purpose: string;
  title: string;
}

type DegradedVisualStatus = DegradedVisualRetryRequest;

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function degradedVisualStatuses(
  board: OllLessonRuntimeController["board"],
): DegradedVisualStatus[] {
  if (!board) return [];
  return Object.values(board.nodes).flatMap((node) => {
    if (node.role !== "system-status" || !isRecord(node.content.degradation)) {
      return [];
    }
    const degradation = node.content.degradation;
    if (degradation.kind !== "visual_component"
      || degradation.retryable !== true
      || typeof degradation.visual_id !== "string"
      || typeof degradation.surface !== "string"
      || typeof degradation.purpose !== "string") {
      return [];
    }
    return [{
      boardId: board.board_id,
      boardRevision: board.revision,
      nodeId: node.id,
      visualId: degradation.visual_id,
      surface: degradation.surface,
      purpose: degradation.purpose,
      title: typeof node.content.title === "string"
        ? node.content.title
        : "互动画面暂时不可用",
    }];
  }).sort((left, right) => left.nodeId.localeCompare(right.nodeId));
}

function visibleBoardTargetCandidates(
  candidates: BoardTargetCandidate[],
): BoardTargetCandidate[] {
  const seen = new Set<string>();
  const result: BoardTargetCandidate[] = [];
  const nodeIds = [...new Set(candidates.map((candidate) => candidate.node_id))];
  for (const nodeId of nodeIds) {
    const group = candidates.filter((candidate) => candidate.node_id === nodeId);
    const parent = group.find((candidate) => !candidate.element_id);
    for (const candidate of [...(parent ? [parent] : []), ...group.filter((item) => item.element_id)]) {
      if (seen.has(candidate.target_id)) continue;
      seen.add(candidate.target_id);
      result.push(candidate);
      if (result.length === 6) return result;
    }
  }
  return result;
}

const boardTargetKindLabels: Record<string, string> = {
  node: "整个内容块",
  plot: "整个函数图",
  geometry: "整个几何图",
  scene3d: "整个三维画面",
  "plot-point": "图上的点",
  "plot-curve": "函数曲线",
  "math-fragment": "公式片段",
  "geometry-point": "几何点",
  "geometry-line": "几何线",
  "scene3d-object": "三维对象",
};

const selectionContentKindLabels: Record<SelectionContentKind, string> = {
  text: "文字",
  math: "公式",
  geometry: "图形",
  data: "数据",
  unknown: "暂不确定",
};

const boardOcclusionSelector = "[data-learning-board-occlusion]";
const courseVisualNodeKinds = new Set([
  "diagram",
  "geometry",
  "image",
  "plot",
  "scene3d",
]);
const PENDING_QUESTION_FOOTPRINT_WIDTH = COURSE_PENDING_FOOTPRINT_WIDTH;
const PENDING_QUESTION_FOOTPRINT_HEIGHT = COURSE_PENDING_FOOTPRINT_HEIGHT;

function unionWhiteboardRects(rects: WhiteboardRect[]): WhiteboardRect | null {
  if (rects.length === 0) return null;
  const x = Math.min(...rects.map((rect) => rect.x));
  const y = Math.min(...rects.map((rect) => rect.y));
  const right = Math.max(...rects.map((rect) => rect.x + rect.width));
  const bottom = Math.max(...rects.map((rect) => rect.y + rect.height));
  return { x, y, width: right - x, height: bottom - y };
}

function renderedWorldRect(element: HTMLElement): WhiteboardRect | null {
  const x = Number.parseFloat(element.style.left);
  const y = Number.parseFloat(element.style.top);
  const explicitWidth = Number.parseFloat(element.style.width);
  const explicitHeight = Number.parseFloat(element.style.height);
  const fallbackWidth = element.classList.contains("learning-whiteboard-loading-block")
    ? 360
    : element.classList.contains("learning-selection-enhancement-pin")
      ? 26
      : element.classList.contains("learning-selection-enhancement")
        ? 330
        : element.classList.contains("learning-whiteboard-question-card")
          ? WHITEBOARD_QUESTION_CARD_WIDTH
          : element.classList.contains("learning-variable-controls")
            ? 360
            : element.classList.contains("learning-student-tasks")
              ? 330
          : 0;
  const fallbackHeight = element.classList.contains("learning-whiteboard-loading-block")
    ? 194
    : element.classList.contains("learning-selection-enhancement-pin")
      ? 26
      : element.classList.contains("learning-selection-enhancement")
        ? 360
        : element.classList.contains("learning-whiteboard-question-card")
          ? 130
          : element.classList.contains("learning-variable-controls")
            ? 96
            : element.classList.contains("learning-student-tasks")
              ? 240
          : 0;
  const width = Number.isFinite(explicitWidth)
    ? explicitWidth
    : element.offsetWidth || fallbackWidth;
  const height = Number.isFinite(explicitHeight)
    ? explicitHeight
    : element.offsetHeight || fallbackHeight;
  return Number.isFinite(x) && Number.isFinite(y) && width > 0 && height > 0
    ? { x, y, width, height }
    : null;
}

function measureVisualRegionBounds(
  board: OllLessonRuntimeController["board"],
  nodeLayer: HTMLElement,
): Record<string, WhiteboardRect> {
  if (!board) return {};
  const rectsByRegion = new Map<string, WhiteboardRect[]>();
  for (const element of nodeLayer.querySelectorAll<HTMLElement>(
    ".board-node[data-id]",
  )) {
    const nodeId = element.dataset.id;
    const node = nodeId ? board.nodes[nodeId] : undefined;
    if (!node || !courseVisualNodeKinds.has(String(node.kind ?? "text"))) {
      continue;
    }
    const bounds = renderedWorldRect(element);
    if (!bounds) continue;
    const regionId = typeof node.region_id === "string" && node.region_id
      ? node.region_id
      : "__legacy__";
    const rects = rectsByRegion.get(regionId) ?? [];
    rects.push(bounds);
    rectsByRegion.set(regionId, rects);
  }
  return Object.fromEntries([...rectsByRegion].flatMap(([regionId, rects]) => {
    const bounds = unionWhiteboardRects(rects);
    return bounds ? [[regionId, bounds]] : [];
  }));
}

const emptyInkState: LearningInkState = {
  mode: "navigate",
  component_count: 0,
  selected_count: 0,
  pen_color: "#176b62",
  selection_color: null,
  selection_input: "unknown",
  selection_mode: "rectangle",
  selection_revision: 0,
  content_bounds: null,
  content_bounds_list: [],
  document_version: 0,
  saved: true,
};

function normalizeInkState(state: InkRuntimeState): LearningInkState {
  const enhanced = state as Partial<LearningInkState>;
  return {
    ...state,
    pen_color: enhanced.pen_color ?? "#176b62",
    selection_color: enhanced.selection_color ?? null,
    selection_mode: enhanced.selection_mode ?? "rectangle",
    selection_revision: enhanced.selection_revision ?? 0,
    content_bounds: enhanced.content_bounds ?? null,
    content_bounds_list: enhanced.content_bounds_list ?? (
      enhanced.content_bounds ? [enhanced.content_bounds] : []
    ),
  };
}

const inkColorPresets = [
  { color: "#176b62", label: "深青" },
  { color: "#1769aa", label: "蓝色" },
  { color: "#7a5aa3", label: "紫色" },
  { color: "#c75445", label: "红色" },
  { color: "#202b2a", label: "黑色" },
];

function InkColorControl({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (color: string) => void;
}) {
  return (
    <div className="learning-ink-colors" aria-label={label}>
      {inkColorPresets.map((preset) => (
        <button
          key={preset.color}
          type="button"
          className="learning-ink-color-swatch"
          style={{ "--ink-swatch": preset.color } as CSSProperties}
          onClick={() => onChange(preset.color)}
          aria-label={`${label}：${preset.label}`}
          aria-pressed={value.toLowerCase() === preset.color}
        />
      ))}
    </div>
  );
}

function learningBoardInsets(viewport: HTMLElement): ViewportInsets & {
  occlusions: Array<{ x: number; y: number; width: number; height: number }>;
} {
  const compact = viewport.clientWidth <= 900;
  const viewportRect = viewport.getBoundingClientRect();
  const occlusions = [
    ...viewport.ownerDocument.querySelectorAll<HTMLElement>(boardOcclusionSelector),
  ].flatMap((element) => {
    if (element.hidden) return [];
    const rect = element.getBoundingClientRect();
    const left = Math.max(viewportRect.left, rect.left);
    const top = Math.max(viewportRect.top, rect.top);
    const right = Math.min(viewportRect.right, rect.right);
    const bottom = Math.min(viewportRect.bottom, rect.bottom);
    return right > left && bottom > top
      ? [{ x: left - viewportRect.left, y: top - viewportRect.top, width: right - left, height: bottom - top }]
      : [];
  });
  return {
    top: compact ? 78 : 92,
    right: compact ? 18 : 28,
    bottom: compact ? 180 : 190,
    left: compact ? 18 : 28,
    occlusions,
  };
}

function ensureScene3dInteractionHints(viewport: HTMLElement): void {
  const scenes = viewport.querySelectorAll<HTMLElement>(".scene3d-runtime");
  scenes.forEach((scene, index) => {
    if (scene.querySelector(".learning-scene3d-interaction-hint")) return;
    const hint = viewport.ownerDocument.createElement("div");
    hint.className = "learning-scene3d-interaction-hint";
    hint.id = `learning-scene3d-interaction-hint-${index}`;
    hint.textContent = "拖动画面旋转 · 滚动缩放";
    const image = scene.querySelector<SVGElement>("svg[role='img']");
    image?.setAttribute("aria-describedby", hint.id);
    scene.append(hint);
  });
}

export function LearningWhiteboard({
  runtime,
  inkSessionId,
  inkMergeSourceSessionId,
  onInkMergeComplete,
  loadingState,
  questions = [],
  courseRegions = [],
  playbackCourseTarget,
  onPlaceQuestion,
  onUpdateCourseRegion,
  onInkActivity,
  selectionEnhancements = [],
  selectionSources = [],
  onClassifyInkSelection,
  onAskInkSelection,
  onVoiceInkSelection,
  onReferenceInkSelection,
  onDeleteSelectionEnhancement,
  onDeleteSelectionSources,
  onRetryDegradedVisual,
}: {
  runtime?: OllLessonRuntimeController | null;
  inkSessionId?: string;
  inkMergeSourceSessionId?: string;
  onInkMergeComplete?: (
    sourceSessionId: string,
    targetSessionId: string,
  ) => void;
  loadingState?: WhiteboardLoadingState | null;
  questions?: WhiteboardQuestionRecord[];
  courseRegions?: CourseRegionRecord[];
  playbackCourseTarget?: {
    courseId: string;
    sequence: number;
  } | null;
  onPlaceQuestion?: (
    questionId: string,
    position: { x: number; y: number },
  ) => void;
  onUpdateCourseRegion?: (
    courseRegionId: string,
    patch: Partial<Pick<
      CourseRegionRecord,
      "runtimeRegionId" | "bounds" | "reservedWidth"
    >>,
  ) => void;
  onInkActivity?: () => void;
  selectionEnhancements?: SelectionEnhancementArtifact[];
  selectionSources?: InkSelectionSnapshot[];
  onClassifyInkSelection?: (request: {
    snapshot: InkSelectionSnapshot;
    boardContext: SelectionBoardContext;
    selectionImage: File;
  }) => Promise<SelectionClassification>;
  onAskInkSelection?: (request: {
    snapshot: InkSelectionSnapshot;
    question: string;
    contentKind: SelectionContentKind;
    recognizedContent?: string;
    recognitionConfidence?: "high" | "medium" | "low";
    toolId: SelectionToolId;
    boardContext: SelectionBoardContext;
    contextImage: File;
  }) => Promise<void> | void;
  onVoiceInkSelection?: (request: {
    snapshot: InkSelectionSnapshot;
    contentKind: SelectionContentKind;
    boardContext: SelectionBoardContext;
    contextImage: File;
  }) => Promise<void> | void;
  onReferenceInkSelection?: (request: {
    snapshot: InkSelectionSnapshot;
    contentKind: SelectionContentKind;
    boardContext: SelectionBoardContext;
    contextImage: File;
    label: string;
  }) => Promise<void> | void;
  onDeleteSelectionEnhancement?: (turnId: string) => void;
  onDeleteSelectionSources?: (sourceIds: string[]) => void;
  onRetryDegradedVisual?: (
    request: DegradedVisualRetryRequest,
  ) => Promise<void> | void;
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const runtimeRef = useRef<OllLessonRuntimeController | null>(runtime ?? null);
  const mountedRef = useRef<MountedInfiniteBoard | null>(null);
  const cameraControllerRef = useRef<WhiteboardCameraController | null>(null);
  const focusedLoadingTurnRef = useRef<string | null>(null);
  const availableTaskKeysRef = useRef(new Set<string>());
  const availableTaskKeysSeededRef = useRef(false);
  const coursesObservedInProgressRef = useRef(new Set<string>());
  const restoredCourseFocusRef = useRef<string | null>(null);
  const measuredCourseBoundsKeyRef = useRef("");
  const renderedAttentionRef = useRef("");
  const renderedFocusRef = useRef<string[]>([]);
  const renderedCompositionRef = useRef("");
  const renderedCompositionCursorRef = useRef(-1);
  const inkRuntimeRef = useRef<LearningInkRuntime | null>(null);
  const inkMergeAttemptRef = useRef<string | null>(null);
  const inkReplayObservedSourceRef = useRef<string | null>(null);
  const inkActivityReportedRef = useRef(false);
  const onInkActivityRef = useRef(onInkActivity);
  const onUpdateCourseRegionRef = useRef(onUpdateCourseRegion);
  const inkSelectionVersionRef = useRef({
    documentVersion: 0,
    selectedCount: 0,
    selectionRevision: 0,
  });
  const missingInkSelectionSourcesRef = useRef(new Set<string>());
  const selectionSourceCheckVersionRef = useRef<number | null>(null);
  const selectionClassificationRequestRef = useRef(0);
  const unsubscribeInkRef = useRef<(() => void) | null>(null);
  const sliderOperationsRef = useRef(new Map<string, {
    input: StudentInputMethod;
    value: number;
    operationId?: string;
  }>());
  const [inkState, setInkState] = useState<LearningInkState>(emptyInkState);
  const [inkAvailable, setInkAvailable] = useState(false);
  const [inkSupportsColors, setInkSupportsColors] = useState(false);
  const [inkColorPaletteOpen, setInkColorPaletteOpen] = useState(false);
  const [inkError, setInkError] = useState("");
  const [taskError, setTaskError] = useState("");
  const [enhancementLayer, setEnhancementLayer] =
    useState<HTMLDivElement | null>(null);
  const [lessonLoadingPosition, setLessonLoadingPosition] = useState({
    left: 120,
    top: 120,
  });
  const [runtimeRegionBounds, setRuntimeRegionBounds] = useState<
    Record<string, WhiteboardRect>
  >({});
  const [runtimeVisualRegionBounds, setRuntimeVisualRegionBounds] = useState<
    Record<string, WhiteboardRect>
  >({});
  const [runtimeAttachmentBounds, setRuntimeAttachmentBounds] = useState<
    Record<string, WhiteboardRect>
  >({});
  const [interactionMeasuredSizes, setInteractionMeasuredSizes] = useState<
    Record<string, { width: number; height: number }>
  >({});
  const [selectionQuestionOpen, setSelectionQuestionOpen] = useState(false);
  const [selectionQuestion, setSelectionQuestion] = useState("");
  const [selectionContentKind, setSelectionContentKind] =
    useState<SelectionContentKind>("unknown");
  const [selectionRequestPending, setSelectionRequestPending] = useState(false);
  const [selectionRequestStatus, setSelectionRequestStatus] = useState("");
  const [selectionLoadingSource, setSelectionLoadingSource] = useState<{
    sourceId: string;
    bounds: InkSelectionSnapshot["bounds"];
  } | null>(null);
  const [preparedSelection, setPreparedSelection] =
    useState<PreparedSelectionContext | null>(null);
  const [selectedBoardTargetIds, setSelectedBoardTargetIds] =
    useState<string[]>([]);
  const [selectionClassification, setSelectionClassification] =
    useState<SelectionClassification | null>(null);
  const [selectionClassificationStatus, setSelectionClassificationStatus] =
    useState<"idle" | "loading" | "ready" | "error">("idle");
  const [retryingDegradedNodeId, setRetryingDegradedNodeId] =
    useState<string | null>(null);
  const [requestedDegradedNodeIds, setRequestedDegradedNodeIds] =
    useState<Set<string>>(() => new Set());
  const inkColorPaletteAvailable = inkSupportsColors && (
    inkState.mode === "draw"
    || (inkState.mode === "select" && inkState.selected_count > 0)
  );

  useEffect(() => {
    onUpdateCourseRegionRef.current = onUpdateCourseRegion;
  }, [onUpdateCourseRegion]);

  useEffect(() => {
    if (!playbackCourseTarget) return;
    coursesObservedInProgressRef.current.add(playbackCourseTarget.courseId);
    cameraControllerRef.current?.markCourseActive(
      playbackCourseTarget.courseId,
      true,
    );
    mountedRef.current?.view.releaseHostCamera();
  }, [playbackCourseTarget]);
  const variableControls = runtime ? variableControlModels(runtime.board) : [];
  const studentTasks = runtime?.studentTasks ?? [];
  const availableStudentTasks = runtime
    ? studentTasks.filter((task) => task.available)
    : [];
  const degradedVisuals = runtime ? degradedVisualStatuses(runtime.board) : [];
  const quickSelectionTools = selectionClassificationStatus === "ready"
    && selectionClassification
    && selectionClassification.confidence !== "low"
    && selectionClassification.kind !== "unknown"
    ? availableSelectionTools(selectionClassification.kind)
    : [];
  const loadingStateId = loadingState?.id;
  const composerQuestions = questions.filter((question) =>
    question.origin === "composer");
  const pendingComposerQuestion = [...composerQuestions].reverse().find(
    (question) => question.status === "pending",
  );
  const selectionLoadingQuestion = selectionLoadingSource
    ? [...questions].reverse().find((question) =>
        question.origin === "selection"
        && question.status === "pending"
        && question.source?.sourceId === selectionLoadingSource.sourceId)
    : undefined;
  const courseRegionByQuestion = useMemo(() => new Map(
    courseRegions.map((region) => [region.questionId, region]),
  ), [courseRegions]);
  const runtimeRegionIdForTopic = useCallback((topicId: string) => {
    const nodeRegionIds = new Set(Object.values(runtime?.board?.nodes ?? {})
      .flatMap((node) => node.region_id ? [node.region_id] : []));
    if (nodeRegionIds.has(topicId)) return topicId;
    if (
      (runtime?.outline.length ?? 0) === 1
      && Object.keys(runtime?.board?.nodes ?? {}).length > 0
      && nodeRegionIds.size === 0
    ) return "__legacy__";
    return topicId;
  }, [runtime?.board?.nodes, runtime?.outline]);
  const presentationTopics = (() => {
    const topics = runtime?.outline ?? [];
    if (!runtime || topics.length === 0) return [];
    const explicitVariableAliases = new Set(
      topics.flatMap((topic) => topic.variableAliases ?? []),
    );
    const explicitTaskAliases = new Set(
      topics.flatMap((topic) => topic.taskAliases ?? []),
    );
    const unassignedVariableAliases = variableControls
      .map((control) => control.alias)
      .filter((alias) => !explicitVariableAliases.has(alias));
    const unassignedTaskAliases = studentTasks
      .map((task) => task.task_id)
      .filter((alias) => !explicitTaskAliases.has(alias));
    return topics.map((topic, index) => ({
      ...topic,
      // Sessions created before per-course ownership was persisted have one
      // ungrouped topic. Keep their controls and tasks visible without using
      // a display label to guess ownership. New sessions always carry the
      // explicit aliases assembled from their canonical lesson artifact.
      variableAliases: topic.variableAliases
        ?? (topics.length === 1 || index === topics.length - 1
          ? unassignedVariableAliases
          : []),
      taskAliases: topic.taskAliases
        ?? (topics.length === 1 || index === topics.length - 1
          ? unassignedTaskAliases
          : []),
    }));
  })();

  const interactionPlans = presentationTopics.flatMap((topic) => {
    const topicControls = variableControls.filter((control) =>
      topic.variableAliases?.includes(control.alias));
    const reservedTopicTasks = studentTasks.filter((task) =>
      topic.taskAliases?.includes(task.task_id));
    const topicTasks = availableStudentTasks.filter((task) =>
      topic.taskAliases?.includes(task.task_id));
    const clusters = buildInteractionClusters(
      runtime?.board ?? null,
      {
        ...topic,
        id: runtimeRegionIdForTopic(topic.id),
      },
      topicControls.map((control) => control.alias),
      reservedTopicTasks.map((task) => task.task_id),
    );
    return clusters.map((cluster) => {
      const controls = topicControls.filter((control) =>
        cluster.variableAliases.includes(control.alias));
      const tasks = topicTasks.filter((task) =>
        cluster.taskIds.includes(task.task_id));
      const controlsWidth = controls.length > 0 ? 360 : 0;
      const tasksWidth = cluster.taskIds.length > 0 ? 330 : 0;
      const controlsHeight = controls.length > 0
        ? Math.max(96, 58 + controls.length * 34)
        : 0;
      const tasksHeight = cluster.taskIds.length > 0
        ? 60 + cluster.taskIds.length * 220
        : 0;
      const estimatedWidth = Math.max(controlsWidth, tasksWidth);
      const estimatedHeight = controlsHeight + tasksHeight
        + (controlsHeight > 0 && tasksHeight > 0 ? 28 : 0);
      const measured = interactionMeasuredSizes[cluster.id];
      return {
        id: cluster.id,
        topic,
        anchorNodeId: cluster.anchorNodeId,
        controls,
        tasks,
        controlsHeight,
        width: measured?.width ?? estimatedWidth,
        height: measured?.height ?? estimatedHeight,
      };
    });
  });

  const regionLayoutConstraints = useMemo(() => Object.fromEntries(
    (runtime?.outline ?? []).flatMap((topic) => {
      const region = topic.questionId
        ? courseRegionByQuestion.get(topic.questionId)
        : undefined;
      if (!region) return [];
      const attachments = interactionPlans
        .filter((plan) =>
          plan.topic.id === topic.id && Boolean(plan.anchorNodeId))
        .map((plan) => ({
          id: plan.id,
          anchorNodeId: plan.anchorNodeId,
          width: plan.width,
          height: plan.height,
          gap: 42,
        }));
      return [[runtimeRegionIdForTopic(topic.id), {
        x: region.origin.x + COURSE_RUNTIME_OFFSET_X,
        y: region.origin.y,
        flow: "reading",
        reservedWidth: Math.max(
          0,
          region.reservedWidth - COURSE_RUNTIME_OFFSET_X,
        ),
        ...(attachments.length > 0 ? { attachments } : {}),
      } satisfies RegionLayoutConstraint]];
    }),
  ), [
    courseRegionByQuestion,
    interactionPlans,
    runtime?.outline,
    runtimeRegionIdForTopic,
  ]);

  const coursePresentations = interactionPlans.flatMap((plan) => {
    const { topic } = plan;
    const region = topic.questionId
      ? courseRegionByQuestion.get(topic.questionId)
      : undefined;
    const boardBounds = runtimeRegionBounds[runtimeRegionIdForTopic(topic.id)]
      ?? (presentationTopics.length === 1
        ? runtimeRegionBounds.__legacy__
        : undefined);
    if (
      !region
      && !boardBounds
      && (courseRegions.length > 0 || presentationTopics.length > 1)
    ) return [];
    const { controls, tasks } = plan;
    const base = boardBounds ?? {
      x: (region?.origin.x ?? 100) + COURSE_RUNTIME_OFFSET_X,
      y: region?.origin.y ?? 90,
      width: 760,
      height: 300,
    };
    const attachmentBounds = runtimeAttachmentBounds[plan.id];
    const visualBounds = runtimeVisualRegionBounds[
      runtimeRegionIdForTopic(topic.id)
    ] ?? (presentationTopics.length === 1
      ? runtimeVisualRegionBounds.__legacy__
      : undefined);
    const fallback = visualBounds
      ? {
          x: visualBounds.x,
          y: visualBounds.y + visualBounds.height + 42,
        }
      : {
          x: base.x,
          y: base.y + base.height + 42,
        };
    const interactionPosition = attachmentBounds ?? {
      ...fallback,
      width: plan.width,
      height: plan.height,
    };
    return [{
      id: plan.id,
      topic,
      controls,
      tasks,
      width: plan.width,
      height: plan.height,
      controlsPosition: {
        x: interactionPosition.x,
        y: interactionPosition.y,
      },
      tasksPosition: {
        x: interactionPosition.x,
        y: interactionPosition.y
          + (controls.length > 0 ? plan.controlsHeight + 28 : 0),
      },
    }];
  });

  useLayoutEffect(() => {
    if (!enhancementLayer || coursePresentations.length === 0) return;
    const frame = window.requestAnimationFrame(() => {
      const controlsElements = [...enhancementLayer.querySelectorAll<HTMLElement>(
        "[data-interaction-controls-id]",
      )];
      const taskElements = [...enhancementLayer.querySelectorAll<HTMLElement>(
        "[data-interaction-tasks-id]",
      )];
      const next = Object.fromEntries(coursePresentations.flatMap((presentation) => {
        const controlsElement = controlsElements.find((element) =>
          element.dataset.interactionControlsId === presentation.id);
        const tasksElement = taskElements.find((element) =>
          element.dataset.interactionTasksId === presentation.id);
        if (!controlsElement && !tasksElement) return [];
        const controlsWidth = controlsElement
          ? controlsElement.offsetWidth || 360
          : 0;
        const tasksWidth = tasksElement ? tasksElement.offsetWidth || 330 : 0;
        const controlsHeight = controlsElement?.offsetHeight || 0;
        const tasksHeight = tasksElement?.offsetHeight || 0;
        return [[presentation.id, {
          width: Math.max(controlsWidth, tasksWidth, presentation.width),
          height: Math.max(presentation.height, controlsHeight + tasksHeight
            + (controlsHeight > 0 && tasksHeight > 0 ? 28 : 0)),
        }]];
      }));
      setInteractionMeasuredSizes((current) =>
        JSON.stringify(current) === JSON.stringify(next) ? current : next);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [coursePresentations, enhancementLayer]);

  const occupiedRectsForQuestion = useCallback((questionId: string) => {
    const elements: HTMLElement[] = [];
    const mounted = mountedRef.current;
    if (mounted) {
      elements.push(...mounted.elements.nodes.querySelectorAll<HTMLElement>(
        ".board-node",
      ));
    }
    if (enhancementLayer) {
      elements.push(...enhancementLayer.querySelectorAll<HTMLElement>([
        "[data-question-id]",
        "[data-loading-id]",
        "[data-course-controls-id]",
        "[data-course-tasks-id]",
        ".learning-selection-enhancement",
        ".learning-selection-enhancement-pin",
      ].join(",")));
    }
    const occupied = elements.flatMap((element) => {
      if (element.dataset.questionId === questionId) return [];
      if (loadingStateId && element.dataset.loadingId === loadingStateId) return [];
      const bounds = renderedWorldRect(element);
      return bounds ? [bounds] : [];
    });
    occupied.push(...courseRegions
      .filter((region) => region.questionId !== questionId)
      .map(courseRegionOccupiedRect));
    occupied.push(...inkState.content_bounds_list);
    return occupied;
  }, [
    courseRegions,
    enhancementLayer,
    inkState.content_bounds_list,
    loadingStateId,
  ]);

  useEffect(() => {
    onInkActivityRef.current = onInkActivity;
  }, [onInkActivity]);

  useEffect(() => {
    if (!loadingStateId || !enhancementLayer) return;
    const frame = window.requestAnimationFrame(() => {
      const viewport = viewportRef.current;
      const view = mountedRef.current?.view;
      if (!viewport || !view) return;
      const center = view.viewportToBoard({
        x: viewport.clientWidth / 2,
        y: viewport.clientHeight / 2,
      });
      setLessonLoadingPosition({
        left: center.x - 180,
        top: center.y - 105,
      });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [enhancementLayer, loadingStateId]);

  useEffect(() => {
    if (!enhancementLayer || !onPlaceQuestion) return;
    if (inkSessionId && !inkAvailable) return;
    const unplaced = composerQuestions.filter((question) => !question.position);
    if (unplaced.length === 0) return;
    const frame = window.requestAnimationFrame(() => {
      const viewport = viewportRef.current;
      const view = mountedRef.current?.view;
      if (!viewport || !view) return;
      const center = view.viewportToBoard({
        x: viewport.clientWidth / 2,
        y: viewport.clientHeight / 2,
      });
      const reserved: WhiteboardRect[] = [];
      unplaced.forEach((question) => {
        const existingTopic = question.status === "answered"
          ? runtime?.outline.find((topic) => topic.questionId === question.id)
          : undefined;
        const existingRuntimeBounds = existingTopic
          ? runtimeRegionBounds[runtimeRegionIdForTopic(existingTopic.id)]
            ?? (runtime?.outline.length === 1
              ? runtimeRegionBounds.__legacy__
              : undefined)
          : undefined;
        // A restored voice/direct lesson can predate persisted question cards.
        // Wait until its already-rendered course footprint is measurable, then
        // put the recovered question immediately to its left. The resulting
        // logical region keeps the existing lesson in place instead of
        // misclassifying it as a brand-new course after the old board.
        if (existingTopic && !existingRuntimeBounds) return;
        const preferred = {
          x: center.x - 180 - WHITEBOARD_QUESTION_CARD_WIDTH - 24,
          y: center.y - 105,
        };
        // Every composer question starts a complete lesson. Its placement must
        // not depend on whether React has already rendered the asynchronous
        // loading state for that turn.
        const width = PENDING_QUESTION_FOOTPRINT_WIDTH;
        const height = PENDING_QUESTION_FOOTPRINT_HEIGHT;
        const occupied = [...occupiedRectsForQuestion(question.id), ...reserved];
        const startsNewTopic = occupied.length > 0
          || courseRegions.some((region) => region.questionId !== question.id)
          || Boolean(runtime?.board && Object.keys(runtime.board.nodes).length > 0);
        const position = existingRuntimeBounds
          ? {
              x: existingRuntimeBounds.x - COURSE_RUNTIME_OFFSET_X,
              y: existingRuntimeBounds.y,
            }
          : startsNewTopic
            ? findNewTopicWhiteboardPosition({
                width,
                height,
                occupied,
                gutter: COURSE_REGION_GUTTER,
              })
            : findOpenWhiteboardPosition({ preferred, width, height, occupied });
        reserved.push({ ...position, width, height });
        onPlaceQuestion(question.id, position);
      });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    composerQuestions,
    courseRegions,
    enhancementLayer,
    inkAvailable,
    inkSessionId,
    occupiedRectsForQuestion,
    onPlaceQuestion,
    runtime?.board,
    runtime?.outline,
    runtimeRegionBounds,
    runtimeRegionIdForTopic,
  ]);

  useEffect(() => {
    const ink = inkRuntimeRef.current;
    if (
      !ink
      || !inkAvailable
      || typeof ink.hasSelectionSource !== "function"
    ) return;
    // Restoring js-draw from its saved SVG can recreate internal component
    // identities. The initial readiness transition therefore cannot prove
    // that the learner erased a source. Only check after a subsequent ink
    // document change observed during this mounted page.
    const lastCheckedVersion = selectionSourceCheckVersionRef.current;
    if (lastCheckedVersion === null) {
      selectionSourceCheckVersionRef.current = inkState.document_version;
      return;
    }
    if (inkState.document_version <= lastCheckedVersion) return;
    selectionSourceCheckVersionRef.current = inkState.document_version;
    if (selectionSources.length === 0) return;
    const currentSourceIds = new Set(selectionSources.map((source) => source.source_id));
    for (const sourceId of missingInkSelectionSourcesRef.current) {
      if (!currentSourceIds.has(sourceId)) {
        missingInkSelectionSourcesRef.current.delete(sourceId);
      }
    }
    const missing = selectionSources
      .filter((source) => (
        !missingInkSelectionSourcesRef.current.has(source.source_id)
        && ink.hasSelectionSource(source) === false
      ))
      .map((source) => source.source_id);
    if (missing.length === 0) return;
    missing.forEach((sourceId) => missingInkSelectionSourcesRef.current.add(sourceId));
    onDeleteSelectionSources?.(missing);
  }, [
    inkAvailable,
    inkState.document_version,
    onDeleteSelectionSources,
    selectionSources,
  ]);

  useEffect(() => {
    const position = pendingComposerQuestion?.position;
    if (
      !pendingComposerQuestion
      || !position
      || focusedLoadingTurnRef.current === pendingComposerQuestion.id
    ) return;
    const frame = window.requestAnimationFrame(() => {
      const controller = cameraControllerRef.current;
      const layer = enhancementLayer;
      if (!controller || !layer) return;
      const elements = [...layer.querySelectorAll<HTMLElement>(
        "[data-question-id], [data-loading-id]",
      )].filter((element) =>
        element.dataset.questionId === pendingComposerQuestion.id
        || (loadingStateId === pendingComposerQuestion.id
          && element.dataset.loadingId === loadingStateId));
      const currentLoadingIsRendered = loadingStateId === pendingComposerQuestion.id
        && elements.some((element) =>
          element.dataset.loadingId === loadingStateId);
      const actualBounds = unionWhiteboardRects(elements.flatMap((element) => {
        const bounds = renderedWorldRect(element);
        return bounds ? [bounds] : [];
      }));
      controller.request({
        source: "question-loading",
        key: `question-loading:${pendingComposerQuestion.id}`,
        courseId: pendingComposerQuestion.id,
        rect: currentLoadingIsRendered && actualBounds ? actualBounds : {
          x: position.x,
          y: position.y,
          width: PENDING_QUESTION_FOOTPRINT_WIDTH,
          height: PENDING_QUESTION_FOOTPRINT_HEIGHT,
        },
      });
      focusedLoadingTurnRef.current = pendingComposerQuestion.id;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    enhancementLayer,
    loadingStateId,
    pendingComposerQuestion,
    pendingComposerQuestion?.id,
    pendingComposerQuestion?.position,
  ]);

  const retryDegradedVisual = useCallback(async (
    degraded: DegradedVisualStatus,
  ) => {
    if (!onRetryDegradedVisual || retryingDegradedNodeId) return;
    setRetryingDegradedNodeId(degraded.nodeId);
    try {
      await onRetryDegradedVisual({
        boardId: degraded.boardId,
        boardRevision: degraded.boardRevision,
        nodeId: degraded.nodeId,
        visualId: degraded.visualId,
        surface: degraded.surface,
        purpose: degraded.purpose,
        title: degraded.title,
      });
      setRequestedDegradedNodeIds((current) => new Set(current).add(degraded.nodeId));
    } catch (cause) {
      setTaskError(cause instanceof Error ? cause.message : "暂时无法重试这个画面");
    } finally {
      setRetryingDegradedNodeId(null);
    }
  }, [onRetryDegradedVisual, retryingDegradedNodeId]);

  const requestTaskHint = useCallback((taskId: string) => {
    try {
      runtimeRef.current?.requestStudentTaskHint(taskId);
      setTaskError("");
    } catch (cause) {
      setTaskError(cause instanceof Error ? cause.message : "暂时无法显示提示");
    }
  }, []);

  const retryTask = useCallback((taskId: string) => {
    try {
      runtimeRef.current?.retryStudentTask(taskId);
      setTaskError("");
    } catch (cause) {
      setTaskError(cause instanceof Error ? cause.message : "暂时无法重新开始任务");
    }
  }, []);

  const startSliderOperation = useCallback((
    alias: string,
    value: number,
    input: StudentInputMethod,
  ) => {
    if (sliderOperationsRef.current.has(alias)) return;
    const operationId = runtimeRef.current?.handleStudentVariableInput(alias, value, {
      phase: "start",
      control: "slider",
      input,
    });
    sliderOperationsRef.current.set(alias, {
      input,
      value,
      ...(typeof operationId === "string" ? { operationId } : {}),
    });
  }, []);

  const updateSliderOperation = useCallback((alias: string, value: number) => {
    if (!sliderOperationsRef.current.has(alias)) {
      startSliderOperation(alias, value, "unknown");
    }
    const active = sliderOperationsRef.current.get(alias);
    if (!active) return;
    active.value = value;
    runtimeRef.current?.handleStudentVariableInput(alias, value, {
      phase: "update",
      control: "slider",
      input: active.input,
      ...(active.operationId ? { operation_id: active.operationId } : {}),
    });
  }, [startSliderOperation]);

  const commitSliderOperation = useCallback((alias: string, value: number) => {
    const active = sliderOperationsRef.current.get(alias);
    if (!active) return;
    sliderOperationsRef.current.delete(alias);
    runtimeRef.current?.handleStudentVariableInput(alias, value, {
      phase: "commit",
      control: "slider",
      input: active.input,
      ...(active.operationId ? { operation_id: active.operationId } : {}),
    });
  }, []);

  useEffect(() => {
    runtimeRef.current = runtime ?? null;
  }, [runtime]);

  const setInkMode = useCallback((mode: InkMode) => {
    try {
      inkRuntimeRef.current?.setMode(mode);
      setInkError("");
    } catch (cause) {
      setInkError(cause instanceof Error ? cause.message : "无法切换书写工具");
    }
  }, []);

  const runInkHistory = useCallback((action: "undo" | "redo") => {
    const ink = inkRuntimeRef.current;
    if (!ink) return;
    void Promise.resolve(action === "undo" ? ink.undo() : ink.redo()).catch(
      (cause) => setInkError(cause instanceof Error ? cause.message : "笔迹历史操作失败"),
    );
  }, []);

  const selectAllInk = useCallback(() => {
    try {
      const ink = inkRuntimeRef.current;
      if (!ink) return;
      ink.setMode("select");
      ink.selectAll();
      setInkError("");
    } catch (cause) {
      setInkError(cause instanceof Error ? cause.message : "无法选择笔迹");
    }
  }, []);

  const setPenColor = useCallback((color: string) => {
    try {
      const ink = inkRuntimeRef.current;
      if (!ink?.setPenColor) {
        throw new Error("当前 Ink Runtime 不支持笔迹颜色");
      }
      ink.setPenColor(color);
      setInkError("");
    } catch (cause) {
      setInkError(cause instanceof Error ? cause.message : "无法设置笔迹颜色");
    }
  }, []);

  const setSelectionColor = useCallback((color: string) => {
    const ink = inkRuntimeRef.current;
    if (!ink) return;
    if (!ink.setSelectionColor) {
      setInkError("当前 Ink Runtime 不支持选区改色");
      return;
    }
    void Promise.resolve(ink.setSelectionColor(color)).then(
      () => setInkError(""),
      (cause) => setInkError(cause instanceof Error ? cause.message : "无法修改选中笔迹的颜色"),
    );
  }, []);

  const setSelectionMode = useCallback((mode: "rectangle" | "lasso") => {
    inkRuntimeRef.current?.setSelectionMode?.(mode);
  }, []);

  const readSelectionContext = useCallback(async (): Promise<PreparedSelectionContext> => {
    const ink = inkRuntimeRef.current;
    if (!ink) throw new Error("笔迹功能尚未就绪");
    await ink.ready;
    const snapshot = await ink.captureSelectionSnapshot();
    const mounted = mountedRef.current;
    const candidates = mounted
      ? visibleBoardTargetCandidates(mounted.view.queryBoardTargets({
          bounds: snapshot.bounds,
          ...(snapshot.region?.points ? { path: snapshot.region.points } : {}),
          limit: 12,
        }))
      : [];
    return {
      snapshot,
      candidates,
      boardId: runtimeRef.current?.board?.board_id
        ?? `learning-whiteboard:${inkSessionId ?? "unsaved"}`,
      boardRevision: runtimeRef.current?.board?.revision ?? 0,
      recorded: false,
    };
  }, [inkSessionId]);

  const recordSelectionContext = useCallback((
    prepared: PreparedSelectionContext,
  ): PreparedSelectionContext => {
    if (prepared.recorded) return prepared;
    const { snapshot } = prepared;
    runtimeRef.current?.recordStudentInkSelection({
      source_id: snapshot.source_id,
      document_id: snapshot.document_id,
      document_version: snapshot.document_version,
      bounds: snapshot.bounds,
      checksum: snapshot.checksum,
    }, inkState.selection_input);
    return { ...prepared, recorded: true };
  }, [inkState.selection_input]);

  const captureSelection = useCallback(async (): Promise<PreparedSelectionContext> => {
    const prepared = await readSelectionContext();
    return recordSelectionContext(prepared);
  }, [readSelectionContext, recordSelectionContext]);

  useEffect(() => {
    const requestId = ++selectionClassificationRequestRef.current;
    if (
      inkState.mode !== "select"
      || inkState.selected_count === 0
      || !onClassifyInkSelection
    ) {
      const resetTimeout = window.setTimeout(() => {
        if (requestId !== selectionClassificationRequestRef.current) return;
        setSelectionClassification(null);
        setSelectionClassificationStatus("idle");
        if (inkState.mode !== "select" || inkState.selected_count === 0) {
          setPreparedSelection(null);
          setSelectedBoardTargetIds([]);
        }
      }, 0);
      return () => window.clearTimeout(resetTimeout);
    }
    let active = true;
    const loadingTimeout = window.setTimeout(() => {
      if (!active || requestId !== selectionClassificationRequestRef.current) return;
      setSelectionClassification(null);
      setSelectionClassificationStatus("loading");
    }, 0);
    const timeout = window.setTimeout(() => void (async () => {
      try {
        const prepared = await readSelectionContext();
        const selectionImage = await selectionSnapshotToPngFile(prepared.snapshot);
        const classification = await onClassifyInkSelection({
          snapshot: prepared.snapshot,
          boardContext: {
            boardId: prepared.boardId,
            boardRevision: prepared.boardRevision,
            targets: prepared.candidates,
          },
          selectionImage,
        });
        if (!active || requestId !== selectionClassificationRequestRef.current) return;
        setPreparedSelection(prepared);
        setSelectionClassification(classification);
        setSelectionContentKind(
          classification.confidence === "low" ? "unknown" : classification.kind,
        );
        setSelectionClassificationStatus("ready");
      } catch {
        if (!active || requestId !== selectionClassificationRequestRef.current) return;
        setSelectionClassification(null);
        setSelectionContentKind("unknown");
        setSelectionClassificationStatus("error");
      }
    })(), 250);
    return () => {
      active = false;
      window.clearTimeout(loadingTimeout);
      window.clearTimeout(timeout);
    };
  }, [
    inkState.mode,
    inkState.selected_count,
    inkState.selection_revision,
    onClassifyInkSelection,
    readSelectionContext,
  ]);

  const openSelectionQuestion = useCallback(async () => {
    if (selectionQuestionOpen) {
      setSelectionQuestionOpen(false);
      return;
    }
    setSelectionQuestionOpen(true);
    setSelectionRequestPending(true);
    try {
      const candidate = preparedSelection ?? await captureSelection();
      const prepared = recordSelectionContext(candidate);
      setPreparedSelection(prepared);
      setSelectedBoardTargetIds(
        prepared.candidates.length === 1
          ? [prepared.candidates[0]!.target_id]
          : [],
      );
      setInkError("");
    } catch (cause) {
      setSelectionQuestionOpen(false);
      setInkError(cause instanceof Error ? cause.message : "无法读取当前选区");
    } finally {
      setSelectionRequestPending(false);
    }
  }, [
    captureSelection,
    preparedSelection,
    recordSelectionContext,
    selectionQuestionOpen,
  ]);

  const askSelection = useCallback(async (
    question: string,
    toolId: SelectionToolId = "custom-question",
    requestContentKind?: SelectionContentKind,
    boardTargetIds?: string[],
  ) => {
    const value = question.trim();
    if (!value || !onAskInkSelection || selectionRequestPending) return;
    setSelectionRequestPending(true);
    setSelectionRequestStatus(
      toolId === "generate-plot"
        ? "正在生成函数图像…"
        : "正在生成选区辅助内容…",
    );
    try {
      const candidate = preparedSelection ?? await captureSelection();
      const prepared = recordSelectionContext(candidate);
      setSelectionLoadingSource({
        sourceId: prepared.snapshot.source_id,
        bounds: prepared.snapshot.bounds,
      });
      const requestedBoardTargetIds = boardTargetIds ?? selectedBoardTargetIds;
      const targets = prepared.candidates.filter((candidate) =>
        requestedBoardTargetIds.includes(candidate.target_id),
      );
      const mounted = mountedRef.current;
      if (!mounted) throw new Error("白板尚未就绪");
      const contextImage = await selectionContextToPngFile(
        prepared.snapshot,
        mounted,
        targets,
      );
      await onAskInkSelection({
        snapshot: prepared.snapshot,
        question: value,
        contentKind: requestContentKind ?? selectionContentKind,
        recognizedContent: selectionClassification?.content,
        recognitionConfidence: selectionClassification?.confidence,
        toolId,
        boardContext: {
          boardId: prepared.boardId,
          boardRevision: prepared.boardRevision,
          targets,
        },
        contextImage,
      });
      setSelectionQuestion("");
      setSelectionQuestionOpen(false);
      setPreparedSelection(null);
      setSelectedBoardTargetIds([]);
      setInkError("");
    } catch (cause) {
      setInkError(cause instanceof Error ? cause.message : "无法发送当前选区");
    } finally {
      setSelectionRequestPending(false);
      setSelectionRequestStatus("");
      setSelectionLoadingSource(null);
    }
  }, [
    captureSelection,
    onAskInkSelection,
    preparedSelection,
    recordSelectionContext,
    selectedBoardTargetIds,
    selectionContentKind,
    selectionClassification,
    selectionRequestPending,
  ]);

  const askSelectionByVoice = useCallback(async () => {
    if (!onVoiceInkSelection || selectionRequestPending) return;
    setSelectionRequestPending(true);
    try {
      const candidate = preparedSelection ?? await captureSelection();
      const prepared = recordSelectionContext(candidate);
      const targets = prepared.candidates.filter((candidate) =>
        selectedBoardTargetIds.includes(candidate.target_id),
      );
      const mounted = mountedRef.current;
      if (!mounted) throw new Error("白板尚未就绪");
      const contextImage = await selectionContextToPngFile(
        prepared.snapshot,
        mounted,
        targets,
      );
      await onVoiceInkSelection({
        snapshot: prepared.snapshot,
        contentKind: selectionContentKind,
        boardContext: {
          boardId: prepared.boardId,
          boardRevision: prepared.boardRevision,
          targets,
        },
        contextImage,
      });
      setSelectionQuestionOpen(false);
      setPreparedSelection(null);
      setSelectedBoardTargetIds([]);
      setInkError("");
    } catch (cause) {
      setInkError(cause instanceof Error ? cause.message : "无法针对当前选区开始语音提问");
    } finally {
      setSelectionRequestPending(false);
    }
  }, [
    captureSelection,
    onVoiceInkSelection,
    preparedSelection,
    recordSelectionContext,
    selectedBoardTargetIds,
    selectionContentKind,
    selectionRequestPending,
  ]);

  const referenceSelectionForLesson = useCallback(async () => {
    if (!onReferenceInkSelection || selectionRequestPending) return;
    setSelectionRequestPending(true);
    try {
      const candidate = preparedSelection ?? await captureSelection();
      const prepared = recordSelectionContext(candidate);
      const targets = prepared.candidates.filter((candidate) =>
        selectedBoardTargetIds.includes(candidate.target_id),
      );
      const mounted = mountedRef.current;
      if (!mounted) throw new Error("白板尚未就绪");
      const contextImage = await selectionContextToPngFile(
        prepared.snapshot,
        mounted,
        targets,
      );
      await onReferenceInkSelection({
        snapshot: prepared.snapshot,
        contentKind: selectionContentKind,
        boardContext: {
          boardId: prepared.boardId,
          boardRevision: prepared.boardRevision,
          targets,
        },
        contextImage,
        label: targets[0]?.label ?? "选中的笔迹",
      });
      setSelectionQuestionOpen(false);
      setPreparedSelection(null);
      setSelectedBoardTargetIds([]);
      setInkError("");
    } catch (cause) {
      setInkError(cause instanceof Error ? cause.message : "无法引用当前选区");
    } finally {
      setSelectionRequestPending(false);
    }
  }, [
    captureSelection,
    onReferenceInkSelection,
    preparedSelection,
    recordSelectionContext,
    selectedBoardTargetIds,
    selectionContentKind,
    selectionRequestPending,
  ]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const mounted = mountInfiniteBoard(viewport);
    mountedRef.current = mounted;
    const reportCameraDecision = (decision: WhiteboardCameraDecision) => {
      if (!import.meta.env.DEV || import.meta.env.MODE === "test") return;
      console.debug("[learn-camera]", {
        action: decision.action,
        source: decision.request.source,
        key: decision.request.key,
        courseId: decision.request.courseId,
        rect: decision.request.rect,
        reason: decision.reason,
      });
    };
    const cameraController = new WhiteboardCameraController(
      (request) => {
        const courseFrame = request.source === "course-end"
          || request.source === "course-restore";
        const focusedViewport = mounted.view.focusWorldRect(request.rect, {
          exclusive: true,
          framing: courseFrame ? "course" : "content",
        });
        // A course region's persisted bounds are a placement footprint, not a
        // camera target. Remember the complete world area exposed by the final
        // course frame so the next course starts beyond that view instead of
        // appearing inside it. The completion effect still derives its camera
        // target from course-owned cards, so this footprint cannot zoom a
        // completed course back out later.
        if (courseFrame && focusedViewport) {
          onUpdateCourseRegionRef.current?.(request.courseId, {
            bounds: focusedViewport,
          });
        }
      },
      (callback) => window.requestAnimationFrame(callback),
      (frame) => window.cancelAnimationFrame(frame),
      reportCameraDecision,
    );
    cameraControllerRef.current = cameraController;
    const enhancementHost = viewport.ownerDocument.createElement("div");
    enhancementHost.className = "learning-selection-enhancement-layer";
    enhancementHost.dataset.ollInkInput = "ignore";
    const unmountEnhancementLayer =
      mounted.view.mountWorldLayer(enhancementHost);
    setEnhancementLayer(enhancementHost);
    const sliderOperations = sliderOperationsRef.current;
    inkActivityReportedRef.current = false;
    setInkAvailable(false);
    setInkSupportsColors(false);
    setInkState(emptyInkState);
    inkSelectionVersionRef.current = {
      documentVersion: 0,
      selectedCount: 0,
      selectionRevision: 0,
    };
    selectionSourceCheckVersionRef.current = null;
    let active = true;
    let ink: LearningInkRuntime | null = null;
    let inkDestroyed = false;
    const destroyInk = (): Promise<void> | undefined => {
      if (!ink || inkDestroyed) return undefined;
      inkDestroyed = true;
      unsubscribeInkRef.current?.();
      unsubscribeInkRef.current = null;
      if (inkRuntimeRef.current === ink) inkRuntimeRef.current = null;
      return ink.destroy();
    };
    try {
      mounted.view.setViewportInsets(learningBoardInsets(viewport));
      mounted.view.setVariableInputHandler((alias, value, event) => {
        return runtimeRef.current?.handleStudentVariableInput(alias, value, event);
      });
      mounted.view.setScene3dInputHandler((nodeId, view, event) => {
        const result = runtimeRef.current?.handleStudentScene3dInput(
          nodeId,
          view,
          event,
        );
        return typeof result === "string" ? result : undefined;
      });
      if (inkSessionId) {
        ink = mountInkRuntime({
          board: mounted.view,
          viewport,
          storageKey: `octos-learning-ink:v1:${inkSessionId}`,
          documentId: `learning-session:${inkSessionId}:student-ink`,
          locale: "zh-CN",
        }) as LearningInkRuntime;
        inkRuntimeRef.current = ink;
        ink.setMode("navigate");
        setInkSupportsColors(
          typeof ink.setPenColor === "function" &&
          typeof ink.setSelectionColor === "function",
        );
        unsubscribeInkRef.current = ink.subscribe((state) => {
          if (!active) return;
          const next = normalizeInkState(state);
          const previous = inkSelectionVersionRef.current;
          if (
            previous.documentVersion !== next.document_version
            || previous.selectedCount !== next.selected_count
            || previous.selectionRevision !== next.selection_revision
          ) {
            setSelectionQuestionOpen(false);
            setPreparedSelection(null);
            setSelectedBoardTargetIds([]);
          }
          inkSelectionVersionRef.current = {
            documentVersion: next.document_version,
            selectedCount: next.selected_count,
            selectionRevision: next.selection_revision,
          };
          setInkState(next);
          if (
            next.component_count > 0
            && next.saved
            && !inkActivityReportedRef.current
          ) {
            inkActivityReportedRef.current = true;
            onInkActivityRef.current?.();
          }
        });
        setInkError("");
        void ink.ready.then(
          () => {
            if (active) setInkAvailable(true);
          },
          (cause) => {
            if (!active) return;
            setInkAvailable(false);
            setInkSupportsColors(false);
            setInkError(cause instanceof Error ? cause.message : "笔迹功能加载失败");
            const destruction = destroyInk();
            if (destruction) void destruction.catch(() => undefined);
          },
        );
      }
    } catch (cause) {
      setInkAvailable(false);
      setInkSupportsColors(false);
      setInkError(cause instanceof Error ? cause.message : "笔迹功能加载失败");
      const destruction = destroyInk();
      if (destruction) void destruction.catch(() => undefined);
    }
    return () => {
      active = false;
      cameraController.destroy();
      if (cameraControllerRef.current === cameraController) {
        cameraControllerRef.current = null;
      }
      setEnhancementLayer(null);
      unmountEnhancementLayer();
      mountedRef.current = null;
      for (const [alias, operation] of sliderOperations) {
        runtimeRef.current?.handleStudentVariableInput(alias, operation.value, {
          phase: "commit",
          control: "slider",
          input: operation.input,
          ...(operation.operationId ? { operation_id: operation.operationId } : {}),
        });
      }
      sliderOperations.clear();
      const destruction = destroyInk();
      mounted.destroy();
      if (destruction) void destruction.catch(() => undefined);
    };
  }, [inkSessionId]);

  useEffect(() => {
    const mounted = mountedRef.current;
    if (!mounted) return;
    const { view } = mounted;
    view.setRegionLayouts(regionLayoutConstraints);
    const nextRegionBounds = view.getRegionBoundsMap();
    setRuntimeRegionBounds((current) =>
      JSON.stringify(current) === JSON.stringify(nextRegionBounds)
        ? current
        : nextRegionBounds);
    const nextAttachmentBounds = view.getAttachmentBoundsMap();
    setRuntimeAttachmentBounds((current) =>
      JSON.stringify(current) === JSON.stringify(nextAttachmentBounds)
        ? current
        : nextAttachmentBounds);
    const nextVisualBounds = measureVisualRegionBounds(
      runtimeRef.current?.board ?? null,
      mounted.elements.nodes,
    );
    setRuntimeVisualRegionBounds((current) =>
      JSON.stringify(current) === JSON.stringify(nextVisualBounds)
        ? current
        : nextVisualBounds);
  }, [enhancementLayer, regionLayoutConstraints]);

  useEffect(() => {
    if (!inkMergeSourceSessionId) {
      inkReplayObservedSourceRef.current = null;
      return;
    }
    // A restored lesson is already settled before the learner presses Replay.
    // Do not interpret that initial state as this replay having finished. Only
    // arm restoration after the Runtime has actually entered playback (or an
    // equivalent unsettled transition) for this source document.
    if (runtime?.playing || runtime?.deliverySettled === false) {
      inkReplayObservedSourceRef.current = inkMergeSourceSessionId;
    }
  }, [
    inkMergeSourceSessionId,
    runtime?.deliverySettled,
    runtime?.playing,
  ]);

  useEffect(() => {
    const ink = inkRuntimeRef.current;
    if (
      !ink ||
      !inkAvailable ||
      !inkSessionId ||
      !inkMergeSourceSessionId ||
      inkReplayObservedSourceRef.current !== inkMergeSourceSessionId ||
      !runtime?.deliverySettled ||
      runtime.playing
    ) return;
    const mergeKey = `${inkSessionId}\u0000${inkMergeSourceSessionId}`;
    if (inkMergeAttemptRef.current === mergeKey) return;
    inkMergeAttemptRef.current = mergeKey;
    void ink.mergeSavedDocument(
      `octos-learning-ink:v1:${inkMergeSourceSessionId}`,
      `learning-session:${inkMergeSourceSessionId}:student-ink`,
    ).then(
      () => {
        setInkError("");
        onInkMergeComplete?.(inkMergeSourceSessionId, inkSessionId);
      },
      (cause) => {
        setInkError(cause instanceof Error
          ? cause.message
          : "上一遍笔迹暂时无法恢复");
      },
    );
  }, [
    inkAvailable,
    inkMergeSourceSessionId,
    inkSessionId,
    onInkMergeComplete,
    runtime?.deliverySettled,
    runtime?.playing,
  ]);

  useEffect(() => {
    const activeRuntime = runtimeRef.current;
    if (!activeRuntime) return;
    const mounted = mountedRef.current;
    const view = mounted?.view;
    const attentionTargets = activeRuntime.attentionTargets;
    const attentionKey = attentionTargets.length > 0
      ? `${activeRuntime.currentOperation?.operation_id ?? activeRuntime.cursor}\u0000${attentionTargets.join("\u0000")}`
      : "";
    const attentionChanged = attentionKey !== renderedAttentionRef.current;
    const boardFocus = activeRuntime.board?.focus ?? [];
    const renderedFocus = renderedFocusRef.current;
    const focusChanged =
      boardFocus.length !== renderedFocus.length ||
      boardFocus.some((target, index) => target !== renderedFocus[index]);
    const atPlaybackBoundary =
      activeRuntime.currentOperation?.type === "beat.end" ||
      activeRuntime.currentOperation?.type === "step.commit";
    const actionOperation = activeRuntime.currentOperation?.action?.op;
    const compositionKey = `${activeRuntime.currentBeatId ?? ""}\u0000${activeRuntime.compositionTargets.join("\u0000")}`;
    const compositionChanged = compositionKey !== renderedCompositionRef.current;
    const compositionContentChanged =
      actionOperation === "board.create" ||
      actionOperation === "board.revise" ||
      actionOperation === "board.emphasize" ||
      actionOperation === "board.group" ||
      actionOperation === "board.connect";
    const compositionOperationChanged =
      compositionContentChanged &&
      activeRuntime.cursor !== renderedCompositionCursorRef.current;
    view?.setScene3dViews(activeRuntime.scene3dViews);
    view?.render(activeRuntime.board, activeRuntime.currentOperation);
    const nextRegionBounds = view?.getRegionBoundsMap() ?? {};
    setRuntimeRegionBounds((current) =>
      JSON.stringify(current) === JSON.stringify(nextRegionBounds)
        ? current
        : nextRegionBounds);
    const nextAttachmentBounds = view?.getAttachmentBoundsMap() ?? {};
    setRuntimeAttachmentBounds((current) =>
      JSON.stringify(current) === JSON.stringify(nextAttachmentBounds)
        ? current
        : nextAttachmentBounds);
    const nextVisualBounds = mounted
      ? measureVisualRegionBounds(activeRuntime.board, mounted.elements.nodes)
      : {};
    setRuntimeVisualRegionBounds((current) =>
      JSON.stringify(current) === JSON.stringify(nextVisualBounds)
        ? current
        : nextVisualBounds);
    const viewport = viewportRef.current;
    if (viewport) ensureScene3dInteractionHints(viewport);
    if (attentionTargets.length > 0 && attentionChanged) {
      view?.focusTargets(attentionTargets);
    } else if (
      activeRuntime.compositionTargets.length > 0 &&
      (compositionChanged || compositionOperationChanged)
    ) {
      // A Beat's declared focus describes the visual composition needed for
      // its narration. Apply it while the Beat is unfolding so a newly written
      // formula does not replace the diagram it is explaining. This reuses the
      // existing focus action and does not add a playback delay.
      view?.focusTargets(activeRuntime.compositionTargets);
    } else if (atPlaybackBoundary && focusChanged) {
      // React can batch every operation produced by advanceBeat() into the
      // boundary render. In that case the board already contains the new Beat
      // focus, but the view never observed the intermediate board.focus frame.
      view?.focusTargets(boardFocus);
    }
    renderedAttentionRef.current = attentionKey;
    renderedFocusRef.current = [...boardFocus];
    renderedCompositionRef.current = compositionKey;
    renderedCompositionCursorRef.current = activeRuntime.cursor;
  }, [
    runtime?.attentionTargets,
    runtime?.board,
    runtime?.compositionTargets,
    runtime?.currentOperation,
    runtime?.currentBeatId,
    runtime?.cursor,
    runtime?.scene3dViews,
  ]);

  useEffect(() => {
    if (!enhancementLayer || !onUpdateCourseRegion || courseRegions.length === 0) {
      return;
    }
    const measurementKey = JSON.stringify({
      courses: courseRegions.map((region) => ({
        id: region.id,
        origin: region.origin,
      })),
      topics: runtime?.outline.map((topic) => ({
        id: topic.id,
        questionId: topic.questionId,
      })),
      runtimeRegionBounds,
      presentations: coursePresentations.map((presentation) => ({
        topicId: presentation.topic.id,
        controls: presentation.controls.map((control) => control.alias),
        tasks: presentation.tasks.map((task) => ({
          id: task.task_id,
          status: task.status,
          attempts: task.attempts.length,
          hint: task.current_hint,
        })),
        controlsPosition: presentation.controlsPosition,
        tasksPosition: presentation.tasksPosition,
      })),
    });
    if (measuredCourseBoundsKeyRef.current === measurementKey) return;
    measuredCourseBoundsKeyRef.current = measurementKey;
    const frame = window.requestAnimationFrame(() => {
      const worldElements = [
        ...enhancementLayer.querySelectorAll<HTMLElement>([
          "[data-question-id]",
          "[data-loading-id]",
          "[data-course-controls-id]",
          "[data-course-tasks-id]",
        ].join(",")),
      ];
      for (const region of courseRegions) {
        const topic = runtime?.outline.find((candidate) =>
          candidate.questionId === region.questionId);
        const rects: WhiteboardRect[] = [];
        const runtimeRegionId = topic
          ? runtimeRegionIdForTopic(topic.id)
          : undefined;
        if (runtimeRegionId && runtimeRegionBounds[runtimeRegionId]) {
          rects.push(runtimeRegionBounds[runtimeRegionId]!);
        }
        for (const element of worldElements) {
          const belongs = element.dataset.questionId === region.questionId
            || element.dataset.loadingId === region.questionId
            || element.dataset.courseControlsId === region.id
            || element.dataset.courseTasksId === region.id;
          if (!belongs) continue;
          const bounds = renderedWorldRect(element);
          if (bounds) rects.push(bounds);
        }
        const bounds = unionWhiteboardRects(rects);
        onUpdateCourseRegion(region.id, {
          ...(runtimeRegionId ? { runtimeRegionId } : {}),
          ...(bounds ? { bounds } : {}),
        });
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    courseRegions,
    enhancementLayer,
    onUpdateCourseRegion,
    runtime?.outline,
    runtimeRegionBounds,
    runtimeRegionIdForTopic,
    coursePresentations,
  ]);

  useEffect(() => {
    const pendingCourseId = pendingComposerQuestion?.id
      ?? (loadingState?.kind === "lesson" ? loadingStateId : undefined);
    if (pendingCourseId) {
      coursesObservedInProgressRef.current.add(pendingCourseId);
    }

    const inferredTopic = presentationTopics.find((candidate) =>
      candidate.steps.some((step) => step.id === runtime?.currentStepId))
      ?? presentationTopics.at(-1);
    const requestedTopic = playbackCourseTarget
      ? presentationTopics.find((candidate) =>
          (candidate.questionId ?? candidate.id)
            === playbackCourseTarget.courseId)
      : undefined;
    const requestedTopicIsCurrent = requestedTopic
      ? !runtime?.currentStepId
        || requestedTopic.steps.some((step) => step.id === runtime.currentStepId)
      : false;
    const topic = requestedTopicIsCurrent ? requestedTopic : inferredTopic;
    if (!runtime || !topic) return;
    const courseId = topic.questionId ?? topic.id;
    // While a new composer turn is being prepared, the Runtime still ends in
    // the previous topic. Do not mistake that restored predecessor for the
    // course that is currently in progress.
    if (pendingCourseId && pendingCourseId !== courseId) return;
    const reachedCourseEnd = runtime.deliverySettled
      && (runtime.waiting || runtime.completed);
    if (!reachedCourseEnd) {
      const currentStepBelongsToCourse = Boolean(
        runtime.currentStepId
        && topic.steps.some((step) => step.id === runtime.currentStepId),
      );
      // Appending a new course briefly publishes its outline before the
      // player has entered that course's first Step. Keep the camera on the
      // new question/loading area during that hand-off; otherwise the last
      // focus operation from the previous course becomes visible for one
      // frame before the new course starts.
      if (!currentStepBelongsToCourse) return;
      // A restored lesson can briefly report delivery as unsettled while the
      // host hydrates it. Wait for hydration before choosing the last course;
      // otherwise the initial partial board can produce the wrong footprint.
      if (!runtime.completed) {
        const cameraController = cameraControllerRef.current;
        if (cameraController && !cameraController.canActivateCourse(courseId)) {
          return;
        }
        const enteredLoadingCourse = cameraController
          ?.markCourseActive(courseId) ?? false;
        if (
          enteredLoadingCourse
          || !coursesObservedInProgressRef.current.has(courseId)
        ) {
          mountedRef.current?.view.releaseHostCamera();
        }
        coursesObservedInProgressRef.current.add(courseId);
      }
      return;
    }
    const observedInProgress = coursesObservedInProgressRef.current.has(courseId);
    const restoringLastCourse = runtime.completed
      && !observedInProgress
      && restoredCourseFocusRef.current !== courseId
      && !pendingCourseId;
    if (!observedInProgress && !restoringLastCourse) return;

    const frame = window.requestAnimationFrame(() => {
      const view = mountedRef.current?.view;
      const viewport = viewportRef.current;
      if (!view || !viewport) return;
      const region = topic.questionId
        ? courseRegionByQuestion.get(topic.questionId)
        : undefined;
      const runtimeRegionId = runtimeRegionIdForTopic(topic.id);
      const rects: WhiteboardRect[] = [];
      const explicitlyOwnedNodeIds = topic.nodeIds ?? [];
      const currentCourseNodeIds = new Set(explicitlyOwnedNodeIds.length > 0
        ? explicitlyOwnedNodeIds
        : Object.values(runtime.board?.nodes ?? {})
            .filter((node) => node.region_id === runtimeRegionId)
            .map((node) => node.id));
      for (const element of viewport.querySelectorAll<HTMLElement>(
        ".board-node[data-id]",
      )) {
        if (!element.dataset.id || !currentCourseNodeIds.has(element.dataset.id)) {
          continue;
        }
        const bounds = renderedWorldRect(element);
        if (bounds) rects.push(bounds);
      }
      const latestRegionBounds = view.getRegionBoundsMap();
      const latestRuntimeBounds = latestRegionBounds[runtimeRegionId]
        ?? (presentationTopics.length === 1
          ? latestRegionBounds.__legacy__
          : undefined);
      // New multi-course sessions have explicit region ownership on every
      // lesson node. Prefer those currently rendered nodes over the persisted
      // monotonically-growing placement footprint: the latter is useful for
      // keeping future courses apart, but it is not a camera target and may
      // contain space reserved earlier in the session.
      if (currentCourseNodeIds.size === 0 && latestRuntimeBounds) {
        rects.push(latestRuntimeBounds);
      }

      if (enhancementLayer) {
        for (const element of enhancementLayer.querySelectorAll<HTMLElement>([
          "[data-question-id]",
          "[data-loading-id]",
          "[data-course-controls-id]",
          "[data-course-tasks-id]",
        ].join(","))) {
          const belongsToCourse = element.dataset.questionId === topic.questionId
            || element.dataset.loadingId === topic.questionId
            || element.dataset.courseControlsId === courseId
            || element.dataset.courseTasksId === courseId;
          if (!belongsToCourse) continue;
          const bounds = renderedWorldRect(element);
          if (bounds) rects.push(bounds);
        }
      }

      if (rects.length === 0 && region?.bounds) rects.push(region.bounds);

      const bounds = unionWhiteboardRects(rects);
      if (!bounds) return;
      cameraControllerRef.current?.request({
        source: restoringLastCourse ? "course-restore" : "course-end",
        key: restoringLastCourse
          ? `course-restore:${courseId}`
          : `course-end:${courseId}:${playbackCourseTarget?.sequence
            ?? runtime.cursor}`,
        courseId,
        rect: bounds,
      });
      if (restoringLastCourse) restoredCourseFocusRef.current = courseId;
      coursesObservedInProgressRef.current.delete(courseId);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    courseRegionByQuestion,
    enhancementLayer,
    loadingState?.kind,
    loadingStateId,
    pendingComposerQuestion?.id,
    playbackCourseTarget,
    presentationTopics,
    runtime,
    runtimeRegionIdForTopic,
  ]);

  useLayoutEffect(() => {
    if (availableTaskKeysSeededRef.current) return;
    availableTaskKeysRef.current = new Set(coursePresentations.flatMap((presentation) =>
      presentation.tasks.map((task) =>
        `${presentation.topic.id}\u0000${task.task_id}`)));
    availableTaskKeysSeededRef.current = true;
  }, [coursePresentations]);

  useEffect(() => {
    const currentKeys = new Set(coursePresentations.flatMap((presentation) =>
      presentation.tasks.map((task) =>
        `${presentation.topic.id}\u0000${task.task_id}`)));
    const previousKeys = availableTaskKeysRef.current;
    availableTaskKeysRef.current = currentKeys;
    // Restoring an existing whiteboard must not move its camera. Only a task
    // that becomes available while this board is mounted is a new teaching
    // event and may adjust the view once.
    const newlyAvailable = coursePresentations.filter((presentation) =>
      presentation.tasks.some((task) =>
        !previousKeys.has(`${presentation.topic.id}\u0000${task.task_id}`)));
    if (newlyAvailable.length === 0) return;
    const frame = window.requestAnimationFrame(() => {
      const view = mountedRef.current?.view;
      const viewport = viewportRef.current;
      const layer = enhancementLayer;
      const presentation = newlyAvailable.at(-1);
      if (!view || !viewport || !layer || !presentation) return;
      const courseId = presentation.topic.questionId ?? presentation.topic.id;
      const rects: WhiteboardRect[] = [];
      const focusIds = new Set(runtime?.board?.focus ?? []);
      for (const element of viewport.querySelectorAll<HTMLElement>(
        ".board-node[data-id], .board-group[data-id]",
      )) {
        if (!element.dataset.id || !focusIds.has(element.dataset.id)) continue;
        const bounds = renderedWorldRect(element);
        if (bounds) rects.push(bounds);
      }
      for (const element of layer.querySelectorAll<HTMLElement>(
        "[data-course-controls-id], [data-course-tasks-id]",
      )) {
        if (
          element.dataset.courseControlsId !== courseId
          && element.dataset.courseTasksId !== courseId
        ) continue;
        const bounds = renderedWorldRect(element);
        if (bounds) rects.push(bounds);
      }
      if (rects.length === 0) {
        const regionBounds = runtimeRegionBounds[
          runtimeRegionIdForTopic(presentation.topic.id)
        ] ?? (presentationTopics.length === 1
          ? runtimeRegionBounds.__legacy__
          : undefined);
        if (regionBounds) rects.push(regionBounds);
      }
      if (rects.length === 0) {
        rects.push({
          x: Math.min(
            presentation.controlsPosition.x,
            presentation.tasksPosition.x,
          ),
          y: Math.min(
            presentation.controlsPosition.y,
            presentation.tasksPosition.y,
          ),
          width: presentation.controls.length > 0 && presentation.tasks.length > 0
            ? 760
            : 372,
          height: 220,
        });
      }
      const bounds = unionWhiteboardRects(rects);
      if (bounds) {
        cameraControllerRef.current?.request({
          source: "student-task",
          key: `student-task:${courseId}:${presentation.tasks
            .map((task) => task.task_id).join(",")}`,
          courseId,
          rect: bounds,
        });
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    coursePresentations,
    enhancementLayer,
    presentationTopics.length,
    runtime?.board?.focus,
    runtime?.completed,
    runtime?.deliverySettled,
    runtime?.waiting,
    runtimeRegionBounds,
    runtimeRegionIdForTopic,
  ]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || typeof ResizeObserver === "undefined") return;
    let animationFrame = 0;
    let lastInsets = "";
    const update = () => {
      if (animationFrame) cancelAnimationFrame(animationFrame);
      animationFrame = requestAnimationFrame(() => {
        animationFrame = 0;
        const mounted = mountedRef.current;
        if (!mounted) return;
        const insets = learningBoardInsets(viewport);
        const signature = JSON.stringify(insets);
        if (signature === lastInsets) return;
        lastInsets = signature;
        mounted.view.setViewportInsets(insets);
      });
    };
    const observer = new ResizeObserver(() => {
      update();
    });
    observer.observe(viewport);
    let observedOcclusions = new Set<Element>();
    const syncOcclusions = () => {
      const next = new Set(
        viewport.ownerDocument.querySelectorAll<Element>(boardOcclusionSelector),
      );
      let changed = next.size !== observedOcclusions.size;
      for (const element of observedOcclusions) {
        if (next.has(element)) continue;
        observer.unobserve(element);
        changed = true;
      }
      for (const element of next) {
        if (observedOcclusions.has(element)) continue;
        observer.observe(element);
        changed = true;
      }
      observedOcclusions = next;
      if (changed) update();
    };
    const mutation = typeof MutationObserver === "undefined" ? null : new MutationObserver(() => {
      syncOcclusions();
    });
    const root = viewport.closest(".learning-workspace") ?? viewport.parentElement;
    if (root) mutation?.observe(root, { childList: true, subtree: true });
    syncOcclusions();
    update();
    return () => {
      if (animationFrame) cancelAnimationFrame(animationFrame);
      mutation?.disconnect();
      observer.disconnect();
    };
  }, []);

  return (
    <div className="learning-oll-board-shell">
      <div
        ref={viewportRef}
        className="learning-oll-board"
        data-testid="oll-lesson-board"
        aria-label="OLL 无限白板"
      />
      {!runtime
        && inkState.component_count === 0
        && composerQuestions.length === 0
        && !loadingState ? (
        <div className="learning-whiteboard-empty" aria-live="polite">
          <span>这块白板会保存我们的思考过程</span>
          <strong>向 Octos 提问，我们从这里开始</strong>
        </div>
      ) : null}
      {degradedVisuals.length > 0 ? (
        <aside
          className="learning-degraded-visuals"
          data-learning-board-occlusion=""
          aria-label="未完成的互动画面"
        >
          {degradedVisuals.map((degraded) => {
            const requested = requestedDegradedNodeIds.has(degraded.nodeId);
            const retrying = retryingDegradedNodeId === degraded.nodeId;
            return (
              <div key={degraded.nodeId} className="learning-degraded-visual">
                <div>
                  <strong>{degraded.title}</strong>
                  <span>{degraded.purpose}</span>
                </div>
                {onRetryDegradedVisual ? (
                  <button
                    type="button"
                    onClick={() => void retryDegradedVisual(degraded)}
                    disabled={retrying || requested}
                  >
                    <RotateCcw size={14} />
                    {retrying ? "正在重试" : requested ? "已发起重试" : "只重试这个画面"}
                  </button>
                ) : null}
              </div>
            );
          })}
        </aside>
      ) : null}
      {inkSessionId && inkAvailable ? (
        <div className="learning-ink-toolbar" data-learning-board-occlusion="" aria-label="白板书写工具">
          <button
            type="button"
            className={inkState.mode === "navigate" ? "is-active" : ""}
            onClick={() => setInkMode("navigate")}
            aria-label="浏览白板"
            aria-pressed={inkState.mode === "navigate"}
          >
            <Hand size={17} />
          </button>
          <button
            type="button"
            className={inkState.mode === "draw" ? "is-active" : ""}
            onClick={() => setInkMode("draw")}
            aria-label="书写笔迹"
            aria-pressed={inkState.mode === "draw"}
          >
            <PenLine size={17} />
          </button>
          <button
            type="button"
            className={inkState.mode === "erase" ? "is-active" : ""}
            onClick={() => setInkMode("erase")}
            aria-label="擦除笔迹"
            aria-pressed={inkState.mode === "erase"}
          >
            <Eraser size={17} />
          </button>
          <button
            type="button"
            className={inkState.mode === "select" && inkState.selection_mode === "rectangle" ? "is-active" : ""}
            onClick={() => {
              setSelectionMode("rectangle");
              setInkMode("select");
            }}
            aria-label="框选多个笔迹"
            aria-pressed={inkState.mode === "select" && inkState.selection_mode === "rectangle"}
            title="拖动矩形，可一次选中一条或多条笔迹"
          >
            <BoxSelect size={17} />
          </button>
          {inkColorPaletteAvailable ? (
            <>
              <button
                type="button"
                className={inkColorPaletteOpen ? "is-active" : ""}
                onClick={() => setInkColorPaletteOpen((current) => !current)}
                aria-label={inkColorPaletteOpen ? "隐藏调色板" : "显示调色板"}
                aria-expanded={inkColorPaletteOpen}
              >
                <Palette size={17} />
              </button>
              {inkColorPaletteOpen ? (
                <InkColorControl
                  label={inkState.mode === "draw" ? "笔色" : "选区颜色"}
                  value={inkState.mode === "draw"
                    ? inkState.pen_color
                    : inkState.selection_color ?? inkState.pen_color}
                  onChange={inkState.mode === "draw"
                    ? setPenColor
                    : setSelectionColor}
                />
              ) : null}
            </>
          ) : null}
          {inkState.mode === "select"
            && inkState.selected_count > 0
            && onAskInkSelection ? (
              <>
                {selectionClassificationStatus === "loading" ? (
                  <span className="learning-ink-classification-status">
                    正在识别选区…
                  </span>
                ) : null}
                {selectionRequestStatus ? (
                  <span
                    className="learning-ink-classification-status"
                    role="status"
                  >
                    {selectionRequestStatus}
                  </span>
                ) : null}
                {quickSelectionTools.map((tool) => (
                  <button
                    key={tool.id}
                    type="button"
                    className="learning-ink-quick-action"
                    onClick={() => void askSelection(
                      tool.prompt,
                      tool.id,
                      tool.requestContentKind,
                      [],
                    )}
                    disabled={selectionRequestPending}
                  >
                    {tool.label}
                  </button>
                ))}
                <button
                  type="button"
                  className="learning-ink-ask"
                  onClick={() => void openSelectionQuestion()}
                  aria-expanded={selectionQuestionOpen}
                  title={selectionClassificationStatus === "error"
                    ? "未能自动识别选区，可在提问面板中手动选择内容类型"
                    : undefined}
                  disabled={selectionRequestPending}
                >
                  <MessageCircle size={16} />
                  问小章鱼
                </button>
              </>
            ) : null}
          <button
            type="button"
            className="learning-ink-select-all"
            onClick={selectAllInk}
            aria-label="选择全部笔迹"
          >
            全选
          </button>
          <button
            type="button"
            onClick={() => runInkHistory("undo")}
            aria-label="撤销笔迹"
          >
            <Undo2 size={17} />
          </button>
          <button
            type="button"
            onClick={() => runInkHistory("redo")}
            aria-label="重做笔迹"
          >
            <Redo2 size={17} />
          </button>
          <span className="learning-ink-status" role="status">
            {inkState.component_count} 项笔迹
            {inkState.selected_count > 0
              ? ` · 已选 ${inkState.selected_count}`
              : ""}
            {inkState.saved ? " · 已保存" : " · 保存中"}
          </span>
        </div>
      ) : null}
      {selectionQuestionOpen && inkState.selected_count > 0 ? (
        <form
          className="learning-selection-question"
          data-learning-board-occlusion=""
          onSubmit={(event) => {
            event.preventDefault();
            void askSelection(selectionQuestion);
          }}
        >
          <header>
            <strong>针对当前选区提问</strong>
            <span>
              将发送 {inkState.selected_count} 项选中笔迹，以及你在下面明确选择的局部白板内容；不会发送整块白板。
            </span>
          </header>
          {preparedSelection?.candidates.length ? (
            <fieldset className="learning-selection-targets">
              <legend>这段笔迹是在问哪部分白板内容？</legend>
              <label>
                <input
                  type="radio"
                  name="selection-board-target"
                  checked={selectedBoardTargetIds.length === 0}
                  onChange={() => setSelectedBoardTargetIds([])}
                />
                只看我的笔迹
              </label>
              {preparedSelection.candidates.map((candidate) => (
                <label
                  key={candidate.target_id}
                  className={selectedBoardTargetIds.includes(candidate.target_id) ? "is-selected" : ""}
                >
                  <input
                    type="radio"
                    name="selection-board-target"
                    checked={selectedBoardTargetIds.includes(candidate.target_id)}
                    onChange={() => setSelectedBoardTargetIds([candidate.target_id])}
                  />
                  <span>{candidate.label ?? candidate.target_id}</span>
                  <small>{boardTargetKindLabels[candidate.kind] ?? "局部内容"}</small>
                </label>
              ))}
            </fieldset>
          ) : (
            <p className="learning-selection-no-target">
              当前框选没有覆盖课程对象，本次只参考你的原始笔迹。
            </p>
          )}
          <label>
            我写的内容更像
            <select
              value={selectionContentKind}
              onChange={(event) =>
                setSelectionContentKind(
                  event.target.value as SelectionContentKind,
                )}
            >
              <option value="unknown">暂不确定</option>
              <option value="text">文字</option>
              <option value="math">公式</option>
              <option value="geometry">图形</option>
              <option value="data">数据</option>
            </select>
          </label>
          {selectionClassification ? (
            <p className="learning-selection-classification">
              自动识别为：{selectionContentKindLabels[selectionClassification.kind]}
              {selectionClassification.content
                ? `（${selectionClassification.content}）`
                : ""}
              {selectionClassification.confidence === "low"
                ? "；把握较低，请手动确认"
                : ""}
            </p>
          ) : selectionClassificationStatus === "error" ? (
            <p className="learning-selection-classification">
              没有可靠识别出内容，请手动选择类型。
            </p>
          ) : null}
          <p className="learning-selection-action-label">
            <strong>接下来让小章鱼做什么？</strong>
            <small>下面是操作，不会改变上面已经确认的选区。</small>
          </p>
          <div className="learning-selection-suggestions">
            {availableSelectionTools(
              selectionContentKind,
              preparedSelection?.candidates
                .filter((candidate) => selectedBoardTargetIds.includes(candidate.target_id))
                .map((candidate) => candidate.kind),
            ).map((tool) => (
              <button
                key={tool.id}
                type="button"
                onClick={() => void askSelection(
                  tool.prompt,
                  tool.id,
                  tool.requestContentKind,
                )}
                disabled={selectionRequestPending}
              >
                {tool.label}
              </button>
            ))}
            {onReferenceInkSelection ? (
              <button
                type="button"
                onClick={() => void referenceSelectionForLesson()}
                disabled={selectionRequestPending}
              >
                {selectionLessonTool.label}
              </button>
            ) : null}
          </div>
          <div className="learning-selection-question-input">
            <input
              value={selectionQuestion}
              onChange={(event) => setSelectionQuestion(event.target.value)}
              placeholder="例如：这一步为什么不对？"
              aria-label="针对选区的问题"
              disabled={selectionRequestPending}
            />
            {onVoiceInkSelection ? (
              <button
                type="button"
                onClick={() => void askSelectionByVoice()}
                disabled={selectionRequestPending}
                aria-label="针对当前选区语音提问"
              >
                <Mic size={16} />
              </button>
            ) : null}
            <button
              type="submit"
              disabled={
                selectionRequestPending || !selectionQuestion.trim()
              }
            >
              发送
            </button>
          </div>
        </form>
      ) : null}
      {inkError ? (
        <div className="learning-ink-error" role="alert">
          笔迹功能：{inkError}
        </div>
      ) : null}
      {enhancementLayer
        ? createPortal(
            <>
              {composerQuestions.map((question, index) => (
                <WhiteboardQuestionCard
                  key={question.id}
                  question={question}
                  left={question.position?.x
                    ?? lessonLoadingPosition.left
                      - WHITEBOARD_QUESTION_CARD_WIDTH - 24}
                  top={question.position?.y
                    ?? lessonLoadingPosition.top + index * 150}
                />
              ))}
              {loadingState ? (
                <WhiteboardLoadingBlock
                  state={loadingState}
                  left={pendingComposerQuestion?.position
                    ? pendingComposerQuestion.position.x
                      + WHITEBOARD_QUESTION_CARD_WIDTH + 24
                    : lessonLoadingPosition.left}
                  top={pendingComposerQuestion?.position?.y
                    ?? lessonLoadingPosition.top}
                />
              ) : null}
              {coursePresentations.map((presentation) => {
                const courseId = presentation.topic.questionId
                  ?? presentation.topic.id;
                return (
                  <div key={`course-ui:${presentation.id}`}>
                    {presentation.controls.length > 0 ? (
                      <div
                        className="learning-variable-controls is-world"
                        style={{
                          left: presentation.controlsPosition.x,
                          top: presentation.controlsPosition.y,
                          width: 360,
                        }}
                        data-course-controls-id={courseId}
                        data-interaction-controls-id={presentation.id}
                        data-oll-ink-input="ignore"
                        aria-label={`${presentation.topic.title}的课程变量控制`}
                        data-testid="oll-variable-controls"
                      >
                        {presentation.controls.map((control) => {
                          const inputId = `oll-variable-${control.alias}`;
                          return (
                            <div
                              className={runtime?.activeVariableAnimation?.variable === control.alias
                                ? "learning-variable-control is-animating"
                                : "learning-variable-control"}
                              key={control.alias}
                            >
                              <label htmlFor={inputId}>{control.label}</label>
                              <input
                                id={inputId}
                                type="range"
                                min={control.min}
                                max={control.max}
                                step={control.step}
                                value={control.value}
                                onPointerDown={(event) => {
                                  startSliderOperation(
                                    control.alias,
                                    control.value,
                                    studentInputMethod(event.pointerType),
                                  );
                                }}
                                onKeyDown={(event) => {
                                  if ([
                                    "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown",
                                    "Home", "End", "PageUp", "PageDown",
                                  ].includes(event.key)) {
                                    startSliderOperation(
                                      control.alias,
                                      control.value,
                                      "keyboard",
                                    );
                                  }
                                }}
                                onChange={(event) => {
                                  updateSliderOperation(
                                    control.alias,
                                    Number(event.target.value),
                                  );
                                }}
                                onPointerUp={(event) => {
                                  commitSliderOperation(
                                    control.alias,
                                    Number(event.currentTarget.value),
                                  );
                                }}
                                onPointerCancel={(event) => {
                                  commitSliderOperation(
                                    control.alias,
                                    Number(event.currentTarget.value),
                                  );
                                }}
                                onKeyUp={(event) => {
                                  commitSliderOperation(
                                    control.alias,
                                    Number(event.currentTarget.value),
                                  );
                                }}
                                onBlur={(event) => {
                                  commitSliderOperation(
                                    control.alias,
                                    Number(event.currentTarget.value),
                                  );
                                }}
                                aria-label={control.label}
                              />
                              <output>
                                {formatVariableValue(control.value, control.unit)}
                              </output>
                              <button
                                type="button"
                                onClick={() => {
                                  const initial = runtime?.board
                                    ?.variables?.[control.alias]?.initial;
                                  if (runtime && typeof initial === "number") {
                                    const operationId = runtime.handleStudentVariableInput(
                                      control.alias,
                                      control.value,
                                      {
                                        phase: "start",
                                        control: "reset",
                                        input: "unknown",
                                      },
                                    );
                                    runtime.handleStudentVariableInput(
                                      control.alias,
                                      initial,
                                      {
                                        phase: "commit",
                                        control: "reset",
                                        input: "unknown",
                                        ...(typeof operationId === "string"
                                          ? { operation_id: operationId }
                                          : {}),
                                      },
                                    );
                                  }
                                }}
                                aria-label={`复位${control.label}`}
                              >
                                复位
                              </button>
                            </div>
                          );
                        })}
                        <small>
                          {runtime?.activeVariableAnimation
                            ? "老师正在演示这个变量，结束后即可继续拖动"
                            : "讲解过程中也可以拖动；老师演示同一变量时会暂时接管"}
                        </small>
                      </div>
                    ) : null}
                    {presentation.tasks.length > 0 ? (
                      <section
                        className="learning-student-tasks is-world"
                        style={{
                          left: presentation.tasksPosition.x,
                          top: presentation.tasksPosition.y,
                          width: 330,
                        }}
                        data-course-tasks-id={courseId}
                        data-interaction-tasks-id={presentation.id}
                        data-oll-ink-input="ignore"
                        aria-label={`${presentation.topic.title}的动手任务`}
                        data-testid="oll-student-tasks"
                      >
                        <header>
                          <span>动手试一试</span>
                          <small>直接操作白板上的图形、视角或控制器</small>
                        </header>
                        {presentation.tasks.map((task) => {
                          const lastAttempt = task.attempts.at(-1);
                          const attempts = task.attempts.length;
                          return (
                            <article
                              key={task.task_id}
                              className={`learning-student-task is-${task.status}`}
                              aria-live="polite"
                            >
                              <p>{task.prompt}</p>
                              {task.status === "succeeded" ? (
                                <div className="learning-student-task-feedback is-success">
                                  <CheckCircle2 size={17} />
                                  <span>
                                    {task.success_message
                                      ?? "完成得很好，已经达到目标。"}
                                  </span>
                                </div>
                              ) : lastAttempt ? (
                                <div className="learning-student-task-feedback">
                                  <span>
                                    {task.status === "needs_hint"
                                      ? "还没达到目标，可以查看提示后再试。"
                                      : "已经记录这次操作，再调整一下试试。"}
                                  </span>
                                  <small>已尝试 {attempts} 次</small>
                                </div>
                              ) : (
                                <div className="learning-student-task-feedback">
                                  <span>轮到你操作了，完成后这里会立即反馈。</span>
                                </div>
                              )}
                              {task.current_hint ? (
                                <div
                                  className="learning-student-task-hint"
                                  role="status"
                                >
                                  <Lightbulb size={16} />
                                  <span>{task.current_hint}</span>
                                </div>
                              ) : null}
                              {task.status !== "succeeded" && attempts > 0 ? (
                                <div className="learning-student-task-actions">
                                  {task.hints_revealed < task.hints.length ? (
                                    <button
                                      type="button"
                                      onClick={() => requestTaskHint(task.task_id)}
                                    >
                                      <Lightbulb size={15} />
                                      {task.current_hint ? "下一个提示" : "给我提示"}
                                    </button>
                                  ) : null}
                                  <button
                                    type="button"
                                    onClick={() => retryTask(task.task_id)}
                                  >
                                    <RotateCcw size={15} />
                                    重新开始
                                  </button>
                                </div>
                              ) : null}
                            </article>
                          );
                        })}
                        {taskError ? (
                          <div
                            className="learning-student-task-error"
                            role="alert"
                          >
                            {taskError}
                          </div>
                        ) : null}
                      </section>
                    ) : null}
                  </div>
                );
              })}
              {selectionQuestionOpen
                ? preparedSelection?.candidates.map((candidate, index) => (
                    <div
                      key={candidate.target_id}
                      className={selectedBoardTargetIds.includes(candidate.target_id)
                        ? "learning-selection-target-highlight is-selected"
                        : "learning-selection-target-highlight"}
                      style={{
                        left: candidate.world_bounds.x,
                        top: candidate.world_bounds.y,
                        width: candidate.world_bounds.width,
                        height: candidate.world_bounds.height,
                      }}
                      aria-hidden="true"
                    >
                      <span>{index + 1}</span>
                    </div>
                  ))
                : null}
              <SelectionEnhancementLayer
                artifacts={selectionEnhancements}
                sources={selectionSources}
                questions={questions}
                loading={selectionRequestStatus && selectionLoadingSource
                  ? {
                      turnId: selectionLoadingQuestion?.id
                        ?? `selection:${selectionLoadingSource.sourceId}`,
                      sourceId: selectionLoadingSource.sourceId,
                      bounds: selectionLoadingSource.bounds,
                      state: {
                        id: `selection:${selectionLoadingSource.sourceId}`,
                        kind: "selection",
                        title: selectionRequestStatus.replace(/…$/, ""),
                        detail: selectionRequestStatus.includes("函数图像")
                          ? "正在识别公式，并把可查看的图像放在选区旁边。"
                          : "正在理解这部分内容，并把辅助说明放在选区旁边。",
                      },
                    }
                  : null}
                currentDocumentVersion={inkState.document_version}
                invalidTargetTurnIds={new Set(selectionEnhancements
                  .filter((artifact) =>
                    Boolean(artifact.board?.targets.length)
                    && !selectionArtifactTargetsExist(
                      artifact,
                      runtime?.board ?? null,
                    ),
                  )
                  .map((artifact) => artifact.turn_id))}
                onDelete={(turnId) =>
                  onDeleteSelectionEnhancement?.(turnId)}
              />
            </>,
            enhancementLayer,
          )
        : null}
    </div>
  );
}

/** @deprecated Use LearningWhiteboard. Kept for callers that still name the
 * shared whiteboard after its optional OLL course layer. */
export const OllLessonBoard = LearningWhiteboard;
