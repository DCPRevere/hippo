# Evals

Benchmark harnesses for evaluating hippo against standard memory benchmarks.

## Setup

```bash
pip install -r evals/requirements.txt
```

Start hippo with a real LLM (not mock):

```bash
GRAPH_BACKEND=memory PORT=21693 ./target/release/hippo
```

## LoCoMo

Tests long-term conversational memory. 10 conversations, ~6K turns, ~2K questions.

```bash
# Download dataset
curl -Lo evals/locomo/locomo10.json \
  https://raw.githubusercontent.com/snap-research/LoCoMo/main/data/locomo10.json

# Run
python evals/locomo/run.py
```

## LongMemEval

Tests multi-session reasoning, temporal reasoning, knowledge updates. 500 questions.

```bash
# Download dataset (oracle = smallest, tests QA in isolation)
curl -Lo evals/longmemeval/longmemeval_oracle.json \
  https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_oracle.json

# Run
python evals/longmemeval/run.py

# With GPT-4o judge (recommended for accurate scoring)
OPENAI_API_KEY=... python evals/longmemeval/run.py

# Larger variant with distractor sessions
python evals/longmemeval/run.py --variant s
```

## Scoring

LoCoMo uses token-level F1 (computed locally). LongMemEval uses GPT-4o as judge (falls back to substring matching without OPENAI_API_KEY).
