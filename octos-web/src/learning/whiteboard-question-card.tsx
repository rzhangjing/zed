import { useState } from "react";
import type { WhiteboardQuestionRecord } from "./whiteboard-questions";

export const WHITEBOARD_QUESTION_CARD_WIDTH = 270;

const statusCopy: Record<WhiteboardQuestionRecord["status"], string> = {
  pending: "正在准备回答",
  answered: "已回答",
  failed: "没有生成成功",
};

export function WhiteboardQuestionCard({
  question,
  left,
  top,
  linked = false,
}: {
  question: WhiteboardQuestionRecord;
  left: number;
  top: number;
  linked?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <article
      className={linked
        ? "learning-whiteboard-question-card is-linked"
        : "learning-whiteboard-question-card"}
      style={{ left, top, width: WHITEBOARD_QUESTION_CARD_WIDTH }}
      data-question-id={question.id}
    >
      <header>
        <strong>我的问题</strong>
        <span className={`is-${question.status}`}>
          {statusCopy[question.status]}
        </span>
      </header>
      <p className={expanded ? "is-expanded" : undefined}>{question.text}</p>
      {question.text.length > 72 ? (
        <button
          type="button"
          onClick={() => setExpanded((current) => !current)}
          aria-expanded={expanded}
        >
          {expanded ? "收起" : "展开完整问题"}
        </button>
      ) : null}
    </article>
  );
}
