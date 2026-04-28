import {
  AuthenticationError,
  ForbiddenError,
  HippoError,
  RateLimitError,
} from "./errors.js";
import type {
  AskRequest,
  AskResponse,
  AuditResponse,
  ContextRequest,
  ContextResponse,
  CorrectRequest,
  CorrectResponse,
  CreateKeyRequest,
  CreateKeyResponse,
  CreateUserRequest,
  CreateUserResponse,
  DreamReport,
  EventsOptions,
  GraphEvent,
  GraphsListResponse,
  HealthResponse,
  HippoClientOptions,
  ListKeysResponse,
  ListUsersResponse,
  OnRequestHook,
  OnResponseHook,
  RememberBatchRequest,
  RememberBatchResponse,
  RememberRequest,
  RememberResponse,
  RetractRequest,
  RetractResponse,
} from "./models.js";

declare const process: { env: Record<string, string | undefined> } | undefined;

function getEnv(name: string): string | undefined {
  if (typeof process !== "undefined" && process?.env) {
    return process.env[name];
  }
  return undefined;
}

const RETRYABLE_STATUS_CODES = new Set([429, 502, 503, 504]);

// `/health` is the only route the server mounts at the root; everything else
// is under `/api`. We prepend it transparently so callers can pass plain
// paths like `/remember`.
function apiPath(path: string): string {
  if (path === "/health" || path.startsWith("/api/") || path === "/api") {
    return path;
  }
  return `/api${path}`;
}

/**
 * Parse a Retry-After header value into seconds.
 * Accepts either a number of seconds or an HTTP-date.
 */
function parseRetryAfter(value: string): number | undefined {
  const asNumber = Number(value);
  if (!Number.isNaN(asNumber) && asNumber >= 0) {
    return asNumber;
  }
  const date = new Date(value);
  if (!Number.isNaN(date.getTime())) {
    const seconds = (date.getTime() - Date.now()) / 1000;
    return Math.max(0, seconds);
  }
  return undefined;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function appendQuery(path: string, params: Record<string, string | number | undefined>): string {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined) {
      qs.set(k, String(v));
    }
  }
  const s = qs.toString();
  if (!s) return path;
  return `${path}${path.includes("?") ? "&" : "?"}${s}`;
}

export class HippoClient {
  private readonly baseUrl: string;
  private readonly apiKey: string | undefined;
  private readonly maxRetries: number;
  private readonly defaultTimeout: number;
  private readonly onRequest: OnRequestHook | undefined;
  private readonly onResponse: OnResponseHook | undefined;

  constructor(options: HippoClientOptions = {}) {
    const url = options.baseUrl ?? getEnv("HIPPO_URL") ?? "http://localhost:3000";
    this.baseUrl = url.replace(/\/+$/, "");
    this.apiKey = options.apiKey ?? getEnv("HIPPO_API_KEY");
    this.maxRetries = options.maxRetries ?? 3;
    this.defaultTimeout = options.timeout ?? 30_000;
    this.onRequest = options.onRequest;
    this.onResponse = options.onResponse;
  }

  // ---------------------------------------------------------------------------
  // HTTP helpers
  // ---------------------------------------------------------------------------

  private buildHeaders(includeContentType = true): Record<string, string> {
    const h: Record<string, string> = {};
    if (includeContentType) {
      h["Content-Type"] = "application/json";
    }
    if (this.apiKey) {
      h["Authorization"] = `Bearer ${this.apiKey}`;
    }
    return h;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    options?: { timeout?: number; raw?: boolean },
  ): Promise<T> {
    const url = `${this.baseUrl}${apiPath(path)}`;
    const timeout = options?.timeout ?? this.defaultTimeout;
    const raw = options?.raw ?? false;
    let lastError: unknown;
    let retryAfterOverride: number | undefined;

    for (let attempt = 0; attempt <= this.maxRetries; attempt++) {
      if (attempt > 0) {
        if (retryAfterOverride !== undefined && retryAfterOverride > 0) {
          await sleep(retryAfterOverride * 1000);
          retryAfterOverride = undefined;
        } else {
          const backoff = 0.5 * Math.pow(2, attempt - 1);
          const jitter = backoff * 0.5 * Math.random();
          await sleep((backoff + jitter) * 1000);
        }
      }

      this.onRequest?.(method, url, body);
      const start = Date.now();

      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), timeout);

      const init: RequestInit = {
        method,
        headers: this.buildHeaders(body !== undefined),
        signal: controller.signal,
      };
      if (body !== undefined) {
        init.body = JSON.stringify(body);
      }

      let res: Response;
      try {
        res = await fetch(url, init);
      } catch (err) {
        clearTimeout(timer);
        if (err instanceof DOMException || (err as Error)?.name === "AbortError") {
          throw new HippoError(`Request timed out after ${timeout}ms`, 0);
        }
        throw err;
      } finally {
        clearTimeout(timer);
      }

      const durationMs = Date.now() - start;
      this.onResponse?.(method, url, res.status, durationMs);

      if (res.ok) {
        if (res.status === 204) {
          return undefined as T;
        }
        if (raw) {
          return (await res.text()) as unknown as T;
        }
        return (await res.json()) as T;
      }

      if (RETRYABLE_STATUS_CODES.has(res.status) && attempt < this.maxRetries) {
        const retryAfterHeader = res.headers.get("Retry-After");
        if (retryAfterHeader) {
          retryAfterOverride = parseRetryAfter(retryAfterHeader);
        }
        await res.text().catch(() => "");
        lastError = res;
        continue;
      }

      const text = await res.text().catch(() => "");
      let parsed: unknown;
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }

      const msg =
        typeof parsed === "object" && parsed !== null && "error" in parsed
          ? String((parsed as Record<string, unknown>).error)
          : `HTTP ${res.status}: ${text}`;

      switch (res.status) {
        case 401:
          throw new AuthenticationError(msg, parsed);
        case 403:
          throw new ForbiddenError(msg, parsed);
        case 429: {
          const retry = res.headers.get("Retry-After");
          throw new RateLimitError(
            msg,
            parsed,
            retry ? parseRetryAfter(retry) : undefined,
          );
        }
        default:
          throw new HippoError(msg, res.status, parsed);
      }
    }

    throw lastError ?? new HippoError("Request failed after retries", 0);
  }

  // ---------------------------------------------------------------------------
  // Core endpoints
  // ---------------------------------------------------------------------------

  async remember(
    params: RememberRequest,
    options?: { timeout?: number },
  ): Promise<RememberResponse> {
    return this.request<RememberResponse>("POST", "/remember", params, options);
  }

  async rememberBatch(
    params: RememberBatchRequest,
    options?: { timeout?: number },
  ): Promise<RememberBatchResponse> {
    return this.request<RememberBatchResponse>("POST", "/remember/batch", params, options);
  }

  async context(
    params: ContextRequest,
    options?: { timeout?: number },
  ): Promise<ContextResponse> {
    return this.request<ContextResponse>("POST", "/context", params, options);
  }

  async ask(
    params: AskRequest,
    options?: { timeout?: number },
  ): Promise<AskResponse> {
    return this.request<AskResponse>("POST", "/ask", params, options);
  }

  // ---------------------------------------------------------------------------
  // Verb aliases — match the brand vocabulary from docs/DREAMS.md.
  // observe = remember, recall = ask. Aliases are wire-identical and exist
  // so callers can write `client.observe(...)` and `client.recall(...)` to
  // match the Dreamer narrative.
  // ---------------------------------------------------------------------------

  async observe(
    params: RememberRequest,
    options?: { timeout?: number },
  ): Promise<RememberResponse> {
    return this.remember(params, options);
  }

  async recall(
    params: AskRequest,
    options?: { timeout?: number },
  ): Promise<AskResponse> {
    return this.ask(params, options);
  }

  // ---------------------------------------------------------------------------
  // REST resources
  // ---------------------------------------------------------------------------

  async getEntity(
    id: string,
    options?: { graph?: string; timeout?: number },
  ): Promise<Record<string, unknown>> {
    const path = appendQuery(`/entities/${encodeURIComponent(id)}`, { graph: options?.graph });
    return this.request<Record<string, unknown>>("GET", path, undefined, options);
  }

  async deleteEntity(
    id: string,
    options?: { graph?: string; timeout?: number },
  ): Promise<Record<string, unknown>> {
    const path = appendQuery(`/entities/${encodeURIComponent(id)}`, { graph: options?.graph });
    return this.request<Record<string, unknown>>("DELETE", path, undefined, options);
  }

  async entityEdges(
    id: string,
    options?: { graph?: string; timeout?: number },
  ): Promise<Array<Record<string, unknown>>> {
    const path = appendQuery(`/entities/${encodeURIComponent(id)}/edges`, { graph: options?.graph });
    return this.request<Array<Record<string, unknown>>>("GET", path, undefined, options);
  }

  async getEdge(
    id: number,
    options?: { graph?: string; timeout?: number },
  ): Promise<Record<string, unknown>> {
    const path = appendQuery(`/edges/${id}`, { graph: options?.graph });
    return this.request<Record<string, unknown>>("GET", path, undefined, options);
  }

  async edgeProvenance(
    id: number,
    options?: { graph?: string; timeout?: number },
  ): Promise<Record<string, unknown>> {
    const path = appendQuery(`/edges/${id}/provenance`, { graph: options?.graph });
    return this.request<Record<string, unknown>>("GET", path, undefined, options);
  }

  // ---------------------------------------------------------------------------
  // Destructive ops
  // ---------------------------------------------------------------------------

  /** Trigger one dream pass synchronously and receive the aggregated report.
   * Useful for demos and evals; in production the Dreamer runs continuously
   * via the maintenance loop. */
  async dream(options?: { timeout?: number }): Promise<DreamReport> {
    return this.request<DreamReport>("POST", "/maintain", {}, options);
  }

  /** Alias for `dream()`. */
  async maintain(options?: { timeout?: number }): Promise<DreamReport> {
    return this.dream(options);
  }

  /** Explicit user/agent retraction. */
  async retract(
    params: RetractRequest,
    options?: { timeout?: number },
  ): Promise<RetractResponse> {
    return this.request<RetractResponse>("POST", "/retract", params, options);
  }

  /** Convenience: retract an old fact and observe a new one in one call. */
  async correct(
    params: CorrectRequest,
    options?: { timeout?: number },
  ): Promise<CorrectResponse> {
    return this.request<CorrectResponse>("POST", "/correct", params, options);
  }

  // ---------------------------------------------------------------------------
  // Graph operations
  // ---------------------------------------------------------------------------

  async graph(options?: { graph?: string; timeout?: number }): Promise<Record<string, unknown>> {
    const path = appendQuery("/graph", { graph: options?.graph });
    return this.request<Record<string, unknown>>("GET", path, undefined, options);
  }

  /** GET /graph?format=graphml|csv — returns the raw export text. */
  async graphExport(
    format: "graphml" | "csv",
    options?: { graph?: string; timeout?: number },
  ): Promise<string> {
    const path = appendQuery("/graph", { graph: options?.graph, format });
    return this.request<string>("GET", path, undefined, { ...options, raw: true });
  }

  async listGraphs(options?: { timeout?: number }): Promise<GraphsListResponse> {
    return this.request<GraphsListResponse>("GET", "/graphs", undefined, options);
  }

  async dropGraph(
    name: string,
    options?: { timeout?: number },
  ): Promise<Record<string, unknown>> {
    return this.request<Record<string, unknown>>(
      "DELETE",
      `/graphs/drop/${encodeURIComponent(name)}`,
      undefined,
      options,
    );
  }

  async seed(payload: unknown, options?: { timeout?: number }): Promise<Record<string, unknown>> {
    return this.request<Record<string, unknown>>("POST", "/seed", payload, options);
  }

  async backup(options?: { graph?: string; timeout?: number }): Promise<string> {
    return this.request<string>("POST", "/admin/backup", { graph: options?.graph }, {
      ...options,
      raw: true,
    });
  }

  async restore(payload: unknown, options?: { timeout?: number }): Promise<Record<string, unknown>> {
    return this.request<Record<string, unknown>>("POST", "/admin/restore", payload, options);
  }

  async openapi(options?: { timeout?: number }): Promise<string> {
    return this.request<string>("GET", "/openapi.yaml", undefined, { ...options, raw: true });
  }

  async metrics(options?: { timeout?: number }): Promise<string> {
    return this.request<string>("GET", "/metrics", undefined, { ...options, raw: true });
  }

  // ---------------------------------------------------------------------------
  // SSE streaming
  // ---------------------------------------------------------------------------

  async *events(options?: EventsOptions): AsyncGenerator<GraphEvent, void, unknown> {
    const params = new URLSearchParams();
    if (options?.graph) {
      params.set("graph", options.graph);
    }
    const qs = params.toString();
    const url = `${this.baseUrl}${apiPath("/events")}${qs ? `?${qs}` : ""}`;

    const headers: Record<string, string> = {
      Accept: "text/event-stream",
    };
    if (this.apiKey) {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }

    const res = await fetch(url, {
      method: "GET",
      headers,
      signal: options?.signal,
    });

    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new HippoError(`SSE connection failed: HTTP ${res.status}: ${text}`, res.status);
    }

    const reader = res.body?.getReader();
    if (!reader) {
      throw new HippoError("Response body is not readable", 0);
    }

    const decoder = new TextDecoder();
    let buffer = "";
    let currentEvent = "message";
    let currentData = "";

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";

        for (const line of lines) {
          if (line.startsWith("event:")) {
            currentEvent = line.slice(6).trim();
          } else if (line.startsWith("data:")) {
            currentData += line.slice(5).trim();
          } else if (line === "") {
            if (currentData) {
              let parsed: unknown;
              try {
                parsed = JSON.parse(currentData);
              } catch {
                parsed = currentData;
              }
              yield { event: currentEvent, data: parsed };
            }
            currentEvent = "message";
            currentData = "";
          }
        }
      }
    } finally {
      reader.releaseLock();
    }
  }

  // ---------------------------------------------------------------------------
  // Admin endpoints
  // ---------------------------------------------------------------------------

  async createUser(
    params: CreateUserRequest,
    options?: { timeout?: number },
  ): Promise<CreateUserResponse> {
    return this.request<CreateUserResponse>("POST", "/admin/users", params, options);
  }

  async listUsers(options?: { timeout?: number }): Promise<ListUsersResponse> {
    return this.request<ListUsersResponse>("GET", "/admin/users", undefined, options);
  }

  async deleteUser(userId: string, options?: { timeout?: number }): Promise<void> {
    return this.request<void>(
      "DELETE",
      `/admin/users/${encodeURIComponent(userId)}`,
      undefined,
      options,
    );
  }

  async createKey(
    userId: string,
    params: CreateKeyRequest,
    options?: { timeout?: number },
  ): Promise<CreateKeyResponse> {
    return this.request<CreateKeyResponse>(
      "POST",
      `/admin/users/${encodeURIComponent(userId)}/keys`,
      params,
      options,
    );
  }

  async listKeys(userId: string, options?: { timeout?: number }): Promise<ListKeysResponse> {
    return this.request<ListKeysResponse>(
      "GET",
      `/admin/users/${encodeURIComponent(userId)}/keys`,
      undefined,
      options,
    );
  }

  async deleteKey(
    userId: string,
    label: string,
    options?: { timeout?: number },
  ): Promise<void> {
    return this.request<void>(
      "DELETE",
      `/admin/users/${encodeURIComponent(userId)}/keys/${encodeURIComponent(label)}`,
      undefined,
      options,
    );
  }

  async audit(
    options?: { user_id?: string; action?: string; limit?: number; timeout?: number },
  ): Promise<AuditResponse> {
    const path = appendQuery("/admin/audit", {
      user_id: options?.user_id,
      action: options?.action,
      limit: options?.limit,
    });
    return this.request<AuditResponse>("GET", path, undefined, options);
  }

  // ---------------------------------------------------------------------------
  // Observability
  // ---------------------------------------------------------------------------

  async health(options?: { timeout?: number }): Promise<HealthResponse> {
    return this.request<HealthResponse>("GET", "/health", undefined, options);
  }
}
