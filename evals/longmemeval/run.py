#!/usr/bin/env python3
"""Run LongMemEval benchmark against a hippo instance.

Usage:
    # Start hippo with a real LLM:
    GRAPH_BACKEND=memory PORT=21693 ./target/release/hippo

    # Download LongMemEval data (oracle variant -- smallest, tests QA ability):
    curl -Lo evals/longmemeval/longmemeval_oracle.json \
      https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_oracle.json

    # Run:
    python evals/longmemeval/run.py

    # Use the medium variant for more realistic evaluation:
    python evals/longmemeval/run.py --variant s

Environment:
    HIPPO_URL       Hippo base URL (default: http://localhost:21693)
    HIPPO_API_KEY   API key (optional for local)
    OPENAI_API_KEY  For GPT-4o judge scoring (optional, falls back to substring match)
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from hippo import HippoClient

DATA_DIR = Path(__file__).parent


def load_data(variant: str) -> list[dict]:
    """Load a LongMemEval dataset variant."""
    if variant == "oracle":
        path = DATA_DIR / "longmemeval_oracle.json"
    elif variant == "s":
        path = DATA_DIR / "longmemeval_s_cleaned.json"
    elif variant == "m":
        path = DATA_DIR / "longmemeval_m_cleaned.json"
    else:
        raise ValueError(f"Unknown variant: {variant}")

    if not path.exists():
        print(f"Error: {path} not found.")
        print(f"Download it from the LongMemEval repo:")
        print(
            f"  curl -Lo {path} https://raw.githubusercontent.com/xiaowu0162/LongMemEval/main/data/{path.name}"
        )
        sys.exit(1)

    return json.loads(path.read_text())


def ingest_sessions(
    client: HippoClient, entry: dict, graph: str
) -> None:
    """Ingest chat history sessions into hippo."""
    sessions = entry.get("haystack_sessions", [])
    dates = entry.get("haystack_dates", [])

    for i, session in enumerate(sessions):
        date_str = dates[i] if i < len(dates) else ""
        lines = []
        if date_str:
            lines.append(f"[{date_str}]")
        for turn in session:
            role = turn.get("role", "user")
            content = turn.get("content", "")
            lines.append(f"{role}: {content}")

        statement = "\n".join(lines)
        try:
            client.remember(statement, graph=graph, timeout=120.0)
        except Exception as e:
            print(f"  Warning: session {i+1} failed: {e}")


def ask_question(client: HippoClient, question: str, graph: str) -> str:
    """Ask a question via /ask."""
    try:
        resp = client.ask(question, graph=graph, limit=50, timeout=60.0)
        return resp.answer
    except Exception as e:
        return f"Error: {e}"


def judge_with_gpt(question: str, expected: str, prediction: str, qtype: str) -> bool:
    """Use GPT-4o as judge (LongMemEval standard). Returns True if correct."""
    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        return fallback_judge(expected, prediction, qtype)

    try:
        from openai import OpenAI

        client = OpenAI(api_key=api_key)

        if qtype == "knowledge-update":
            prompt = (
                f"Question: {question}\n"
                f"Expected answer: {expected}\n"
                f"Model response: {prediction}\n\n"
                "Is the model response correct? It is correct if it includes the "
                "updated answer, even if it also mentions old information. "
                "Answer yes or no only."
            )
        elif qtype == "temporal-reasoning":
            prompt = (
                f"Question: {question}\n"
                f"Expected answer: {expected}\n"
                f"Model response: {prediction}\n\n"
                "Is the model response correct? Forgive off-by-one errors for day "
                "counts. Answer yes or no only."
            )
        else:
            prompt = (
                f"Question: {question}\n"
                f"Expected answer: {expected}\n"
                f"Model response: {prediction}\n\n"
                "Is the model response correct? Answer yes or no only."
            )

        resp = client.chat.completions.create(
            model="gpt-4o",
            messages=[{"role": "user", "content": prompt}],
            max_tokens=5,
            temperature=0,
        )
        answer = resp.choices[0].message.content.strip().lower()
        return answer.startswith("yes")
    except Exception:
        return fallback_judge(expected, prediction, qtype)


def fallback_judge(expected: str, prediction: str, qtype: str) -> bool:
    """Simple substring match fallback when GPT-4o is unavailable."""
    pred_lower = prediction.lower()
    exp_lower = expected.lower()

    if qtype.endswith("_abs") or "abstention" in qtype:
        abstain_phrases = ["don't know", "not sure", "no information", "cannot"]
        return any(p in pred_lower for p in abstain_phrases)

    # Check if key words from expected appear in prediction.
    exp_words = set(exp_lower.split())
    pred_words = set(pred_lower.split())
    if not exp_words:
        return True
    overlap = len(exp_words & pred_words) / len(exp_words)
    return overlap >= 0.5


def main():
    parser = argparse.ArgumentParser(description="Run LongMemEval against hippo")
    parser.add_argument(
        "--variant",
        default="oracle",
        choices=["oracle", "s", "m"],
        help="Dataset variant (default: oracle)",
    )
    parser.add_argument(
        "--limit", type=int, default=0, help="Limit number of questions (0 = all)"
    )
    args = parser.parse_args()

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

    data = load_data(args.variant)
    if args.limit > 0:
        data = data[: args.limit]
    print(f"Loaded {len(data)} questions (variant={args.variant})")

    has_gpt = bool(os.environ.get("OPENAI_API_KEY"))
    print(f"Judge: {'GPT-4o' if has_gpt else 'substring fallback (set OPENAI_API_KEY for GPT-4o)'}")

    all_results = []
    type_scores: dict[str, list[float]] = defaultdict(list)

    for i, entry in enumerate(data):
        qid = entry["question_id"]
        qtype = entry["question_type"]
        question = entry["question"]
        expected = entry["answer"]
        is_abstention = qid.endswith("_abs")

        graph = f"longmemeval_{qid}"

        # Ingest
        sessions = entry.get("haystack_sessions", [])
        if sessions:
            ingest_sessions(client, entry, graph)

        # Ask
        prediction = ask_question(client, question, graph)

        # Judge
        if is_abstention:
            correct = fallback_judge(expected, prediction, "abstention")
        else:
            correct = judge_with_gpt(question, expected, prediction, qtype)

        score = 1.0 if correct else 0.0
        type_scores[qtype].append(score)
        if is_abstention:
            type_scores["abstention"].append(score)

        all_results.append(
            {
                "question_id": qid,
                "question_type": qtype,
                "question": question,
                "expected": expected,
                "hypothesis": prediction,
                "correct": correct,
                "is_abstention": is_abstention,
            }
        )

        if (i + 1) % 25 == 0:
            print(f"  {i + 1}/{len(data)} done")

    # Print results
    print("\n" + "=" * 60)
    print(f"LongMemEval Results (variant={args.variant})")
    print("=" * 60)

    total_score = 0.0
    total_count = 0
    type_avgs = {}
    for qtype in sorted(type_scores.keys()):
        if qtype == "abstention":
            continue
        scores = type_scores[qtype]
        avg = sum(scores) / len(scores) if scores else 0.0
        type_avgs[qtype] = avg
        print(f"  {qtype:30s}: {avg:.3f}  (n={len(scores)})")
        total_score += sum(scores)
        total_count += len(scores)

    overall = total_score / total_count if total_count else 0.0
    task_avg = sum(type_avgs.values()) / len(type_avgs) if type_avgs else 0.0
    print(f"  {'overall':30s}: {overall:.3f}  (n={total_count})")
    print(f"  {'task-averaged':30s}: {task_avg:.3f}")

    if "abstention" in type_scores:
        abs_scores = type_scores["abstention"]
        abs_avg = sum(abs_scores) / len(abs_scores) if abs_scores else 0.0
        print(f"  {'abstention':30s}: {abs_avg:.3f}  (n={len(abs_scores)})")

    # Save results
    output_file = DATA_DIR / f"results_{args.variant}.json"
    output_file.write_text(json.dumps(all_results, indent=2))
    print(f"\nDetailed results saved to {output_file}")

    # Also save JSONL for LongMemEval's official eval script
    jsonl_file = DATA_DIR / f"hypotheses_{args.variant}.jsonl"
    with jsonl_file.open("w") as f:
        for r in all_results:
            f.write(
                json.dumps(
                    {"question_id": r["question_id"], "hypothesis": r["hypothesis"]}
                )
                + "\n"
            )
    print(f"JSONL output saved to {jsonl_file} (for official eval script)")


if __name__ == "__main__":
    main()
