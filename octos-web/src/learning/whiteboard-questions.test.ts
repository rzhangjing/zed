import { beforeEach, describe, expect, it } from "vitest";
import {
  hasSavedWhiteboardQuestion,
  loadWhiteboardQuestions,
  saveWhiteboardQuestions,
  whiteboardQuestionsStorageKey,
  type WhiteboardQuestionRecord,
} from "./whiteboard-questions";

describe("whiteboard questions", () => {
  beforeEach(() => localStorage.clear());

  it("preserves the learner's exact question, source and state", () => {
    const sessionId = "learn-question-store";
    const question: WhiteboardQuestionRecord = {
      id: "turn-1",
      sessionId,
      text: "  为什么 y=x² 的图像是这样？\n请结合我圈出的部分。  ",
      origin: "selection",
      createdAt: "2026-08-17T15:00:00.000Z",
      status: "failed",
      error: "当前公式暂不支持生成函数图像",
      source: {
        sourceId: "source-1",
        bounds: { x: 10, y: 20, width: 90, height: 40 },
      },
    };

    saveWhiteboardQuestions(sessionId, [question]);

    expect(loadWhiteboardQuestions(sessionId)).toEqual([question]);
    expect(hasSavedWhiteboardQuestion(sessionId)).toBe(true);
  });

  it("ignores records belonging to another session", () => {
    localStorage.setItem(
      whiteboardQuestionsStorageKey("learn-a"),
      JSON.stringify([{
        id: "turn-1",
        sessionId: "learn-b",
        text: "不应串到另一块白板",
        origin: "composer",
        createdAt: "2026-08-17T15:00:00.000Z",
        status: "answered",
      }]),
    );

    expect(loadWhiteboardQuestions("learn-a")).toEqual([]);
  });
});
