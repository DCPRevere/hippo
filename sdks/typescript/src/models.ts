// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

export interface ScoringParams {
  w_relevance: number;
  w_confidence: number;
  w_recency: number;
  w_salience: number;
  mmr_lambda: number;
}

export interface LlmUsage {
  llm_calls: number;
  embed_calls: number;
  input_tokens: number;
  output_tokens: number;
}

export interface OpExecutionTrace {
  op: string;
  outcome: string;
  details?: string;
}

export interface RememberTrace {
  operations: Array<Record<string, unknown>>;
  revised_operations?: Array<Record<string, unknown>>;
  execution: OpExecutionTrace[];
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

export interface RememberRequest {
  statement: string;
  source_agent?: string;
  source_credibility_hint?: number;
  graph?: string;
  ttl_secs?: number;
}

export interface RememberBatchRequest {
  statements: string[];
  source_agent?: string;
  parallel?: boolean;
  graph?: string;
  ttl_secs?: number;
}

export interface ContextRequest {
  query: string;
  limit?: number;
  max_hops?: number;
  memory_tier_filter?: string;
  graph?: string;
  /** ISO 8601 timestamp; the server filters edges valid at this instant. */
  at?: string;
  scoring?: ScoringParams;
}

export interface AskRequest {
  question: string;
  limit?: number;
  graph?: string;
  verbose?: boolean;
  max_iterations?: number;
}

export interface CreateUserRequest {
  user_id: string;
  display_name: string;
  role?: string;
  graphs?: string[];
}

// ---------------------------------------------------------------------------
// Dreamer / destructive ops
// ---------------------------------------------------------------------------

/** Explicit user/agent retraction. Distinct from supersession (which the
 * Dreamer writes append-only). */
export interface RetractRequest {
  edge_id: number;
  reason?: string;
  graph?: string;
}

export interface RetractResponse {
  edge_id: number;
  reason?: string;
}

/** Convenience: retract an old fact and observe a new one in one call. */
export interface CorrectRequest {
  edge_id: number;
  statement: string;
  reason?: string;
  source_agent?: string;
  graph?: string;
}

export interface CorrectResponse {
  retracted_edge_id: number;
  reason?: string;
  remember: RememberResponse;
}

/** Aggregated dream-pass summary returned by POST /maintain. */
export interface DreamReport {
  facts_visited: number;
  links_written: number;
  inferences_written: number;
  supersessions_written: number;
  contradictions_seen: number;
  consolidations_written: number;
  tokens_used: number;
  duration_ms: number;
}

export interface CreateKeyRequest {
  label: string;
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

export interface RememberResponse {
  entities_created: number;
  entities_resolved: number;
  facts_written: number;
  contradictions_invalidated: number;
  usage: LlmUsage;
  trace: RememberTrace;
}

export interface BatchRememberResult {
  statement: string;
  ok: boolean;
  facts_written?: number;
  entities_created?: number;
  error?: string;
}

export interface RememberBatchResponse {
  total: number;
  succeeded: number;
  failed: number;
  results: BatchRememberResult[];
}

export interface ContextFact {
  fact: string;
  subject: string;
  relation_type: string;
  object: string;
  confidence: number;
  salience: number;
  valid_at: string;
  edge_id: number;
  hops: number;
  source_agents: string[];
  memory_tier: string;
}

export interface ContextResponse {
  facts: ContextFact[];
}

export interface AskResponse {
  answer: string;
  facts?: ContextFact[];
  iterations: number;
}

export interface CreateUserResponse {
  user_id: string;
  api_key: string;
}

export interface UserSummary {
  user_id: string;
  display_name: string;
  role: string;
  graphs: string[];
  key_count: number;
}

export interface ListUsersResponse {
  users: UserSummary[];
}

export interface CreateKeyResponse {
  user_id: string;
  label: string;
  api_key: string;
}

export interface KeySummary {
  label: string;
  created_at: string;
}

export interface ListKeysResponse {
  keys: KeySummary[];
}

export interface HealthResponse {
  status: string;
  graph: string;
}

export interface GraphsListResponse {
  default: string;
  graphs: string[];
}

export interface AuditEntry {
  id: string;
  user_id: string;
  action: string;
  details: string;
  timestamp: string;
}

export interface AuditResponse {
  entries: AuditEntry[];
}

// ---------------------------------------------------------------------------
// SSE event type
// ---------------------------------------------------------------------------

export interface GraphEvent {
  event: string;
  data: unknown;
}

// ---------------------------------------------------------------------------
// Client options
// ---------------------------------------------------------------------------

export type OnRequestHook = (method: string, url: string, body?: unknown) => void;
export type OnResponseHook = (method: string, url: string, status: number, durationMs: number) => void;

export interface HippoClientOptions {
  baseUrl?: string;
  apiKey?: string;
  maxRetries?: number;
  timeout?: number;
  onRequest?: OnRequestHook;
  onResponse?: OnResponseHook;
}

export interface EventsOptions {
  graph?: string;
  signal?: AbortSignal;
}
