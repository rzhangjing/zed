import { describe, expect, it } from "vitest";
import { buildVoiceTurns } from "@/home/voice/use-voice-conversation";
import { projectionToRenderThreads } from "@/store/projection-render-adapter";
import { project } from "@/store/projection";
import { hydrateProjectionEnvelopes } from "./hydrate-projection";

describe("hydrateProjectionEnvelopes", () => {
  it("restores the real direct-voice Learn hydrate shape without browser state", () => {
    const sessionId = "learn-1787398124712-u9i7c0";
    const turnId = "a9975c2c-cd54-4a3a-b773-6c4d110ccd97";
    const envelopes = hydrateProjectionEnvelopes(sessionId, undefined, {
      session_id: sessionId,
      cursor: { stream: sessionId, seq: 541 },
      messages: [
        {
          seq: 0,
          role: "user",
          content: [
            "[[LEARNING_SESSION]]",
            "version: 4",
            `session_id: ${sessionId}`,
            "entry: direct",
            "provisional: true",
            "[[/LEARNING_SESSION]]",
            "[[LEARNING_CONTEXT]]",
            "active: true",
            `session_id: ${sessionId}`,
            `turn_id: ${turnId}`,
            "[[/LEARNING_CONTEXT]]",
            "自然对数意义是怎么推导的？",
          ].join("\n"),
          thread_id: turnId,
          persisted_at: "2026-08-22T11:29:12.808855Z",
          media: ["uploads/utterance.wav"],
        },
        {
          seq: 1,
          role: "assistant",
          content: "课程已经放到白板上啦。",
          thread_id: turnId,
          persisted_at: "2026-08-22T11:29:52.754669Z",
          message_id: "assistant-row-1",
        },
      ],
      replayed_envelopes: [],
      replayed_tool_envelopes: [],
    });

    expect(envelopes).not.toBeNull();
    const threads = projectionToRenderThreads(project(envelopes ?? []));
    expect(threads).toHaveLength(1);
    expect(threads[0].id).toBe(turnId);
    expect(buildVoiceTurns(threads)[0]).toMatchObject({
      id: turnId,
      userText: "自然对数意义是怎么推导的？",
      assistantText: "课程已经放到白板上啦。",
    });
  });

  it("prefers a canonical projection snapshot when the server supplies one", () => {
    const envelopes = hydrateProjectionEnvelopes("session-a", undefined, {
      session_id: "session-a",
      cursor: { stream: "session-a", seq: 1 },
      messages: [],
      projection_snapshot: {
        cursor: { stream: "session-a", seq: 1 },
        envelopes: [{
          session_id: "session-a",
          thread_id: "thread-a",
          turn_id: "turn-a",
          seq: 1,
          payload: {
            type: "user_message",
            data: { text: "canonical", files: [] },
          },
        }],
      },
    });

    expect(envelopes).toHaveLength(1);
    expect(envelopes?.[0].payload).toMatchObject({
      type: "user_message",
      data: { text: "canonical" },
    });
  });
});
