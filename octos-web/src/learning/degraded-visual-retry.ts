import type { DegradedVisualRetryRequest } from "./oll/oll-lesson-runtime";

export function buildDegradedVisualRetryPrompt(
  degraded: DegradedVisualRetryRequest,
): string {
  const surfaceNames: Record<string, string> = {
    geometry: "二维几何",
    plot: "函数图像",
    scene3d: "三维场景",
    diagram: "关系图",
    image: "图片",
    table: "表格",
  };
  const surface = surfaceNames[degraded.surface] ?? degraded.surface;
  return `请重新生成没有成功展示的${surface}“${degraded.purpose}”。只补充这个画面，不要重做整堂课。`;
}

function line(name: string, value: string | number): string {
  return `${name}: ${String(value).replace(/[\r\n]+/g, " ")}`;
}

/**
 * Application-owned retry contract. The stable board target is supplied by
 * the Runtime; the model is not asked to rediscover the failed node from text.
 */
export function buildDegradedVisualRetryContext(
  degraded: DegradedVisualRetryRequest,
): string {
  return [
    "[[LEARNING_DEGRADED_VISUAL_RETRY]]",
    "request_source: explicit_board_follow_up",
    "retry_scope: one_visual_component",
    "preserve_existing_board: required",
    line("board_id", degraded.boardId),
    line("board_revision", degraded.boardRevision),
    line("visual_id", degraded.visualId),
    line("surface", degraded.surface),
    line("purpose", degraded.purpose),
    line("board_summary", `一个${degraded.surface}画面生成失败；只补充该画面`),
    line("board_targets", JSON.stringify([{
      as: "failed-visual",
      type: "node",
      target_id: degraded.nodeId,
      label: degraded.title,
      fragments: [],
    }])),
    "[[/LEARNING_DEGRADED_VISUAL_RETRY]]",
  ].join("\n");
}
