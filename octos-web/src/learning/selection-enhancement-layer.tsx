import { Minimize2, Trash2 } from "lucide-react";
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  plotPathData,
  renderScene3d,
  sampleImplicitPlotExpression,
  samplePlotExpression,
} from "octos-lesson-language/web-runtime";
import type { InkSelectionSnapshot } from "octos-lesson-language/ink-runtime";
import { MarkdownContent } from "@/components/markdown-renderer";
import type { SelectionEnhancementArtifact } from "./selection-enhancements";
import type { WhiteboardQuestionRecord } from "./whiteboard-questions";
import {
  WhiteboardLoadingBlock,
  type WhiteboardLoadingState,
} from "./whiteboard-loading-block";

const DEFAULT_CARD_SCALE = 1;
const MIN_CARD_SCALE = .85;
const MAX_CARD_SCALE = 2.25;
const CARD_WIDTH = 330;
const CARD_FONT_SIZE = 13;
const SCENE3D_HEIGHT = 270;
const CARD_GAP = 24;
const RESTORED_SOURCE_OVERLAP_THRESHOLD = .8;

export interface SelectionEnhancementLoading {
  turnId: string;
  sourceId: string;
  bounds: InkSelectionSnapshot["bounds"];
  state: WhiteboardLoadingState;
}

function clampedCardScale(value: number): number {
  return Math.min(MAX_CARD_SCALE, Math.max(MIN_CARD_SCALE, value));
}

function boundsOverlapRatio(
  left: InkSelectionSnapshot["bounds"],
  right: InkSelectionSnapshot["bounds"],
): number {
  const intersectionWidth = Math.max(
    0,
    Math.min(left.x + left.width, right.x + right.width)
      - Math.max(left.x, right.x),
  );
  const intersectionHeight = Math.max(
    0,
    Math.min(left.y + left.height, right.y + right.height)
      - Math.max(left.y, right.y),
  );
  const smallerArea = Math.min(
    left.width * left.height,
    right.width * right.height,
  );
  if (smallerArea <= 0) return 0;
  return intersectionWidth * intersectionHeight / smallerArea;
}

function SelectionQuestionSection({
  question,
}: {
  question: WhiteboardQuestionRecord;
}) {
  return (
    <section className="learning-selection-enhancement-question">
      <div>
        <strong>我的问题</strong>
        <span>{question.status === "answered"
          ? "已回答"
          : question.status === "pending"
            ? "正在准备回答"
            : "没有生成成功"}</span>
      </div>
      <MarkdownContent
        text={question.text}
        className="learning-selection-markdown learning-selection-question-text"
      />
    </section>
  );
}

function SelectionPlot({
  artifact,
}: {
  artifact: SelectionEnhancementArtifact & {
    response: Extract<
      SelectionEnhancementArtifact["response"],
      { kind: "plot" }
    >;
  };
}) {
  const { expression, x_range: xRange, y_range: yRange } = artifact.response;
  const left = 24;
  const top = 12;
  const right = 276;
  const bottom = 150;
  const mapX = (value: number) =>
    left + (value - xRange.min) / (xRange.max - xRange.min) * (right - left);
  const mapY = (value: number) =>
    bottom - (value - yRange.min) / (yRange.max - yRange.min) * (bottom - top);
  let path = "";
  let error = "";
  try {
    path = plotPathData(
      artifact.response.plot_kind === "implicit"
        ? sampleImplicitPlotExpression(expression, xRange, yRange, {
            level: artifact.response.level,
            samples: artifact.response.samples,
          })
        : samplePlotExpression(expression, xRange, yRange),
      mapX,
      mapY,
    );
  } catch (cause) {
    error = cause instanceof Error ? cause.message : "函数表达式无法绘制";
  }
  const xAxis = Math.min(bottom, Math.max(top, mapY(0)));
  const yAxis = Math.min(right, Math.max(left, mapX(0)));
  return (
    <div className="learning-selection-plot">
      {error ? (
        <span role="alert">{error}</span>
      ) : (
        <svg viewBox="0 0 300 164" aria-label={artifact.response.title}>
          <line x1={left} y1={xAxis} x2={right} y2={xAxis} />
          <line x1={yAxis} y1={top} x2={yAxis} y2={bottom} />
          <path d={path} />
        </svg>
      )}
      <code>
        {artifact.response.plot_kind === "implicit"
          ? `${expression} = ${artifact.response.level ?? 0}`
          : `y = ${expression}`}
      </code>
    </div>
  );
}

function SelectionScene3d({
  artifact,
}: {
  artifact: SelectionEnhancementArtifact & {
    response: Extract<
      SelectionEnhancementArtifact["response"],
      { kind: "scene3d" }
    >;
  };
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    container.replaceChildren();
    try {
      renderScene3d(
        container,
        {
          id: `selection-scene3d:${artifact.turn_id}`,
          kind: "scene3d",
          content: artifact.response.content,
        },
        undefined,
        {},
      );
    } catch (cause) {
      const message = container.ownerDocument.createElement("span");
      message.setAttribute("role", "alert");
      message.textContent = cause instanceof Error
        ? cause.message
        : "三维函数图无法显示";
      container.replaceChildren(message);
    }
    return () => container.replaceChildren();
  }, [artifact]);
  return (
    <div className="learning-selection-scene3d">
      <div ref={containerRef} />
    </div>
  );
}

export function SelectionEnhancementLayer({
  artifacts,
  sources,
  questions = [],
  loading = null,
  currentDocumentVersion,
  invalidTargetTurnIds = new Set(),
  onDelete,
}: {
  artifacts: SelectionEnhancementArtifact[];
  sources: InkSelectionSnapshot[];
  questions?: WhiteboardQuestionRecord[];
  loading?: SelectionEnhancementLoading | null;
  currentDocumentVersion: number;
  invalidTargetTurnIds?: ReadonlySet<string>;
  onDelete: (turnId: string) => void;
}) {
  const [minimizedTurnIds, setMinimizedTurnIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [cardScaleByTurnId, setCardScaleByTurnId] = useState<
    Readonly<Record<string, number>>
  >({});
  const resizingCardRef = useRef<{
    turnId: string;
    pointerId: number;
    startX: number;
    startY: number;
    startScale: number;
  } | null>(null);
  const updateCardScale = (turnId: string, scale: number) => {
    setCardScaleByTurnId((current) => ({
      ...current,
      [turnId]: clampedCardScale(scale),
    }));
  };
  const beginCardResize = (
    event: ReactPointerEvent<HTMLButtonElement>,
    turnId: string,
    startScale: number,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    if (!Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) {
      return;
    }
    resizingCardRef.current = {
      turnId,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      startScale,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };
  const continueCardResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = resizingCardRef.current;
    if (
      !resize
      || resize.pointerId !== event.pointerId
      || !Number.isFinite(event.clientX)
      || !Number.isFinite(event.clientY)
    ) return;
    event.preventDefault();
    const diagonalMovement = (
      event.clientX - resize.startX + event.clientY - resize.startY
    ) / 2;
    updateCardScale(
      resize.turnId,
      resize.startScale + diagonalMovement / 220,
    );
  };
  const finishCardResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = resizingCardRef.current;
    if (!resize || resize.pointerId !== event.pointerId) return;
    resizingCardRef.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };
  const resizeCardWithKeyboard = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    turnId: string,
    currentScale: number,
  ) => {
    if (["ArrowUp", "ArrowRight"].includes(event.key)) {
      event.preventDefault();
      updateCardScale(turnId, currentScale + .1);
    } else if (["ArrowDown", "ArrowLeft"].includes(event.key)) {
      event.preventDefault();
      updateCardScale(turnId, currentScale - .1);
    } else if (event.key === "Home") {
      event.preventDefault();
      updateCardScale(turnId, DEFAULT_CARD_SCALE);
    }
  };
  type LayoutItem =
    | { kind: "question"; key: string; question: WhiteboardQuestionRecord }
    | {
        kind: "artifact";
        key: string;
        artifact: SelectionEnhancementArtifact;
        question?: WhiteboardQuestionRecord;
      }
    | { kind: "loading"; key: string; loading: SelectionEnhancementLoading };
  const sourceById = new Map(sources.map((source) => [source.source_id, source]));
  const selectionQuestions = questions.filter((question) =>
    question.origin === "selection" && question.source);
  const sourceIds = new Set([
    ...artifacts.map((artifact) => artifact.source.source_id),
    ...selectionQuestions.map((question) => question.source!.sourceId),
    ...(loading ? [loading.sourceId] : []),
  ]);
  const artifactSourceById = new Map(artifacts.map((candidate) => [
    candidate.source.source_id,
    candidate.source,
  ]));
  const questionSourceById = new Map(selectionQuestions.map((question) => [
    question.source!.sourceId,
    question.source!.bounds,
  ]));
  const sourceGroups: Array<{
    sourceIds: Set<string>;
    identity?: string;
    documentId?: string;
    bounds: InkSelectionSnapshot["bounds"][];
    hasLocalSnapshot: boolean;
  }> = [];
  for (const sourceId of sourceIds) {
    const source = sourceById.get(sourceId);
    // A fresh rectangle/lasso capture receives a new source_id even when it
    // contains the same immutable strokes. Version 3 snapshots carry their
    // stable component IDs; older snapshots fall back to their captured SVG.
    // The gesture region is deliberately excluded so drawing a slightly
    // different rectangle around the same writing does not restart the card
    // stack at the same coordinates.
    const sourceIdentity = source?.component_ids?.length
      ? [...source.component_ids].sort().join("\u0000")
      : source?.svg;
    const artifactSource = artifactSourceById.get(sourceId);
    const documentId = source?.document_id ?? artifactSource?.document_id;
    const bounds = source?.bounds
      ?? questionSourceById.get(sourceId)
      ?? artifactSource?.bounds;
    const identity = source
      ? `${source.document_id}\u0000${sourceIdentity}`
      : undefined;
    const group = sourceGroups.find((candidate) => {
      if (identity && candidate.identity) return identity === candidate.identity;
      // Durable artifacts intentionally remain visible even when their
      // browser-local snapshots are unavailable after refresh. In that case,
      // recover their former stack from the document id and substantially
      // overlapping persisted bounds instead of treating every source_id as a
      // new lane at the same coordinates.
      if (source && candidate.hasLocalSnapshot) return false;
      if (
        documentId
        && candidate.documentId
        && documentId !== candidate.documentId
      ) return false;
      return Boolean(bounds && candidate.bounds.some((candidateBounds) =>
        boundsOverlapRatio(bounds, candidateBounds)
          >= RESTORED_SOURCE_OVERLAP_THRESHOLD));
    });
    if (group) {
      group.sourceIds.add(sourceId);
      if (bounds) group.bounds.push(bounds);
      group.hasLocalSnapshot ||= Boolean(source);
      group.identity ??= identity;
      group.documentId ??= documentId;
    } else {
      sourceGroups.push({
        sourceIds: new Set([sourceId]),
        identity,
        documentId,
        bounds: bounds ? [bounds] : [],
        hasLocalSnapshot: Boolean(source),
      });
    }
  }
  const layoutItems: Array<LayoutItem & { left: number; top: number }> = [];
  for (const { sourceIds: groupedSourceIds } of sourceGroups) {
    const sourceQuestions = selectionQuestions
      .filter((question) => groupedSourceIds.has(question.source!.sourceId))
      .sort((left, right) => left.createdAt.localeCompare(right.createdAt));
    const sourceArtifacts = artifacts
      .filter((artifact) => groupedSourceIds.has(artifact.source.source_id))
      .sort((left, right) => left.created_at.localeCompare(right.created_at));
    const artifactByTurnId = new Map(sourceArtifacts.map((artifact) => [
      artifact.turn_id,
      artifact,
    ]));
    const ordered: LayoutItem[] = [];
    for (const question of sourceQuestions) {
      const matchingArtifact = artifactByTurnId.get(question.id);
      if (matchingArtifact) {
        ordered.push({
          kind: "artifact",
          key: `artifact:${matchingArtifact.turn_id}`,
          artifact: matchingArtifact,
          question,
        });
        artifactByTurnId.delete(question.id);
      } else {
        ordered.push({
          kind: "question",
          key: `question:${question.id}`,
          question,
        });
      }
    }
    for (const artifact of sourceArtifacts) {
      if (!artifactByTurnId.has(artifact.turn_id)) continue;
      ordered.push({
        kind: "artifact",
        key: `artifact:${artifact.turn_id}`,
        artifact,
      });
    }
    if (
      loading
      && groupedSourceIds.has(loading.sourceId)
      && !sourceQuestions.some((question) => question.id === loading.turnId)
      && !ordered.some((item) => item.kind === "loading")
    ) {
      ordered.push({
        kind: "loading",
        key: `loading:${loading.turnId}`,
        loading,
      });
    }
    const fallbackBounds = [...groupedSourceIds]
      .flatMap((sourceId) => sourceById.get(sourceId)?.bounds ?? [])[0]
      ?? sourceQuestions[0]?.source?.bounds
      ?? sourceArtifacts[0]?.source.bounds
      ?? loading?.bounds;
    if (!fallbackBounds) continue;
    let cursor = fallbackBounds.x + fallbackBounds.width + 30;
    for (const item of ordered) {
      layoutItems.push({
        ...item,
        left: cursor,
        top: fallbackBounds.y,
      });
      const width = item.kind === "question"
        ? CARD_WIDTH
        : item.kind === "loading"
          ? 330
          : minimizedTurnIds.has(item.artifact.turn_id)
            ? 26
            : CARD_WIDTH * (cardScaleByTurnId[item.artifact.turn_id]
              ?? DEFAULT_CARD_SCALE);
      cursor += width + CARD_GAP;
    }
  }
  return (
    <>
      {layoutItems.map((item) => {
        if (item.kind === "question") {
          const failed = item.question.status === "failed";
          const questionLoading = loading?.turnId === item.question.id
            ? loading
            : null;
          return (
            <article
              key={item.key}
              className={failed
                ? "learning-selection-enhancement is-failed"
                : "learning-selection-enhancement is-pending"}
              style={{
                left: item.left,
                top: item.top,
                width: CARD_WIDTH,
                fontSize: CARD_FONT_SIZE,
              }}
              data-source-id={item.question.source?.sourceId}
              data-question-id={item.question.id}
            >
              <div className="learning-selection-source-link" aria-hidden="true" />
              <SelectionQuestionSection question={item.question} />
              <header>
                <div>
                  <span>小章鱼辅助</span>
                  <small>来自当前选区</small>
                </div>
              </header>
              <div
                className="learning-selection-enhancement-content learning-selection-enhancement-placeholder"
                role={failed ? "alert" : "status"}
                aria-live="polite"
              >
                <strong>{failed
                  ? "回答生成失败"
                  : questionLoading?.state.title ?? "正在生成选区辅助内容"}</strong>
                <p>{failed
                  ? item.question.error ?? "选区辅助内容生成失败，请重试"
                  : questionLoading?.state.detail
                    ?? "正在理解这部分内容，并把辅助说明放在选区旁边。"}</p>
                {!failed ? <span aria-hidden="true" /> : null}
              </div>
            </article>
          );
        }
        if (item.kind === "loading") {
          return (
            <WhiteboardLoadingBlock
              key={item.key}
              state={item.loading.state}
              left={item.left}
              top={item.top}
            />
          );
        }
        const artifact = item.artifact;
        const question = item.question;
        const sourceMissing = !sourceById.has(artifact.source.source_id);
        const stale = currentDocumentVersion > artifact.source.document_version;
        const targetInvalid = invalidTargetTurnIds.has(artifact.turn_id);
        const minimized = minimizedTurnIds.has(artifact.turn_id);
        const cardScale = cardScaleByTurnId[artifact.turn_id]
          ?? DEFAULT_CARD_SCALE;
        if (minimized) {
          return (
            <button
              key={artifact.turn_id}
              type="button"
              className={targetInvalid
                ? "learning-selection-enhancement-pin is-invalid-target"
                : "learning-selection-enhancement-pin"}
              style={{
                left: item.left,
                top: item.top + 8,
              }}
              data-source-id={artifact.source.source_id}
              onClick={() => {
                setMinimizedTurnIds((current) => {
                  const next = new Set(current);
                  next.delete(artifact.turn_id);
                  return next;
                });
              }}
              aria-label={question
                ? `展开问题和小章鱼辅助：${artifact.response.title}`
                : `展开小章鱼辅助：${artifact.response.title}`}
              title={`展开：${artifact.response.title}`}
            >
              ?
            </button>
          );
        }
        return (
          <article
            key={artifact.turn_id}
            className={targetInvalid
              ? "learning-selection-enhancement is-invalid-target"
              : "learning-selection-enhancement"}
            style={{
              left: item.left,
              top: item.top,
              width: CARD_WIDTH * cardScale,
              fontSize: CARD_FONT_SIZE * cardScale,
              "--learning-selection-scene3d-height":
                `${SCENE3D_HEIGHT * cardScale}px`,
            } as CSSProperties}
            data-source-id={artifact.source.source_id}
            data-card-scale={cardScale.toFixed(2)}
          >
            <div className="learning-selection-source-link" aria-hidden="true" />
            {question ? <SelectionQuestionSection question={question} /> : null}
            <header>
              <div>
                <span>小章鱼辅助</span>
                <small>
                  {targetInvalid
                    ? "引用的白板对象已失效，请重新选择"
                    : sourceMissing
                      ? "原选区快照已不在本浏览器，保留生成结果"
                    : stale
                      ? "基于较早版本的原稿"
                      : "来自当前选区"}
                </small>
              </div>
              <div className="learning-selection-enhancement-actions">
                <button
                  type="button"
                  onClick={() => {
                    setMinimizedTurnIds((current) => {
                      const next = new Set(current);
                      next.add(artifact.turn_id);
                      return next;
                    });
                  }}
                  aria-label={question
                    ? "最小化问题和辅助内容"
                    : "最小化这条辅助内容"}
                  title="最小化"
                >
                  <Minimize2 size={15} />
                </button>
                <button
                  type="button"
                  onClick={() => onDelete(artifact.turn_id)}
                  aria-label="删除这条辅助内容"
                  title="删除"
                >
                  <Trash2 size={15} />
                </button>
              </div>
            </header>
            <div className="learning-selection-enhancement-content">
              <MarkdownContent
                text={artifact.response.title}
                className="learning-selection-markdown learning-selection-result-title"
              />
              <MarkdownContent
                text={artifact.response.text}
                className="learning-selection-markdown learning-selection-result-text"
              />
              {artifact.response.kind === "explanation"
                && artifact.response.items?.length ? (
                  <ul>
                    {artifact.response.items.map((item) => (
                      <li key={item}>
                        <MarkdownContent
                          text={item}
                          className="learning-selection-markdown"
                        />
                      </li>
                    ))}
                  </ul>
                ) : null}
              {artifact.response.kind === "plot" ? (
                <SelectionPlot
                  artifact={artifact as SelectionEnhancementArtifact & {
                    response: Extract<
                      SelectionEnhancementArtifact["response"],
                      { kind: "plot" }
                    >;
                  }}
                />
              ) : null}
              {artifact.response.kind === "scene3d" ? (
                <SelectionScene3d
                  artifact={artifact as SelectionEnhancementArtifact & {
                    response: Extract<
                      SelectionEnhancementArtifact["response"],
                      { kind: "scene3d" }
                    >;
                  }}
                />
              ) : null}
              {artifact.response.kind === "unsupported" ? (
                <div
                  className="learning-selection-unsupported"
                  role="alert"
                >
                  <strong>当前无法生成这个图像</strong>
                  {artifact.response.alternatives?.length ? (
                    <ul>
                      {artifact.response.alternatives.map((alternative) => (
                        <li key={alternative}>
                          <MarkdownContent
                            text={alternative}
                            className="learning-selection-markdown"
                          />
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </div>
              ) : null}
              <footer>
                <span>系统理解：</span>
                <MarkdownContent
                  text={artifact.interpretation.content || "未能可靠识别"}
                  className="learning-selection-markdown"
                />
              </footer>
            </div>
            <button
              type="button"
              className="learning-selection-enhancement-resize"
              onPointerDown={(event) => beginCardResize(
                event,
                artifact.turn_id,
                cardScale,
              )}
              onPointerMove={continueCardResize}
              onPointerUp={finishCardResize}
              onPointerCancel={finishCardResize}
              onLostPointerCapture={() => {
                if (resizingCardRef.current?.turnId === artifact.turn_id) {
                  resizingCardRef.current = null;
                }
              }}
              onDoubleClick={() => updateCardScale(
                artifact.turn_id,
                DEFAULT_CARD_SCALE,
              )}
              onKeyDown={(event) => resizeCardWithKeyboard(
                event,
                artifact.turn_id,
                cardScale,
              )}
              aria-label={`调整辅助卡片大小，当前 ${Math.round(cardScale * 100)}%`}
              title="拖动放大或缩小；双击恢复原始大小"
            >
              <span aria-hidden="true" />
            </button>
          </article>
        );
      })}
    </>
  );
}
