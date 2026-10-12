import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useOllNarrationTts } from "./use-oll-narration-tts";

const mocks = vi.hoisted(() => ({
  synthesizeSpeech: vi.fn(),
  playAudioBlob: vi.fn(),
  stopAudio: vi.fn(),
}));

vi.mock("@/api/voice", () => ({
  synthesizeSpeech: mocks.synthesizeSpeech,
}));
vi.mock("@/home/voice/audio-playback", () => ({
  playAudioBlob: mocks.playAudioBlob,
  stopAudio: mocks.stopAudio,
}));

describe("useOllNarrationTts", () => {
  beforeEach(() => {
    mocks.synthesizeSpeech.mockReset();
    mocks.playAudioBlob.mockReset();
    mocks.stopAudio.mockReset();
    mocks.synthesizeSpeech.mockResolvedValue(
      new Blob(["audio"], { type: "audio/wav" }),
    );
    mocks.playAudioBlob.mockResolvedValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("OLL-TCH-002 uses the profile-backed speech synthesis path", async () => {
    const onSpeakingChange = vi.fn();
    const onPlaybackStart = vi.fn();
    const onPlaybackComplete = vi.fn();
    renderHook(() =>
      useOllNarrationTts({
        enabled: true,
        playing: true,
        text: "先从三出发。",
        narrationId: "beat-1",
        onSpeakingChange,
        onPlaybackStart,
        onPlaybackComplete,
      }),
    );

    await waitFor(() =>
      expect(mocks.synthesizeSpeech).toHaveBeenCalledWith(
        "先从三出发。",
        expect.any(AbortSignal),
      ),
    );
    await waitFor(() =>
      expect(mocks.playAudioBlob).toHaveBeenCalledTimes(1),
    );
    expect(onSpeakingChange).toHaveBeenLastCalledWith(true);
    expect(onPlaybackStart).toHaveBeenCalledWith("beat-1");
    const onEnded = mocks.playAudioBlob.mock.calls[0]?.[1] as
      | (() => void)
      | undefined;
    act(() => onEnded?.());
    expect(onSpeakingChange).toHaveBeenLastCalledWith(false);
    expect(onPlaybackComplete).toHaveBeenCalledOnce();
    expect(onPlaybackComplete).toHaveBeenCalledWith("beat-1");
  });

  it("reports preparation until audio really starts", async () => {
    let resolveSpeech: ((audio: Blob) => void) | undefined;
    mocks.synthesizeSpeech.mockImplementation(() =>
      new Promise<Blob>((resolve) => {
        resolveSpeech = resolve;
      })
    );
    const onPlaybackStart = vi.fn();
    const { result } = renderHook(() =>
      useOllNarrationTts({
        enabled: true,
        playing: true,
        text: "请稍等一下。",
        narrationId: "beat-preparing",
        onPlaybackStart,
      }),
    );

    await waitFor(() => expect(result.current.preparing).toBe(true));
    expect(onPlaybackStart).not.toHaveBeenCalled();
    act(() => resolveSpeech?.(new Blob(["prepared-audio"])));
    await waitFor(() => expect(mocks.playAudioBlob).toHaveBeenCalledOnce());
    await waitFor(() => expect(result.current.preparing).toBe(false));
    expect(onPlaybackStart).toHaveBeenCalledWith("beat-preparing");
  });

  it("prefetches the next Beat once and reuses it for playback", async () => {
    const firstAudio = new Blob(["first"]);
    const secondAudio = new Blob(["second"]);
    let resolvePlaybackStart: ((started: boolean) => void) | undefined;
    mocks.playAudioBlob.mockImplementationOnce(() =>
      new Promise<boolean>((resolve) => {
        resolvePlaybackStart = resolve;
      })
    );
    mocks.synthesizeSpeech.mockImplementation((text: string) =>
      Promise.resolve(text === "第二段。" ? secondAudio : firstAudio)
    );
    const { rerender } = renderHook(
      ({ text, narrationId, upcomingText, upcomingNarrationId }) =>
        useOllNarrationTts({
          enabled: true,
          playing: true,
          text,
          narrationId,
          prefetchEnabled: true,
          upcomingText,
          upcomingNarrationId,
        }),
      {
        initialProps: {
          text: "第一段。",
          narrationId: "beat-1",
          upcomingText: "第二段。",
          upcomingNarrationId: "beat-2" as string | undefined,
        },
      },
    );

    await waitFor(() => expect(mocks.playAudioBlob).toHaveBeenCalledOnce());
    expect(mocks.synthesizeSpeech).toHaveBeenCalledTimes(1);
    act(() => resolvePlaybackStart?.(true));
    await waitFor(() => expect(mocks.synthesizeSpeech).toHaveBeenCalledTimes(2));
    rerender({
      text: "第二段。",
      narrationId: "beat-2",
      upcomingText: "",
      upcomingNarrationId: undefined,
    });
    await waitFor(() =>
      expect(mocks.playAudioBlob).toHaveBeenCalledWith(
        secondAudio,
        expect.any(Function),
        expect.any(AbortSignal),
      ),
    );
    expect(
      mocks.synthesizeSpeech.mock.calls.filter(([text]) => text === "第二段。"),
    ).toHaveLength(1);
  });

  it("cancels stale synthesis and playback when the Beat changes", async () => {
    let firstSignal: AbortSignal | undefined;
    const onPlaybackComplete = vi.fn();
    mocks.synthesizeSpeech.mockImplementation(
      (_text: string, signal: AbortSignal) => {
        firstSignal ??= signal;
        return Promise.resolve(new Blob(["audio"]));
      },
    );
    const { rerender } = renderHook(
      ({ text, narrationId }) =>
        useOllNarrationTts({
          enabled: true,
          playing: true,
          text,
          narrationId,
          onPlaybackComplete,
        }),
      { initialProps: { text: "第一段。", narrationId: "beat-1" } },
    );

    await waitFor(() => expect(mocks.playAudioBlob).toHaveBeenCalledTimes(1));
    const staleOnEnded = mocks.playAudioBlob.mock.calls[0]?.[1] as
      | (() => void)
      | undefined;
    rerender({ text: "第二段。", narrationId: "beat-2" });

    expect(firstSignal?.aborted).toBe(true);
    expect(mocks.stopAudio).toHaveBeenCalled();
    act(() => staleOnEnded?.());
    expect(onPlaybackComplete).not.toHaveBeenCalledWith("beat-1");
    await waitFor(() =>
      expect(mocks.synthesizeSpeech).toHaveBeenLastCalledWith(
        "第二段。",
        expect.any(AbortSignal),
      ),
    );
  });

  it("R-007 keeps visible narration independent when provider synthesis fails", async () => {
    mocks.synthesizeSpeech.mockRejectedValue(new Error("provider down"));
    const onPlaybackComplete = vi.fn();
    const { result, rerender } = renderHook(
      ({ playing, text, narrationId }) => useOllNarrationTts({
        enabled: true,
        playing,
        text,
        narrationId,
        onPlaybackComplete,
      }),
      {
        initialProps: {
          playing: true,
          text: "旁白仍然可见。",
          narrationId: "beat-failed" as string | undefined,
        },
      },
    );

    await waitFor(() =>
      expect(result.current.error).toBe(
        "课程语音暂时不可用，旁白仍会显示。",
      ),
    );
    expect(mocks.playAudioBlob).not.toHaveBeenCalled();
    expect(onPlaybackComplete).toHaveBeenCalledWith("beat-failed");

    rerender({ playing: false, text: "", narrationId: undefined });
    expect(result.current.error).toBe(
      "课程语音暂时不可用，旁白仍会显示。",
    );

    mocks.synthesizeSpeech.mockResolvedValue(
      new Blob(["audio"], { type: "audio/wav" }),
    );
    rerender({
      playing: true,
      text: "下一段恢复语音。",
      narrationId: "beat-recovered",
    });
    await waitFor(() => expect(result.current.error).toBeNull());
  });

  it("releases disabled narration but keeps paused narration pending", () => {
    const onPlaybackComplete = vi.fn();
    const { rerender } = renderHook(
      ({ enabled, playing }) =>
        useOllNarrationTts({
          enabled,
          playing,
          text: "不应朗读。",
          narrationId: "beat-disabled",
          onPlaybackComplete,
        }),
      { initialProps: { enabled: false, playing: true } },
    );
    expect(mocks.synthesizeSpeech).not.toHaveBeenCalled();
    expect(onPlaybackComplete).toHaveBeenCalledWith("beat-disabled");

    onPlaybackComplete.mockClear();
    act(() => rerender({ enabled: true, playing: false }));
    expect(mocks.synthesizeSpeech).not.toHaveBeenCalled();
    expect(mocks.stopAudio).toHaveBeenCalled();
    expect(onPlaybackComplete).not.toHaveBeenCalled();
  });
});
