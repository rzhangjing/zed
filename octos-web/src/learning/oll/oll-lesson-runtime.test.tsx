import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useEffect, useState, type ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import geometryLessonSource from "./fixtures/geometry-auxiliary-line-v2.canonical.jsonl?raw";
import unitCircleSineLessonSource from "./fixtures/unit-circle-sine.canonical.jsonl?raw";
import {
  normalizeAuthoringLesson,
  type AuthoringLesson,
  type CanonicalEvent,
} from "octos-lesson-language";
import type {
  InkMode,
  InkRuntimeState,
} from "octos-lesson-language/ink-runtime";
import {
  INK_SELECTION_FORMAT,
  INK_SELECTION_FORMAT_VERSION,
  type InkSelectionSnapshot,
} from "octos-lesson-language/ink-runtime";
import {
  InfiniteBoardView,
  type StudentTaskSnapshot,
} from "octos-lesson-language/web-runtime";
import { OllLessonBoard } from "./oll-lesson-runtime";
import { WHITEBOARD_QUESTION_CARD_WIDTH } from "../whiteboard-question-card";
import { createCourseRegion } from "../course-regions";
import type { SelectionClassification } from "../selection-enhancements";
import { isLessonDeliverySettled } from "./lesson-delivery";
import { useOllLessonRuntime } from "./use-oll-lesson-runtime";

const mountInkRuntimeMock = vi.hoisted(() => vi.fn());
const selectionContextToPngFileMock = vi.hoisted(() => vi.fn(async () =>
  new File(["selection"], "selection.png", { type: "image/png" }),
));
const selectionSnapshotToPngFileMock = vi.hoisted(() => vi.fn(async () =>
  new File(["selection-only"], "selection-only.png", { type: "image/png" }),
));

vi.mock("./oll-ink-runtime", () => ({
  mountInkRuntime: mountInkRuntimeMock,
}));

vi.mock("../selection-enhancements", async (importOriginal) => ({
  ...await importOriginal<typeof import("../selection-enhancements")>(),
  selectionContextToPngFile: selectionContextToPngFileMock,
  selectionSnapshotToPngFile: selectionSnapshotToPngFileMock,
}));

function RuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-runtime-test",
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <span data-testid="progress">
        {runtime.cursor}/{runtime.totalOperations}
      </span>
      <button type="button" onClick={runtime.nextBeat}>下一 Beat</button>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function CameraRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-camera-runtime-test",
  });
  const [, rerenderHostUi] = useState(0);
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <button type="button" onClick={runtime.nextBeat}>下一 Beat</button>
      <button type="button" onClick={() => rerenderHostUi((value) => value + 1)}>
        更新旁边界面
      </button>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function InkRuntimeProbe({
  onInkActivity,
}: {
  onInkActivity?: () => void;
} = {}) {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-ink-runtime-test",
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard
        runtime={runtime}
        inkSessionId="learn-ink-1"
        onInkActivity={onInkActivity}
      />
    </div>
  );
}

function BlankToLessonWhiteboardProbe() {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "blank-to-lesson-runtime-test",
    startAtEnd: true,
  });
  const [showLesson, setShowLesson] = useState(false);
  return (
    <div style={{ width: 1200, height: 800 }}>
      <button type="button" onClick={() => setShowLesson(true)}>
        显示课程内容
      </button>
      <OllLessonBoard
        runtime={showLesson ? runtime : null}
        inkSessionId="blank-to-lesson-ink"
      />
    </div>
  );
}

function QuestionPlacementProbe({
  onPlaceQuestion,
  inkSessionId,
  pending = false,
  showLoading = pending,
  withCourseRegion = false,
  withTallNarrative = false,
  recovered = false,
}: {
  onPlaceQuestion: (questionId: string, position: { x: number; y: number }) => void;
  inkSessionId?: string;
  pending?: boolean;
  showLoading?: boolean;
  withCourseRegion?: boolean;
  withTallNarrative?: boolean;
  recovered?: boolean;
}) {
  const runtime = useOllLessonRuntime({
    source: unitCircleSineLessonSource,
    storageKey: "question-placement-runtime-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  const board = withTallNarrative && runtime.board ? {
    ...runtime.board,
    nodes: {
      ...runtime.board.nodes,
      ...Object.fromEntries(Array.from({ length: 10 }, (_, index) => [
        `late-narrative-${index}`,
        {
          id: `late-narrative-${index}`,
          kind: "note",
          content: {
            title: `补充讲解 ${index + 1}`,
            text: "右侧讲解继续增长，但不应推动左侧图形下方的滑块。",
          },
          placement: { relation: "new_region" },
        },
      ])),
    },
  } : runtime.board;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard
        runtime={{
          ...runtime,
          board,
          outline: runtime.outline.map((topic) => ({
            ...topic,
            questionId: "lesson-unit-circle-sine-001",
          })),
        }}
        inkSessionId={inkSessionId}
        loadingState={showLoading ? {
          id: "lesson-unit-circle-sine-001",
          kind: "lesson",
          title: "正在搭建这节课",
          detail: "请稍等",
        } : null}
        questions={[{
          id: "lesson-unit-circle-sine-001",
          sessionId: "question-placement",
          text: "请结合单位圆解释正弦函数",
          origin: "composer",
          createdAt: "2026-08-17T00:00:00.000Z",
          status: pending ? "pending" : "answered",
          ...(pending || recovered
            ? {}
            : { position: { x: 2_400, y: 160 } }),
        }]}
        courseRegions={withCourseRegion ? [createCourseRegion(
          "question-placement",
          "lesson-unit-circle-sine-001",
          { x: 2_400, y: 160 },
          { width: 654, height: 220 },
          "2026-08-17T00:00:00.000Z",
        )] : []}
        onPlaceQuestion={onPlaceQuestion}
      />
    </div>
  );
}

function PendingQuestionFocusProbe({ showLoading = true }: { showLoading?: boolean }) {
  const runtime = useOllLessonRuntime({
    source: unitCircleSineLessonSource,
    storageKey: "pending-question-focus-runtime-test",
    startAtEnd: true,
  });
  const [, updateHostUi] = useState(0);
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <button type="button" onClick={() => updateHostUi((value) => value + 1)}>
        更新旁边界面
      </button>
      <OllLessonBoard
        runtime={runtime}
        loadingState={showLoading ? {
          id: "turn-new-topic",
          kind: "lesson",
          title: "正在搭建这节课",
          detail: "请稍等",
        } : null}
        questions={[{
          id: "turn-new-topic",
          sessionId: "pending-question-focus",
          text: "继续讲自然对数",
          origin: "composer",
          createdAt: "2026-08-19T00:00:00.000Z",
          status: "pending",
          position: { x: 2_400, y: 160 },
        }]}
      />
    </div>
  );
}

function CompletedCourseFocusProbe({
  restored = false,
  legacyNodeMetadata = false,
  taskAppearsAtEnd = false,
  completesAtEnd = false,
  onUpdateCourseRegion,
}: {
  restored?: boolean;
  legacyNodeMetadata?: boolean;
  taskAppearsAtEnd?: boolean;
  completesAtEnd?: boolean;
  onUpdateCourseRegion?: ComponentProps<typeof OllLessonBoard>["onUpdateCourseRegion"];
}) {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "completed-course-focus-runtime-test",
    startAtEnd: true,
  });
  const [settled, setSettled] = useState(false);
  const [taskAvailable, setTaskAvailable] = useState(false);
  const [playbackCourseTarget, setPlaybackCourseTarget] = useState<{
    courseId: string;
    sequence: number;
  } | null>(null);
  const [lateTeachingFocus, setLateTeachingFocus] = useState(false);
  const [, updateHostUi] = useState(0);
  if (!runtime || !runtime.board || runtime.outline.length === 0) return null;
  const oldRegion = createCourseRegion(
    "completed-course-focus",
    "old-course",
    { x: 100, y: 120 },
    { width: 1_100, height: 760 },
    "2026-08-17T00:00:00.000Z",
  );
  const currentRegion = createCourseRegion(
    "completed-course-focus",
    "current-course",
    { x: 2_400, y: 180 },
    { width: 1_180, height: 820 },
    "2026-08-17T00:01:00.000Z",
  );
  // Placement bounds grow monotonically so later courses can avoid occupied
  // space. Simulate a stale/corrupted footprint that spans the old course:
  // the completion camera must use currently rendered course nodes instead.
  currentRegion.bounds = { x: 100, y: 120, width: 3_480, height: 880 };
  const currentNodeIds = Object.keys(runtime.board.nodes);
  const firstCurrentNode = Object.values(runtime.board.nodes)[0]!;
  const lessonNodes = Object.fromEntries(Object.entries(runtime.board.nodes).map(
    ([nodeId, node]) => [nodeId, {
      ...node,
      region_id: legacyNodeMetadata ? "__legacy__" : runtime.outline[0]!.id,
    }],
  ));
  if (legacyNodeMetadata) {
    lessonNodes["old-course-node"] = {
      ...structuredClone(firstCurrentNode),
      id: "old-course-node",
      region_id: "__legacy__",
      placement: {
        relation: "new_region",
        region_role: "lesson-origin",
      },
    };
  }
  const studentTask = {
    task_id: "current-course-task",
    status: "not_started",
    hints_revealed: 0,
    attempts: [],
    available: true,
    prompt: "完成当前课程的动手操作",
    hints: [],
  } satisfies StudentTaskSnapshot;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <button type="button" onClick={() => {
        if (taskAppearsAtEnd) setTaskAvailable(true);
        setSettled(true);
      }}>
        结束当前课程
      </button>
      <button type="button" onClick={() => {
        setPlaybackCourseTarget({ courseId: "current-course", sequence: 1 });
        setSettled(true);
      }}>
        从目录重播当前课程
      </button>
      <button type="button" onClick={() => updateHostUi((value) => value + 1)}>
        更新结束界面
      </button>
      <button type="button" onClick={() => setLateTeachingFocus(true)}>
        模拟结束后晚到的板书操作
      </button>
      <OllLessonBoard
        runtime={{
          ...runtime,
          board: {
            ...runtime.board,
            nodes: lessonNodes,
          },
          completed: restored || (completesAtEnd && settled),
          waiting: !(restored || (completesAtEnd && settled)),
          deliverySettled: settled,
          currentOperation: lateTeachingFocus ? {
            operation_id: "late-teaching-focus",
            type: "action.apply",
            lesson_id: runtime.currentOperation?.lesson_id
              ?? "lesson-unit-circle-sine-001",
            event_index: runtime.currentOperation?.event_index ?? 0,
            action: {
              action_id: "late-teaching-focus",
              op: "board.emphasize",
              target: { node_id: "old-course-node" },
              focus: { targets: ["old-course-node"], intent: "detail" },
            },
          } : runtime.currentOperation,
          studentTasks: taskAvailable ? [studentTask] : [],
          outline: [{
            id: "old-topic",
            title: "旧课程",
            steps: [],
            questionId: "old-course",
            ...(legacyNodeMetadata ? { nodeIds: ["old-course-node"] } : {}),
          }, {
            ...runtime.outline[0]!,
            questionId: "current-course",
            nodeIds: currentNodeIds,
            ...(taskAppearsAtEnd
              ? { taskAliases: [studentTask.task_id] }
              : {}),
          }],
        }}
        courseRegions={[oldRegion, currentRegion]}
        playbackCourseTarget={playbackCourseTarget}
        onUpdateCourseRegion={onUpdateCourseRegion}
      />
    </div>
  );
}

function SelectionInkRuntimeProbe({
  onAsk,
  onClassify,
}: {
  onAsk: (request: {
    snapshot: InkSelectionSnapshot;
    question: string;
  }) => Promise<void>;
  onClassify: (request: {
    snapshot: InkSelectionSnapshot;
    selectionImage: File;
  }) => Promise<SelectionClassification>;
}) {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-selection-runtime-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <span data-testid="selection-operation-count">
        {runtime.studentOperations.filter(
          (operation) => operation.kind === "ink_selection",
        ).length}
      </span>
      <OllLessonBoard
        runtime={runtime}
        inkSessionId="learn-selection-1"
        onClassifyInkSelection={onClassify}
        onAskInkSelection={onAsk}
      />
    </div>
  );
}

function SelectionSourceLifecycleProbe({
  source,
  onDeleteSources,
}: {
  source: InkSelectionSnapshot;
  onDeleteSources: (sourceIds: string[]) => void;
}) {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-selection-source-lifecycle-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard
        runtime={runtime}
        inkSessionId="selection-source-lifecycle"
        selectionSources={[source]}
        onDeleteSelectionSources={onDeleteSources}
      />
    </div>
  );
}

function ReviewRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-review-runtime-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div>
      <span data-testid="review-progress">
        {runtime.cursor}/{runtime.totalOperations}
      </span>
      <span data-testid="review-playing">{String(runtime.playing)}</span>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function VariableRuntimeProbe({
  onVariable,
}: {
  onVariable?: (value: number) => void;
} = {}) {
  const runtime = useOllLessonRuntime({
    source: unitCircleSineLessonSource,
    storageKey: "oll-variable-runtime-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <span data-testid="student-operation-count">
        {runtime.studentOperations.length}
      </span>
      <span data-testid="student-operation-controls">
        {runtime.studentOperations.map((operation) => operation.control).join(",")}
      </span>
      <OllLessonBoard
        runtime={onVariable
          ? {
              ...runtime,
              handleStudentVariableInput: (alias, value, event) => {
                if (event.control === "geometry_point" && event.phase === "update") {
                  onVariable(value);
                }
                return runtime.handleStudentVariableInput(alias, value, event);
              },
            }
          : runtime}
      />
    </div>
  );
}

function CurrentTopicVariableRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: unitCircleSineLessonSource,
    storageKey: "oll-current-topic-variable-runtime-test",
    startAtEnd: true,
  });
  const currentVariable = runtime?.board?.variables?.theta;
  if (!runtime || !runtime.board || !currentVariable) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard runtime={{
        ...runtime,
        board: {
          ...runtime.board,
          variables: {
            old_lesson_number: {
              ...currentVariable,
              label: "旧课程参数",
            },
            theta: currentVariable,
          },
        },
        outline: [{
          id: "current-topic",
          title: "当前课程",
          steps: runtime.outline.flatMap((topic) => topic.steps),
          variableAliases: ["theta"],
        }],
      }} />
    </div>
  );
}

function StudentTaskRuntimeProbe({ startAtEnd = true }: { startAtEnd?: boolean }) {
  const runtime = useOllLessonRuntime({
    source: unitCircleSineLessonSource,
    storageKey: "oll-student-task-runtime-test",
    startAtEnd,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

const incrementalStudentTaskLessonSource = unitCircleSineLessonSource
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line) as CanonicalEvent)
  .filter((event) => event.event !== "lesson.close")
  .map((event) => JSON.stringify(event))
  .join("\n");

function IncrementalStudentTaskRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: incrementalStudentTaskLessonSource,
    storageKey: "oll-incremental-student-task-runtime-test",
    incremental: true,
    startAtEnd: true,
  });
  const setDeliverySettled = runtime?.setDeliverySettled;
  const deliveryReachedCurrentEnd = Boolean(
    runtime && isLessonDeliverySettled(runtime, false),
  );
  useEffect(() => {
    if (deliveryReachedCurrentEnd) setDeliverySettled?.(true);
  }, [deliveryReachedCurrentEnd, setDeliverySettled]);
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function OutlineRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: geometryLessonSource,
    storageKey: "oll-outline-runtime-test",
    startAtEnd: true,
    topics: [{
      id: "geometry",
      title: "几何证明",
      stepIds: geometryEvents.flatMap((event) =>
        event.step ? [event.step.id] : [],
      ),
    }],
  });
  if (!runtime) return null;
  const firstStep = runtime.outline[0]?.steps[0];
  return (
    <div>
      <span data-testid="outline-progress">
        {runtime.cursor}/{runtime.totalOperations}
      </span>
      <span data-testid="outline-topic">{runtime.outline[0]?.title}</span>
      <span data-testid="outline-current">{runtime.currentStepId}</span>
      <button
        type="button"
        onClick={() => firstStep && runtime.viewStep(firstStep.id)}
      >
        查看第一步
      </button>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

const geometryEvents = geometryLessonSource
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line) as CanonicalEvent);

const scene3dLesson: AuthoringLesson = {
  dsl: "octos.lesson",
  version: "0.1",
  profile: "authoring",
  lesson: {
    mode: "explain",
    language: "zh-CN",
    title: "可旋转立方体",
    goals: ["从不同方向观察立方体"],
    tasks: [{
      as: "find-front-view",
      prompt: "把立方体转到正视图",
      availability: { kind: "after_lesson" },
      allowed_operations: [{
        kind: "scene3d_view",
        node: "cube-scene",
        controls: ["orbit", "preset", "reset"],
      }],
      completion: {
        kind: "scene3d_view_target",
        node: "cube-scene",
        yaw: 0,
        pitch: 0,
        zoom: 1,
        angular_tolerance: 0.04,
        zoom_tolerance: 0.04,
      },
      hints: ["可以拖动观察，也可以使用正视按钮。"],
      hint_after_attempts: 1,
      success_message: "正确，这是立方体的正视图。",
    }],
  },
  steps: [{
    key: "show-cube",
    purpose: "建立立方体的三维空间表示",
    beats: [{
      key: "inspect-cube",
      say: "拖动立方体，从不同方向观察它的面和棱。",
      actions: [{
        do: "write",
        as: "cube-scene",
        kind: "scene3d",
        role: "diagram",
        content: {
          title: "立方体",
          fallback: "一个中心在原点的立方体，标出了顶点 A、棱 AB 和顶面。",
          axes: true,
          camera: { yaw: 0.72, pitch: 0.55, zoom: 1 },
          objects: [{
            as: "cube",
            kind: "box",
            color: "teal",
            center: { x: 0, y: 0, z: 0 },
            size: { x: 2, y: 2, z: 2 },
          }],
          highlights: [{
            as: "vertex-a",
            kind: "point",
            points: [{ x: -1, y: -1, z: 1 }],
            label: "顶点 A",
            color: "red",
          }, {
            as: "edge-ab",
            kind: "edge",
            points: [{ x: -1, y: -1, z: 1 }, { x: 1, y: -1, z: 1 }],
            label: "棱 AB",
            color: "orange",
          }, {
            as: "top-face",
            kind: "face",
            points: [
              { x: -1, y: -1, z: 1 },
              { x: 1, y: -1, z: 1 },
              { x: 1, y: 1, z: 1 },
              { x: -1, y: 1, z: 1 },
            ],
            label: "顶面",
            color: "purple",
          }],
        },
        place: { relation: "new_region" },
      }, {
        do: "focus",
        when: "after_speech",
        targets: ["cube-scene"],
        intent: "current_step",
      }],
    }],
  }],
  close: {
    summary: "完成立方体多视角观察。",
    focus: ["cube-scene"],
  },
};

const scene3dLessonSource = normalizeAuthoringLesson(scene3dLesson, {
  lessonId: "scene3d-product-test",
  boardId: "scene3d-board",
  baseRevision: 0,
}).map((event) => JSON.stringify(event)).join("\n");

const degradedVisualLesson: AuthoringLesson = {
  dsl: "octos.lesson",
  version: "0.1",
  profile: "authoring",
  lesson: {
    mode: "explain",
    language: "zh-CN",
    title: "局部降级课程",
    goals: ["继续可播放的课程"],
  },
  steps: [{
    key: "degraded-visual",
    purpose: "一个画面不可用时继续课程",
    beats: [{
      key: "show-status",
      say: "这个画面暂时不可用，我们继续后面的内容。",
      actions: [{
        do: "write",
        as: "paraboloid-scene",
        kind: "note",
        role: "system-status",
        content: {
          title: "这个互动画面暂时没有生成成功",
          items: ["课程其余部分可以继续。"],
          degradation: {
            kind: "visual_component",
            visual_id: "paraboloid-scene",
            surface: "scene3d",
            purpose: "展示可旋转的抛物面与水平截面",
            retryable: true,
          },
        },
        place: { relation: "new_region" },
      }, {
        do: "focus",
        when: "after_speech",
        targets: ["paraboloid-scene"],
        intent: "current_step",
      }],
    }],
  }],
  close: {
    summary: "其余课程仍然可用。",
    focus: ["paraboloid-scene"],
  },
};

const degradedVisualLessonSource = normalizeAuthoringLesson(degradedVisualLesson, {
  lessonId: "degraded-visual-test",
  boardId: "degraded-visual-board",
  baseRevision: 0,
}).map((event) => JSON.stringify(event)).join("\n");

function DegradedVisualRuntimeProbe({
  onRetry,
}: {
  onRetry: NonNullable<ComponentProps<typeof OllLessonBoard>["onRetryDegradedVisual"]>;
}) {
  const runtime = useOllLessonRuntime({
    source: degradedVisualLessonSource,
    storageKey: "oll-degraded-visual-runtime-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <OllLessonBoard runtime={runtime} onRetryDegradedVisual={onRetry} />
    </div>
  );
}

function Scene3dRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: scene3dLessonSource,
    storageKey: "oll-scene3d-runtime-test",
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <span data-testid="scene3d-operation-count">
        {runtime.studentOperations.filter(
          (operation) => operation.kind === "scene3d_view",
        ).length}
      </span>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function IncrementalRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: JSON.stringify(geometryEvents[0]),
    storageKey: "oll-incremental-runtime-test",
    incremental: true,
  });
  if (!runtime) return null;
  return (
    <div style={{ width: 1200, height: 800 }}>
      <span data-testid="stream-status">{runtime.status}</span>
      <span data-testid="stream-total">{runtime.totalOperations}</span>
      <button type="button" onClick={runtime.nextBeat}>推进增量课程</button>
      <button type="button" onClick={() => runtime.appendEvents([geometryEvents[1]!])}>追加课程步骤</button>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function IncrementalReviewRuntimeProbe() {
  const runtime = useOllLessonRuntime({
    source: JSON.stringify(geometryEvents[0]),
    storageKey: "oll-incremental-review-runtime-test",
    incremental: true,
    startAtEnd: true,
  });
  if (!runtime) return null;
  return (
    <div>
      <span data-testid="incremental-review-status">{runtime.status}</span>
      <span data-testid="incremental-review-progress">
        {runtime.cursor}/{runtime.totalOperations}
      </span>
      <span data-testid="incremental-review-playing">
        {String(runtime.playing)}
      </span>
      <button
        type="button"
        onClick={() => runtime.appendEvents(geometryEvents.slice(1, -1))}
      >
        恢复历史课程
      </button>
      <OllLessonBoard runtime={runtime} />
    </div>
  );
}

function ReviewToLiveRuntimeProbe() {
  const [review, setReview] = useState(true);
  const runtime = useOllLessonRuntime({
    source: JSON.stringify(geometryEvents[0]),
    storageKey: "oll-review-to-live-runtime-test",
    incremental: true,
    autoPlay: !review,
    startAtEnd: review,
    narrationTiming: "external",
  });
  if (!runtime) return null;
  return (
    <div>
      <span data-testid="review-to-live-status">{runtime.status}</span>
      <span data-testid="review-to-live-playing">
        {String(runtime.playing)}
      </span>
      <span data-testid="review-to-live-speech">{runtime.activeSpeech}</span>
      <button
        type="button"
        onClick={() => runtime.appendEvents([geometryEvents[1]!])}
      >
        恢复已有步骤
      </button>
      <button type="button" onClick={() => setReview(false)}>
        开始新语音轮次
      </button>
      <button
        type="button"
        onClick={() => runtime.appendEvents([geometryEvents[2]!])}
      >
        追加新课程步骤
      </button>
    </div>
  );
}

describe("OLL lesson Runtime integration", () => {
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    localStorage.clear();
    mountInkRuntimeMock.mockReset();
    selectionContextToPngFileMock.mockClear();
    selectionSnapshotToPngFileMock.mockClear();
    vi.restoreAllMocks();
  });

  it("keeps the lesson visible and retries only a degraded visual component", async () => {
    const onRetry = vi.fn(async () => undefined);
    render(<DegradedVisualRuntimeProbe onRetry={onRetry} />);

    expect(await screen.findAllByText("这个互动画面暂时没有生成成功")).toHaveLength(2);
    expect(screen.getByText("展示可旋转的抛物面与水平截面")).toBeTruthy();
    const retry = screen.getByRole("button", { name: "只重试这个画面" });
    fireEvent.click(retry);

    await waitFor(() => expect(onRetry).toHaveBeenCalledWith({
      boardId: "degraded-visual-board",
      boardRevision: 1,
      nodeId: "degraded-visual-test:node:paraboloid-scene",
      visualId: "paraboloid-scene",
      surface: "scene3d",
      purpose: "展示可旋转的抛物面与水平截面",
      title: "这个互动画面暂时没有生成成功",
    }));
    expect(
      (await screen.findByRole("button", { name: "已发起重试" })).hasAttribute("disabled"),
    ).toBe(true);
    expect(screen.getByTestId("oll-lesson-board")).toBeTruthy();
  });

  it("pins a lesson region to the existing composer question origin", async () => {
    const setRegionLayouts = vi.spyOn(
      InfiniteBoardView.prototype,
      "setRegionLayouts",
    );
    const onPlaceQuestion = vi.fn();
    render(
      <QuestionPlacementProbe
        onPlaceQuestion={onPlaceQuestion}
        withCourseRegion
        withTallNarrative
      />,
    );

    await waitFor(() => {
      expect(setRegionLayouts).toHaveBeenCalledWith({
        __legacy__: {
          x: 2_694,
          y: 160,
          flow: "reading",
          reservedWidth: 886,
          attachments: expect.arrayContaining([
            expect.objectContaining({
              anchorNodeId: "lesson-unit-circle-sine-001:node:sine-plot",
              width: 360,
            }),
          ]),
        },
      });
    });
    expect(onPlaceQuestion).not.toHaveBeenCalled();
    const controls = await screen.findByTestId("oll-variable-controls");
    const controlTop = Number.parseFloat(controls.style.top);
    const anchorVisual = document.querySelector<HTMLElement>(
      "[data-id='lesson-unit-circle-sine-001:node:sine-plot']",
    )!;
    const anchorBottom = Number.parseFloat(anchorVisual.style.top)
      + Number.parseFloat(anchorVisual.style.height);
    const lessonBottom = Math.max(...Array.from(
      document.querySelectorAll<HTMLElement>(".board-node"),
    ).map((node) => Number.parseFloat(node.style.top)
      + Number.parseFloat(node.style.height)));
    expect(controlTop).toBeGreaterThanOrEqual(anchorBottom + 42);
    expect(controlTop).toBeLessThan(lessonBottom + 42);
  });

  it("keeps wheel zoom available while ink selection owns pointer input", async () => {
    const state: InkRuntimeState = {
      mode: "select",
      component_count: 1,
      selected_count: 0,
      selection_revision: 0,
      document_version: 1,
      saved: true,
    };
    mountInkRuntimeMock.mockImplementation((options) => ({
      ready: Promise.resolve(),
      subscribe: vi.fn((listener: (next: InkRuntimeState) => void) => {
        listener(state);
        return () => undefined;
      }),
      setMode: vi.fn((mode: InkMode) => {
        options.board.setInputOwner(mode === "navigate" ? "runtime" : "ink");
      }),
      destroy: vi.fn(() => Promise.resolve()),
    }));
    render(<InkRuntimeProbe />);

    await waitFor(() => expect(mountInkRuntimeMock).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "框选多个笔迹" }));
    const board = screen.getByTestId("oll-lesson-board");
    const world = board.querySelector<HTMLElement>(
      "[data-oll-board-runtime-world]",
    );
    const transformBefore = world?.style.transform;

    fireEvent.wheel(board, { deltaY: -120, clientX: 400, clientY: 300 });

    expect(world?.style.transform).not.toBe(transformBefore);
    expect(board.classList.contains("manual-navigation")).toBe(true);
  });

  it("reserves a new course area before its loading state renders", async () => {
    const onPlaceQuestion = vi.fn();
    render(
      <QuestionPlacementProbe
        onPlaceQuestion={onPlaceQuestion}
        pending
        showLoading={false}
      />,
    );

    await waitFor(() => expect(onPlaceQuestion).toHaveBeenCalled());
    const oldLessonRight = Math.max(...Array.from(
      document.querySelectorAll<HTMLElement>(".board-node"),
    ).map((node) => Number.parseFloat(node.style.left)
      + Number.parseFloat(node.style.width)));
    const position = onPlaceQuestion.mock.calls[0]?.[1] as { x: number; y: number };
    expect(position.x).toBeGreaterThanOrEqual(oldLessonRight + 180);
  });

  it("restores a missing historical question beside its existing course", async () => {
    const onPlaceQuestion = vi.fn();
    render(
      <QuestionPlacementProbe
        onPlaceQuestion={onPlaceQuestion}
        recovered
      />,
    );

    await waitFor(() => expect(onPlaceQuestion).toHaveBeenCalled());
    const lessonLeft = Math.min(...Array.from(
      document.querySelectorAll<HTMLElement>(".board-node"),
    ).map((node) => Number.parseFloat(node.style.left)));
    const position = onPlaceQuestion.mock.calls[0]?.[1] as { x: number; y: number };
    expect(position.x + 294).toBe(lessonLeft);
  });

  it("focuses a new question and loading block once without reclaiming the camera", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<PendingQuestionFocusProbe />);

    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledWith({
      x: 2_400,
      y: 160,
      width: 654,
      height: 194,
    }, { exclusive: true, framing: "content" }));
    expect(focusWorldRect).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "更新旁边界面" }));
    await act(async () => undefined);
    expect(focusWorldRect).toHaveBeenCalledTimes(1);
  });

  it("focuses the complete course footprint before loading UI catches up", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<PendingQuestionFocusProbe showLoading={false} />);

    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledWith({
      x: 2_400,
      y: 160,
      width: 654,
      height: 220,
    }, { exclusive: true, framing: "content" }));
  });

  it("ends inside the current course region instead of fitting every course", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));

    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));
    expect(focusWorldRect.mock.calls[0]?.[1]).toEqual({
      exclusive: true,
      framing: "course",
    });
    const bounds = focusWorldRect.mock.calls[0]?.[0];
    expect(bounds!.x).toBeGreaterThan(2_000);
    expect(bounds!.y).toBeGreaterThanOrEqual(180);
    expect(bounds!.width).toBeLessThan(2_000);

    fireEvent.click(screen.getByRole("button", { name: "更新结束界面" }));
    await act(async () => undefined);
    expect(focusWorldRect).toHaveBeenCalledTimes(1);
  });

  it("reserves the final course viewport before placing the next course", async () => {
    const focusedViewport = {
      x: 1_760,
      y: -320,
      width: 4_140,
      height: 2_245,
    };
    vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect")
      .mockReturnValue(focusedViewport);
    const onUpdateCourseRegion = vi.fn();
    render(<CompletedCourseFocusProbe
      onUpdateCourseRegion={onUpdateCourseRegion}
    />);

    await act(async () => undefined);
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));

    await waitFor(() => expect(onUpdateCourseRegion).toHaveBeenCalledWith(
      "current-course",
      { bounds: focusedViewport },
    ));
  });

  it("uses one course-end camera request when an after-lesson task appears", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe taskAppearsAtEnd />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));

    expect(await screen.findByTestId("oll-student-tasks")).toBeTruthy();
    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));
    const bounds = focusWorldRect.mock.calls[0]?.[0];
    expect(bounds!.x).toBeGreaterThan(2_000);
    expect(bounds!.width).toBeLessThan(2_000);

    // The task-availability effect runs at the same boundary. It must not
    // schedule a second, later camera move that overwrites the course target.
    await act(async () => undefined);
    expect(focusWorldRect).toHaveBeenCalledTimes(1);
  });

  it("does not run restore focus after naturally completing the same course", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe completesAtEnd />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));
    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "更新结束界面" }));
    await act(async () => undefined);
    expect(focusWorldRect).toHaveBeenCalledTimes(1);
  });

  it("keeps the completed-course camera when a later OLL operation requests another focus", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe legacyNodeMetadata />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));
    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));
    const world = document.querySelector<HTMLElement>(
      "[data-oll-board-runtime-world]",
    )!;
    const completedCourseTransform = world.style.transform;

    fireEvent.click(screen.getByRole("button", {
      name: "模拟结束后晚到的板书操作",
    }));
    await act(async () => undefined);

    expect(world.style.transform).toBe(completedCourseTransform);
  });

  it("uses Step-owned nodes when older cards share legacy region metadata", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe legacyNodeMetadata />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));

    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));
    const bounds = focusWorldRect.mock.calls[0]?.[0];
    const currentNodes = Array.from(document.querySelectorAll<HTMLElement>(
      ".board-node[data-id]:not([data-id='old-course-node'])",
    ));
    const currentLeft = Math.min(...currentNodes.map((node) =>
      Number.parseFloat(node.style.left)));
    const currentRight = Math.max(...currentNodes.map((node) =>
      Number.parseFloat(node.style.left) + Number.parseFloat(node.style.width)));
    expect(bounds!.x).toBe(currentLeft);
    expect(bounds!.width).toBe(currentRight - currentLeft);
  });

  it("focuses the explicitly replayed course instead of the whole restored board", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe legacyNodeMetadata />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", {
      name: "从目录重播当前课程",
    }));

    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));
    const bounds = focusWorldRect.mock.calls[0]?.[0];
    const currentNodes = Array.from(document.querySelectorAll<HTMLElement>(
      ".board-node[data-id]:not([data-id='old-course-node'])",
    ));
    const currentLeft = Math.min(...currentNodes.map((node) =>
      Number.parseFloat(node.style.left)));
    const currentRight = Math.max(...currentNodes.map((node) =>
      Number.parseFloat(node.style.left) + Number.parseFloat(node.style.width)));
    expect(bounds!.x).toBe(currentLeft);
    expect(bounds!.width).toBe(currentRight - currentLeft);
  });

  it("restores a completed multi-course board to its last course once", async () => {
    const focusWorldRect = vi.spyOn(InfiniteBoardView.prototype, "focusWorldRect");
    render(<CompletedCourseFocusProbe restored />);

    await act(async () => undefined);
    focusWorldRect.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "结束当前课程" }));
    await waitFor(() => expect(focusWorldRect).toHaveBeenCalledTimes(1));

    const bounds = focusWorldRect.mock.calls[0]?.[0];
    expect(bounds!.x).toBeGreaterThan(2_000);
    expect(bounds!.width).toBeLessThan(2_000);

    fireEvent.click(screen.getByRole("button", { name: "更新结束界面" }));
    await act(async () => undefined);

    expect(focusWorldRect).toHaveBeenCalledTimes(1);
  });

  it("keeps a newly anchored composer question away from restored student ink", async () => {
    const inkBounds = { x: -1_000, y: -1_000, width: 2_000, height: 2_000 };
    const state: InkRuntimeState = {
      mode: "navigate",
      component_count: 1,
      selected_count: 0,
      selection_revision: 0,
      content_bounds: inkBounds,
      document_version: 2,
      saved: true,
    };
    mountInkRuntimeMock.mockReturnValue({
      ready: Promise.resolve(),
      subscribe: vi.fn((listener: (next: InkRuntimeState) => void) => {
        listener(state);
        return () => undefined;
      }),
      setMode: vi.fn(),
      destroy: vi.fn(() => Promise.resolve()),
    });
    const onPlaceQuestion = vi.fn();
    render(
      <QuestionPlacementProbe
        inkSessionId="question-placement-ink"
        onPlaceQuestion={onPlaceQuestion}
        pending
      />,
    );

    await waitFor(() => expect(onPlaceQuestion).toHaveBeenCalled());
    const position = onPlaceQuestion.mock.calls.at(-1)?.[1];
    expect(position).toBeTruthy();
    expect(
      position.x < inkBounds.x + inkBounds.width
      && position.x + WHITEBOARD_QUESTION_CARD_WIDTH > inkBounds.x
      && position.y < inkBounds.y + inkBounds.height
      && position.y + 130 > inkBounds.y,
    ).toBe(false);
  });

  it("mounts writing as a persistent whiteboard capability", async () => {
    const listeners = new Set<(state: InkRuntimeState) => void>();
    let state: InkRuntimeState & { pen_color: string; selection_color: string | null } = {
      mode: "navigate",
      component_count: 2,
      selected_count: 0,
      pen_color: "#176b62",
      selection_color: null,
      selection_revision: 0,
      document_version: 3,
      saved: true,
    };
    const ink = {
      options: { style: { penColor: "#176b62" } },
      ready: Promise.resolve(),
      subscribe: vi.fn((listener: (next: InkRuntimeState) => void) => {
        listeners.add(listener);
        listener(state);
        return () => listeners.delete(listener);
      }),
      setMode: vi.fn((mode: InkMode) => {
        state = { ...state, mode };
        listeners.forEach((listener) => listener(state));
      }),
      selectAll: vi.fn(() => {
        state = { ...state, selected_count: 2, selection_color: "#176b62" };
        listeners.forEach((listener) => listener(state));
      }),
      setPenColor: vi.fn(function (
        this: { options: { style: { penColor: string } } },
        color: string,
      ) {
        // The real Runtime method reads `this.options.style`. Keeping that
        // contract in the mock catches accidentally detached method calls.
        this.options.style.penColor = color;
        state = { ...state, pen_color: color };
        listeners.forEach((listener) => listener(state));
      }),
      setSelectionColor: vi.fn((color: string) => {
        state = { ...state, selection_color: color };
        listeners.forEach((listener) => listener(state));
      }),
      undo: vi.fn(),
      redo: vi.fn(),
      destroy: vi.fn(() => Promise.resolve()),
    };
    mountInkRuntimeMock.mockReturnValue(ink);

    const onInkActivity = vi.fn();
    render(<InkRuntimeProbe onInkActivity={onInkActivity} />);

    await waitFor(() => expect(mountInkRuntimeMock).toHaveBeenCalledOnce());
    expect(document.querySelector(".learning-selection-enhancement-layer")
      ?.getAttribute("data-oll-ink-input")).toBe("ignore");
    expect(mountInkRuntimeMock).toHaveBeenCalledWith(expect.objectContaining({
      storageKey: "octos-learning-ink:v1:learn-ink-1",
      documentId: "learning-session:learn-ink-1:student-ink",
      locale: "zh-CN",
    }));
    expect(ink.setMode).toHaveBeenCalledWith("navigate");
    expect(onInkActivity).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "启用白板书写" })).toBeNull();
    expect(screen.queryByRole("button", { name: "退出书写模式" })).toBeNull();
    expect(screen.getByRole("status").textContent).toContain("2 项笔迹");
    fireEvent.click(screen.getByRole("button", { name: "书写笔迹" }));
    expect(ink.setMode).toHaveBeenLastCalledWith("draw");
    expect(screen.queryByRole("button", { name: "笔色：蓝色" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "显示调色板" }));
    expect(screen.getByRole("button", { name: "隐藏调色板" })).toBeTruthy();
    expect(screen.queryByText("笔色")).toBeNull();
    expect(screen.queryByLabelText("自定义笔色")).toBeNull();
    expect(document.querySelector('input[type="color"]')).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "笔色：蓝色" }));
    expect(ink.setPenColor).toHaveBeenCalledWith("#1769aa");
    expect(ink.options.style.penColor).toBe("#1769aa");
    fireEvent.click(screen.getByRole("button", { name: "隐藏调色板" }));
    expect(screen.queryByRole("button", { name: "笔色：蓝色" })).toBeNull();

    expect(screen.queryByRole("button", { name: "自由圈选笔迹" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "框选多个笔迹" }));
    expect(ink.setMode).toHaveBeenLastCalledWith("select");
    fireEvent.click(screen.getByRole("button", { name: "选择全部笔迹" }));
    expect(ink.selectAll).toHaveBeenCalledOnce();
    expect(screen.getByRole("status").textContent).toContain("已选 2");
    fireEvent.click(screen.getByRole("button", { name: "显示调色板" }));
    fireEvent.click(screen.getByRole("button", { name: "选区颜色：红色" }));
    expect(ink.setSelectionColor).toHaveBeenCalledWith("#c75445");

    fireEvent.click(screen.getByRole("button", { name: "浏览白板" }));
    expect(ink.setMode).toHaveBeenLastCalledWith("navigate");
    expect(screen.getByRole("status").textContent).toContain("2 项笔迹");
    expect(screen.getByRole("button", { name: "书写笔迹" })).toBeTruthy();
    expect(ink.destroy).not.toHaveBeenCalled();
  });

  it("keeps the same ink document when course content enters a blank whiteboard", async () => {
    const state: InkRuntimeState = {
      mode: "navigate",
      component_count: 1,
      selected_count: 0,
      selection_revision: 0,
      document_version: 1,
      saved: true,
    };
    const ink = {
      ready: Promise.resolve(),
      subscribe: vi.fn((listener: (next: InkRuntimeState) => void) => {
        listener(state);
        return () => undefined;
      }),
      setMode: vi.fn(),
      selectAll: vi.fn(),
      undo: vi.fn(),
      redo: vi.fn(),
      destroy: vi.fn(() => Promise.resolve()),
    };
    mountInkRuntimeMock.mockReturnValue(ink);

    render(<BlankToLessonWhiteboardProbe />);

    await waitFor(() => expect(mountInkRuntimeMock).toHaveBeenCalledOnce());
    expect(screen.getByRole("status").textContent).toContain("1 项笔迹");
    fireEvent.click(screen.getByRole("button", { name: "显示课程内容" }));
    await waitFor(() => expect(screen.getByText("① 看清题目")).toBeTruthy());
    expect(mountInkRuntimeMock).toHaveBeenCalledOnce();
    expect(ink.destroy).not.toHaveBeenCalled();
    expect(screen.getByRole("status").textContent).toContain("1 项笔迹");
  });

  it("asks about an immutable ink selection without modifying the source", async () => {
    const listeners = new Set<(state: InkRuntimeState) => void>();
    let state: InkRuntimeState & { pen_color: string; selection_color: string | null } = {
      mode: "select",
      component_count: 2,
      selected_count: 1,
      pen_color: "#176b62",
      selection_color: "#176b62",
      selection_input: "pen",
      selection_revision: 1,
      document_version: 3,
      saved: true,
    };
    const snapshot: InkSelectionSnapshot = {
      format: INK_SELECTION_FORMAT,
      format_version: INK_SELECTION_FORMAT_VERSION,
      source_id: "source-ui-1",
      document_id: "learning-session:learn-selection-1:student-ink",
      document_version: 3,
      created_at: "2026-08-14T12:00:00.000Z",
      bounds: { x: -10_000, y: -10_000, width: 20_000, height: 20_000 },
      region: {
        kind: "rectangle",
        closed: true,
        points: [
          { x: -10_000, y: -10_000 },
          { x: 10_000, y: -10_000 },
          { x: 10_000, y: 10_000 },
          { x: -10_000, y: 10_000 },
        ],
      },
      component_ids: ["stroke:selection-ui-1"],
      checksum: { algorithm: "sha-256", value: "a".repeat(64) },
      svg: '<svg data-oll-ink-selection="1"><path d="M0 0L10 10"/></svg>',
    };
    const ink = {
      ready: Promise.resolve(),
      subscribe: vi.fn((listener: (next: InkRuntimeState) => void) => {
        listeners.add(listener);
        listener(state);
        return () => listeners.delete(listener);
      }),
      setMode: vi.fn((mode: InkMode) => {
        state = { ...state, mode };
        listeners.forEach((listener) => listener(state));
      }),
      selectAll: vi.fn(),
      setPenColor: vi.fn(),
      setSelectionColor: vi.fn(),
      captureSelectionSnapshot: vi.fn(async () => snapshot),
      undo: vi.fn(),
      redo: vi.fn(),
      destroy: vi.fn(() => Promise.resolve()),
    };
    const onAsk = vi.fn(async () => undefined);
    const onClassify = vi.fn(async (): Promise<SelectionClassification> => ({
      kind: "math",
      content: "y=x^2",
      confidence: "high",
    }));
    mountInkRuntimeMock.mockReturnValue(ink);

    render(<SelectionInkRuntimeProbe onAsk={onAsk} onClassify={onClassify} />);

    await waitFor(() => expect(mountInkRuntimeMock).toHaveBeenCalledOnce());
    expect(selectionContextToPngFileMock).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "框选多个笔迹" }));
    await waitFor(() => expect(onClassify).toHaveBeenCalledWith({
      snapshot,
      boardContext: expect.objectContaining({
        boardId: expect.any(String),
        boardRevision: expect.any(Number),
        targets: expect.any(Array),
      }),
      selectionImage: expect.any(File),
    }));
    expect(await screen.findByRole("button", { name: "生成函数图像" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "问小章鱼" }));
    expect(await screen.findByText("接下来让小章鱼做什么？")).toBeTruthy();
    expect(screen.getByText("下面是操作，不会改变上面已经确认的选区。")).toBeTruthy();
    const targets = await screen.findAllByRole("radio");
    expect(targets.length).toBeGreaterThan(1);
    expect(document.querySelectorAll(".learning-selection-target-highlight").length)
      .toBeGreaterThan(0);
    fireEvent.click(targets[1]!);
    expect(document.querySelectorAll(".learning-selection-target-highlight.is-selected"))
      .toHaveLength(1);
    fireEvent.change(screen.getByLabelText("我写的内容更像"), {
      target: { value: "math" },
    });
    const generatePlot = screen.getAllByRole("button", { name: "生成函数图像" }).at(-1)!;
    await waitFor(() => expect((generatePlot as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(generatePlot);

    await waitFor(() => {
      expect(onAsk).toHaveBeenCalledWith({
        snapshot,
        question: "请按我选中的公式生成函数图像。",
        contentKind: "math",
        recognizedContent: "y=x^2",
        recognitionConfidence: "high",
        toolId: "generate-plot",
        boardContext: {
          boardId: expect.any(String),
          boardRevision: expect.any(Number),
          targets: [expect.objectContaining({
            target_id: expect.any(String),
            node_id: expect.any(String),
            kind: expect.any(String),
            world_bounds: expect.objectContaining({
              width: expect.any(Number),
              height: expect.any(Number),
            }),
            overlap: expect.any(Number),
            distance: expect.any(Number),
            z_index: expect.any(Number),
          })],
        },
        contextImage: expect.any(File),
      });
      expect(screen.getByTestId("selection-operation-count").textContent)
        .toBe("1");
    });
    onAsk.mockClear();
    let rejectSelectionRequest: ((cause: Error) => void) | undefined;
    onAsk.mockImplementationOnce(() => new Promise<void>((_resolve, reject) => {
      rejectSelectionRequest = reject;
    }));
    fireEvent.click(screen.getByRole("button", { name: "生成函数图像" }));
    expect(await screen.findByText("正在生成函数图像…")).toBeTruthy();
    expect(screen.getByLabelText(
      /正在生成函数图像。正在识别公式，并把可查看的图像放在选区旁边/,
    )).toBeTruthy();
    await waitFor(() => {
      expect(onAsk).toHaveBeenCalledWith(expect.objectContaining({
        snapshot,
        question: "请按我选中的公式生成函数图像。",
        contentKind: "math",
        toolId: "generate-plot",
        boardContext: expect.objectContaining({ targets: [] }),
        contextImage: expect.any(File),
      }));
    });
    await act(async () => {
      rejectSelectionRequest?.(new Error("当前公式暂不支持生成函数图像"));
    });
    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain(
        "当前公式暂不支持生成函数图像",
      );
    });
    expect(screen.queryByText("正在生成函数图像…")).toBeNull();
    expect(ink.captureSelectionSnapshot).toHaveBeenCalledTimes(2);
    expect(selectionContextToPngFileMock).toHaveBeenCalledTimes(2);
    expect(ink.setSelectionColor).not.toHaveBeenCalled();
    expect(ink.undo).not.toHaveBeenCalled();
    expect(ink.redo).not.toHaveBeenCalled();

    onClassify.mockResolvedValueOnce({
      kind: "geometry",
      content: "圈选标记",
      confidence: "high",
    });
    act(() => {
      state = { ...state, selection_revision: 2 };
      listeners.forEach((listener) => listener(state));
    });
    await waitFor(() => expect(onClassify).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "生成函数图像" })).toBeNull();
    });
    expect(screen.getByRole("button", { name: "问小章鱼" })).toBeTruthy();
  });

  it("waits for a post-restore ink change before reporting an erased source", async () => {
    const listeners = new Set<(state: InkRuntimeState) => void>();
    const sourcePresent = false;
    let state: InkRuntimeState = {
      mode: "navigate",
      component_count: 1,
      selected_count: 0,
      selection_input: "mouse",
      selection_revision: 0,
      document_version: 3,
      saved: true,
    };
    const source: InkSelectionSnapshot = {
      format: INK_SELECTION_FORMAT,
      format_version: INK_SELECTION_FORMAT_VERSION,
      source_id: "source-erased-1",
      document_id: "learning-session:selection-source-lifecycle:student-ink",
      document_version: 3,
      created_at: "2026-08-20T10:00:00.000Z",
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
      component_ids: ["stroke:erased-1"],
      checksum: { algorithm: "sha-256", value: "e".repeat(64) },
      svg: '<svg data-oll-ink-selection="1"><path d="M0 0L10 10"/></svg>',
    };
    const ink = {
      ready: Promise.resolve(),
      subscribe: vi.fn((listener: (next: InkRuntimeState) => void) => {
        listeners.add(listener);
        listener(state);
        return () => listeners.delete(listener);
      }),
      setMode: vi.fn(),
      selectAll: vi.fn(),
      undo: vi.fn(),
      redo: vi.fn(),
      hasSelectionSource: vi.fn(() => sourcePresent),
      destroy: vi.fn(() => Promise.resolve()),
    };
    const onDeleteSources = vi.fn();
    mountInkRuntimeMock.mockReturnValue(ink);

    render(
      <SelectionSourceLifecycleProbe
        source={source}
        onDeleteSources={onDeleteSources}
      />,
    );
    await act(async () => {
      await ink.ready;
    });
    expect(ink.hasSelectionSource).not.toHaveBeenCalled();
    expect(onDeleteSources).not.toHaveBeenCalled();

    act(() => {
      state = {
        ...state,
        component_count: 0,
        document_version: 4,
      };
      listeners.forEach((listener) => listener(state));
    });

    await waitFor(() => expect(onDeleteSources).toHaveBeenCalledWith([
      source.source_id,
    ]));
  });

  it("reports a failed restore and disposes the read-only ink layer", async () => {
    const ink = {
      ready: Promise.reject(new Error("保存的笔迹校验失败")),
      setMode: vi.fn(),
      subscribe: vi.fn(() => () => undefined),
      destroy: vi.fn(() => Promise.resolve()),
    };
    mountInkRuntimeMock.mockReturnValue(ink);
    render(<InkRuntimeProbe />);

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain(
        "保存的笔迹校验失败",
      );
    });
    expect(ink.destroy).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "启用白板书写" })).toBeNull();
  });

  it("plays Canonical Beats without replacing existing board nodes", () => {
    render(<RuntimeProbe />);

    fireEvent.click(screen.getByRole("button", { name: "下一 Beat" }));

    expect(screen.getByTestId("progress").textContent).not.toMatch(/^0\//);
    const board = screen.getByTestId("oll-lesson-board");
    expect(board.querySelectorAll(".board-node").length).toBeGreaterThan(0);
    expect(screen.getByText("① 已知与目标")).toBeTruthy();

    const diagram = board.querySelector<HTMLElement>(
      '[data-id="lesson-geometry-v2-001:node:clean-diagram"]',
    );
    const instanceId = diagram?.dataset.instanceId;
    expect(instanceId).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "下一 Beat" }));

    expect(
      board.querySelector<HTMLElement>(
        '[data-id="lesson-geometry-v2-001:node:clean-diagram"]',
      )?.dataset.instanceId,
    ).toBe(instanceId);
    expect(screen.getByText("② 连接 AD")).toBeTruthy();
  });

  it("opens a historical lesson at its final board state without playing", async () => {
    render(<ReviewRuntimeProbe />);

    await waitFor(() => {
      const [cursor, total] = screen
        .getByTestId("review-progress")
        .textContent!.split("/")
        .map(Number);
      expect(cursor).toBe(total);
      expect(total).toBeGreaterThan(0);
    });
    expect(screen.getByTestId("review-playing").textContent).toBe("false");
    expect(screen.getByText("关键想法")).toBeTruthy();
  });

  it("uses one persisted theta for the slider, unit circle, projection, and sine plot", async () => {
    vi.spyOn(SVGSVGElement.prototype, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 300,
      bottom: 210,
      width: 300,
      height: 210,
      toJSON: () => ({}),
    } as DOMRect);
    const directValues: number[] = [];
    const first = render(<VariableRuntimeProbe onVariable={(value) => directValues.push(value)} />);
    const slider = await screen.findByRole("slider", { name: "旋转角 θ" });
    const board = screen.getByTestId("oll-lesson-board");
    const unitPointId = "lesson-unit-circle-sine-001:node:unit-circle:fragment:point-p";
    const footId = "lesson-unit-circle-sine-001:node:unit-circle:fragment:foot";
    const plotPointId = "lesson-unit-circle-sine-001:node:sine-plot:fragment:current-angle";
    const unitPoint = board.querySelector<SVGCircleElement>(`[data-id="${unitPointId}"]`);
    const plotPoint = board.querySelector<SVGCircleElement>(`[data-id="${plotPointId}"]`);
    expect(unitPoint).toBeTruthy();
    expect(plotPoint).toBeTruthy();
    expect(unitPoint?.dataset.ollVariableControl).toBe("theta");
    const initialUnitCx = unitPoint?.getAttribute("cx");
    const initialUnitCy = unitPoint?.getAttribute("cy");
    const initialPlotCx = plotPoint?.getAttribute("cx");

    fireEvent.pointerDown(slider, { pointerType: "mouse" });
    expect(board.classList.contains("dragging")).toBe(false);
    fireEvent.change(slider, { target: { value: String(Math.PI / 2) } });
    fireEvent.pointerUp(slider, { pointerType: "mouse" });

    await waitFor(() => {
      expect(screen.getByText("π/2", { selector: "output" })).toBeTruthy();
      expect(screen.getByTestId("student-operation-count").textContent).toBe("1");
    });
    const updatedUnitPoint = board.querySelector<SVGCircleElement>(`[data-id="${unitPointId}"]`);
    const updatedFoot = board.querySelector<SVGCircleElement>(`[data-id="${footId}"]`);
    const updatedPlotPoint = board.querySelector<SVGCircleElement>(`[data-id="${plotPointId}"]`);
    expect(updatedUnitPoint?.getAttribute("cx")).not.toBe(initialUnitCx);
    expect(updatedUnitPoint?.getAttribute("cy")).not.toBe(initialUnitCy);
    expect(updatedUnitPoint?.getAttribute("cx")).toBe(updatedFoot?.getAttribute("cx"));
    expect(updatedPlotPoint?.getAttribute("cx")).not.toBe(initialPlotCx);

    const controlPoint = board.querySelector<SVGCircleElement>(`[data-id="${unitPointId}"]`)!;
    const controlSvg = controlPoint.closest("svg")!;
    Object.defineProperty(controlSvg, "viewBox", {
      configurable: true,
      value: { baseVal: { width: 300, height: 210 } },
    });
    const centerX = Number(controlPoint.dataset.angleCenterX);
    const centerY = Number(controlPoint.dataset.angleCenterY);
    controlPoint.dispatchEvent(new MouseEvent("pointerdown", {
      bubbles: true,
      clientX: centerX - 70,
      clientY: centerY,
    }));
    expect(directValues.at(-1)).toBeCloseTo(Math.PI);
    fireEvent.pointerUp(window);
    await waitFor(() => {
      expect(screen.getByText("π", { selector: "output" })).toBeTruthy();
      expect(screen.getByTestId("student-operation-count").textContent).toBe("2");
      expect(screen.getByTestId("student-operation-controls").textContent)
        .toBe("slider,geometry_point");
    });
    fireEvent.click(screen.getByRole("button", { name: "复位旋转角 θ" }));
    expect(screen.getByText("0", { selector: "output" })).toBeTruthy();
    expect(screen.getByTestId("student-operation-count").textContent).toBe("3");
    const finalSlider = screen.getByRole("slider", { name: "旋转角 θ" });
    fireEvent.pointerDown(finalSlider, { pointerType: "touch" });
    fireEvent.change(finalSlider, {
      target: { value: String(Math.PI) },
    });
    fireEvent.pointerUp(finalSlider, { pointerType: "touch" });
    expect(screen.getByTestId("student-operation-count").textContent).toBe("4");

    first.unmount();
    render(<VariableRuntimeProbe />);
    const restoredSlider = await screen.findByRole("slider", { name: "旋转角 θ" });
    expect(Number((restoredSlider as HTMLInputElement).value)).toBeCloseTo(Math.PI);
    expect(screen.getByText("π", { selector: "output" })).toBeTruthy();
    expect(screen.getByTestId("student-operation-count").textContent).toBe("4");
  });

  it("shows only the current lesson's controls on a multi-lesson whiteboard", async () => {
    render(<CurrentTopicVariableRuntimeProbe />);

    expect(await screen.findByRole("slider", { name: "旋转角 θ" })).toBeTruthy();
    expect(screen.queryByRole("slider", { name: "旧课程参数" })).toBeNull();
  });

  it("renders a highlighted 3D scene, completes its view task, and restores progress", async () => {
    const first = render(<Scene3dRuntimeProbe />);

    const scene = await screen.findByRole("img", { name: "立方体" });
    expect(scene.closest("[data-oll-scene3d]")).toBeTruthy();
    expect(screen.getByRole("button", { name: "等轴" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "俯视" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "复位" })).toBeTruthy();
    expect(screen.getByText("拖动画面旋转 · 滚动缩放")).toBeTruthy();
    expect(screen.getByText("直接操作白板上的图形、视角或控制器")).toBeTruthy();
    expect(screen.getByText("把立方体转到正视图")).toBeTruthy();
    expect(screen.getByText("顶点 A")).toBeTruthy();
    expect(screen.getByText("棱 AB")).toBeTruthy();
    expect(screen.getByText("顶面")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "正视" }));
    await waitFor(() => {
      expect(screen.getByTestId("scene3d-operation-count").textContent).toBe("1");
      expect(screen.getByText("正确，这是立方体的正视图。")).toBeTruthy();
    });

    first.unmount();
    render(<Scene3dRuntimeProbe />);
    expect(await screen.findByRole("img", { name: "立方体" })).toBeTruthy();
    expect(screen.getByTestId("scene3d-operation-count").textContent).toBe("1");
    expect(screen.getByText("正确，这是立方体的正视图。")).toBeTruthy();
  });

  it("shows an after-lesson task with feedback, hints, retry, success, and restore", async () => {
    const duringLesson = render(<StudentTaskRuntimeProbe startAtEnd={false} />);
    expect(screen.queryByTestId("oll-student-tasks")).toBeNull();
    expect(screen.queryByRole("slider", { name: "旋转角 θ" })).toBeNull();
    duringLesson.unmount();

    const first = render(<StudentTaskRuntimeProbe />);
    expect(await screen.findByText("把圆周点拖到 sin θ = 1")).toBeTruthy();
    expect(screen.getByRole("slider", { name: "旋转角 θ" })).toBeTruthy();
    const controlsCard = screen.getByTestId("oll-variable-controls");
    const tasksCard = screen.getByTestId("oll-student-tasks");
    expect(tasksCard.style.left).toBe(controlsCard.style.left);
    expect(Number.parseFloat(tasksCard.style.top)).toBeGreaterThan(
      Number.parseFloat(controlsCard.style.top),
    );
    expect(screen.getByText("轮到你操作了，完成后这里会立即反馈。")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "给我提示" })).toBeNull();

    const slider = screen.getByRole("slider", { name: "旋转角 θ" });
    fireEvent.pointerDown(slider, { pointerType: "mouse" });
    fireEvent.change(slider, { target: { value: String(Math.PI / 4) } });
    fireEvent.pointerUp(slider, { pointerType: "mouse" });

    await waitFor(() => {
      expect(screen.getByText("已尝试 1 次")).toBeTruthy();
    });
    fireEvent.click(screen.getByRole("button", { name: "给我提示" }));
    expect(screen.getByText("观察圆周点的纵坐标怎样随 θ 变化。")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "重新开始" }));
    expect(screen.getByText("0", { selector: "output" })).toBeTruthy();

    const resetSlider = screen.getByRole("slider", { name: "旋转角 θ" });
    fireEvent.pointerDown(resetSlider, { pointerType: "touch" });
    fireEvent.change(resetSlider, { target: { value: String(Math.PI / 2) } });
    fireEvent.pointerUp(resetSlider, { pointerType: "touch" });
    await waitFor(() => {
      expect(screen.getByText("正确，圆周点在最高点时 sin θ = 1。")).toBeTruthy();
    });

    first.unmount();
    render(<StudentTaskRuntimeProbe />);
    expect(await screen.findByText("正确，圆周点在最高点时 sin θ = 1。")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "重新开始" })).toBeNull();
  });

  it("opens an after-lesson task when the current incremental delivery settles", async () => {
    render(<IncrementalStudentTaskRuntimeProbe />);
    expect(await screen.findByText("把圆周点拖到 sin θ = 1")).toBeTruthy();

    const slider = screen.getByRole("slider", { name: "旋转角 θ" });
    fireEvent.pointerDown(slider, { pointerType: "mouse" });
    fireEvent.change(slider, { target: { value: String(Math.PI / 4) } });
    expect(screen.getByText("把圆周点拖到 sin θ = 1")).toBeTruthy();
    fireEvent.pointerUp(slider, { pointerType: "mouse" });
    await waitFor(() => {
      expect(screen.getByText("已尝试 1 次")).toBeTruthy();
    });

    const retrySlider = screen.getByRole("slider", { name: "旋转角 θ" });
    fireEvent.pointerDown(retrySlider, { pointerType: "mouse" });
    fireEvent.change(retrySlider, { target: { value: String(Math.PI / 3) } });
    await Promise.resolve();
    expect(screen.getByText("把圆周点拖到 sin θ = 1")).toBeTruthy();
    fireEvent.change(retrySlider, { target: { value: String(Math.PI / 2) } });
    fireEvent.pointerUp(retrySlider, { pointerType: "mouse" });
    await waitFor(() => {
      expect(screen.getByText("正确，圆周点在最高点时 sin θ = 1。")).toBeTruthy();
    });
  });

  it("groups the outline and seeks backwards to a selected Step", () => {
    render(<OutlineRuntimeProbe />);
    const [initialCursor, total] = screen
      .getByTestId("outline-progress")
      .textContent!.split("/")
      .map(Number);
    expect(initialCursor).toBe(total);
    expect(screen.getByTestId("outline-topic").textContent).toBe("几何证明");

    fireEvent.click(screen.getByRole("button", { name: "查看第一步" }));

    const [cursorAfterSeek] = screen
      .getByTestId("outline-progress")
      .textContent!.split("/")
      .map(Number);
    expect(cursorAfterSeek).toBeLessThan(total!);
    expect(screen.getByTestId("outline-current").textContent).toBe(
      geometryEvents[1]?.step?.id,
    );
    expect(screen.getByText("① 已知与目标")).toBeTruthy();
  });

  it("applies each Beat focus even when React batches advanceBeat updates", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 960,
      bottom: 608,
      width: 960,
      height: 608,
      toJSON: () => ({}),
    } as DOMRect);
    render(<RuntimeProbe />);

    const board = screen.getByTestId("oll-lesson-board");
    const world = board.querySelector<HTMLElement>("[data-oll-board-runtime-world]");
    expect(world).toBeTruthy();
    const transforms: string[] = [];

    for (let beat = 0; beat < 11; beat += 1) {
      fireEvent.click(screen.getByRole("button", { name: "下一 Beat" }));
      transforms.push(world?.style.transform ?? "");
    }

    const scales = transforms.map((transform) => transform.match(/scale\(([^)]+)\)/)?.[1]);
    expect(new Set(scales).size).toBeGreaterThanOrEqual(4);
    expect(transforms[5]).not.toBe(transforms[6]);
    expect(transforms[7]).not.toBe(transforms[8]);
  });

  it("keeps learner pan and zoom control after the lesson has ended", () => {
    vi.useFakeTimers();
    render(<ReviewRuntimeProbe />);

    const board = screen.getByTestId("oll-lesson-board");
    fireEvent.pointerDown(board, {
      pointerType: "mouse",
      clientX: 240,
      clientY: 180,
    });
    fireEvent.pointerMove(window, {
      pointerType: "mouse",
      clientX: 340,
      clientY: 240,
    });
    fireEvent.pointerUp(window, { pointerType: "mouse" });

    expect(board.classList.contains("manual-navigation")).toBe(true);
    act(() => vi.advanceTimersByTime(30_000));
    expect(
      board.classList.contains("manual-navigation"),
      "elapsed time must not hand the completed lesson camera back to automatic focus",
    ).toBe(true);
  });

  it("ignores ordinary host rerenders but lets a new teaching Beat reclaim the camera", () => {
    render(<CameraRuntimeProbe />);
    fireEvent.click(screen.getByRole("button", { name: "下一 Beat" }));

    const board = screen.getByTestId("oll-lesson-board");
    fireEvent.pointerDown(board, {
      pointerType: "mouse",
      clientX: 240,
      clientY: 180,
    });
    fireEvent.pointerMove(window, {
      pointerType: "mouse",
      clientX: 340,
      clientY: 240,
    });
    fireEvent.pointerUp(window, { pointerType: "mouse" });
    expect(board.classList.contains("manual-navigation")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "更新旁边界面" }));
    expect(
      board.classList.contains("manual-navigation"),
      "re-rendering the current Beat must preserve the learner's camera",
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "下一 Beat" }));
    expect(
      board.classList.contains("manual-navigation"),
      "a new Beat with an explicit teaching focus may reclaim the camera once",
    ).toBe(false);
  });

  it("grows an active /learn board when a validated Canonical Step arrives", () => {
    render(<IncrementalRuntimeProbe />);
    fireEvent.click(screen.getByRole("button", { name: "推进增量课程" }));
    expect(screen.getByTestId("stream-status").textContent).toBe("waiting");
    const totalBefore = Number(screen.getByTestId("stream-total").textContent);

    fireEvent.click(screen.getByRole("button", { name: "追加课程步骤" }));
    expect(Number(screen.getByTestId("stream-total").textContent)).toBeGreaterThan(totalBefore);
    fireEvent.click(screen.getByRole("button", { name: "推进增量课程" }));
    expect(screen.getByText("① 已知与目标")).toBeTruthy();
  });

  it("applies an incrementally restored history directly to its available end", async () => {
    render(<IncrementalReviewRuntimeProbe />);
    fireEvent.click(screen.getByRole("button", { name: "恢复历史课程" }));

    await waitFor(() => {
      const [cursor, total] = screen
        .getByTestId("incremental-review-progress")
        .textContent!.split("/")
        .map(Number);
      expect(cursor).toBe(total);
      expect(total).toBeGreaterThan(0);
    });
    expect(screen.getByTestId("incremental-review-status").textContent).toBe(
      "waiting",
    );
    expect(screen.getByTestId("incremental-review-playing").textContent).toBe(
      "false",
    );
    expect(screen.getByText("关键想法")).toBeTruthy();
  });

  it("plays a new incremental step after a reviewed lesson returns to live mode", async () => {
    render(<ReviewToLiveRuntimeProbe />);
    fireEvent.click(screen.getByRole("button", { name: "恢复已有步骤" }));
    expect(screen.getByTestId("review-to-live-status").textContent).toBe(
      "waiting",
    );

    fireEvent.click(screen.getByRole("button", { name: "开始新语音轮次" }));
    fireEvent.click(screen.getByRole("button", { name: "追加新课程步骤" }));

    await waitFor(() => {
      expect(screen.getByTestId("review-to-live-playing").textContent).toBe(
        "true",
      );
      expect(screen.getByTestId("review-to-live-speech").textContent).toContain(
        "现在连接 A 和 D",
      );
    });
  });
});
