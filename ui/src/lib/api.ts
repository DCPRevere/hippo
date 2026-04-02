import { features } from './features';
import type {
	RememberRequest,
	RememberResponse,
	ContextRequest,
	GraphContext,
	AskRequest,
	AskResponse,
	GraphDump,
	GraphDumpBackup,
	Entity,
	Edge,
	HealthResponse,
	GraphListResponse,
	User,
	ApiKey,
	Tenant,
	UsagePeriod,
	BatchRememberRequest,
	BatchRememberResponse,
	SeedRequest
} from './types';

function getToken(): string | null {
	if (typeof localStorage === 'undefined') return null;
	return localStorage.getItem('hippo_api_key');
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
	const url = `${features.baseUrl}${path}`;
	const headers: Record<string, string> = {
		'Content-Type': 'application/json'
	};
	const token = getToken();
	if (token) {
		headers['Authorization'] = `Bearer ${token}`;
	}

	const res = await fetch(url, {
		method,
		headers,
		body: body ? JSON.stringify(body) : undefined
	});

	if (!res.ok) {
		const text = await res.text().catch(() => res.statusText);
		throw new Error(`${res.status}: ${text}`);
	}

	const contentType = res.headers.get('content-type') || '';
	if (contentType.includes('application/json')) {
		return res.json();
	}
	return res.text() as unknown as T;
}

// Core endpoints
export function remember(req: RememberRequest): Promise<RememberResponse> {
	return request('POST', '/api/remember', req);
}

export function rememberBatch(req: BatchRememberRequest): Promise<BatchRememberResponse> {
	return request('POST', '/api/remember/batch', req);
}

export function context(req: ContextRequest): Promise<GraphContext> {
	return request('POST', '/api/context', req);
}

export function ask(req: AskRequest): Promise<AskResponse> {
	return request('POST', '/api/ask', req);
}

// Graph
export function getGraph(graphName?: string): Promise<GraphDump> {
	const qs = graphName ? `?graph=${encodeURIComponent(graphName)}` : '';
	return request('GET', `/api/graph${qs}`);
}

export function listGraphs(): Promise<GraphListResponse> {
	return request('GET', '/api/graphs');
}

export function dropGraph(name: string): Promise<void> {
	return request('DELETE', `/api/graphs/drop/${encodeURIComponent(name)}`);
}

// REST resources
export function getEntity(id: string): Promise<Entity> {
	return request('GET', `/api/entities/${encodeURIComponent(id)}`);
}

export function deleteEntity(id: string): Promise<{ id: string; name: string; edges_invalidated: number }> {
	return request('DELETE', `/api/entities/${encodeURIComponent(id)}`);
}

export function getEntityEdges(id: string): Promise<Edge[]> {
	return request('GET', `/api/entities/${encodeURIComponent(id)}/edges`);
}

export function getEdge(id: number): Promise<Edge> {
	return request('GET', `/api/edges/${id}`);
}

export function getEdgeProvenance(id: number): Promise<Edge[]> {
	return request('GET', `/api/edges/${id}/provenance`);
}

// Operations
export function maintain(): Promise<void> {
	return request('POST', '/api/maintain');
}

export function seed(req: SeedRequest): Promise<{ entities_created: number; edges_created: number }> {
	return request('POST', '/api/seed', req);
}

// Observability
export function health(): Promise<HealthResponse> {
	return request('GET', '/api/health');
}

export function getMetrics(): Promise<string> {
	return request('GET', '/api/metrics');
}

// Admin user management
export function listUsers(): Promise<{ users: User[] }> {
	return request('GET', '/api/admin/users');
}

export function createUser(
	userId: string,
	displayName: string,
	role: string
): Promise<{ user_id: string; api_key: string }> {
	return request('POST', '/api/admin/users', {
		user_id: userId,
		display_name: displayName,
		role
	});
}

export function deleteUser(userId: string): Promise<{ ok: boolean }> {
	return request('DELETE', `/api/admin/users/${encodeURIComponent(userId)}`);
}

export function listKeys(userId: string): Promise<{ keys: ApiKey[] }> {
	return request('GET', `/api/admin/users/${encodeURIComponent(userId)}/keys`);
}

export function createKey(
	userId: string,
	label: string
): Promise<{ user_id: string; label: string; api_key: string }> {
	return request('POST', `/api/admin/users/${encodeURIComponent(userId)}/keys`, { label });
}

export function deleteKey(userId: string, label: string): Promise<{ ok: boolean }> {
	return request('DELETE', `/api/admin/users/${encodeURIComponent(userId)}/keys/${encodeURIComponent(label)}`);
}

// Admin backup/restore
export function backup(graph?: string): Promise<GraphDumpBackup> {
	return request('POST', '/api/admin/backup', { graph });
}

export function restore(data: GraphDumpBackup): Promise<{ entities_created: number; edges_created: number }> {
	return request('POST', '/api/admin/restore', data);
}

// Cloud-only endpoints
export function getTenant(id: string): Promise<Tenant> {
	return request('GET', `/tenants/${encodeURIComponent(id)}`);
}

export function getUsage(tenantId: string): Promise<UsagePeriod> {
	return request('GET', `/tenants/${encodeURIComponent(tenantId)}/usage`);
}

export function deleteTenant(id: string): Promise<void> {
	return request('DELETE', `/tenants/${encodeURIComponent(id)}`);
}
