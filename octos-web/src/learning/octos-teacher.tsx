import { useEffect, useRef, useState } from "react";

import type { VoiceState } from "@/home/voice/use-voice-conversation";
import { OctosSkinArt } from "@/components/octos-skin-art";
import { MarkdownContent } from "@/components/markdown-renderer";
import { useTeacherSkin } from "@/hooks/use-teacher-skin";

const STATE_LABEL: Record<VoiceState, string> = {
  idle: "轻触开始",
  listening: "我在听",
  thinking: "正在想",
  speaking: "正在讲",
  error: "轻触重试",
};

export function OctosTeacher({
  state,
  speech,
  preparing = false,
  stateLabel,
  onClick,
}: {
  state: VoiceState;
  speech: string;
  preparing?: boolean;
  stateLabel?: string;
  onClick: () => void;
}) {
  const { skin } = useTeacherSkin();
  const [reactionKey, setReactionKey] = useState(0);
  const [reacting, setReacting] = useState(false);
  const reactionTimerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (reactionTimerRef.current !== null) {
        window.clearTimeout(reactionTimerRef.current);
      }
    },
    [],
  );

  const handleClick = () => {
    if (reactionTimerRef.current !== null) {
      window.clearTimeout(reactionTimerRef.current);
    }
    setReactionKey((current) => current + 1);
    setReacting(true);
    reactionTimerRef.current = window.setTimeout(() => {
      setReacting(false);
      reactionTimerRef.current = null;
    }, 760);
    onClick();
  };

  return (
    <div className="octos-teacher" data-learning-board-occlusion="">
      {speech && (
        <div className="octos-teacher-caption" aria-live="polite">
          <MarkdownContent
            text={speech}
            className="octos-teacher-caption-content"
          />
        </div>
      )}
      <button
        type="button"
        className="octos-teacher-avatar"
        data-state={state}
        data-preparing={preparing ? "true" : undefined}
        data-reacting={reacting ? "true" : undefined}
        onClick={handleClick}
        aria-busy={preparing}
        aria-label={
          preparing
            ? "Octos 正在准备下一步"
            : state === "speaking" || state === "thinking"
            ? "打断 Octos"
            : "和 Octos 说话"
        }
      >
        <span className="octos-teacher-halo" />
        {preparing && (
          <span className="octos-teacher-preparing" aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
        )}
        {reacting && (
          <span
            key={reactionKey}
            className="octos-teacher-reaction"
            aria-hidden="true"
          >
            <span>✦</span>
            <span>●</span>
            <span>✦</span>
          </span>
        )}
        <OctosSkinArt
          skin={skin}
          className="octos-teacher-avatar-art"
          eager
          activity={preparing ? "thinking" : state}
          reactionKey={reactionKey}
        />
        <span className="octos-teacher-state">
          {preparing ? "稍等一下" : stateLabel ?? STATE_LABEL[state]}
        </span>
      </button>
    </div>
  );
}
