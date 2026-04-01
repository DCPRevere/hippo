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
	salience?: number;
	valid_at?: string;
	invalid_at?: string;
	memory_tier: string;
	source_agents: string[];
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

export interface RememberResponse {
	entities_created: string[];
	entities_resolved: string[];
	facts_written: number;
	contradictions_invalidated: number;
	usage?: LlmUsage;
}

export interface ContextRequest {
	query: string;
	limit?: number;
	max_hops?: number;
	graph?: string;
	scoring?: ScoringParams;
}

export interface ContextFact {
	fact: string;
	subject: string;
	relation_type: string;
	object: string;
	confidence: number;
	salience?: number;
	valid_at?: string;
	edge_id: number;
	hops: number;
	source_agents: string[];
}

export interface ScoringParams {
	w_relevance?: number;
	w_confidence?: number;
	w_recency?: number;
	w_salience?: number;
	mmr_lambda?: number;
}

export interface LlmUsage {
	input_tokens: number;
	output_tokens: number;
}

export interface AskRequest {
	question: string;
	limit?: number;
	graph?: string;
}

export interface AskResponse {
	answer: string;
	facts_used: ContextFact[];
	usage?: LlmUsage;
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
