"""Shared scoring utilities for LoCoMo and LongMemEval."""

from __future__ import annotations

import re
import string


def normalize(text: str) -> str:
    """Lowercase, strip punctuation and articles, collapse whitespace."""
    text = str(text).lower()
    text = text.translate(str.maketrans("", "", string.punctuation))
    text = re.sub(r"\b(a|an|the)\b", " ", text)
    return " ".join(text.split())


def token_f1(prediction: str, answer: str) -> float:
    """Token-level F1 between prediction and answer (LoCoMo-style)."""
    pred_tokens = normalize(prediction).split()
    ans_tokens = normalize(answer).split()
    if not ans_tokens:
        return 1.0 if not pred_tokens else 0.0
    if not pred_tokens:
        return 0.0
    common = set(pred_tokens) & set(ans_tokens)
    if not common:
        return 0.0
    precision = len(common) / len(pred_tokens)
    recall = len(common) / len(ans_tokens)
    return 2 * precision * recall / (precision + recall)


def multihop_f1(prediction: str, answer: str) -> float:
    """Multi-hop F1: answer is comma-separated sub-answers."""
    sub_answers = [a.strip() for a in answer.split(",")]
    pred_parts = [p.strip() for p in prediction.split(",")]
    if not sub_answers:
        return 1.0 if not pred_parts else 0.0
    scores = []
    for sub in sub_answers:
        best = max(token_f1(p, sub) for p in pred_parts)
        scores.append(best)
    return sum(scores) / len(scores)


def adversarial_score(prediction: str) -> float:
    """Adversarial: 1.0 if model correctly abstains."""
    pred_lower = prediction.lower()
    abstain_phrases = [
        "no information available",
        "not mentioned",
        "not discussed",
        "no information",
        "cannot be determined",
        "not enough information",
        "i don't know",
        "i don't have",
    ]
    return 1.0 if any(p in pred_lower for p in abstain_phrases) else 0.0
