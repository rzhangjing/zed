export function isCurrentInkMergeCompletion(
  completedSourceSessionId: string,
  completedTargetSessionId: string,
  currentSourceSessionId: string | null,
  currentTargetSessionId: string,
): boolean {
  return completedSourceSessionId === currentSourceSessionId
    && completedTargetSessionId === currentTargetSessionId;
}
