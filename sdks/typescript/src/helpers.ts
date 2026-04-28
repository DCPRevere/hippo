import type {
  BatchRememberResult,
  ContextFact,
  ContextResponse,
  RememberBatchResponse,
  RememberResponse,
} from "./models.js";

/**
 * Find the first fact whose subject matches `name` (case-insensitive).
 */
export function findSubject(
  response: ContextResponse,
  name: string,
): ContextFact | undefined {
  const lower = name.toLowerCase();
  return response.facts.find((f) => f.subject.toLowerCase() === lower);
}

/**
 * Filter facts where `entityName` is the subject or object (case-insensitive).
 */
export function factsAbout(
  response: ContextResponse,
  entityName: string,
): ContextFact[] {
  const lower = entityName.toLowerCase();
  return response.facts.filter(
    (f) => f.subject.toLowerCase() === lower || f.object.toLowerCase() === lower,
  );
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
export function failures(response: RememberBatchResponse): BatchRememberResult[] {
  return response.results.filter((r) => !r.ok);
}
