import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { HippoClient } from "../src/client.js";
import {
  AuthenticationError,
  ForbiddenError,
  HippoError,
  RateLimitError,
} from "../src/errors.js";
import {
  findSubject,
  factsAbout,
  isDuplicate,
  failures,
} from "../src/helpers.js";
import type { ContextFact, ContextResponse, RememberResponse } from "../src/models.js";

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

function sseResponse(events: Array<{ event?: string; data: string }>): Response {
  let body = "";
  for (const e of events) {
    if (e.event) {
      body += `event:${e.event}\n`;
    }
    body += `data:${e.data}\n\n`;
  }
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  });
}

function fact(overrides: Partial<ContextFact> = {}): ContextFact {
  return {
    fact: "Alice works at Acme",
    subject: "Alice",
    relation_type: "WORKS_AT",
    object: "Acme",
    confidence: 0.95,
    salience: 1,
    valid_at: "2025-01-01T00:00:00Z",
    edge_id: 42,
    hops: 0,
    source_agents: ["test"],
    memory_tier: "long_term",
    ...overrides,
  };
}

function rememberBody(over: Partial<RememberResponse> = {}): RememberResponse {
  return {
    entities_created: 0,
    entities_resolved: 0,
    facts_written: 0,
    contradictions_invalidated: 0,
    usage: { llm_calls: 0, embed_calls: 0, input_tokens: 0, output_tokens: 0 },
    trace: { operations: [], execution: [] },
    ...over,
  };
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
    const client = new HippoClient({ baseUrl: "https://example.com///", maxRetries: 0 });
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
    const client = new HippoClient({ apiKey: "sk-test", maxRetries: 0 });
    await client.health();
    const headers = fetchSpy.mock.calls[0][1].headers;
    expect(headers["Authorization"]).toBe("Bearer sk-test");
  });

  it("omits Authorization when no apiKey", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ baseUrl: "http://localhost:3000", maxRetries: 0 });
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
    fetchSpy.mockResolvedValue(jsonResponse(rememberBody({ facts_written: 1, entities_created: 1 })));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    const result = await client.remember({ statement: "Alice likes cats" });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/remember",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.facts_written).toBe(1);
  });

  it("sends optional fields", async () => {
    fetchSpy.mockResolvedValue(jsonResponse(rememberBody()));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.remember({
      statement: "x",
      source_agent: "test",
      source_credibility_hint: 0.7,
      graph: "g",
      ttl_secs: 60,
    });
    const sent = JSON.parse(fetchSpy.mock.calls[0][1].body);
    expect(sent.source_agent).toBe("test");
    expect(sent.source_credibility_hint).toBe(0.7);
    expect(sent.graph).toBe("g");
    expect(sent.ttl_secs).toBe(60);
  });
});

describe("rememberBatch", () => {
  it("posts multiple statements", async () => {
    const body = { total: 2, succeeded: 2, failed: 0, results: [] };
    fetchSpy.mockResolvedValue(jsonResponse(body));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    const result = await client.rememberBatch({ statements: ["a", "b"] });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/remember/batch",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.total).toBe(2);
    expect(result.succeeded).toBe(2);
  });
});

describe("context", () => {
  it("returns a list of facts", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ facts: [fact()] }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    const result = await client.context({ query: "Alice" });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/context",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.facts).toHaveLength(1);
    expect(result.facts[0].subject).toBe("Alice");
  });

  it("sends advanced fields", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ facts: [] }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.context({
      query: "q",
      memory_tier_filter: "working",
      at: "2025-01-01T00:00:00Z",
      scoring: {
        w_relevance: 0.6, w_confidence: 0.1, w_recency: 0.2,
        w_salience: 0.1, mmr_lambda: 0.5,
      },
    });
    const sent = JSON.parse(fetchSpy.mock.calls[0][1].body);
    expect(sent.memory_tier_filter).toBe("working");
    expect(sent.scoring.w_relevance).toBe(0.6);
  });
});

describe("ask", () => {
  it("returns an answer with iterations", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ answer: "Yes", facts: [fact()], iterations: 1 }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    const result = await client.ask({ question: "Does Alice like cats?" });

    expect(result.answer).toBe("Yes");
    expect(result.facts).toHaveLength(1);
    expect(result.iterations).toBe(1);
  });

  it("sends max_iterations", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ answer: "x", iterations: 3 }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.ask({ question: "q", max_iterations: 3 });
    const sent = JSON.parse(fetchSpy.mock.calls[0][1].body);
    expect(sent.max_iterations).toBe(3);
  });
});

// ---------------------------------------------------------------------------
// REST resources, retract / correct, maintain
// ---------------------------------------------------------------------------

describe("REST resources", () => {
  it("getEntity hits /api/entities/{id}", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ id: "alice" }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.getEntity("alice");
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/entities/alice",
      expect.objectContaining({ method: "GET" }),
    );
  });

  it("deleteEntity returns the body", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ id: "alice", edges_invalidated: 3 }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    const out = await client.deleteEntity("alice");
    expect((out as Record<string, unknown>).edges_invalidated).toBe(3);
  });

  it("entityEdges hits /edges path", async () => {
    fetchSpy.mockResolvedValue(jsonResponse([]));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.entityEdges("alice", { graph: "g1" });
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/entities/alice/edges?graph=g1",
      expect.anything(),
    );
  });

  it("getEdge by numeric id", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ edge_id: 42 }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.getEdge(42);
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/edges/42",
      expect.anything(),
    );
  });

  it("edgeProvenance hits provenance subpath", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ edge_id: 42, supersedes: [] }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.edgeProvenance(42);
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/edges/42/provenance",
      expect.anything(),
    );
  });
});

describe("retract / correct / maintain", () => {
  it("retract POSTs to /api/retract", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ edge_id: 7 }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    const out = await client.retract({ edge_id: 7, reason: "bad" });
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/retract",
      expect.objectContaining({ method: "POST" }),
    );
    expect(out.edge_id).toBe(7);
  });

  it("correct POSTs to /api/correct", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      retracted_edge_id: 7,
      remember: rememberBody({ facts_written: 1 }),
    }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    const out = await client.correct({ edge_id: 7, statement: "Alice is a dentist" });
    expect(out.retracted_edge_id).toBe(7);
  });

  it("dream/maintain POSTs to /api/maintain", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      facts_visited: 1, links_written: 0, inferences_written: 0,
      supersessions_written: 0, contradictions_seen: 0, consolidations_written: 0,
      tokens_used: 0, duration_ms: 0,
    }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.dream();
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/maintain",
      expect.objectContaining({ method: "POST" }),
    );
  });
});

describe("graphs", () => {
  it("listGraphs returns names", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ default: "default", graphs: ["default"] }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    const out = await client.listGraphs();
    expect(out.default).toBe("default");
  });

  it("dropGraph DELETE", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ ok: true }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.dropGraph("other");
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/graphs/drop/other",
      expect.objectContaining({ method: "DELETE" }),
    );
  });
});

// ---------------------------------------------------------------------------
// Admin endpoints
// ---------------------------------------------------------------------------

describe("admin - users", () => {
  it("createUser sends correct body", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ user_id: "u1", api_key: "key123" }));
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    const result = await client.createUser({
      user_id: "u1",
      display_name: "User One",
      role: "reader",
    });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/admin/users",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.api_key).toBe("key123");
  });

  it("listUsers returns user list", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      users: [{ user_id: "u1", display_name: "User One", role: "admin", graphs: [], key_count: 1 }],
    }));
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    const result = await client.listUsers();

    expect(result.users).toHaveLength(1);
  });

  it("deleteUser sends DELETE", async () => {
    fetchSpy.mockResolvedValue(emptyResponse());
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    await client.deleteUser("u1");

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/admin/users/u1",
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("deleteUser encodes user_id", async () => {
    fetchSpy.mockResolvedValue(emptyResponse());
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    await client.deleteUser("user with spaces");

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/admin/users/user%20with%20spaces",
      expect.anything(),
    );
  });
});

describe("admin - keys", () => {
  it("createKey posts to correct path", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      user_id: "u1", label: "my-key", api_key: "sk-new",
    }));
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    const result = await client.createKey("u1", { label: "my-key" });

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/admin/users/u1/keys",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.api_key).toBe("sk-new");
  });

  it("listKeys returns keys", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({
      keys: [{ label: "default", created_at: "2025-01-01T00:00:00Z" }],
    }));
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    const result = await client.listKeys("u1");

    expect(result.keys).toHaveLength(1);
  });

  it("deleteKey sends DELETE to correct path", async () => {
    fetchSpy.mockResolvedValue(emptyResponse());
    const client = new HippoClient({ apiKey: "admin-key", maxRetries: 0 });

    await client.deleteKey("u1", "my-key");

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/admin/users/u1/keys/my-key",
      expect.objectContaining({ method: "DELETE" }),
    );
  });
});

describe("audit", () => {
  it("includes query params", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ entries: [] }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });
    await client.audit({ user_id: "alice", limit: 10 });
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/admin/audit?user_id=alice&limit=10",
      expect.anything(),
    );
  });
});

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

describe("health", () => {
  it("returns status", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ maxRetries: 0 });

    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(result.graph).toBe("default");
  });

  it("hits root /health (no /api prefix)", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ baseUrl: "https://x.test", maxRetries: 0 });
    await client.health();
    expect(fetchSpy).toHaveBeenCalledWith(
      "https://x.test/health",
      expect.anything(),
    );
  });
});

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

describe("error handling", () => {
  it("throws AuthenticationError on 401", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "invalid token" }, 401));
    const client = new HippoClient({ apiKey: "bad", maxRetries: 0 });

    await expect(client.health()).rejects.toThrow(AuthenticationError);
  });

  it("throws ForbiddenError on 403", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "admin only" }, 403));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    await expect(client.listUsers()).rejects.toThrow(ForbiddenError);
  });

  it("throws RateLimitError on 429 with Retry-After", async () => {
    fetchSpy.mockResolvedValue(
      jsonResponse({ error: "slow down" }, 429, { "Retry-After": "30" }),
    );
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

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
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

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
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

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
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect((err as HippoError).message).toBe("custom message");
    }
  });
});

// ---------------------------------------------------------------------------
// Retry behavior
// ---------------------------------------------------------------------------

describe("retry with exponential backoff", () => {
  it("retries on 502 then succeeds", async () => {
    fetchSpy
      .mockResolvedValueOnce(new Response("Bad Gateway", { status: 502 }))
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3 });
    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("retries on 503 then succeeds", async () => {
    fetchSpy
      .mockResolvedValueOnce(new Response("Service Unavailable", { status: 503 }))
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3 });
    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("retries on 504 then succeeds", async () => {
    fetchSpy
      .mockResolvedValueOnce(new Response("Gateway Timeout", { status: 504 }))
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3 });
    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("retries on 429 then succeeds", async () => {
    fetchSpy
      .mockResolvedValueOnce(jsonResponse({ error: "rate limited" }, 429))
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3 });
    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("throws after exhausting retries", async () => {
    fetchSpy.mockResolvedValue(new Response("Bad Gateway", { status: 502 }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 2 });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(HippoError);
      expect((err as HippoError).status).toBe(502);
    }

    expect(fetchSpy).toHaveBeenCalledTimes(3);
  });

  it("does not retry non-retryable errors", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "not found" }, 404));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3 });

    await expect(client.health()).rejects.toThrow(HippoError);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it("does not retry 401", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "invalid token" }, 401));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3 });

    await expect(client.health()).rejects.toThrow(AuthenticationError);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });
});

describe("maxRetries: 0 disables retry", () => {
  it("does not retry on 502 when maxRetries is 0", async () => {
    fetchSpy.mockResolvedValue(new Response("Bad Gateway", { status: 502 }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    await expect(client.health()).rejects.toThrow(HippoError);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it("does not retry on 429 when maxRetries is 0", async () => {
    fetchSpy.mockResolvedValue(jsonResponse({ error: "slow down" }, 429));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0 });

    await expect(client.health()).rejects.toThrow(RateLimitError);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });
});

describe("Retry-After header respect", () => {
  it("uses Retry-After seconds value for delay", async () => {
    fetchSpy
      .mockResolvedValueOnce(
        jsonResponse({ error: "rate limited" }, 429, { "Retry-After": "1" }),
      )
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 1 });
    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("uses Retry-After HTTP date value", async () => {
    const futureDate = new Date(Date.now() + 2000).toUTCString();
    fetchSpy
      .mockResolvedValueOnce(
        jsonResponse({ error: "rate limited" }, 429, { "Retry-After": futureDate }),
      )
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 1 });
    const result = await client.health();

    expect(result.status).toBe("ok");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });
});

// ---------------------------------------------------------------------------
// Timeout
// ---------------------------------------------------------------------------

describe("timeout", () => {
  it("throws on timeout", async () => {
    fetchSpy.mockImplementation((_url: string, init: RequestInit) => {
      return new Promise((_resolve, reject) => {
        init.signal?.addEventListener("abort", () => {
          const err = new DOMException("The operation was aborted", "AbortError");
          reject(err);
        });
      });
    });

    const client = new HippoClient({ apiKey: "k", timeout: 50, maxRetries: 0 });

    try {
      await client.health();
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(HippoError);
      expect((err as HippoError).message).toContain("timed out");
    }
  });

  it("per-request timeout overrides default", async () => {
    fetchSpy.mockImplementation((_url: string, init: RequestInit) => {
      return new Promise((_resolve, reject) => {
        init.signal?.addEventListener("abort", () => {
          const err = new DOMException("The operation was aborted", "AbortError");
          reject(err);
        });
      });
    });

    const client = new HippoClient({ apiKey: "k", timeout: 60000, maxRetries: 0 });

    try {
      await client.health({ timeout: 50 });
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(HippoError);
      expect((err as HippoError).message).toContain("timed out");
    }
  });
});

// ---------------------------------------------------------------------------
// onRequest / onResponse hooks
// ---------------------------------------------------------------------------

describe("onRequest and onResponse hooks", () => {
  it("calls onRequest before each fetch", async () => {
    const onRequest = vi.fn();
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0, onRequest });

    await client.health();

    expect(onRequest).toHaveBeenCalledTimes(1);
    expect(onRequest).toHaveBeenCalledWith("GET", "http://localhost:3000/health", undefined);
  });

  it("calls onRequest with body for POST", async () => {
    const onRequest = vi.fn();
    fetchSpy.mockResolvedValue(jsonResponse(rememberBody()));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0, onRequest });

    await client.remember({ statement: "test" });

    expect(onRequest).toHaveBeenCalledWith(
      "POST",
      "http://localhost:3000/api/remember",
      { statement: "test" },
    );
  });

  it("calls onResponse after each fetch", async () => {
    const onResponse = vi.fn();
    fetchSpy.mockResolvedValue(jsonResponse({ status: "ok", graph: "default" }));
    const client = new HippoClient({ apiKey: "k", maxRetries: 0, onResponse });

    await client.health();

    expect(onResponse).toHaveBeenCalledTimes(1);
    expect(onResponse).toHaveBeenCalledWith(
      "GET",
      "http://localhost:3000/health",
      200,
      expect.any(Number),
    );
  });

  it("calls hooks on each retry attempt", async () => {
    const onRequest = vi.fn();
    const onResponse = vi.fn();
    fetchSpy
      .mockResolvedValueOnce(new Response("Bad Gateway", { status: 502 }))
      .mockResolvedValueOnce(jsonResponse({ status: "ok", graph: "default" }));

    const client = new HippoClient({ apiKey: "k", maxRetries: 3, onRequest, onResponse });

    await client.health();

    expect(onRequest).toHaveBeenCalledTimes(2);
    expect(onResponse).toHaveBeenCalledTimes(2);
    expect(onResponse.mock.calls[0][2]).toBe(502);
    expect(onResponse.mock.calls[1][2]).toBe(200);
  });
});

// ---------------------------------------------------------------------------
// Response helper functions
// ---------------------------------------------------------------------------

describe("response helpers", () => {
  describe("findSubject", () => {
    it("finds a fact by subject", () => {
      const response: ContextResponse = {
        facts: [fact({ subject: "Alice" }), fact({ subject: "Bob" })],
      };
      const found = findSubject(response, "Alice");
      expect(found?.subject).toBe("Alice");
    });

    it("returns undefined when not found", () => {
      const response: ContextResponse = { facts: [fact({ subject: "Alice" })] };
      expect(findSubject(response, "Charlie")).toBeUndefined();
    });
  });

  describe("factsAbout", () => {
    it("filters facts where entity is subject or object", () => {
      const response: ContextResponse = {
        facts: [
          fact({ subject: "Alice", object: "Acme", edge_id: 1 }),
          fact({ subject: "Bob", object: "Alice", edge_id: 2 }),
          fact({ subject: "Carol", object: "Dave", edge_id: 3 }),
        ],
      };
      const out = factsAbout(response, "alice");
      expect(out.map((f) => f.edge_id)).toEqual([1, 2]);
    });

    it("returns empty array when no matches", () => {
      const response: ContextResponse = { facts: [fact({ subject: "Alice" })] };
      expect(factsAbout(response, "Bob")).toHaveLength(0);
    });
  });

  describe("isDuplicate", () => {
    it("returns true when facts_written is 0", () => {
      expect(isDuplicate(rememberBody({ facts_written: 0 }))).toBe(true);
    });

    it("returns false when facts_written is > 0", () => {
      expect(isDuplicate(rememberBody({ facts_written: 2 }))).toBe(false);
    });
  });

  describe("failures", () => {
    it("filters failed results from batch response", () => {
      const failed = failures({
        total: 3, succeeded: 2, failed: 1,
        results: [
          { statement: "ok1", ok: true },
          { statement: "bad", ok: false, error: "parse error" },
          { statement: "ok2", ok: true },
        ],
      });
      expect(failed).toHaveLength(1);
      expect(failed[0].error).toBe("parse error");
    });

    it("returns empty array when no failures", () => {
      expect(failures({
        total: 2, succeeded: 2, failed: 0,
        results: [
          { statement: "a", ok: true },
          { statement: "b", ok: true },
        ],
      })).toHaveLength(0);
    });
  });
});

// ---------------------------------------------------------------------------
// SSE events streaming
// ---------------------------------------------------------------------------

describe("events() SSE streaming", () => {
  it("parses SSE stream into GraphEvent objects", async () => {
    fetchSpy.mockResolvedValue(
      sseResponse([
        { event: "entity_created", data: '{"name":"Alice"}' },
        { event: "edge_created", data: '{"source":"Alice","target":"Bob"}' },
      ]),
    );

    const client = new HippoClient({ apiKey: "k" });
    const events: Array<{ event: string; data: unknown }> = [];

    for await (const event of client.events({ graph: "mydb" })) {
      events.push(event);
    }

    expect(events).toHaveLength(2);
    expect(events[0].event).toBe("entity_created");
    expect(events[0].data).toEqual({ name: "Alice" });
    expect(events[1].event).toBe("edge_created");
    expect(events[1].data).toEqual({ source: "Alice", target: "Bob" });
  });

  it("uses default event type 'message' when not specified", async () => {
    fetchSpy.mockResolvedValue(
      sseResponse([{ data: '{"msg":"hello"}' }]),
    );

    const client = new HippoClient({ apiKey: "k" });
    const events: Array<{ event: string; data: unknown }> = [];

    for await (const event of client.events()) {
      events.push(event);
    }

    expect(events).toHaveLength(1);
    expect(events[0].event).toBe("message");
  });

  it("hits /api/events with graph query parameter", async () => {
    fetchSpy.mockResolvedValue(sseResponse([]));

    const client = new HippoClient({ apiKey: "k" });

    for await (const _ of client.events({ graph: "testdb" })) {
      // no events
    }

    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:3000/api/events?graph=testdb",
      expect.anything(),
    );
  });

  it("sends Authorization header", async () => {
    fetchSpy.mockResolvedValue(sseResponse([]));

    const client = new HippoClient({ apiKey: "sk-secret" });

    for await (const _ of client.events()) {
      // no events
    }

    const headers = fetchSpy.mock.calls[0][1].headers;
    expect(headers["Authorization"]).toBe("Bearer sk-secret");
  });

  it("throws on non-OK response", async () => {
    fetchSpy.mockResolvedValue(new Response("Unauthorized", { status: 401 }));

    const client = new HippoClient({ apiKey: "bad" });

    try {
      for await (const _ of client.events()) {
        // should not reach
      }
      expect.unreachable("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(HippoError);
      expect((err as HippoError).status).toBe(401);
    }
  });

  it("supports cancellation via AbortSignal", async () => {
    const abortCtrl = new AbortController();
    const encoder = new TextEncoder();

    const stream = new ReadableStream({
      async start(ctrl) {
        ctrl.enqueue(encoder.encode("event:tick\ndata:{\"n\":1}\n\n"));
        ctrl.enqueue(encoder.encode("event:tick\ndata:{\"n\":2}\n\n"));
        ctrl.close();
      },
    });

    fetchSpy.mockResolvedValue(new Response(stream, {
      status: 200,
      headers: { "Content-Type": "text/event-stream" },
    }));

    const client = new HippoClient({ apiKey: "k" });
    const events: Array<{ event: string; data: unknown }> = [];

    for await (const event of client.events({ signal: abortCtrl.signal })) {
      events.push(event);
      abortCtrl.abort();
      break;
    }

    expect(events).toHaveLength(1);
    expect(events[0].event).toBe("tick");
  });
});
