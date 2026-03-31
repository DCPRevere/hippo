# LLM Response Fixtures

These fixtures capture real LLM responses for deterministic eval replay.

## Generating fixtures
EVAL_RECORD=1 cargo test eval_ -- --test-threads=1 --nocapture

## Running evals with fixtures (fast, free, deterministic)
EVAL_REPLAY=1 cargo test eval_ -- --test-threads=4 --nocapture
