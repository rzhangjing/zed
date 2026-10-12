import type { InkSelectionSnapshot } from "octos-lesson-language/ink-runtime";
import type {
  SelectionBoardContext,
  SelectionContentKind,
} from "./selection-enhancements";

export interface ComposerBoardReference {
  id: string;
  label: string;
  snapshot: InkSelectionSnapshot;
  contentKind: SelectionContentKind;
  boardContext: SelectionBoardContext;
  contextImage: File;
}

export interface UploadedComposerBoardReference {
  reference: ComposerBoardReference;
  mediaPath: string;
}

function line(name: string, value: string | number): string {
  return `${name}: ${String(value).replace(/[\r\n]+/g, " ")}`;
}

/**
 * This block is visible in the outbound turn and contains only references the
 * learner explicitly attached. It never serializes the whole board.
 */
export function buildComposerBoardReferenceContext(
  references: UploadedComposerBoardReference[],
): string {
  if (references.length === 0) return "";
  const lines = [
    "[[LEARNING_COMPOSER_REFERENCES]]",
    "request_source: explicit_board_follow_up",
    "reference_policy: explicit_only",
    "preserve_source_ink: required",
  ];
  references.forEach(({ reference, mediaPath }, referenceIndex) => {
    lines.push(
      "[[REFERENCE]]",
      line("reference_id", reference.id),
      line("label", reference.label),
      line("selection_media", mediaPath),
      line("source_id", reference.snapshot.source_id),
      line("source_document_id", reference.snapshot.document_id),
      line("source_document_version", reference.snapshot.document_version),
      line("source_checksum", reference.snapshot.checksum.value),
      line("content_hint", reference.contentKind),
      line("board_id", reference.boardContext.boardId),
      line("board_revision", reference.boardContext.boardRevision),
      line(
        "board_targets",
        JSON.stringify(reference.boardContext.targets.map((target, index) => ({
          as: `board-ref-${referenceIndex + 1}-${index + 1}`,
          type: "node",
          target_id: target.node_id,
          label: target.label,
          fragments: target.element_id
            ? [{ as: "selected-part", target_id: target.target_id }]
            : [],
        }))),
      ),
      "[[/REFERENCE]]",
    );
  });
  lines.push("[[/LEARNING_COMPOSER_REFERENCES]]");
  return lines.join("\n");
}
