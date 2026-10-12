import {
  referencedMathVariables,
  type SemanticBoardState,
} from "octos-lesson-language";

const visualNodeKinds = new Set([
  "diagram",
  "geometry",
  "image",
  "plot",
  "scene3d",
]);

export interface InteractionClusterTopic {
  id: string;
  nodeIds?: string[];
  variableAliases?: string[];
  taskTargets?: Record<string, {
    variableAliases: string[];
    nodeIds: string[];
  }>;
}

export interface InteractionCluster {
  id: string;
  anchorNodeId: string;
  nodeIds: string[];
  variableAliases: string[];
  taskIds: string[];
}

function referencedContentVariables(
  value: unknown,
  allowedVariables: ReadonlySet<string>,
  result: Set<string>,
): void {
  if (Array.isArray(value)) {
    value.forEach((item) =>
      referencedContentVariables(item, allowedVariables, result));
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value)) {
    if (
      key === "variable"
      && typeof child === "string"
      && allowedVariables.has(child)
    ) {
      result.add(child);
    }
    if (key === "expression" && typeof child === "string") {
      for (const alias of referencedMathVariables(child, [
        "x",
        "y",
        "z",
        ...allowedVariables,
      ])) {
        if (allowedVariables.has(alias)) result.add(alias);
      }
    }
    referencedContentVariables(child, allowedVariables, result);
  }
}

/**
 * Build interaction groups only from explicit machine-readable relationships:
 * visual expressions/bindings, task operation targets, and course ownership.
 * Lesson prose is deliberately never interpreted as a relationship signal.
 */
export function buildInteractionClusters(
  board: SemanticBoardState | null,
  topic: InteractionClusterTopic,
  controlAliases: readonly string[],
  taskIds: readonly string[],
): InteractionCluster[] {
  if (!board || (controlAliases.length === 0 && taskIds.length === 0)) return [];

  const topicNodeIds = (topic.nodeIds?.length
    ? topic.nodeIds
    : Object.values(board.nodes)
        .filter((node) => (node.region_id ?? "__legacy__") === topic.id)
        .map((node) => node.id))
    .filter((nodeId) => Boolean(board.nodes[nodeId]));
  if (topicNodeIds.length === 0) return [];

  const allowedVariables = new Set(topic.variableAliases ?? controlAliases);
  const activeControls = new Set(
    controlAliases.filter((alias) => allowedVariables.has(alias)),
  );
  const variablesByNode = new Map<string, Set<string>>();
  const nodesByVariable = new Map<string, Set<string>>();

  for (const nodeId of topicNodeIds) {
    const node = board.nodes[nodeId];
    if (!node || !visualNodeKinds.has(String(node.kind ?? ""))) continue;
    const variables = new Set<string>();
    referencedContentVariables(node.content, allowedVariables, variables);
    variablesByNode.set(nodeId, variables);
    for (const alias of variables) {
      const nodes = nodesByVariable.get(alias) ?? new Set<string>();
      nodes.add(nodeId);
      nodesByVariable.set(alias, nodes);
    }
  }

  const clusters: InteractionCluster[] = [];
  const visitedVariables = new Set<string>();
  for (const initialAlias of activeControls) {
    if (visitedVariables.has(initialAlias)) continue;
    const variableAliases = new Set<string>();
    const nodeIds = new Set<string>();
    const queue = [initialAlias];
    while (queue.length > 0) {
      const alias = queue.shift()!;
      if (visitedVariables.has(alias)) continue;
      visitedVariables.add(alias);
      variableAliases.add(alias);
      for (const nodeId of nodesByVariable.get(alias) ?? []) {
        nodeIds.add(nodeId);
        for (const peerAlias of variablesByNode.get(nodeId) ?? []) {
          if (activeControls.has(peerAlias) && !visitedVariables.has(peerAlias)) {
            queue.push(peerAlias);
          }
        }
      }
    }
    if (nodeIds.size === 0) continue;
    const orderedNodeIds = topicNodeIds.filter((nodeId) => nodeIds.has(nodeId));
    clusters.push({
      id: `${topic.id}:interaction:${clusters.length + 1}`,
      anchorNodeId: orderedNodeIds.at(-1)!,
      nodeIds: orderedNodeIds,
      variableAliases: [...variableAliases],
      taskIds: [],
    });
  }

  const assignedTaskIds = new Set<string>();
  for (const taskId of taskIds) {
    const targets = topic.taskTargets?.[taskId];
    const cluster = clusters.find((candidate) =>
      targets?.variableAliases.some((alias) =>
        candidate.variableAliases.includes(alias))
      || targets?.nodeIds.some((nodeId) => candidate.nodeIds.includes(nodeId)));
    if (!cluster) continue;
    cluster.taskIds.push(taskId);
    assignedTaskIds.add(taskId);
  }

  for (const taskId of taskIds) {
    if (assignedTaskIds.has(taskId)) continue;
    const targetNodeIds = topic.taskTargets?.[taskId]?.nodeIds
      .filter((nodeId) => topicNodeIds.includes(nodeId)) ?? [];
    const anchorNodeId = targetNodeIds.at(-1);
    if (!anchorNodeId) continue;
    const existing = clusters.find((cluster) =>
      cluster.anchorNodeId === anchorNodeId
      && cluster.variableAliases.length === 0);
    if (existing) {
      existing.taskIds.push(taskId);
      existing.nodeIds = [...new Set([...existing.nodeIds, ...targetNodeIds])];
      assignedTaskIds.add(taskId);
      continue;
    }
    clusters.push({
      id: `${topic.id}:interaction:${clusters.length + 1}`,
      anchorNodeId,
      nodeIds: targetNodeIds,
      variableAliases: [],
      taskIds: [taskId],
    });
    assignedTaskIds.add(taskId);
  }

  const unassignedControls = controlAliases.filter((alias) =>
    !clusters.some((cluster) => cluster.variableAliases.includes(alias)));
  const unassignedTasks = taskIds.filter((taskId) => !assignedTaskIds.has(taskId));
  const fallbackControls = topic.nodeIds?.length ? [] : unassignedControls;
  const fallbackTasks = unassignedTasks.filter((taskId) =>
    !topic.taskTargets?.[taskId]);
  // Modern lesson outlines identify their future nodes up front. Do not pin a
  // control to an unrelated card while its actual bound visual has not been
  // delivered yet. Untargeted legacy tasks may still share an existing visual
  // interaction cluster instead of becoming a separate floating card.
  if (fallbackControls.length > 0 || fallbackTasks.length > 0) {
    const visualFallback = [...topicNodeIds].reverse().find((nodeId) =>
      visualNodeKinds.has(String(board.nodes[nodeId]?.kind ?? "")));
    if (!visualFallback) return clusters;
    const existing = clusters.find((cluster) =>
      cluster.anchorNodeId === visualFallback);
    if (existing) {
      existing.variableAliases = [...new Set([
        ...existing.variableAliases,
        ...fallbackControls,
      ])];
      existing.taskIds = [...new Set([...existing.taskIds, ...fallbackTasks])];
      return clusters;
    }
    clusters.push({
      id: `${topic.id}:interaction:${clusters.length + 1}`,
      anchorNodeId: visualFallback,
      nodeIds: [visualFallback],
      variableAliases: fallbackControls,
      taskIds: fallbackTasks,
    });
  }

  return clusters;
}
