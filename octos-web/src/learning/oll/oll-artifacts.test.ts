import { describe, expect, it, vi } from "vitest";
import { reduceCanonicalEvents } from "octos-lesson-language";
import type { Thread } from "@/store/thread-store";
import {
  buildOllLessonTopics,
  collectPersistedOllLessonArtifacts,
  collectOllLessonArtifacts,
  composeOllClassroomEvents,
  courseReplayStartStep,
  isOllLessonArtifact,
  loadOllLessonArtifact,
  mergeOllLessonArtifacts,
  ollArtifactIdentity,
} from "./oll-artifacts";

function threadWithLesson(path: string): Thread {
  return {
    id: "client-turn-1",
    turnId: "server-turn-1",
    userMsg: {
      id: "user-1",
      role: "user",
      text: "讲解这道题",
      files: [],
      toolCalls: [],
      status: "complete",
      timestamp: 1,
    },
    responses: [{
      id: "assistant-1",
      role: "assistant",
      text: "我们开始。",
      files: [{ filename: "turn.octos-lesson.json", path }],
      toolCalls: [],
      status: "complete",
      timestamp: 2,
    }],
    pendingAssistant: null,
  };
}

const authoringLesson = {
  dsl: "octos.lesson",
  version: "0.1",
  profile: "authoring",
  lesson: {
    mode: "explain",
    language: "zh-CN",
    title: "测试课程",
    goals: ["解释一个概念"],
  },
  steps: [{
    key: "explain",
    purpose: "写出结论",
    beats: [{
      key: "write",
      say: "先写出核心结论。",
      actions: [{
        do: "write",
        as: "answer",
        kind: "note",
        role: "conclusion",
        content: { text: "核心结论" },
        place: { relation: "new_region", region_role: "lesson_origin" },
      }],
    }],
  }],
  close: { summary: "完成讲解", focus: ["answer"] },
};

const threeStepAuthoringLesson = {
  ...authoringLesson,
  steps: ["first", "second", "third"].map((key) => ({
    key,
    purpose: `讲解 ${key}`,
    beats: [{
      key: `write-${key}`,
      say: `讲解 ${key}。`,
      actions: [{
        do: "write",
        as: `answer-${key}`,
        kind: "note",
        role: "conclusion",
        content: { text: `结论 ${key}` },
        place: { relation: "new_region", region_role: "lesson_origin" },
      }],
    }],
  })),
  close: { summary: "完成三步讲解", focus: ["answer-third"] },
};

describe("OLL lesson artifacts", () => {
  it("retains machine-readable task targets in lesson topics", () => {
    const events = [{
      dsl: "octos.lesson",
      version: "0.1",
      profile: "canonical",
      event: "lesson.open",
      lesson_id: "course",
      sequence: 0,
      board: {
        board_id: "board",
        base_revision: 0,
        region_intent: "new_topic",
        region_id: "course-region",
      },
      lesson: {
        mode: "explain",
        language: "zh-CN",
        title: "交互课程",
        goals: [],
        variables: [{
          as: "theta",
          initial: 0,
          min: 0,
          max: 6.28,
          control: { kind: "slider" },
        }],
        tasks: [{
          as: "rotate",
          prompt: "旋转图形",
          availability: { kind: "after_lesson" },
          allowed_operations: [{
            kind: "variable_change",
            variable: "theta",
            controls: ["slider"],
          }],
          completion: {
            kind: "expression_target",
            expression: "theta",
            value: 1,
            tolerance: 0.1,
          },
          hints: [],
        }],
      },
    }, {
      dsl: "octos.lesson",
      version: "0.1",
      profile: "canonical",
      event: "lesson.step",
      lesson_id: "course",
      sequence: 1,
      step: {
        id: "step",
        purpose: "展示图形",
        beats: [{
          id: "beat",
          stage: {
            before_speech: [],
            during_speech: [{
              action_id: "create",
              op: "board.create",
              node: { id: "plot", kind: "plot", content: {} },
            }],
            after_speech: [],
          },
        }],
      },
    }] as Parameters<typeof buildOllLessonTopics>[0][number];

    expect(buildOllLessonTopics([events])[0]?.taskTargets).toEqual({
      rotate: { variableAliases: ["theta"], nodeIds: [] },
    });
  });

  it("rebuilds artifact references from durable session files", () => {
    expect(
      collectPersistedOllLessonArtifacts([
        {
          filename: "turn-2.octos-lesson.json",
          path: "skill-output/study/oll/turn-2.octos-lesson.json",
          size_bytes: 200,
          modified_at: "2026-07-28T12:02:00.000Z",
        },
        {
          filename: "notes.txt",
          path: "notes.txt",
          size_bytes: 10,
          modified_at: "2026-07-28T12:00:00.000Z",
        },
        {
          filename: "turn-1.octos-lesson.json",
          path: "study/oll/turn-1.octos-lesson.json",
          size_bytes: 100,
          modified_at: "2026-07-28T12:01:00.000Z",
        },
      ]),
    ).toEqual([
      expect.objectContaining({
        path: "study/oll/turn-1.octos-lesson.json",
        turnId: "turn-1",
      }),
      expect.objectContaining({
        path: "skill-output/study/oll/turn-2.octos-lesson.json",
        turnId: "turn-2",
      }),
    ]);
  });

  it("keeps only the newest playable prefix for each turn and lets the final artifact supersede it", () => {
    const file = (filename: string, modified_at: string) => ({
      filename,
      path: `study/oll/${filename}`,
      size_bytes: 100,
      modified_at,
    });

    const partials = collectPersistedOllLessonArtifacts([
      file("turn-1.part-000.octos-lesson.json", "2026-08-14T10:00:00.000Z"),
      file("turn-1.part-001.octos-lesson.json", "2026-08-14T10:00:01.000Z"),
      file("turn-2.part-000.octos-lesson.json", "2026-08-14T10:00:02.000Z"),
    ]);
    expect(partials).toEqual([
      expect.objectContaining({
        filename: "turn-1.part-001.octos-lesson.json",
        turnId: "turn-1",
      }),
      expect.objectContaining({
        filename: "turn-2.part-000.octos-lesson.json",
        turnId: "turn-2",
      }),
    ]);
    expect(ollArtifactIdentity(partials[0]!)).toBe(
      encodeURIComponent("turn-1.octos-lesson.json"),
    );

    const [finalArtifact] = collectPersistedOllLessonArtifacts([
      file("turn-1.part-001.octos-lesson.json", "2026-08-14T10:00:01.000Z"),
      file("turn-1.octos-lesson.json", "2026-08-14T10:00:03.000Z"),
    ]);
    expect(finalArtifact).toEqual(expect.objectContaining({
      filename: "turn-1.octos-lesson.json",
      turnId: "turn-1",
    }));
  });

  it("replaces a persisted prefix with the delivered final artifact without duplicating the turn", () => {
    const partial = {
      id: "partial",
      filename: "turn-1.part-002.octos-lesson.json",
      path: "study/oll/turn-1.part-002.octos-lesson.json",
      threadId: "turn-1",
      turnId: "turn-1",
    };
    const finalArtifact = {
      id: "final",
      filename: "turn-1.octos-lesson.json",
      path: "/workspace/study/oll/turn-1.octos-lesson.json",
      threadId: "client-turn",
      turnId: "server-turn",
    };

    expect(mergeOllLessonArtifacts([partial], [finalArtifact])).toEqual([
      finalArtifact,
    ]);
  });

  it("recognizes and collects delivered OLL authoring files", () => {
    expect(isOllLessonArtifact({ filename: "turn.OCTOS-LESSON.JSON" })).toBe(true);
    expect(isOllLessonArtifact({ filename: "turn.octos-board.json" })).toBe(false);
    expect(collectOllLessonArtifacts([
      threadWithLesson("study/oll/turn.octos-lesson.json"),
    ])).toEqual([
      expect.objectContaining({
        path: "study/oll/turn.octos-lesson.json",
        turnId: "server-turn-1",
      }),
    ]);
  });

  it("validates and normalizes an Authoring artifact before playback", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => authoringLesson,
    }));
    const [artifact] = collectOllLessonArtifacts([
      threadWithLesson("study/oll/turn.octos-lesson.json"),
    ]);
    const events = await loadOllLessonArtifact(artifact!, "session-1");
    expect(events.map((event) => event.event)).toEqual([
      "lesson.open",
      "lesson.step",
      "lesson.close",
    ]);
    expect(events[1]?.step?.beats[0]?.narration?.text).toBe("先写出核心结论。");
    const artifactIdentity = ollArtifactIdentity(artifact!);
    expect(events[0]?.board?.region_id).toBe(`topic-${artifactIdentity}`);
    const createdNode = events[1]?.step?.beats
      .flatMap((beat) => Object.values(beat.stage).flat())
      .find((action) => action.op === "board.create")?.node;
    expect(createdNode?.region_id).toBe(`topic-${artifactIdentity}`);
    vi.unstubAllGlobals();
  });

  it("composes multiple teaching turns as one open incremental classroom", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => authoringLesson,
    }));
    const [artifact] = collectOllLessonArtifacts([
      threadWithLesson("study/oll/turn.octos-lesson.json"),
    ]);
    const first = await loadOllLessonArtifact(artifact!, "session-1");
    const secondArtifact = {
      ...artifact!,
      filename: "turn-2.octos-lesson.json",
      path: "study/oll/turn-2.octos-lesson.json",
      turnId: "server-turn-2",
    };
    const second = await loadOllLessonArtifact(secondArtifact, "session-1");
    const firstClassroom = composeOllClassroomEvents([first], "session-1");
    const classroom = composeOllClassroomEvents([first, second], "session-1");
    expect(classroom.map((event) => event.event)).toEqual([
      "lesson.open",
      "lesson.step",
      "lesson.step",
    ]);
    expect(classroom.map((event) => event.sequence)).toEqual([0, 1, 2]);
    expect(new Set(classroom.map((event) => event.lesson_id)).size).toBe(1);
    expect(classroom[1]?.step?.id).not.toBe(classroom[2]?.step?.id);
    expect(classroom.slice(0, firstClassroom.length)).toEqual(firstClassroom);
    expect(buildOllLessonTopics([first, second])).toEqual([
      {
        id: first[0]?.board?.region_id,
        title: first[0]?.lesson?.title,
        stepIds: [first[1]?.step?.id],
        nodeIds: first[1]?.step?.beats.flatMap((beat) =>
          Object.values(beat.stage).flatMap((actions) =>
            actions.flatMap((action) => action.node?.id ?? []))) ?? [],
        variableAliases: [],
        taskAliases: [],
        taskTargets: {},
      },
      {
        id: second[0]?.board?.region_id,
        title: second[0]?.lesson?.title,
        stepIds: [second[1]?.step?.id],
        nodeIds: second[1]?.step?.beats.flatMap((beat) =>
          Object.values(beat.stage).flatMap((actions) =>
            actions.flatMap((action) => action.node?.id ?? []))) ?? [],
        variableAliases: [],
        taskAliases: [],
        taskTargets: {},
      },
    ]);
    const topics = buildOllLessonTopics([first, second]);
    expect(courseReplayStartStep(
      topics,
      second[1]?.step?.id,
    )).toBe(second[1]?.step?.id);
    expect(courseReplayStartStep(
      topics,
      first[1]?.step?.id,
    )).toBe(first[1]?.step?.id);
    const createdRegions = classroom.slice(1).map((event) =>
      event.step?.beats
        .flatMap((beat) => Object.values(beat.stage).flat())
        .find((action) => action.op === "board.create")?.node?.region_id,
    );
    expect(createdRegions).toEqual([
      `topic-${ollArtifactIdentity(artifact!)}`,
      `topic-${ollArtifactIdentity(secondArtifact)}`,
    ]);
    vi.unstubAllGlobals();
  });

  it("keeps variables from independent teaching turns isolated when composing one whiteboard", async () => {
    const lessonWithVariable = (variable: string, initial: number) => ({
      ...structuredClone(authoringLesson),
      lesson: {
        ...structuredClone(authoringLesson.lesson),
        variables: [{
          as: variable,
          initial,
          min: -10,
          max: 10,
          label: variable,
          control: { kind: "slider" },
        }],
      },
      steps: [{
        ...structuredClone(authoringLesson.steps[0]),
        beats: [{
          ...structuredClone(authoringLesson.steps[0]!.beats[0]),
          actions: [
            {
              do: "write",
              as: "graph",
              kind: "plot",
              role: "diagram",
              content: {
                axes: {
                  x: { min: -4, max: 4 },
                  y: { min: -1, max: 10 },
                },
                curves: [{ as: "curve", expression: "x^2", label: "y=x²" }],
                points: [{ as: "moving", x: initial, y: initial ** 2 }],
                bindings: [
                  { target: "moving.x", expression: variable },
                  { target: "moving.y", expression: `${variable}^2` },
                ],
              },
              place: { relation: "new_region" },
            },
            { do: "animate", variable, value: initial + 1 },
          ],
        }],
      }],
      close: { summary: "完成讲解", focus: ["graph"] },
    });
    vi.stubGlobal("fetch", vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => lessonWithVariable("x", 0),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => lessonWithVariable("theta", 1),
      }));
    const firstArtifact = {
      id: "first",
      filename: "first-turn.octos-lesson.json",
      path: "study/oll/first-turn.octos-lesson.json",
      threadId: "first-turn",
      turnId: "first-turn",
    };
    const secondArtifact = {
      id: "second",
      filename: "second-turn.octos-lesson.json",
      path: "study/oll/second-turn.octos-lesson.json",
      threadId: "second-turn",
      turnId: "second-turn",
    };
    const first = await loadOllLessonArtifact(firstArtifact, "session-1");
    const second = await loadOllLessonArtifact(secondArtifact, "session-1");
    const classroom = composeOllClassroomEvents([first, second], "session-1");
    const variableNames = classroom[0]?.lesson?.variables?.map(({ as }) => as) ?? [];
    const animationVariables = classroom.flatMap((event) =>
      event.step?.beats.flatMap((beat) =>
        Object.values(beat.stage).flatMap((actions) =>
          actions.flatMap((action) => action.animation?.variable ?? []))) ?? []);

    expect(variableNames).toHaveLength(2);
    expect(new Set(variableNames).size).toBe(2);
    expect(variableNames).toContain("x");
    expect(variableNames).not.toContain("theta");
    expect(new Set(animationVariables)).toEqual(new Set(variableNames));
    expect(buildOllLessonTopics([first, second]).map((topic) =>
      topic.variableAliases)).toEqual([[variableNames[0]], [variableNames[1]]]);
    const plotContents = classroom.flatMap((event) =>
      event.step?.beats.flatMap((beat) =>
        Object.values(beat.stage).flatMap((actions) =>
          actions.flatMap((action) => action.node?.kind === "plot"
            ? [action.node.content]
            : []))) ?? []);
    expect(plotContents.map((content) =>
      (content.curves as Array<{ expression: string }>).map(({ expression }) =>
        expression,
      ),
    )).toEqual([["x^2"], ["x^2"]]);
    const bindingExpressions = plotContents.map((content) =>
      (content.bindings as Array<{ expression: string }>).map(
        ({ expression }) => expression,
      ));
    expect(bindingExpressions[0]).toEqual(["x", "x^2"]);
    expect(bindingExpressions[1]?.every((expression) =>
      !["theta", "theta^2"].includes(expression))).toBe(true);
    expect(() => reduceCanonicalEvents(classroom)).not.toThrow();
    vi.unstubAllGlobals();
  });

  it("resolves an explicit follow-up against a stable node from an earlier lesson", async () => {
    const firstArtifact = {
      id: "first",
      filename: "first.octos-lesson.json",
      path: "study/oll/first.octos-lesson.json",
      threadId: "first",
      turnId: "first",
    };
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => authoringLesson,
    }));
    const first = await loadOllLessonArtifact(firstArtifact, "session-1");
    const firstClassroom = composeOllClassroomEvents([first], "session-1");
    const firstState = reduceCanonicalEvents(firstClassroom);
    const priorNodeId = Object.keys(firstState.nodes)[0]!;

    const followUp = {
      ...structuredClone(authoringLesson),
      board_context: {
        board_id: "learning-board-session-1",
        revision: firstState.revision,
        references: [{
          as: "board-ref-1-1",
          type: "node",
          target_id: priorNodeId,
          label: "之前的核心结论",
          fragments: [],
        }],
      },
      lesson: {
        ...authoringLesson.lesson,
        title: "围绕原结论继续讲解",
      },
      steps: [{
        key: "follow-up",
        purpose: "在原结论旁补充说明",
        beats: [{
          key: "write-follow-up",
          say: "现在看着刚才的结论，补充它成立的原因。",
          actions: [{
            do: "write",
            as: "follow-up-note",
            kind: "note",
            role: "explanation",
            content: { text: "这条说明保留原内容，只在旁边补充。" },
            place: { relation: "near", anchor: "board-ref-1-1" },
          }, {
            do: "focus",
            when: "after_speech",
            targets: ["board-ref-1-1", "follow-up-note"],
            intent: "compare_original_and_enhancement",
          }],
        }],
      }],
      close: { summary: "完成补充", focus: ["follow-up-note"] },
    };
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => followUp,
    }));
    const second = await loadOllLessonArtifact({
      ...firstArtifact,
      id: "second",
      filename: "second.octos-lesson.json",
      path: "study/oll/second.octos-lesson.json",
      turnId: "second",
    }, "session-1");
    const classroom = composeOllClassroomEvents([first, second], "session-1");
    const state = reduceCanonicalEvents(classroom);
    const followUpNode = Object.values(state.nodes).find((node) =>
      node.content.text === "这条说明保留原内容，只在旁边补充。",
    );

    expect(followUpNode?.placement?.anchor).toBe(priorNodeId);
    expect(state.nodes[priorNodeId]).toBeTruthy();
    expect(state.focus).toContain(priorNodeId);
    vi.unstubAllGlobals();
  });

  it("normalizes absolute live paths and persisted handles identically", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => authoringLesson,
    }));
    const filename = "ce3a5e4c-3ae4-4c8b-9c3f-fbe8eb4fc56b.octos-lesson.json";
    const live = await loadOllLessonArtifact({
      id: `live:${filename}`,
      filename,
      path: `/Users/learner/.octos/profiles/default/data/users/session/workspace/skill-output/study/oll/${filename}`,
      threadId: "client-turn",
      turnId: "server-turn-id",
    }, "session-1");
    const restored = await loadOllLessonArtifact({
      id: `persisted:${filename}`,
      filename,
      path: `pf/cHJvZmlsZS1yZWxhdGl2ZS1wYXRo/${filename}`,
      threadId: filename,
      turnId: filename,
    }, "session-1");

    expect(restored).toEqual(live);
    vi.unstubAllGlobals();
  });

  it("deduplicates one artifact delivered through live and persisted paths", () => {
    const filename = "same-turn.octos-lesson.json";
    const persisted = {
      id: `persisted:${filename}`,
      filename,
      path: `pf/b3BhcXVl/${filename}`,
      threadId: "same-turn",
      turnId: "same-turn",
    };
    const live = {
      id: `live:${filename}`,
      filename,
      path: `/profile/session/workspace/study/oll/${filename}`,
      threadId: "client-turn",
      turnId: "server-turn",
    };

    expect(mergeOllLessonArtifacts([persisted], [live])).toEqual([persisted]);
    expect(mergeOllLessonArtifacts([], [live])).toEqual([live]);
  });

  it("keeps sequence 7 stable when the third lesson changes path source after refresh", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => threeStepAuthoringLesson,
    }));
    const ref = (
      filename: string,
      path: string,
    ) => ({
      id: `${filename}:${path}`,
      filename,
      path,
      threadId: filename,
      turnId: filename,
    });
    const firstRef = ref(
      "first.octos-lesson.json",
      "pf/first/first.octos-lesson.json",
    );
    const secondRef = ref(
      "second.octos-lesson.json",
      "pf/second/second.octos-lesson.json",
    );
    const thirdFilename = "third.octos-lesson.json";
    const thirdLiveRef = ref(
      thirdFilename,
      `/profile/session/workspace/skill-output/study/oll/${thirdFilename}`,
    );
    const thirdRestoredRef = ref(
      thirdFilename,
      `pf/third/${thirdFilename}`,
    );
    const [first, second, thirdLive, thirdRestored] = await Promise.all([
      loadOllLessonArtifact(firstRef, "session-1"),
      loadOllLessonArtifact(secondRef, "session-1"),
      loadOllLessonArtifact(thirdLiveRef, "session-1"),
      loadOllLessonArtifact(thirdRestoredRef, "session-1"),
    ]);

    const liveClassroom = composeOllClassroomEvents(
      [first, second, thirdLive],
      "session-1",
    );
    const restoredClassroom = composeOllClassroomEvents(
      [first, second, thirdRestored],
      "session-1",
    );

    expect(liveClassroom[7]?.sequence).toBe(7);
    expect(restoredClassroom[7]).toEqual(liveClassroom[7]);
    expect(restoredClassroom).toEqual(liveClassroom);
    vi.unstubAllGlobals();
  });
});
