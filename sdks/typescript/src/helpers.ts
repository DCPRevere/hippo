import type {
  BatchResult,
  ContextResponse,
  GraphEdge,
  GraphNode,
  RememberBatchResponse,
  RememberResponse,
} from "./models.js";

/**
 * Find a node by name in a ContextResponse.
 */
export function findNode(
  response: ContextResponse,
  name: string,
): GraphNode | undefined {
  return response.nodes.find(
    (n) => (n as Record<string, unknown>).name === name,
  );
}

/**
 * Filter edges that involve a given entity name (as source or target).
 */
export function factsAbout(
  response: ContextResponse,
  entityName: string,
): GraphEdge[] {
  return response.edges.filter((e) => {
    const edge = e as Record<string, unknown>;
    return edge.source === entityName || edge.target === entityName;
  });
}

/**
 * Returns true if a remember response wrote zero facts (likely a duplicate).
 */
export function isDuplicate(response: RememberResponse): boolean {
  return response.facts_written === 0;
}

/**
 * Filter failed results from a batch remember response.
 */
export function failures(response: RememberBatchResponse): BatchResult[] {
  return response.results.filter((r) => {
    const result = r as Record<string, unknown>;
    return result.error !== undefined;
  });
}
