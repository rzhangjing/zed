import { useEffect, useRef, useState } from "react";
import { synthesizeSpeech } from "@/api/voice";
import {
  playAudioBlob,
  stopAudio,
} from "@/home/voice/audio-playback";

export interface OllNarrationTtsOptions {
  enabled: boolean;
  playing: boolean;
  text: string;
  narrationId?: string;
  prefetchEnabled?: boolean;
  upcomingText?: string;
  upcomingNarrationId?: string;
  onSpeakingChange?: (speaking: boolean) => void;
  onPlaybackStart?: (narrationId: string) => void;
  onPlaybackComplete?: (narrationId: string) => void;
}

export interface OllNarrationTtsState {
  error: string | null;
  preparing: boolean;
}

interface PrefetchedSpeech {
  key: string;
  controller: AbortController;
  promise: Promise<Blob | null>;
  claimed: boolean;
}

function speechKey(narrationId: string | undefined, text: string): string | null {
  return narrationId && text ? `${narrationId}\u0000${text}` : null;
}

/**
 * Plays the Runtime's current narration through the profile's system TTS.
 *
 * The hook knows nothing about the learner's input mode. Text and voice input
 * therefore share this exact synthesis, cancellation, and playback path.
 */
export function useOllNarrationTts({
  enabled,
  playing,
  text,
  narrationId,
  prefetchEnabled = false,
  upcomingText = "",
  upcomingNarrationId,
  onSpeakingChange,
  onPlaybackStart,
  onPlaybackComplete,
}: OllNarrationTtsOptions): OllNarrationTtsState {
  const normalizedText = text.trim();
  const normalizedUpcomingText = upcomingText.trim();
  const currentKey = speechKey(narrationId, normalizedText);
  const upcomingKey = speechKey(upcomingNarrationId, normalizedUpcomingText);
  const [failure, setFailure] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [startedKey, setStartedKey] = useState<string | null>(null);
  const prefetchedSpeechRef = useRef<PrefetchedSpeech | null>(null);

  useEffect(() => {
    const request = new AbortController();
    let current = true;
    let completed = false;
    const completePlayback = () => {
      if (!current || completed || !narrationId) return;
      completed = true;
      onPlaybackComplete?.(narrationId);
    };

    if (!enabled || !playing || !normalizedText) {
      queueMicrotask(() => {
        if (!current) return;
        setPreparing(false);
        setStartedKey(null);
      });
      onSpeakingChange?.(false);
      stopAudio();
      if (!enabled && playing && normalizedText) completePlayback();
      return () => {
        current = false;
        request.abort();
      };
    }

    queueMicrotask(() => {
      if (!current) return;
      setPreparing(true);
      setStartedKey(null);
    });
    const cached = currentKey && prefetchedSpeechRef.current?.key === currentKey
      ? prefetchedSpeechRef.current
      : null;
    if (cached) {
      cached.claimed = true;
      prefetchedSpeechRef.current = null;
    }
    const audioRequest = cached?.controller ?? request;
    const audioPromise = cached?.promise ?? synthesizeSpeech(
      normalizedText,
      audioRequest.signal,
    ).then((audio) => audio as Blob | null);

    void audioPromise
      .then((audio) =>
        audio ?? synthesizeSpeech(normalizedText, audioRequest.signal)
      )
      .then(async (audio) => {
        if (!current || audioRequest.signal.aborted) return;
        setFailure(null);
        const started = await playAudioBlob(
          audio,
          () => {
            if (!current) return;
            onSpeakingChange?.(false);
            completePlayback();
          },
          audioRequest.signal,
        );
        if (!current || audioRequest.signal.aborted) return;
        setPreparing(false);
        if (started) {
          onSpeakingChange?.(true);
          setStartedKey(currentKey);
          if (narrationId) onPlaybackStart?.(narrationId);
          return;
        }
        if (!started && current) {
          setStartedKey(null);
          onSpeakingChange?.(false);
          setFailure("当前设备无法播放课程语音，旁白仍会显示。");
          completePlayback();
        }
      })
      .catch((cause: unknown) => {
        if (
          !current ||
          audioRequest.signal.aborted ||
          (cause instanceof DOMException && cause.name === "AbortError")
        ) {
          return;
        }
        setPreparing(false);
        setStartedKey(null);
        onSpeakingChange?.(false);
        setFailure("课程语音暂时不可用，旁白仍会显示。");
        completePlayback();
      });

    return () => {
      current = false;
      request.abort();
      if (cached) cached.controller.abort();
      onSpeakingChange?.(false);
      stopAudio();
    };
  }, [
    enabled,
    narrationId,
    normalizedText,
    currentKey,
    onPlaybackComplete,
    onPlaybackStart,
    onSpeakingChange,
    playing,
  ]);

  useEffect(() => {
    const existing = prefetchedSpeechRef.current;
    if (
      !enabled ||
      !prefetchEnabled ||
      !currentKey ||
      startedKey !== currentKey
    ) {
      if ((!enabled || !prefetchEnabled) && existing && !existing.claimed) {
        existing.controller.abort();
        prefetchedSpeechRef.current = null;
      }
      return;
    }
    if (!upcomingKey || upcomingKey === currentKey) return;
    if (existing?.key === upcomingKey) return;
    if (existing && !existing.claimed) existing.controller.abort();

    const controller = new AbortController();
    const entry: PrefetchedSpeech = {
      key: upcomingKey,
      controller,
      claimed: false,
      promise: synthesizeSpeech(normalizedUpcomingText, controller.signal)
        .then((audio) => audio)
        .catch(() => null),
    };
    prefetchedSpeechRef.current = entry;
  }, [
    currentKey,
    enabled,
    normalizedUpcomingText,
    prefetchEnabled,
    startedKey,
    upcomingKey,
  ]);

  useEffect(() => () => {
    const pending = prefetchedSpeechRef.current;
    if (pending && !pending.claimed) pending.controller.abort();
    prefetchedSpeechRef.current = null;
  }, []);

  return {
    error: enabled ? failure : null,
    preparing,
  };
}
