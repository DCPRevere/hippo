# Configuration

Hippo reads configuration from three sources, in order of precedence:

1. Environment variables (highest precedence)
2. The TOML file at `HIPPO_CONFIG` (default: `hippo.toml` in the working directory)
3. Compiled defaults

Environment variables and TOML values cover the same surface; pick whichever fits the deployment shape. CLI flags are not used.

## Required for production

These have no safe default and must be set before deploying.

| Variable | Notes |
|---|---|
| `ANTHROPIC_OAUTH_TOKEN` *or* `ANTHROPIC_API_KEY` | Required for Anthropic LLM calls. Use OAuth for Claude.ai-issued tokens. |
| `OPENAI_API_KEY` | Required if `LLM_PROVIDER=openai`. |
| `HIPPO_AUTH=true` | Required in production. Without this, requests are not authenticated. |
| `HIPPO_TLS=true` plus `HIPPO_TLS_CERT` / `HIPPO_TLS_KEY` | Required if hippo terminates TLS rather than running behind a TLS-aware proxy. |
| `HIPPO_RATE_LIMIT=true` plus `HIPPO_RPM` | Strongly recommended. Default `HIPPO_RPM` is 60 per user when enabled. |

## Storage backend

| Variable | Default | Notes |
|---|---|---|
| `GRAPH_BACKEND` | `memory` | One of `memory` / `sqlite` / `postgres` / `qdrant` / `falkordb`. See [backend matrix](#backend-readiness-matrix). |
| `GRAPH_NAME` | `hippo` | Used to namespace data in Postgres / Qdrant / FalkorDB. |
| `SQLITE_PATH` | `./hippo.sqlite` | Path to the SQLite file. Persists across restarts. |
| `POSTGRES_URL` | (none) | Required when `GRAPH_BACKEND=postgres`. e.g. `postgres://user:pass@host:5432/hippo`. |
| `QDRANT_URL` | (none) | Required when `GRAPH_BACKEND=qdrant`. |
| `FALKORDB_URL` | `redis://localhost:6379` | Used when `GRAPH_BACKEND=falkordb`. |

## LLM

| Variable | Default | Notes |
|---|---|---|
| `LLM_PROVIDER` | `anthropic` | `anthropic` / `openai` / `ollama`. |
| `ANTHROPIC_MODEL` | `claude-sonnet-4-5` | Model name. |
| `OPENAI_MODEL` | `gpt-5.4` | Used when `LLM_PROVIDER=openai`. |
| `OPENAI_EMBEDDING_MODEL` | `text-embedding-3-small` | |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | Override for OpenAI-compatible endpoints. |
| `OLLAMA_URL` | `http://localhost:11434` | When `LLM_PROVIDER=ollama`. |
| `LLM_MAX_TOKENS` | `4096` | Per-request output cap. |
| `EXTRACTION_PROMPT` | (built-in) | Replace the default extraction prompt. Leave unset unless you know what you're doing. |
| `MOCK_LLM` | `false` | Test-only. Returns deterministic stub responses without hitting any provider. |

## Pipeline behaviour

| Variable | Default | Notes |
|---|---|---|
| `INFER_PRE_CONTEXT` | `true` | Whether `remember` gathers context before extraction. |
| `INFER_ENRICHMENT` | `true` | Whether `remember` revises operations after enrichment. |
| `INFER_MAINTENANCE` | `true` | Whether the Inferrer Dreamer runs during maintenance. |
| `MAINTENANCE_INTERVAL_SECS` | `60` | How often the maintenance loop runs. `0` disables the loop (manual `/maintain` only). |
| `DEFAULT_CONTEXT_LIMIT` | `10` | Default `limit` on `/context`. |
| `DEFAULT_TTL_SECS` | (none) | Default TTL on every fact. Unset = no expiry by default. |

## Retrieval scoring

| Variable | Default | Notes |
|---|---|---|
| `SCORING_W_RELEVANCE` | `0.50` | Weight on cosine similarity. |
| `SCORING_W_CONFIDENCE` | `0.10` | Weight on stored confidence. |
| `SCORING_W_RECENCY` | `0.25` | Weight on recency. |
| `SCORING_W_SALIENCE` | `0.15` | Weight on salience-on-use. |
| `SCORING_MMR_LAMBDA` | `0.70` | MMR diversity vs. relevance trade-off. |

## Auth, rate limiting, TLS

| Variable | Default | Notes |
|---|---|---|
| `HIPPO_AUTH` | `false` | When `true`, all endpoints require an API key. **Set this in production.** |
| `HIPPO_INSECURE` | `false` | Test-only override that disables auth even when `HIPPO_AUTH=true`. **Never set in production.** |
| `ALLOW_ADMIN` | `false` | Test-only: treats every request as admin. **Never set in production.** |
| `HIPPO_RATE_LIMIT` | `false` | Enable per-user rate limiting. |
| `HIPPO_RPM` | `60` | Requests-per-minute when rate limiting is on. |
| `HIPPO_TLS` | `false` | Enable TLS termination. |
| `HIPPO_TLS_CERT` / `HIPPO_TLS_KEY` | (none) | Required when `HIPPO_TLS=true`. |

## Eval / fixtures (development only)

| Variable | Default | Notes |
|---|---|---|
| `EVAL_RECORD` | unset | Record LLM responses to fixtures. |
| `EVAL_REPLAY` | unset | Replay fixtures instead of calling the LLM. |
| `FIXTURE_PATH` | `./fixtures` | Where fixtures live. |

## Network

| Variable | Default | Notes |
|---|---|---|
| `PORT` | `21693` | HTTP listen port. |
| `HIPPO_CONFIG` | `hippo.toml` | Override the TOML config path. |

## Backend readiness matrix

The Dreamer architecture (continuous background processing — see `docs/DREAMS.md`) requires support for `mark_visited`, `last_visited`, `bump_salience`, `supersede_edge`, `retract_edge` from the storage backend. Status as of the current release:

| Backend | Status | Caveats |
|---|---|---|
| **SQLite** | ✅ Full | Reference backend with full Dreamer parity tests. Recommended for single-node deployments. |
| **Postgres** | ✅ Full | All Dreamer methods implemented. Recommended for multi-node / managed-cloud deployments. |
| **Falkor (Cypher)** | ⚠️ Partial | Dreamer methods implemented via Cypher; parity tests are not yet automated. Validate behaviour in your environment before relying on it. |
| **Qdrant** | ⚠️ Limited | `bump_salience` and `retract_edge` work. `supersede_edge`, `mark_visited`, `last_visited` are no-ops with a tracing warning. The Dreamer revisit-window filter does not function on Qdrant — every pass re-processes the same entities. **Not recommended for production Dreamer use.** |
| **In-memory** | ✅ Full | Lost on process restart. Test-only. |

## Cost ceilings — strongly recommended in production

The Dreamer makes background LLM calls. Cap them via:

```toml
[pipeline.tuning]
dreamer_worker_count = 1            # parallel workers per pass
dreamer_max_units = 100             # max entities processed per pass
dreamer_max_tokens = 50000          # hard ceiling on tokens per pass
```

Equivalent env vars are not yet wired (see [Known gaps](#known-gaps)). Use TOML for these settings in production.

## Known gaps

Items that are documented but not yet supported:

- TOML-only (no env var equivalents): the `dreamer_*` budget fields and most `[pipeline.tuning]` thresholds. Set them in `hippo.toml`.
- Qdrant lacks parts of the Dreamer surface; see the matrix above.
- Hot-reload: changing `hippo.toml` requires a restart. Env-var changes always do.

## Quick prod template

Minimal `hippo.toml` for a Postgres-backed production deployment:

```toml
[graph]
backend = "postgres"
name = "hippo_prod"

[graph.postgres]
url = "postgres://hippo:secret@db.internal:5432/hippo"

[llm]
provider = "anthropic"
max_tokens = 4096

[auth]
enabled = true

[rate_limit]
enabled = true
requests_per_minute = 60

[pipeline]
maintenance_interval_secs = 60

[pipeline.tuning]
dreamer_worker_count = 2
dreamer_max_units = 200
dreamer_max_tokens = 100000
```

Plus environment:

```
ANTHROPIC_OAUTH_TOKEN=...
HIPPO_AUTH=true
HIPPO_RATE_LIMIT=true
```

## Production-readiness checklist

- [ ] Backend = SQLite or Postgres (not Qdrant for Dreamer).
- [ ] `HIPPO_AUTH=true`, `HIPPO_INSECURE` and `ALLOW_ADMIN` unset.
- [ ] `HIPPO_RATE_LIMIT=true` with a sane `HIPPO_RPM`.
- [ ] TLS enabled either at hippo (`HIPPO_TLS`) or at a fronting proxy.
- [ ] Dreamer cost ceilings set in `hippo.toml`.
- [ ] LLM credentials provided via the appropriate `*_API_KEY` / `*_OAUTH_TOKEN`.
- [ ] Container runs as non-root (the bundled Dockerfile already does this).
- [ ] Schema migrations run before first start (Postgres `CREATE TABLE IF NOT EXISTS` is additive but verify by running `setup_schema()` manually if you've upgraded across a major version).
