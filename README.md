# Hippo

> Graph-native episodic memory for AI agents

> ⭐ **40 commits, 6 eval modes, 15 API endpoints, working/long-term memory tiers, MCP server**

## What is this?

A production-grade memory layer for AI agents. Unlike vector-only solutions (RAG), this uses a knowledge graph ([FalkorDB](https://www.falkordb.com/)) backed by three retrieval modes: fulltext, vector similarity, and graph traversal. Facts are explicitly versioned, contradictions are detected automatically, and knowledge decays realistically over time.

## Why not just a vector database?

Vector databases retrieve by semantic similarity alone. They lose the structural relationships between entities — you can't traverse from "Alice" to "Alice's doctor" to "the doctor's office" in a vector store. They also can't detect contradictions ("Alice works at Google" vs "Alice works at Meta") because similar embeddings don't imply conflict. Graph structure makes multi-hop reasoning, temporal queries, and contradiction detection possible without re-encoding context into prompts.

## Key features

- Entity extraction and resolution (deduplication via fulltext + vector + LLM)
- Three-mode retrieval: fulltext + vector + graph walk
- Automatic contradiction detection (batch-classified, pre-filtered by cosine similarity)
- Temporal queries: "what did the system know on date X?"
- Multi-hop traversal with relevance decay per hop (0.6^(hop-1))
- Confidence decay over time (stale facts lose confidence after 30 days)
- Contradiction history: supersession chain preserved, not deleted
- Multi-source confidence compounding (Bayesian: `1 - (1 - old) * (1 - new)`, capped at 0.99)
- Salience tracking: frequently retrieved facts rank higher
- Introspective reflection: knowledge gaps, suggested questions, memory stats
- Background maintenance: link discovery, contradiction scanning, placeholder resolution
- Per-test graph isolation for parallel test runs

## API Reference

### POST /remember

Ingest a natural language statement. Extracts entities and facts via LLM, resolves entities against existing graph nodes, detects duplicates/contradictions, and writes new edges.

**Request:**
```json
{
  "statement": "Alice works at Google as a software engineer",
  "source_agent": "user-facing-chat"
}
```

`source_agent` is optional (defaults to `"unknown"`). Used for multi-source confidence compounding.

**Response:**
```json
{
  "entities_created": 2,
  "entities_resolved": 0,
  "facts_written": 1,
  "contradictions_invalidated": 0,
  "trace": {
    "extraction": {
      "entities": [
        { "name": "Alice", "entity_type": "person", "resolved": true, "hint": null }
      ],
      "explicit_facts": [
        {
          "subject": "Alice",
          "relation_type": "WORKS_AT",
          "object": "Google",
          "fact": "Alice works at Google as a software engineer",
          "confidence": 0.9
        }
      ],
      "implied_facts": []
    },
    "entity_resolutions": [
      {
        "extracted_name": "Alice",
        "extracted_type": "person",
        "outcome": "exact_match",
        "resolved_to": "Alice",
        "candidates_considered": ["Alice"]
      }
    ],
    "fact_processing": [
      {
        "fact": "Alice works at Google as a software engineer",
        "subject": "Alice",
        "object": "Google",
        "relation_type": "WORKS_AT",
        "outcome": "written",
        "details": null
      }
    ]
  }
}
```

### POST /context

Retrieve relevant facts for a query. Combines fulltext search, entity-centric N-hop graph walks, and vector similarity. Results are scored and ranked.

**Request:**
```json
{
  "query": "What do I know about Alice?",
  "limit": 10,
  "max_hops": 2
}
```

- `limit`: max facts to return (default: 10)
- `max_hops`: graph traversal depth from matched entities (default: 2, max: 3)

**Response:**
```json
{
  "facts": [
    {
      "fact": "Alice works at Google",
      "subject": "Alice",
      "relation_type": "WORKS_AT",
      "object": "Google",
      "confidence": 0.9,
      "salience": 3,
      "valid_at": "2025-01-15T10:00:00Z",
      "edge_id": 42,
      "hops": 1,
      "source_agents": ["user-facing-chat"]
    }
  ]
}
```

### POST /context/temporal

Query what the system knew at a specific point in time. Only returns facts that were valid at the given timestamp (created before `at`, not yet invalidated at `at`).

**Request:**
```json
{
  "query": "Alice",
  "at": "2025-06-01T00:00:00Z",
  "limit": 10
}
```

**Response:** Same shape as `/context`.

### GET /timeline/:entity_name

Full chronological history of an entity, including superseded facts.

**Response:**
```json
{
  "entity": "Alice",
  "events": [
    {
      "fact": "Alice works at Google",
      "relation_type": "WORKS_AT",
      "valid_at": "2025-01-15T10:00:00Z",
      "invalid_at": "2025-07-01T10:00:00Z",
      "superseded": true
    },
    {
      "fact": "Alice works at Meta",
      "relation_type": "WORKS_AT",
      "valid_at": "2025-07-01T10:00:00Z",
      "invalid_at": null,
      "superseded": false
    }
  ]
}
```

### POST /reflect

Introspective analysis of what the system knows (and doesn't know). With `about`, analyzes a specific entity. Without it, returns global memory stats.

**Request:**
```json
{
  "about": "Alice",
  "suggest_questions": true
}
```

**Response (entity-scoped):**
```json
{
  "entity": "Alice",
  "known": [ ... ],
  "uncertain": [ ... ],
  "gaps": ["LIVES_IN", "ATTENDED"],
  "suggested_questions": ["Where does Alice live?", "What school did Alice attend?"],
  "stats": null
}
```

- `known`: facts with decayed confidence >= 0.6
- `uncertain`: facts with decayed confidence < 0.6
- `gaps`: relation types that exist in the graph but are missing for this entity
- `suggested_questions`: LLM-generated questions to fill the gaps

**Response (global, `about` omitted):**
```json
{
  "entity": null,
  "known": [],
  "uncertain": [],
  "gaps": ["Alice (person) — 1 fact(s)", "..."],
  "suggested_questions": [],
  "stats": {
    "total_entities": 12,
    "total_facts": 25,
    "oldest_fact": "2025-01-15T10:00:00Z",
    "newest_fact": "2025-07-01T10:00:00Z",
    "avg_confidence": 0.87,
    "entities_by_type": { "person": 5, "organization": 4, "place": 3 }
  }
}
```

### GET /provenance/:edge_id

Returns the supersession chain for a fact edge — what it replaced and what replaced it.

**Response:**
```json
{
  "edge_id": 42,
  "superseded_by": {
    "old_edge_id": 42,
    "new_edge_id": 57,
    "superseded_at": "2025-07-01T10:00:00Z",
    "old_fact": "Alice works at Google",
    "new_fact": "Alice works at Meta"
  },
  "supersedes": []
}
```

### POST /diagnose

Debug the retrieval pipeline. Returns the same results as `/context` but includes detailed step-by-step traces showing what each stage found and how scores were assigned.

**Request:** Same as `/context`.

**Response:**
```json
{
  "query": "Alice",
  "steps": [
    { "step": "fulltext_facts", "description": "...", "results": [...] },
    { "step": "fulltext_entities", "description": "...", "results": [...] },
    { "step": "exact_entity_hop", "description": "...", "results": [...] },
    { "step": "vector_fallback", "description": "...", "results": [...] },
    { "step": "invalidation_filter", "description": "...", "results": [] },
    { "step": "scoring", "description": "score = relevance*0.6 + recency*0.25 + salience_norm*0.15", "results": [...] }
  ],
  "final_facts": [...]
}
```

### POST /maintain

Trigger a maintenance cycle manually (normally runs on a background timer). Performs: confidence/salience decay, link discovery between nearby entities, contradiction scanning, and placeholder entity resolution.

**Response:**
```json
{ "status": "maintenance complete" }
```

### GET /graph

Dump the entire graph. Returns all entities and edges, split into active and invalidated.

**Response:**
```json
{
  "entities": [
    { "name": "Alice", "entity_type": "person", "resolved": true }
  ],
  "active_edges": [
    {
      "subject": "Alice", "relation_type": "WORKS_AT", "object": "Meta",
      "fact": "Alice works at Meta", "salience": 3, "confidence": 0.9,
      "valid_at": "2025-07-01T10:00:00Z", "invalid_at": null
    }
  ],
  "invalidated_edges": [...]
}
```

### GET /health

**Response:**
```json
{ "status": "ok", "graph": "hippo" }
```

## Architecture

```
Statement
  → LLM: extract entities + facts
  → For each entity: fulltext + vector search → LLM resolve (dedup) → upsert
  → For each fact: embed → cosine prefilter (>0.3) → batch classify → write / skip / invalidate
  → On contradiction: invalidate old edge, record supersession, write new edge
  → On duplicate: Bayesian confidence compounding, merge source_agents

Background maintenance (periodic):
  → Confidence decay (0.995^days after 30 days stale)
  → Salience decay (decrement by 1 if stale >7 days)
  → Link discovery between vector-similar unlinked entities
  → Contradiction scan across same-pair edges
  → Placeholder entity resolution (unresolved → resolved match)
```

## Retrieval pipeline

```
Query
  → embed query
  → fulltext search on fact text (relevance: 0.95)
  → fulltext search on entity names:
      → exact token matches: N-hop walk (relevance: 0.9 × 0.6^(hop-1))
      → partial matches: N-hop walk (relevance: 0.6 × 0.6^(hop-1))
  → vector search on edge embeddings (blended: score × 0.3, additive)
  → filter invalidated edges
  → score = relevance × 0.5 + confidence × 0.1 + recency × 0.25 + salience × 0.15
  → increment salience on returned edges
```

## Comparison vs alternatives

| Feature | This | MemGPT | Zep | ChromaDB |
|---------|------|--------|-----|----------|
| Graph traversal | ✅ | ❌ | ⚠️ | ❌ |
| Contradiction detection | ✅ | ⚠️ | ✅ | ❌ |
| Temporal queries | ✅ | ❌ | ❌ | ❌ |
| Confidence decay | ✅ | ❌ | ❌ | ❌ |
| Working/long-term tiers | ✅ | ✅ | ❌ | ❌ |
| Introspection/reflect | ✅ | ❌ | ❌ | ❌ |
| Provenance chain | ✅ | ❌ | ❌ | ❌ |
| Multi-source compounding | ✅ | ❌ | ❌ | ❌ |
| Source credibility | ✅ | ❌ | ❌ | ❌ |
| MCP server | ✅ | ❌ | ❌ | ❌ |
| Prometheus metrics | ✅ | ❌ | ❌ | ❌ |
| SSE streaming ingest | ✅ | ❌ | ❌ | ❌ |
| Eval regression tracking | ✅ | ❌ | ❌ | ❌ |
| Fixture record/replay | ✅ | ❌ | ❌ | ❌ |

## Running locally

Start dependencies:

```sh
docker compose up -d falkordb ollama ollama-init
```

Wait for the Ollama model pull to finish, then either run with Docker:

```sh
docker compose up hippo
```

Or run directly (requires Rust toolchain):

```sh
export ANTHROPIC_API_KEY="sk-..."
export FALKORDB_URL="redis://localhost:6379"
export OLLAMA_URL="http://localhost:11434"
cargo run --release
```

Verify:

```sh
curl http://localhost:21693/health
```

## Running

```bash
# Start the server
cargo run --

# Run correctness evals (requires LLM API key)
cargo test eval_ -- --nocapture --test-threads=4

# Run scenario integration tests
cargo test scenario_ -- --nocapture --test-threads=1

# Run eval score (7 evals with partial scoring)
cargo run --bin eval-score

# Run with mock LLM (fast, no API key)
MOCK_LLM=1 cargo test -- --test-threads=4

# Record LLM fixtures for replay
EVAL_RECORD=1 cargo test eval_ -- --test-threads=1

# Replay from fixtures (deterministic, free)
EVAL_REPLAY=1 cargo test eval_ -- --test-threads=4

# Track eval regression
cargo run --bin eval-regression

# Benchmark
cargo run --bin benchmark

# MCP server (Claude Desktop integration)
HIPPO_URL=http://localhost:21693 cargo run --bin mcp-server
```

### Eval suite

The eval suite measures correctness of all major features:
- eval_contradiction_detection: Old fact invalidated when new fact contradicts
- eval_temporal_query: Correct time-slice returned by /context/temporal
- eval_multi_hop_retrieval: 2-hop graph traversal works
- eval_entity_resolution: Duplicate entity mentions deduplicated
- eval_reflect_gap_analysis: Knowledge gaps correctly identified
- eval_timeline_history: Supersession chain visible in /timeline
- eval_confidence_compounding: Multi-source Bayesian update works

### Scenario tests

Integration scenarios that test real-world memory patterns:
- `scenario_career_journey` — Multi-phase career with salary contradictions + temporal queries
- `scenario_medical_knowledge` — Patient record updates with medication contradictions + multi-hop
- `scenario_multi_agent_knowledge` — 3 agents collaborate, confidence compounds, sources tracked

## Configuration

| Env var | Default | Description |
|---|---|---|
| `PORT` | `21693` | HTTP listen port |
| `GRAPH_BACKEND` | `falkordb` | Graph backend: `falkordb` or `memory` |
| `FALKORDB_URL` | `redis://localhost:6379` | FalkorDB connection (ignored when `GRAPH_BACKEND=memory`) |
| `ANTHROPIC_API_KEY` | (required) | Or use `ANTHROPIC_OAUTH_TOKEN` |
| `ANTHROPIC_MODEL` | `claude-haiku-4-5-20251001` | Model for all LLM calls |
| `OLLAMA_URL` | `http://localhost:11434` | Ollama for embeddings (nomic-embed-text) |
| `GRAPH_NAME` | `hippo` | Graph name in FalkorDB |
| `MAINTENANCE_INTERVAL_SECS` | `10` | Background maintenance frequency |
| `MOCK_LLM` | `0` | Set to `1` to use heuristic mocks instead of real LLM calls |
| `RUST_LOG` | `hippo=info` | Log level filter |

## Eval Regression Tracking

Results are stored in `~/.hippo-evals/` with git SHA. Each run compares against the previous run.
