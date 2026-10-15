/**
 * Canonical set of `spawn_only` tool names known to the SPA.
 *
 * spawn_only tools are background skills whose foreground `tool/completed`
 * envelope fires ~2ms after `tool/started` (it's only an acknowledgement
 * that the supervisor accepted the work) — the actual background task
 * keeps running for seconds or minutes. The terminal signal for the
 * tool-card chip is `task/updated:completed` (or `task/updated:failed`),
 * not the foreground `tool/completed`.
 *
 * The router consults this set in `handleToolCompleted` to skip the
 * premature `status: complete` flip; `handleTaskUpdated` then drives the
 * real terminal transition once the background task settles.
 *
 * Sources of truth for the names below (checked 2026-05-22):
 *   - `crates/octos/app-skills/harness-starter-<kind>/manifest.json` →
 *     `propose_patch`, `produce_artifact`, `generate_report`,
 *     `synthesize_clip`
 *   - `crates/octos/platform-skills/voice/manifest.json` → `voice_synthesize`
 *   - `mofa-skills/mofa-<plugin>/manifest.json` (external skill repo with
 *     `"spawn_only": true` in the tool entry) → `podcast_generate`,
 *     `mofa_slides`, `mofa_cards`, `mofa_comic`, `mofa_infographic`,
 *     `mofa_publish`, `fm_tts`
 *   - `mark_spawn_only("run_pipeline", ...)` programmatic registration
 *     in `crates/octos/octos-cli/src/{runtime/profile.rs, commands/chat.rs,
 *     session_actor.rs}`
 *
 * Tools that look spawn_only but are NOT:
 *   - `mofa_frame` — the workspace_policy.rs entry has an explicit
 *     comment ("the manifest is not spawn_only: true today — contract
 *     is dormant"); the mofa-skills manifest confirms.
 *   - `deep_search` — the actual tool name registered by
 *     `crates/octos/app-skills/deep-search` is `search`, NOT `deep_search`,
 *     and the manifest does NOT mark it spawn_only. The
 *     `workspace_policy.rs` entry is a dormant contract slot.
 *
 * Keep this list synchronized when new spawn_only tools land on the
 * agent side. The set is checked by exact string equality on the
 * server-emitted `tool_name` field, so abbreviations / aliases don't
 * resolve here.
 */
export const SPAWN_ONLY_TOOL_NAMES: ReadonlySet<string> = new Set<string>([
  // Pipeline supervisor (registered programmatically via
  // `mark_spawn_only("run_pipeline", ...)` in profile.rs / chat.rs /
  // session_actor.rs).
  "run_pipeline",
  // mofa-skills plugins (`mofa-skills/.../manifest.json` with
  // `"spawn_only": true`):
  "podcast_generate",
  "mofa_slides",
  "mofa_cards",
  "mofa_comic",
  "mofa_infographic",
  "mofa_publish",
  "fm_tts",
  // platform-skills/voice (`platform-skills/voice/manifest.json`):
  "voice_synthesize",
  // app-skills harness starters
  // (`crates/octos/app-skills/harness-starter-*/manifest.json`):
  "propose_patch",
  "produce_artifact",
  "generate_report",
  "synthesize_clip",
]);

/** Returns true when `toolName` is a known spawn_only background tool.
 *  Safe to call with `undefined` / empty strings — returns false. */
export function isSpawnOnlyToolName(toolName: string | undefined | null): boolean {
  if (!toolName) return false;
  return SPAWN_ONLY_TOOL_NAMES.has(toolName);
}
