import { buildApiHeaders } from "@/api/client";
import { buildFileUrl } from "@/api/files";
import type { SessionFileInfo } from "@/api/sessions";
import type { Thread } from "@/store/thread-store";
import {
  normalizeAuthoringLesson,
  reduceCanonicalEvents,
  type AuthoringLesson,
  type CanonicalEvent,
} from "octos-lesson-language";

export interface OllLessonArtifactRef {
  id: string;
  filename: string;
  path: string;
  threadId: string;
  turnId: string;
}

export interface OllLessonTopic {
  id: string;
  title: string;
  stepIds: string[];
  /** Nodes created by this lesson's own Steps. This remains reliable even
   * when an older saved lesson has no per-node region_id metadata. */
  nodeIds: string[];
  variableAliases: string[];
  taskAliases: string[];
  taskTargets: Record<string, {
    variableAliases: string[];
    nodeIds: string[];
  }>;
  questionId?: string;
}

const OLL_ARTIFACT_SUFFIX = ".octos-lesson.json";

type JsonRecord = Record<string, unknown>;

function stableIdentifierHash(value: string): string {
  let left = 0x811c9dc5;
  let right = 0x9e3779b9;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    left = Math.imul(left ^ code, 0x01000193);
    right = Math.imul(right ^ code, 0x85ebca6b);
  }
  return `${(left >>> 0).toString(16).padStart(8, "0")}${
    (right >>> 0).toString(16).padStart(8, "0")
  }`;
}

function scopedIdentifier(namespace: string, value: string): string {
  const direct = `${namespace}_${value}`;
  if (direct.length <= 64) return direct;
  return `${namespace}_${value.slice(0, 35)}_${stableIdentifierHash(value).slice(0, 8)}`;
}

function replaceExpressionVariables(
  expression: string,
  variables: ReadonlyMap<string, string>,
): string {
  return expression.replace(/[a-z][a-z0-9_]*/giu, (identifier) =>
    variables.get(identifier.toLocaleLowerCase()) ?? identifier);
}

function namespaceContentVariables(
  value: unknown,
  variables: ReadonlyMap<string, string>,
): void {
  if (Array.isArray(value)) {
    value.forEach((item) => namespaceContentVariables(item, variables));
    return;
  }
  if (!value || typeof value !== "object") return;
  const record = value as JsonRecord;
  if (typeof record.variable === "string") {
    record.variable = variables.get(record.variable.toLocaleLowerCase())
      ?? record.variable;
  }
  if (Array.isArray(record.bindings)) {
    for (const binding of record.bindings) {
      if (!binding || typeof binding !== "object") continue;
      const bindingRecord = binding as JsonRecord;
      if (typeof bindingRecord.expression === "string") {
        bindingRecord.expression = replaceExpressionVariables(
          bindingRecord.expression,
          variables,
        );
      }
    }
  }
  for (const [key, child] of Object.entries(record)) {
    if (key === "bindings") continue;
    namespaceContentVariables(child, variables);
  }
}

/**
 * Every generated lesson is valid on its own, but one /learn whiteboard can
 * append several lessons. Give lesson variables and tasks stable internal
 * identities before composition so two independent turns may both use names
 * such as `x` or `theta` without sharing state or hiding each other's metadata.
 */
function namespaceCanonicalLesson(
  events: CanonicalEvent[],
  artifactIdentity: string,
): CanonicalEvent[] {
  const result = structuredClone(events);
  const open = result.find((event) => event.event === "lesson.open");
  const lesson = open?.lesson;
  if (!lesson) return result;
  const namespace = `v${stableIdentifierHash(artifactIdentity)}`;
  const variableNames = new Map(
    (lesson.variables ?? []).map((variable) => [
      variable.as.toLocaleLowerCase(),
      scopedIdentifier(namespace, variable.as),
    ]),
  );
  for (const variable of lesson.variables ?? []) {
    variable.as = variableNames.get(variable.as.toLocaleLowerCase())!;
  }
  for (const task of lesson.tasks ?? []) {
    task.as = scopedIdentifier(namespace, task.as);
    for (const operation of task.allowed_operations) {
      if (operation.kind !== "variable_change") continue;
      operation.variable = variableNames.get(operation.variable.toLocaleLowerCase())
        ?? operation.variable;
    }
    if (task.completion.kind === "expression_target") {
      task.completion.expression = replaceExpressionVariables(
        task.completion.expression,
        variableNames,
      );
    }
  }
  for (const event of result) {
    for (const beat of event.step?.beats ?? []) {
      for (const stage of Object.values(beat.stage)) {
        for (const action of stage) {
          if (action.animation) {
            action.animation.variable = variableNames.get(
              action.animation.variable.toLocaleLowerCase(),
            ) ?? action.animation.variable;
          }
          if (action.node) namespaceContentVariables(action.node.content, variableNames);
          if (action.revision) {
            namespaceContentVariables(action.revision.content, variableNames);
          }
        }
      }
    }
  }
  return result;
}

interface OllArtifactFilename {
  filename: string;
  turnId: string;
  rank: number;
}

function parseOllArtifactFilename(filename: string): OllArtifactFilename {
  const leaf = filename.replaceAll("\\", "/").split("/").at(-1) ?? filename;
  const partial = /^(.*)\.part-(\d+)\.octos-lesson\.json$/i.exec(leaf);
  if (partial) {
    return {
      filename: `${partial[1]}.octos-lesson.json`,
      turnId: partial[1]!,
      rank: Number.parseInt(partial[2]!, 10),
    };
  }
  const finalArtifact = /^(.*)\.octos-lesson\.json$/i.exec(leaf);
  return {
    filename: leaf,
    turnId: finalArtifact?.[1] ?? leaf,
    rank: Number.POSITIVE_INFINITY,
  };
}

function preferNewestArtifact(
  artifacts: OllLessonArtifactRef[],
): OllLessonArtifactRef[] {
  const selected: OllLessonArtifactRef[] = [];
  const indexByIdentity = new Map<string, number>();
  for (const artifact of artifacts) {
    const identity = ollArtifactIdentity(artifact);
    const existingIndex = indexByIdentity.get(identity);
    if (existingIndex === undefined) {
      indexByIdentity.set(identity, selected.length);
      selected.push(artifact);
      continue;
    }
    const existing = selected[existingIndex]!;
    const existingRank = parseOllArtifactFilename(existing.filename).rank;
    const candidateRank = parseOllArtifactFilename(artifact.filename).rank;
    if (candidateRank > existingRank) selected[existingIndex] = artifact;
  }
  return selected;
}

export function ollArtifactIdentity(
  artifact: Pick<OllLessonArtifactRef, "filename">,
): string {
  // The live projection exposes an absolute path while session/files.list
  // returns an opaque pf/... handle. The turn-scoped filename is the one
  // durable identifier shared by both representations.
  return encodeURIComponent(parseOllArtifactFilename(artifact.filename).filename);
}

export function isOllLessonArtifact(
  file: { filename?: string; path?: string },
): boolean {
  return [file.filename, file.path].some(
    (value) =>
      typeof value === "string" &&
      value.toLowerCase().endsWith(OLL_ARTIFACT_SUFFIX),
  );
}

export function collectOllLessonArtifacts(
  threads: Thread[],
): OllLessonArtifactRef[] {
  const artifacts: OllLessonArtifactRef[] = [];
  for (const thread of threads) {
    const messages = [
      ...thread.responses,
      ...(thread.pendingAssistant ? [thread.pendingAssistant] : []),
    ];
    for (const message of messages) {
      for (const file of message.files) {
        if (!isOllLessonArtifact(file)) continue;
        const artifact = {
          id: `${message.id}:${file.path}`,
          filename: file.filename,
          path: file.path,
          threadId: thread.id,
          turnId: thread.turnId ?? thread.id,
        };
        artifacts.push(artifact);
      }
    }
  }
  return preferNewestArtifact(artifacts);
}

export function mergeOllLessonArtifacts(
  ...groups: OllLessonArtifactRef[][]
): OllLessonArtifactRef[] {
  return preferNewestArtifact(groups.flat());
}

/**
 * Rebuild OLL artifact references from the durable session workspace.
 * Live projection file events are intentionally not the source of truth:
 * they may not be replayed after a browser refresh or a session switch.
 */
export function collectPersistedOllLessonArtifacts(
  files: SessionFileInfo[],
): OllLessonArtifactRef[] {
  return preferNewestArtifact(files
    .filter((file) => isOllLessonArtifact(file))
    .sort((left, right) => {
      const byTime =
        Date.parse(left.modified_at) - Date.parse(right.modified_at);
      return Number.isFinite(byTime) && byTime !== 0
        ? byTime
        : left.path.localeCompare(right.path);
    })
    .map((file) => {
      const turnId = parseOllArtifactFilename(file.filename).turnId;
      return {
        id: `persisted:${file.path}`,
        filename: file.filename,
        path: file.path,
        threadId: turnId,
        turnId,
      };
    }));
}

export async function loadOllLessonArtifact(
  artifact: OllLessonArtifactRef,
  sessionId: string,
  signal?: AbortSignal,
): Promise<CanonicalEvent[]> {
  const response = await fetch(buildFileUrl(artifact.path, { sessionId }), {
    headers: buildApiHeaders(),
    signal,
  });
  if (!response.ok) {
    throw new Error(`OLL 课程读取失败 (${response.status})`);
  }
  const authoring = (await response.json()) as AuthoringLesson;
  try {
    const artifactIdentity = ollArtifactIdentity(artifact);
    const boardId = `learning-board-${sessionId}`;
    const boardContext = authoring.board_context;
    const events = normalizeAuthoringLesson(authoring, {
      lessonId: `learn-${sessionId}-${artifactIdentity}`,
      boardId,
      baseRevision: boardContext?.revision ?? 0,
      regionIntent: boardContext ? "continue_topic" : "new_topic",
      regionId: `topic-${artifactIdentity}`,
    });
    // A referenced lesson intentionally points at nodes created by earlier
    // artifacts. It can only be reduced after the classroom composer has put
    // those earlier events in front of it.
    if (!boardContext?.references.length) reduceCanonicalEvents(events);
    return events;
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : "未知格式错误";
    throw new Error(`OLL 课程格式无效：${message}`);
  }
}

export function composeOllClassroomEvents(
  lessons: CanonicalEvent[][],
  sessionId: string,
): CanonicalEvent[] {
  // Keep the first lesson's public variable names stable for existing saved
  // checkpoints. Later independent lessons are scoped before their metadata
  // and Steps enter the long-lived classroom.
  const classroomLessons = lessons.map((lesson, index) => {
    const open = lesson.find((event) => event.event === "lesson.open");
    return index === 0 || !open
      ? structuredClone(lesson)
      : namespaceCanonicalLesson(lesson, open.lesson_id);
  });
  const firstOpen = classroomLessons[0]?.find(
    (event) => event.event === "lesson.open",
  );
  if (!firstOpen) return [];
  const lessonId = `learning-session-${sessionId}`;
  const result: CanonicalEvent[] = [{
    ...structuredClone(firstOpen),
    lesson_id: lessonId,
    sequence: 0,
    board: {
      board_id: `learning-board-${sessionId}`,
      base_revision: 0,
      region_intent: "new_topic",
    },
  }];
  const variables = classroomLessons.flatMap((lesson) =>
    lesson.find((event) => event.event === "lesson.open")?.lesson?.variables ?? []);
  const tasks = classroomLessons.flatMap((lesson) =>
    lesson.find((event) => event.event === "lesson.open")?.lesson?.tasks ?? []);
  if (result[0]?.lesson) {
    result[0].lesson.variables = structuredClone(variables);
    result[0].lesson.tasks = structuredClone(tasks);
  }
  let activeRegionId = firstOpen.board?.region_id ?? firstOpen.lesson_id;
  for (const lesson of classroomLessons) {
    const open = lesson.find((event) => event.event === "lesson.open");
    if (open?.board?.region_intent === "new_topic") {
      activeRegionId = open.board.region_id ?? open.lesson_id;
    } else if (open?.board?.region_id) {
      activeRegionId = open.board.region_id;
    }
    for (const event of lesson) {
      if (event.event !== "lesson.step") continue;
      const stepEvent = {
        ...structuredClone(event),
        lesson_id: lessonId,
        sequence: result.length,
      };
      for (const beat of stepEvent.step?.beats ?? []) {
        for (const stage of Object.values(beat.stage)) {
          for (const action of stage) {
            if (action.op === "board.create" && action.node) {
              action.node.region_id ??= activeRegionId;
            }
          }
        }
      }
      result.push(stepEvent);
    }
  }
  return result;
}

export function buildOllLessonTopics(
  lessons: CanonicalEvent[][],
  questionIds: Array<string | undefined> = [],
): OllLessonTopic[] {
  return lessons.flatMap((events, index) => {
    const open = events.find((event) => event.event === "lesson.open");
    const stepIds = events.flatMap((event) =>
      event.event === "lesson.step" && event.step ? [event.step.id] : [],
    );
    const nodeIds = events.flatMap((event) =>
      event.event === "lesson.step" && event.step
        ? event.step.beats.flatMap((beat) =>
            Object.values(beat.stage).flatMap((actions) =>
              actions.flatMap((action) =>
                action.op === "board.create" && action.node
                  ? [action.node.id]
                  : [])))
        : [],
    );
    if (!open || stepIds.length === 0) return [];
    const namespace = index === 0
      ? null
      : `v${stableIdentifierHash(open.lesson_id)}`;
    return [{
      id: open.board?.region_id ?? open.lesson_id,
      title: open.lesson?.title ?? `课程主题 ${index + 1}`,
      stepIds,
      nodeIds: [...new Set(nodeIds)],
      variableAliases: (open.lesson?.variables ?? []).map((variable) =>
        namespace ? scopedIdentifier(namespace, variable.as) : variable.as),
      taskAliases: (open.lesson?.tasks ?? []).map((task) =>
        namespace ? scopedIdentifier(namespace, task.as) : task.as),
      taskTargets: Object.fromEntries((open.lesson?.tasks ?? []).map((task) => {
        const taskAlias = namespace ? scopedIdentifier(namespace, task.as) : task.as;
        const variableAliases = task.allowed_operations.flatMap((operation) =>
          operation.kind === "variable_change"
            ? [namespace
                ? scopedIdentifier(namespace, operation.variable)
                : operation.variable]
            : []);
        const nodeIds = task.allowed_operations.flatMap((operation) =>
          operation.kind === "scene3d_view" ? [operation.node] : []);
        return [taskAlias, { variableAliases, nodeIds }];
      })),
      ...(questionIds[index] ? { questionId: questionIds[index] } : {}),
    }];
  });
}

/** Resolve the first Step of the course that owns the current playback Step. */
export function courseReplayStartStep(
  topics: OllLessonTopic[],
  currentStepId: string | undefined,
): string | undefined {
  const currentTopic = topics.find((topic) =>
    topic.stepIds.includes(currentStepId ?? ""))
    ?? topics.at(-1);
  return currentTopic?.stepIds[0];
}
