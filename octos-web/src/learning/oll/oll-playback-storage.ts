const OLL_PLAYBACK_STORAGE_VERSION = "v4";

export type OllFixture = "geometry-v2" | "unit-circle-sine";

export function ollPlaybackStorageKey(
  sessionId: string,
  fixture: OllFixture | undefined,
): string {
  return `octos-learning-oll:${OLL_PLAYBACK_STORAGE_VERSION}:${sessionId}:${fixture ?? "none"}`;
}
