export interface Entity {
	id: string;
	name: string;
	entity_type: string;
	resolved: boolean;
	hint?: string;
	content?: string;
	created_at: string;
}

export interface Edge {
	edge_id: number;
	subject_id: string;
	subject_name: string;
	object_id: string;
	object_name: string;
	fact: string;
	relation_type: string;
	confidence: number;
	salience: number;
	valid_at: string;
	invalid_at?: string;
	memory_tier: string;
	source_agents: string; // pipe-delimited, e.g. "agent1|agent2"
	decayed_confidence: number;
	expires_at?: string;
}

export interface GraphDump {
	graph: string;
	entities: Entity[];
	edges: {
		active: Edge[];
		invalidated: Edge[];
	};
}

export interface RememberRequest {
	statement: string;
	source_agent?: string;
	source_credibility_hint?: number;
	graph?: string;
	ttl_secs?: number;
}

export interface OpExecutionTrace {
	op: string;
	outcome: string;
	details?: string;
}

export interface GraphOp {
	op: string;
	[key: string]: unknown;
}

export interface RememberTrace {
	operations: GraphOp[];
	revised_operations?: GraphOp[];
	execution: OpExecutionTrace[];
}

export interface RememberResponse {
	entities_created: number;
	entities_resolved: number;
	facts_written: number;
	contradictions_invalidated: number;
	usage: LlmUsage;
	trace: RememberTrace;
}

export interface ContextRequest {
	query: string;
	limit?: number;
	max_hops?: number;
	memory_tier_filter?: string;
	at?: string; // ISO datetime string
	graph?: string;
	scoring?: ScoringParams;
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

export interface SubgraphNode {
	id: string;
	name: string;
	type: string;
	properties?: Record<string, string>;
	user_id?: string;
}

export interface SubgraphEdge {
	id: number;
	from: string;
	to: string;
	relation: string;
	fact: string;
	confidence: number;
}

export interface GraphContext {
	nodes: SubgraphNode[];
	edges: SubgraphEdge[];
	principal_id?: string;
}

export interface ScoringParams {
	w_relevance?: number;
	w_confidence?: number;
	w_recency?: number;
	w_salience?: number;
	mmr_lambda?: number;
}

export interface LlmUsage {
	llm_calls: number;
	embed_calls: number;
	input_tokens: number;
	output_tokens: number;
}

export interface AskRequest {
	question: string;
	limit?: number;
	graph?: string;
	verbose?: boolean;
}

export interface AskResponse {
	answer: string;
	facts?: ContextFact[];
}

export interface User {
	user_id: string;
	display_name: string;
	role: string;
	graphs: string[];
	created_at: string;
}

export interface ApiKey {
	label: string;
	prefix: string;
	created_at: string;
}

export interface HealthResponse {
	status: string;
	graph: string;
}

export interface GraphListResponse {
	default: string;
	graphs: string[];
}

export interface BatchRememberRequest {
	statements: string[];
	parallel?: boolean;
	source_agent?: string;
	ttl_secs?: number;
	graph?: string;
}

export interface BatchRememberResult {
	statement: string;
	ok: boolean;
	facts_written?: number;
	entities_created?: number;
	error?: string;
}

export interface BatchRememberResponse {
	total: number;
	succeeded: number;
	failed: number;
	results: BatchRememberResult[];
}

export interface SeedEntity {
	id: string;
	name: string;
	entity_type: string;
	resolved?: boolean;
	hint?: string;
}

export interface SeedEdge {
	subject_id: string;
	object_id: string;
	fact: string;
	relation_type: string;
	confidence?: number;
	salience?: number;
	valid_at?: string;
	source_agents?: string;
	memory_tier?: string;
}

export interface SeedRequest {
	entities: SeedEntity[];
	edges: SeedEdge[];
	graph?: string;
}

export interface BackupEntity {
	id: string;
	name: string;
	entity_type: string;
	resolved: boolean;
	hint?: string;
}

export interface GraphDumpBackup {
	graph: string;
	exported_at: string;
	entities: BackupEntity[];
	edges: SeedEdge[];
}

// Cloud-only types
export interface Tenant {
	id: string;
	name: string;
	display_name: string;
	email: string;
	plan: string;
	status: string;
	hippo_url: string | null;
	created_at: string;
}

export interface UsagePeriod {
	remember_calls: number;
	context_calls: number;
	facts_stored: number;
	entities_stored: number;
	period_start: string;
}
