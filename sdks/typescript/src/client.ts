import {
  AuthenticationError,
  ForbiddenError,
  HippoError,
  RateLimitError,
} from "./errors.js";
import type {
  AskRequest,
  AskResponse,
  ContextRequest,
  ContextResponse,
  CreateKeyRequest,
  CreateKeyResponse,
  CreateUserRequest,
  CreateUserResponse,
  EventsOptions,
  GraphEvent,
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
} from "./models.js";

declare const process: { env: Record<string, string | undefined> } | undefined;

function getEnv(name: string): string | undefined {
  if (typeof process !== "undefined" && process?.env) {
    return process.env[name];
  }
  return undefined;
}

const RETRYABLE_STATUS_CODES = new Set([429, 502, 503, 504]);

/**
 * Parse a Retry-After header value into seconds.
 * Accepts either a number of seconds or an HTTP-date.
 */
function parseRetryAfter(value: string): number | undefined {
  const asNumber = Number(value);
  if (!Number.isNaN(asNumber) && asNumber >= 0) {
    return asNumber;
  }
  // Try parsing as HTTP date
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

  private buildHeaders(): Record<string, string> {
    const h: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (this.apiKey) {
      h["Authorization"] = `Bearer ${this.apiKey}`;
    }
    return h;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    options?: { timeout?: number },
  ): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const timeout = options?.timeout ?? this.defaultTimeout;
    let lastError: unknown;
    let retryAfterOverride: number | undefined;

    for (let attempt = 0; attempt <= this.maxRetries; attempt++) {
      // If this is a retry, wait before the attempt
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
        headers: this.buildHeaders(),
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
        // AbortError from timeout — don't retry
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
        return (await res.json()) as T;
      }

      // Check if we should retry
      if (RETRYABLE_STATUS_CODES.has(res.status) && attempt < this.maxRetries) {
        // Respect Retry-After header if present
        const retryAfterHeader = res.headers.get("Retry-After");
        if (retryAfterHeader) {
          retryAfterOverride = parseRetryAfter(retryAfterHeader);
        }

        // Consume the body to free resources
        await res.text().catch(() => "");
        lastError = res;
        continue;
      }

      // Non-retryable error or retries exhausted — throw
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

    // Should not reach here, but just in case
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
  // SSE streaming
  // ---------------------------------------------------------------------------

  async *events(options?: EventsOptions): AsyncGenerator<GraphEvent, void, unknown> {
    const params = new URLSearchParams();
    if (options?.graph) {
      params.set("graph", options.graph);
    }
    const qs = params.toString();
    const url = `${this.baseUrl}/events${qs ? `?${qs}` : ""}`;

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
        // Keep the last incomplete line in the buffer
        buffer = lines.pop() ?? "";

        for (const line of lines) {
          if (line.startsWith("event:")) {
            currentEvent = line.slice(6).trim();
          } else if (line.startsWith("data:")) {
            currentData += line.slice(5).trim();
          } else if (line === "") {
            // Empty line = dispatch event
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

  // ---------------------------------------------------------------------------
  // Observability
  // ---------------------------------------------------------------------------

  async health(options?: { timeout?: number }): Promise<HealthResponse> {
    return this.request<HealthResponse>("GET", "/health", undefined, options);
  }
}
