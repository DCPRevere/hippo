# zeroclaw-hippo

ZeroClaw memory backend and tool implementations backed by a [Hippo](https://github.com/dcprevere/hippo) knowledge-graph instance.

## What it does

This crate provides:

- **`HippoMemory`** -- implements ZeroClaw's `Memory` trait so the agent runtime can store and retrieve memories through Hippo's natural-language knowledge graph.
- **Three tools** (`hippo_remember`, `hippo_recall`, `hippo_ask`) -- implement ZeroClaw's `Tool` trait, giving agents the ability to interact with Hippo directly during task execution.

## Integration

Add the crate to your ZeroClaw agent:

```toml
[dependencies]
zeroclaw-hippo = { path = "../plugins/zeroclaw" }
```

Register the memory backend and tools with your agent factory:

```rust
use zeroclaw_hippo::{HippoConfig, create_memory, create_tools};

let config = HippoConfig::from_env();
let memory = create_memory(&config)?;
let tools  = create_tools(&config)?;

// Pass `memory` as the agent's Memory implementation.
// Register each tool with the agent's tool registry.
```

> **Note:** The ZeroClaw traits (`Memory`, `Tool`) are defined locally in this crate to avoid a direct dependency on the ZeroClaw runtime. The signatures match the canonical ZeroClaw traits -- verify compatibility with your ZeroClaw version.

## Tools

| Tool | Description |
|---|---|
| `hippo_remember` | Store a fact or statement in the knowledge graph. |
| `hippo_recall` | Search the knowledge graph for relevant facts. |
| `hippo_ask` | Ask a natural-language question answered by the knowledge graph. |

## Memory backend mapping

| ZeroClaw method | Hippo API | Notes |
|---|---|---|
| `store` | `POST /remember` | Key, category, and session are encoded in the statement text. |
| `recall` | `POST /context` | Semantic search; results mapped to `MemoryEntry`. |
| `get` | `POST /context` (limit=1) | Searches by key. |
| `list` | `POST /context` | Filters by category client-side. |
| `forget` | -- | Not supported (returns `false`). Hippo handles invalidation via contradiction detection. |
| `count` | `GET /graph` | Returns entity + edge count. |
| `health_check` | `GET /health` | `true` when status is "ok". |
| `reindex` | -- | No-op (Hippo indexes automatically). |

## Configuration

Set these environment variables, or construct `HippoConfig` directly:

| Variable | Default | Description |
|---|---|---|
| `HIPPO_URL` | `http://localhost:21693` | Hippo instance base URL. |
| `HIPPO_API_KEY` | (none) | Bearer token for authenticated instances. |
| `HIPPO_GRAPH` | (none) | Graph namespace to target. |
