#!/usr/bin/env python3
"""Run LoCoMo benchmark against a hippo instance.

Usage:
    # Start hippo with a real LLM:
    GRAPH_BACKEND=memory PORT=21693 ./target/release/hippo

    # Download LoCoMo data:
    curl -Lo evals/locomo/locomo10.json \
      https://raw.githubusercontent.com/snap-research/LoCoMo/main/data/locomo10.json

    # Run:
    python evals/locomo/run.py

    # Or against a remote instance:
    HIPPO_URL=https://cloud.hippoai.dev HIPPO_API_KEY=... python evals/locomo/run.py

Environment:
    HIPPO_URL       Hippo base URL (default: http://localhost:21693)
    HIPPO_API_KEY   API key (optional for local)
"""

from __future__ import annotations

import json
import os
import sys
import time
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from hippo import HippoClient

from scoring import adversarial_score, multihop_f1, token_f1

DATA_FILE = Path(__file__).parent / "locomo10.json"

CATEGORY_NAMES = {
    1: "multi-hop",
    2: "temporal",
    3: "open-domain",
    4: "single-hop",
    5: "adversarial",
}


def format_conversation(conv: dict) -> list[str]:
    """Convert a LoCoMo conversation into a list of statements for /remember."""
    statements = []
    speaker_a = conv.get("speaker_a", "Speaker A")
    speaker_b = conv.get("speaker_b", "Speaker B")

    session_idx = 1
    while True:
        key = f"session_{session_idx}"
        if key not in conv:
            break
        date_key = f"{key}_date_time"
        date_str = conv.get(date_key, "")
        turns = conv[key]

        # Build session text as a block of dialogue.
        lines = []
        if date_str:
            lines.append(f"[{date_str}]")
        for turn in turns:
            speaker = turn.get("speaker", "?")
            text = turn.get("text", "")
            lines.append(f"{speaker}: {text}")

        if lines:
            session_text = "\n".join(lines)
            statements.append(session_text)
        session_idx += 1

    return statements


def ingest_conversation(client: HippoClient, conv: dict, graph: str) -> None:
    """Ingest a full conversation into hippo via /remember."""
    statements = format_conversation(conv)
    print(f"  Ingesting {len(statements)} sessions into graph '{graph}'...")

    for i, stmt in enumerate(statements):
        try:
            client.remember(stmt, graph=graph, timeout=120.0)
        except Exception as e:
            print(f"  Warning: session {i+1} failed: {e}")

    print(f"  Ingestion complete.")


def ask_question(client: HippoClient, question: str, graph: str) -> str:
    """Ask a question via /ask and return the answer string."""
    try:
        resp = client.ask(
            question,
            graph=graph,
            limit=50,
            timeout=60.0,
        )
        return resp.answer
    except Exception as e:
        return f"Error: {e}"


def score_answer(prediction: str, qa: dict) -> float:
    """Score a single QA pair."""
    category = qa.get("category", 4)
    if category == 5:
        return adversarial_score(prediction)
    answer = str(qa.get("answer", ""))
    if category == 1:
        return multihop_f1(prediction, answer)
    return token_f1(prediction, answer)


def main():
    if not DATA_FILE.exists():
        print(f"Error: {DATA_FILE} not found.")
        print("Download it with:")
        print("  curl -Lo evals/locomo/locomo10.json \\")
        print(
            "    https://raw.githubusercontent.com/snap-research/LoCoMo/main/data/locomo10.json"
        )
        sys.exit(1)

    client = HippoClient(
        base_url=os.environ.get("HIPPO_URL", "http://localhost:21693/api"),
        api_key=os.environ.get("HIPPO_API_KEY"),
        timeout=120.0,
    )

    # Health check (health endpoint is at root, not /api)
    import httpx

    health_url = client.base_url.replace("/api", "") + "/health"
    try:
        r = httpx.get(health_url, timeout=5.0)
        r.raise_for_status()
    except Exception as e:
        print(f"Error: cannot connect to hippo at {health_url}: {e}")
        sys.exit(1)

    data = json.loads(DATA_FILE.read_text())
    print(f"Loaded {len(data)} conversations from LoCoMo")

    all_results = []
    category_scores: dict[int, list[float]] = defaultdict(list)

    for conv_idx, conversation in enumerate(data):
        sample_id = conversation.get("sample_id", str(conv_idx))
        graph = f"locomo_{sample_id}"
        print(f"\nConversation {conv_idx + 1}/{len(data)} (id={sample_id})")

        # Ingest
        start = time.time()
        ingest_conversation(client, conversation.get("conversation", {}), graph)
        ingest_time = time.time() - start
        print(f"  Ingestion took {ingest_time:.1f}s")

        # Answer questions
        qa_pairs = conversation.get("qa", [])
        print(f"  Answering {len(qa_pairs)} questions...")

        for qa_idx, qa in enumerate(qa_pairs):
            question = qa["question"]
            prediction = ask_question(client, question, graph)
            score = score_answer(prediction, qa)
            category = qa.get("category", 4)
            category_scores[category].append(score)

            result = {
                "sample_id": sample_id,
                "question": question,
                "expected": str(qa.get("answer", qa.get("adversarial_answer", ""))),
                "prediction": prediction,
                "category": category,
                "category_name": CATEGORY_NAMES.get(category, "unknown"),
                "f1": score,
            }
            all_results.append(result)

            if (qa_idx + 1) % 50 == 0:
                print(f"    {qa_idx + 1}/{len(qa_pairs)} done")

    # Print results
    print("\n" + "=" * 60)
    print("LoCoMo Results")
    print("=" * 60)

    total_score = 0.0
    total_count = 0
    for cat in sorted(category_scores.keys()):
        scores = category_scores[cat]
        avg = sum(scores) / len(scores) if scores else 0.0
        name = CATEGORY_NAMES.get(cat, "unknown")
        print(f"  {name:12s} (cat {cat}): {avg:.3f}  (n={len(scores)})")
        total_score += sum(scores)
        total_count += len(scores)

    overall = total_score / total_count if total_count else 0.0
    print(f"  {'overall':12s}       : {overall:.3f}  (n={total_count})")

    # Save results
    output_file = Path(__file__).parent / "results.json"
    output_file.write_text(json.dumps(all_results, indent=2))
    print(f"\nDetailed results saved to {output_file}")


if __name__ == "__main__":
    main()
