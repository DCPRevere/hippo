# Changelog

## [Unreleased]

### Added (2026-03-26/27 — overnight SOTA push)

#### Core Memory Features
- **Temporal queries** (`POST /context/temporal`, `GET /timeline/:entity`) — query the knowledge graph as it existed at any timestamp
- **Confidence decay** — Ebbinghaus-inspired: facts lose confidence at 0.5%/day after 30 days inactive; salience decays 1/week if not accessed
- **Salience tracking** — accessed facts gain salience; most-accessed facts prioritised in retrieval
- **Contradiction provenance** — when facts contradict, old fact preserved as superseded (not deleted); `GET /provenance/:edge_id`
- **Multi-hop traversal** — N-hop graph walk with 0.6× relevance decay per hop; blended with vector search
- **Bayesian confidence compounding** — multiple agents asserting same fact compounds confidence: `1 - (1-c1)(1-c2)`
- **Working/Long-term memory tiers** — new facts start as Working, promoted to Long-term when salience≥3 or age>1h+confidence>0.7; stale Working facts purged after 24h
- **Source credibility tracking** — per-agent fact count, contradiction rate, credibility score; `GET /sources`

#### Retrieval Enhancements
- **Structured context** (`POST /context/structured`) — returns current_beliefs, recently_changed, uncertain, entity_summaries
- **Introspective reflection** (`POST /reflect`) — gap analysis, suggested questions via LLM
- **Memory consolidation** (`POST /consolidate`) — link discovery, fact archiving, BFS entity clustering
- **Smart query router** (`POST /query`) — classifies intent and routes to context/temporal/reflect/timeline
- **Entity timeline** (`GET /timeline/:entity`) — full chronological fact history including superseded facts

#### Ingestion Improvements
- **Batch edge classification** — N serial LLM calls → 1 batch call per fact ingestion
- **Vector pre-filter** — cosine similarity pre-filter before LLM classification (threshold 0.3)
- **SSE streaming** (`POST /remember/stream`) — real-time progress events during ingestion
- **Batch ingest** (`POST /remember/batch`) — parallel or sequential multi-statement ingestion

#### Observability & Integration
- **Prometheus metrics** (`GET /metrics`) — counters for remember calls, facts written, contradictions, context queries
- **MCP server** (`mcp-server` binary) — Model Context Protocol integration for Claude Desktop
- **Benchmark binary** (`benchmark`) — 50-fact corpus, recall@K, p50/p95/p99 latency measurements

#### Eval Infrastructure
- **7-eval correctness suite** (`tests/evals.rs`) — contradiction detection, temporal query, multi-hop, entity resolution, reflect, timeline, confidence compounding
- **Scenario-based integration evals** (`tests/scenarios.rs`) — career journey, medical record, multi-agent knowledge scenarios
- **eval-score binary** — runs all evals with partial scoring (0.0–1.0 per eval)
- **eval-regression binary** — stores scores in `~/.hippo-evals/` with git SHA; reports diffs vs previous run
- **Mock LLM mode** (`MOCK_LLM=1`) — heuristic extraction, no API calls, for CI smoke tests
- **Fixture record/replay** (`EVAL_RECORD=1` / `EVAL_REPLAY=1`) — deterministic evals via SHA-256 hashed request→response fixtures

#### Architecture Improvements
- **Per-test graph isolation** — each test gets a uniquely named FalkorDB graph; parallel test runs safe
- **Dynamic port allocation** — tests bind random free ports; 4-way parallel test runs supported
- **Shared test helpers** (`tests/helpers/mod.rs`) — eliminates duplication between test files
- **Fact archiving** — low-confidence facts soft-deleted (archived=true) not hard-deleted; queryable via provenance

#### Bug Fixes / Security
- **SQL injection fixes** — `sanitise()` helper applied consistently across graph queries
- **checked_pairs cap** — unbounded HashSet capped at 10,000 with LRU eviction
- **Pseudo-embedding visibility** — WARN-level log with text preview when Ollama is unavailable
- **Mutex bottleneck documentation** — TODO comment for future connection pool work
- **.gitignore added** — target/ and fixtures/*.json excluded

---

## [0.1.0] — initial version (pre-push)
- Basic remember/context/diagnose pipeline
- FalkorDB graph + vector + fulltext retrieval
- Entity extraction and resolution via LLM
- Background maintenance loop (link discovery, placeholder resolution, contradiction scan)
