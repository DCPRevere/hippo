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
  HealthResponse,
  HippoClientOptions,
  ListKeysResponse,
  ListUsersResponse,
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

export class HippoClient {
  private readonly baseUrl: string;
  private readonly apiKey: string | undefined;

  constructor(options: HippoClientOptions = {}) {
    const url = options.baseUrl ?? getEnv("HIPPO_URL") ?? "http://localhost:3000";
    this.baseUrl = url.replace(/\/+$/, "");
    this.apiKey = options.apiKey ?? getEnv("HIPPO_API_KEY");
  }

  // ---------------------------------------------------------------------------
  // HTTP helpers
  // ---------------------------------------------------------------------------

  private headers(): Record<string, string> {
    const h: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (this.apiKey) {
      h["Authorization"] = `Bearer ${this.apiKey}`;
    }
    return h;
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const init: RequestInit = {
      method,
      headers: this.headers(),
    };
    if (body !== undefined) {
      init.body = JSON.stringify(body);
    }

    const res = await fetch(url, init);

    if (!res.ok) {
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
            retry ? Number(retry) : undefined,
          );
        }
        default:
          throw new HippoError(msg, res.status, parsed);
      }
    }

    if (res.status === 204) {
      return undefined as T;
    }

    return (await res.json()) as T;
  }

  // ---------------------------------------------------------------------------
  // Core endpoints
  // ---------------------------------------------------------------------------

  async remember(params: RememberRequest): Promise<RememberResponse> {
    return this.request<RememberResponse>("POST", "/remember", params);
  }

  async rememberBatch(params: RememberBatchRequest): Promise<RememberBatchResponse> {
    return this.request<RememberBatchResponse>("POST", "/remember/batch", params);
  }

  async context(params: ContextRequest): Promise<ContextResponse> {
    return this.request<ContextResponse>("POST", "/context", params);
  }

  async ask(params: AskRequest): Promise<AskResponse> {
    return this.request<AskResponse>("POST", "/ask", params);
  }

  // ---------------------------------------------------------------------------
  // Admin endpoints
  // ---------------------------------------------------------------------------

  async createUser(params: CreateUserRequest): Promise<CreateUserResponse> {
    return this.request<CreateUserResponse>("POST", "/admin/users", params);
  }

  async listUsers(): Promise<ListUsersResponse> {
    return this.request<ListUsersResponse>("GET", "/admin/users");
  }

  async deleteUser(userId: string): Promise<void> {
    return this.request<void>("DELETE", `/admin/users/${encodeURIComponent(userId)}`);
  }

  async createKey(userId: string, params: CreateKeyRequest): Promise<CreateKeyResponse> {
    return this.request<CreateKeyResponse>(
      "POST",
      `/admin/users/${encodeURIComponent(userId)}/keys`,
      params,
    );
  }

  async listKeys(userId: string): Promise<ListKeysResponse> {
    return this.request<ListKeysResponse>(
      "GET",
      `/admin/users/${encodeURIComponent(userId)}/keys`,
    );
  }

  async deleteKey(userId: string, label: string): Promise<void> {
    return this.request<void>(
      "DELETE",
      `/admin/users/${encodeURIComponent(userId)}/keys/${encodeURIComponent(label)}`,
    );
  }

  // ---------------------------------------------------------------------------
  // Observability
  // ---------------------------------------------------------------------------

  async health(): Promise<HealthResponse> {
    return this.request<HealthResponse>("GET", "/health");
  }
}
