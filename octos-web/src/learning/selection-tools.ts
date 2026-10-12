import type { SelectionContentKind } from "./selection-enhancements";
import type { BoardTargetKind } from "octos-lesson-language/web-runtime";

export type SelectionToolId =
  | "explain"
  | "check-and-suggest"
  | "generate-plot"
  | "custom-question";

export interface SelectionToolDefinition {
  id: SelectionToolId;
  label: string;
  prompt: string;
  contentKinds: SelectionContentKind[];
  requestContentKind?: SelectionContentKind;
  targetKinds?: BoardTargetKind[];
  output: "annotation" | "plot";
  requiresModel: boolean;
  requiresVerifiedComputation: boolean;
  addsBoardContent: boolean;
  changesSource: false;
  action: "local-enhancement";
}

export interface SelectionLessonToolDefinition {
  id: "teach-lesson";
  label: string;
  contentKinds: SelectionContentKind[];
  action: "composer-reference";
}

/**
 * The model does not invent selection tools. The UI exposes this finite list,
 * and the learning-coach validates the selected id before producing an
 * enhancement artifact.
 */
export const selectionToolRegistry: Array<
  SelectionToolDefinition | SelectionLessonToolDefinition
> = [
  {
    id: "explain",
    label: "解释这部分",
    prompt: "请解释我选中的这部分。",
    contentKinds: ["text", "math", "geometry", "data", "unknown"],
    output: "annotation",
    requiresModel: true,
    requiresVerifiedComputation: false,
    addsBoardContent: true,
    changesSource: false,
    action: "local-enhancement",
  },
  {
    id: "check-and-suggest",
    label: "检查并建议",
    prompt: "请检查我选中的内容，并在旁边给出建议。",
    contentKinds: ["text", "math", "geometry", "data", "unknown"],
    output: "annotation",
    requiresModel: true,
    requiresVerifiedComputation: false,
    addsBoardContent: true,
    changesSource: false,
    action: "local-enhancement",
  },
  {
    id: "generate-plot",
    label: "生成函数图像",
    prompt: "请按我选中的公式生成函数图像。",
    contentKinds: ["math"],
    requestContentKind: "math",
    targetKinds: ["math-fragment"],
    output: "plot",
    requiresModel: true,
    requiresVerifiedComputation: true,
    addsBoardContent: true,
    changesSource: false,
    action: "local-enhancement",
  },
  {
    id: "teach-lesson",
    label: "围绕这部分讲一课",
    contentKinds: ["text", "math", "geometry", "data", "unknown"],
    action: "composer-reference",
  },
];

export function availableSelectionTools(
  contentKind: SelectionContentKind,
  targetKinds: BoardTargetKind[] = [],
): SelectionToolDefinition[] {
  return selectionToolRegistry.filter((tool): tool is SelectionToolDefinition =>
    tool.action === "local-enhancement"
      && (tool.contentKinds.includes(contentKind)
        || tool.targetKinds?.some((kind) => targetKinds.includes(kind)) === true),
  );
}

export const selectionLessonTool = selectionToolRegistry.find(
  (tool): tool is SelectionLessonToolDefinition => tool.id === "teach-lesson",
)!;

export function isSelectionToolId(value: string): value is SelectionToolId {
  return value === "explain"
    || value === "check-and-suggest"
    || value === "generate-plot"
    || value === "custom-question";
}
