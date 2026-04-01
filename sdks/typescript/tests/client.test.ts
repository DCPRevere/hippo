import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { HippoClient } from "../src/client.js";
import {
  AuthenticationError,
  ForbiddenError,
  HippoError,
  RateLimitError,
} from "../src/errors.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function jsonResponse(body: unknown, status = 200, headers?: Record<string, string>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  });
}

function emptyResponse(status = 204): Response {
  return new Response(null, { status });
}

let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchSpy = vi.fn();
  vi.stubGlobal("fetch", fetchSpy);
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

describe("HippoClient constructor", () => {
  it("uses provided baseUrl and apiKey", () => {
    const client = new HippoClient({ baseUrl: "https://example.com", apiKey: "sk-test" });
    expect(client).toBeDefined();
  });

  it("falls back to defaults", () => {
    const client = new HippoClient();
    expect(client).toBeDefined();
  });

  it("strips trailing slashes from baseUrl", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ baseUrl: "https://example.com///" });
    await client.health();
    expect(fetchSpy).toHaveBeenCalledWith(
      "https://example.com/health",
      expect.anything(),
    );
  });
});

// ---------------------------------------------------------------------------
// Auth header
// ---------------------------------------------------------------------------

describe("Authorization header", () => {
  it("sends Bearer token when apiKey is set", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ apiKey: "sk-test" });
    await client.health();
    const headers = fetchSpy.mock.calls[0][1].headers;
    expect(headers["Authorization"]).toBe("Bearer sk-test");
  });

  it("omits Authorization when no apiKey", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ baseUrl: "http://localhost:3000" });
    await client.health();
    const headers = fetchSpy.mock.calls[0][1].headers;
    expect(headers["Authorization"]).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Core endpoints
// ---------------------------------------------------------------------------

describe("remember", () => {
  it("posts a statement and returns result", async () => {
    const body = {
      entities_created: 1,
      entities_resolved: 0,
      facts_written: 1,
      contradictions_invalidated: 0,
      usage: {},
      trace: {},
    };
    fetchSpy.mockResolvedValue(jsonResponse(body));
    const client = new HippoClient({ apiKey: "k" });

    const result = await client.remember({ statement: "Alice likes cats" });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/remember",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.facts_written).toBe(1);
  });

  it("sends optional fields", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      entities_created: 0, entities_resolved: 0,
      facts_written: 0, contradictions_invalidated: 0,
      usage: {}, trace: {},
    }));
    const client = new HippoClient({ apiKey: "k" });
    await client.remember({
      statement: "x",
      source_agent: "test",
      graph: "g",
      ttl_secs: 60,
    });
    const sent = JSON.parse(fetchSpy.mock.calls[0][1].body);
    expect(sent.source_agent).toBe("test");
    expect(sent.graph).toBe("g");
    expect(sent.ttl_secs).toBe(60);
  });
});

describe("rememberBatch", () => {
  it("posts multiple statements", async () => {
    const body = { total: 2, succeeded: 2, failed: 0, results: [] };
    fetchSpy.mockResolvedValue(jsonResponse(body));
    const client = new HippoClient({ apiKey: "k" });

    const result = await client.rememberBatch({ statements: ["a", "b"] });

    expect(result.total).toBe(2);
    expect(result.succeeded).toBe(2);
  });
});

describe("context", () => {
  it("returns nodes and edges", async () => {
    const body = { nodes: [{ id: "1" }], edges: [{ from: "1", to: "2" }] };
    fetchSpy.mockResolvedValue(jsonResponse(body));
    const client = new HippoClient({ apiKey: "k" });

    const result = await client.context({ query: "Alice" });

    expect(result.nodes).toHaveLength(1);
    expect(result.edges).toHaveLength(1);
  });
});

describe("ask", () => {
  it("returns an answer", async () => {
    const body = { answer: "Yes", facts: [{ text: "Alice likes cats" }] };
    fetchSpy.mockResolvedValue(jsonResponse(body));
    const client = new HippoClient({ apiKey: "k" });

    const result = await client.ask({ question: "Does Alice like cats?" });

    expect(result.answer).toBe("Yes");
    expect(result.facts).toHaveLength(1);
  });
});

// ---------------------------------------------------------------------------
// Admin endpoints
// ---------------------------------------------------------------------------

describe("admin - users", () => {
  it("createUser sends correct body", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ user_id: "u1", api_key: "key123" }));
    const client = new HippoClient({ apiKey: "admin-key" });

    const result = await client.createUser({
      user_id: "u1",
      display_name: "User One",
      role: "reader",
    });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/admin/users",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.api_key).toBe("key123");
  });

  it("listUsers returns user list", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      users: [{ user_id: "u1", display_name: "User One", role: "admin", graphs: [], key_count: 1 }],
    }));
    const client = new HippoClient({ apiKey: "admin-key" });

    const result = await client.listUsers();

    expect(result.users).toHaveLength(1);
  });

  it("deleteUser sends DELETE", async () => {
    fetchSpy.mockResolvedValue(emptyResponse());
    const client = new HippoClient({ apiKey: "admin-key" });

    await client.deleteUser("u1");

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/admin/users/u1",
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("deleteUser encodes user_id", async () => {
    fetchSpy.mockResolvedValue(emptyResponse());
    const client = new HippoClient({ apiKey: "admin-key" });

    await client.deleteUser("user with spaces");

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/admin/users/user%20with%20spaces",
      expect.anything(),
    );
  });
});

describe("admin - keys", () => {
  it("createKey posts to correct path", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      user_id: "u1", label: "my-key", api_key: "sk-new",
    }));
    const client = new HippoClient({ apiKey: "admin-key" });

    const result = await client.createKey("u1", { label: "my-key" });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/admin/users/u1/keys",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.api_key).toBe("sk-new");
  });

  it("listKeys returns keys", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      keys: [{ label: "default", created_at: "2025-01-01T00:00:00Z" }],
    }));
    const client = new HippoClient({ apiKey: "admin-key" });

    const result = await client.listKeys("u1");

    expect(result.keys).toHaveLength(1);
  });

  it("deleteKey sends DELETE to correct path", async () => {
    fetchSpy.mockResolvedValue(emptyResponse());
    const client = new HippoClient({ apiKey: "admin-key" });

    await client.deleteKey("u1", "my-key");

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/admin/users/u1/keys/my-key",
      expect.objectContaining({ method: "DELETE" }),
    );
  });
});

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

describe("health", () => {
  it("returns status", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient();

    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(result.graph).toBe("default");
  });
});

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

describe("error handling", () => {
  it("throws AuthenticationError on 401", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "invalid token" }, 401));
    const client = new HippoClient({ apiKey: "bad" });

    await expect(client.health()).rejects.toThrow(AuthenticationError);
  });

  it("throws ForbiddenError on 403", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "admin only" }, 403));
    const client = new HippoClient({ apiKey: "k" });

    await expect(client.listUsers()).rejects.toThrow(ForbiddenError);
  });

  it("throws RateLimitError on 429 with Retry-After", async () => {
    fetchSpy.mockResolvedValue(
      jsonResponse({ error: "slow down" }, 429, { "Retry-After": "30" }),
    );
    const client = new HippoClient({ apiKey: "k" });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(RateLimitError);
      expect((err as RateLimitError).retryAfter).toBe(30);
    }
  });

  it("throws HippoError on other status codes", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "not found" }, 404));
    const client = new HippoClient({ apiKey: "k" });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(HippoError);
      expect((err as HippoError).status).toBe(404);
    }
  });

  it("handles non-JSON error responses", async () => {
    fetchSpy.mockResolvedValue(new Response("Bad Gateway", { status: 502 }));
    const client = new HippoClient({ apiKey: "k" });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(HippoError);
      expect((err as HippoError).status).toBe(502);
    }
  });

  it("error message uses error field from JSON body", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "custom message" }, 400));
    const client = new HippoClient({ apiKey: "k" });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect((err as HippoError).message).toBe("custom message");
    }
  });
});
