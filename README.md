<pre align="center">
░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░▒▓███████▓▒░░▒▓███████▓▒░ ░▒▓██████▓▒░
░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░
░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░
░▒▓████████▓▒░▒▓█▓▒░▒▓███████▓▒░░▒▓███████▓▒░░▒▓█▓▒░░▒▓█▓▒░
░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░▒▓█▓▒░      ░▒▓█▓▒░      ░▒▓█▓▒░░▒▓█▓▒░
░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░▒▓█▓▒░      ░▒▓█▓▒░      ░▒▓█▓▒░░▒▓█▓▒░
░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░▒▓█▓▒░      ░▒▓█▓▒░       ░▒▓██████▓▒░
</pre>

<p align="center">
  <strong>🧠 The memory layer that dreams.</strong><br>
  A self-improving memory graph for AI agents. Runs on your server, in your browser, or anywhere in between.
</p>

<p align="center">
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-edition%202021-orange?logo=rust" alt="Rust edition 2021" /></a>
  <a href="https://github.com/dcprevere/hippo"><img src="https://img.shields.io/badge/hippo-v0.1.0-blue" alt="hippo v0.1.0" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="MIT licence" /></a>
</p>

---

## What it is

Hippo is a memory database for AI agents. It extracts entities and relationships from natural language, stores them in a typed graph, and **continuously processes itself between conversations** — finding new connections, resolving contradictions, learning which sources to trust.

Most memory layers are passive: write once, read forever. Hippo is active. In the morning, the graph is genuinely better than you left it the night before — without you doing anything.

```
        write something                         ┌──────────────┐
              │                                 │   Dreamer    │
              ▼                                 │  (background)│
       ┌─────────────┐                          │              │
       │  /remember  │                          │  • Linker    │
       └──────┬──────┘                          │  • Inferrer  │
              │                                 │  • Reconciler│
              ▼                                 │  • Consolid. │
       ┌─────────────┐ ◄────── reads ────────── └──────┬───────┘
       │  the graph  │ ─────── writes ─────────────────┘
       └──────┬──────┘
              │
              ▼
       ┌─────────────┐
       │    /ask     │   ranking by salience + credibility,
       │             │   filtered by supersession
       └─────────────┘
```

## Why hippo

**It dreams.** Between conversations, the Dreamer walks the graph and takes append-only actions: discovers links between unconnected entities, infers implied facts from existing structure, writes `supersedes` relationships when sources disagree, consolidates episodic facts into semantic patterns. See [`docs/DREAMS.md`](docs/DREAMS.md).

**Append-only by design.** The Dreamer never deletes. Contradictions become `supersedes` edges with full provenance; the original fact stays queryable for audit. Users and agents can still explicitly `retract` or `correct` when something is genuinely wrong.

**Sources are weighted.** Hippo tracks each source's accuracy across contradictions and weights future facts accordingly. Trusted CRMs outrank casual chat.

**Salience compounds.** Every retrieval bumps the salience of the edges it surfaces. The facts you actually use rank higher next time.

**Runs anywhere.** Native server, embedded SQLite, Postgres, FalkorDB, or compiled to WebAssembly for in-browser memory that never leaves the device.

## Architecture in one diagram

```
                  ┌───────────────────────────────────────────┐
                  │              GraphBackend                 │
                  │  (trait — implemented by 5 backends)      │
                  ├───────────────┬───────────┬───────────────┤
                  │ InMemory      │ SQLite    │ Postgres      │
                  │ (test, WASM)  │ (default) │ (multi-node)  │
                  ├───────────────┼───────────┼───────────────┤
                  │ FalkorDB (Cypher)         │ Qdrant (vec)  │
                  └───────────────────────────┴───────────────┘
                              ▲
                              │
       ┌──────────────────────┼──────────────────────┐
       │                      │                      │
   pipeline::             pipeline::             pipeline::
   remember               ask                    dreamer
   (write)                (read)                 (background)
       │                      │                      │
       │                      ▼                      ▼
       │              iterative LLM loop      WorkerPool drives
       │              over the subgraph       Linker/Inferrer/
       │                                      Reconciler/Consolid.
       ▼
   plan → enrich → execute
   (LLM extracts ops; graph
    enriches; ops applied)
```

## Quick start

### As a server

```sh
# 1. Set credentials
export ANTHROPIC_OAUTH_TOKEN=...      # or ANTHROPIC_API_KEY

# 2. Choose a backend (default: in-memory)
export GRAPH_BACKEND=sqlite
export SQLITE_PATH=./hippo.sqlite

# 3. Enable production safety
export HIPPO_AUTH=true
export HIPPO_RATE_LIMIT=true

# 4. Run
cargo run --release --bin hippo
```

### Using the TypeScript SDK

```ts
import { HippoClient } from "@dcprevere/hippo-sdk";

const hippo = new HippoClient({
  baseUrl: "http://localhost:21693",
  apiKey: process.env.HIPPO_API_KEY,
});

// Write
await hippo.observe({ statement: "Alice works at Acme as a lawyer" });

// Read
const { answer } = await hippo.recall({ question: "Where does Alice work?" });

// Trigger one dream pass and inspect what changed
const report = await hippo.dream();
console.log(report); // { facts_visited, links_written, supersessions_written, ... }

// Explicitly correct an error
await hippo.correct({
  edge_id: 42,
  statement: "Alice works at Acme as a doctor",
  reason: "extraction error — original transcript said doctor",
});
```

### In the browser (WASM)

```ts
import init, { Hippo } from "@dcprevere/hippo-wasm";

await init();
const hippo = new Hippo(JSON.stringify({
  api_key: localStorage.getItem("openai_key"),
  model: "gpt-5.4",
}));

await hippo.remember("I prefer cycling to driving");
const answer = await hippo.ask("How do I get around?");
```

## API surface

### Core verbs

| Verb | HTTP | What it does |
|---|---|---|
| `observe` / `remember` | `POST /remember` | Extract entities and facts from a natural-language statement; resolve against existing entities; write append-only edges. |
| `recall` / `ask` | `POST /ask` | Iteratively gather context from the graph, return an LLM answer plus the supporting facts. |
| `context` | `POST /context` | Raw subgraph for a query, no LLM synthesis. |
| `dream` | `POST /maintain` | Trigger one dream pass; returns a `DreamReport` with counts. Runs continuously in the background by default. |
| `retract` | `POST /retract` | Explicit user/agent destructive removal of an edge with audit reason. Distinct from autonomous supersession. |
| `correct` | `POST /correct` | Convenience: `retract` + `observe` in one call. |

### Auxiliary

| Verb | HTTP | What it does |
|---|---|---|
| Provenance | `GET /edges/:id/provenance` | Supersession chain for an edge — what it replaced, what replaced it. |
| Graph dump | `GET /graph` | Full graph (entities + active edges + invalidated edges). |
| Events | `GET /events` | Server-sent events stream of graph mutations. |
| Health | `GET /health` | Liveness check. |
| Metrics | `GET /metrics` | Prometheus exposition. |

Full schemas: see [`docs/openapi.yaml`](docs/openapi.yaml).

## Backend support

| Backend | Status | Best for | Dreamer support |
|---|---|---|---|
| **SQLite** | ✅ Stable | Single-node, embedded, dev | Full (reference impl + parity tests) |
| **Postgres** | ✅ Stable | Multi-node, managed cloud | Full (additive `CREATE TABLE IF NOT EXISTS` migrations) |
| **In-memory** | ✅ Stable | Tests, WASM | Full |
| **FalkorDB** | ⚠️ Experimental | Cypher graph queries | Implemented; parity tests not yet automated |
| **Qdrant** | ⚠️ Limited | Vector-first deployments | Partial (revisit-window is a no-op) — **not recommended for production Dreamer use** |

See [`docs/CONFIG.md#backend-readiness-matrix`](docs/CONFIG.md) for details.

## Distinctive features

These are the things hippo does that competing memory layers (Mem0, Zep, Supermemory, Letta) don't, or don't do as well:

- **Continuous background processing.** The Dreamer runs between conversations, not just on writes. It finds links you didn't ask for, infers facts implied by structure, and resolves contradictions with delayed evidence.
- **Append-only contradiction handling.** When two facts disagree, hippo writes a `supersedes` edge tagged with source credibility. Both originals stay in the graph; retrieval filters by supersession at read time. Full audit trail by construction.
- **Source credibility that compounds.** Each source's accuracy is tracked across contradictions and fed back into ranking. Sources that have been wrong before get less weight on future facts.
- **Iterative read path.** `/ask` doesn't retrieve once and synthesise. It asks the LLM what's missing, fetches more, and loops — closer to how thinking actually works.
- **WASM-native.** The same Rust core that runs the server compiles to `wasm32-unknown-unknown` and runs in the browser. Your memory never has to leave the device.
- **Retry on transient LLM failures.** Built-in jittered exponential backoff on 429 / 5xx / connection errors.

## Comparison

|                                       | **Hippo**       | Mem0 v3       | Zep / Graphiti  | Supermemory      | Letta           |
|---------------------------------------|-----------------|---------------|-----------------|------------------|-----------------|
| **Data model**                        | Typed graph + supersession edges | Vector + entity sidecar | Temporal knowledge graph | Atomic memories with `Updates` edges | Stateful agent OS |
| **Contradiction handling**            | Background, append-only `supersedes` | None (ADD-only) | Write-time, sets `invalid_at` | Write-time, flips `isLatest` | App-defined |
| **Background graph processing**       | ✅ Linker / Inferrer / Reconciler / Consolidator | ❌ | ⚠️ community detection only | ⚠️ claimed `Derives`, unverifiable | ❌ |
| **Inference of new edges**            | ✅ from existing structure | ❌ | ❌ | ⚠️ documented but opaque | ❌ |
| **Source-credibility weighting**      | ✅ compounds across contradictions | ❌ | ❌ | ❌ | ❌ |
| **Salience-on-use ranking**           | ✅ retrievals bump salience | ❌ | ❌ | ❌ | ❌ |
| **Append-only by default**            | ✅ user `retract` is the only escape valve | n/a (no contradictions) | ❌ mutates `invalid_at` | ❌ flips `isLatest` flag | ❌ |
| **Embedded / in-process**             | ✅ Rust crate, no server needed | ❌ Python service | ❌ JVM/Go service | ❌ hosted only | ❌ Python service |
| **Runs in the browser (WASM)**        | ✅ first-class | ❌ | ❌ | ❌ | ❌ |
| **Retry on 429/5xx**                  | ✅ jittered exponential backoff | ❌ | ❌ | ❌ | ❌ |
| **Iterative read with LLM-requested context** | ✅ | ❌ retrieve-then-answer | ❌ | ❌ | ⚠️ via agent loop |
| **Open source**                       | ✅ MIT | ✅ Apache 2 | ✅ (Graphiti) | ❌ closed core | ✅ Apache 2 |

Two notes on this table:

- "Mem0 v3" reflects the April 2026 release, which deliberately removed the v2 graph backend in favour of a pure vector + entity-sidecar model. Earlier versions had different shape.
- Supermemory's `Derives` (claimed background inference) and `Automatic Forgetting` are documented but not in any open source we could verify — the ⚠️ reflects that uncertainty, not malice.

## Configuration

Environment variables and `hippo.toml` cover the same surface. See [`docs/CONFIG.md`](docs/CONFIG.md) for the full matrix.

Quick prod template:

```toml
[graph]
backend = "postgres"
name = "hippo_prod"

[graph.postgres]
url = "postgres://hippo:secret@db.internal:5432/hippo"

[auth]
enabled = true

[rate_limit]
enabled = true
requests_per_minute = 60

[pipeline.tuning]
dreamer_worker_count = 2
dreamer_max_units = 200
dreamer_max_tokens = 100000
```

## Production checklist

- [ ] Backend is SQLite or Postgres (not Qdrant for Dreamer use).
- [ ] `HIPPO_AUTH=true`; `HIPPO_INSECURE` and `ALLOW_ADMIN` are unset.
- [ ] `HIPPO_RATE_LIMIT=true` with a sensible `HIPPO_RPM`.
- [ ] TLS terminated either by hippo (`HIPPO_TLS=true`) or a fronting proxy.
- [ ] Dreamer cost ceilings set in `hippo.toml`.
- [ ] LLM credentials set via `*_OAUTH_TOKEN` or `*_API_KEY`.
- [ ] Container runs as non-root (the bundled Dockerfile already does this).

## Project layout

```
hippo/
├── src/
│   ├── backends/           # GraphBackend impls (in_memory, sqlite, postgres, qdrant, falkor)
│   ├── pipeline/
│   │   ├── ask.rs          # iterative read path
│   │   ├── remember.rs     # plan → enrich → execute
│   │   ├── maintain.rs     # housekeeping + drives the Dreamer
│   │   └── dreamer/        # Dreamer trait, WorkerPool, Linker/Inferrer/Reconciler/Consolidator
│   ├── llm/                # LlmClient + RetryingLlm decorator
│   ├── http/               # axum router + handlers
│   └── ...
├── hippo-api/              # shared request/response types (used by SDKs and server)
├── hippo-wasm/             # wasm-bindgen wrapper for in-browser use
├── sdks/typescript/        # @dcprevere/hippo-sdk
├── docs/
│   ├── DREAMS.md           # the Dreamer architecture
│   └── CONFIG.md           # full config reference + backend matrix
└── tests/                  # 267 unit + contract + idempotency tests
```

## Building & testing

```sh
# Native build + unit tests
cargo build --release
cargo test --tests              # 267 tests, ~3s, no network needed

# WASM build
cargo check --target wasm32-unknown-unknown --manifest-path hippo-wasm/Cargo.toml

# Lints (CI runs these with -D warnings)
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Integration / eval tests (require LLM credentials, gated with #[ignore])
cargo test --tests -- --ignored
```

CI runs unit tests + clippy + fmt on every PR; the eval suite runs nightly via [`.github/workflows/eval.yml`](.github/workflows/eval.yml).

## Documentation

- [`docs/DREAMS.md`](docs/DREAMS.md) — the Dreamer architecture, design decisions, and shortlist of must-have features.
- [`docs/CONFIG.md`](docs/CONFIG.md) — full env-var matrix, backend readiness, production checklist.
- [`docs/openapi.yaml`](docs/openapi.yaml) — HTTP API spec.

## License

MIT.
