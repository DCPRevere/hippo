// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

export interface RememberRequest {
  statement: string;
  source_agent?: string;
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
  graph?: string;
}

export interface AskRequest {
  question: string;
  limit?: number;
  graph?: string;
  verbose?: boolean;
}

export interface CreateUserRequest {
  user_id: string;
  display_name: string;
  role?: string;
  graphs?: string[];
}

// ---------------------------------------------------------------------------
// Dreamer types — see docs/DREAMS.md
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

/** Aggregated dream-pass summary. The Dreamer records counts per visit
 * and the pool sums them. See docs/DREAMS.md. */
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

export interface UsageInfo {
  [key: string]: unknown;
}

export interface TraceInfo {
  [key: string]: unknown;
}

export interface RememberResponse {
  entities_created: number;
  entities_resolved: number;
  facts_written: number;
  contradictions_invalidated: number;
  usage: UsageInfo;
  trace: TraceInfo;
}

export interface BatchResult {
  [key: string]: unknown;
}

export interface RememberBatchResponse {
  total: number;
  succeeded: number;
  failed: number;
  results: BatchResult[];
}

export interface GraphNode {
  [key: string]: unknown;
}

export interface GraphEdge {
  [key: string]: unknown;
}

export interface ContextResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface Fact {
  [key: string]: unknown;
}

export interface AskResponse {
  answer: string;
  facts?: Fact[];
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
