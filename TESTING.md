# Testing

## Fast (no LLM, no network, no Docker)

Uses in-memory graph and heuristic mock extraction. No FalkorDB or API keys needed.

```bash
cargo test --test unit_remember --test unit_context --test unit_maintain --test retrieval --test maintenance --test memory_creation --test integration_test -- --test-threads=2
```

By default, integration tests use `GRAPH_BACKEND=memory` and `MOCK_LLM=1`. To run against FalkorDB instead, set `GRAPH_BACKEND=falkordb` (requires Docker with FalkorDB running).

## Full evals with fixtures (no LLM key needed)

Replays pre-recorded LLM responses from `fixtures/llm-responses.json`. Deterministic and fast.

```bash
EVAL_REPLAY=1 FIXTURE_PATH=./fixtures/llm-responses.json cargo test --test evals -- --test-threads=1
```

## Full evals with real LLM

Requires an Anthropic API key or OAuth token.

```bash
ANTHROPIC_OAUTH_TOKEN=<key> cargo test --test evals -- --test-threads=1
```

## Record new fixtures

Runs tests against the real LLM and saves request/response pairs to the fixture file. Merges with existing fixtures (does not overwrite).

```bash
ANTHROPIC_OAUTH_TOKEN=<key> EVAL_RECORD=1 FIXTURE_PATH=./fixtures/llm-responses.json \
  cargo test --test evals -- --test-threads=1
```

## Environment variables

| Variable | Description |
|---|---|
| `GRAPH_BACKEND=falkordb` | Force integration tests to use FalkorDB (default: `memory`) |
| `MOCK_LLM=1` | Use heuristic extraction, no LLM calls |
| `EVAL_RECORD=1` | Call real LLM and save responses to fixture file |
| `EVAL_REPLAY=1` | Replay responses from fixture file, no LLM calls |
| `FIXTURE_PATH` | Path to fixture JSON file (default: `./fixtures/llm-responses.json`) |
| `ALLOW_ADMIN=1` | Enable admin endpoints (required for seeding in tests) |
| `ANTHROPIC_OAUTH_TOKEN` | Anthropic API credential |
