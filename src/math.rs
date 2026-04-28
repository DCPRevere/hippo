use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use sha2::{Digest, Sha256};

use crate::models::EMBEDDING_DIM;

pub fn clean_json(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    let s = s.trim();

    let obj_start = s.find('{');
    let arr_start = s.find('[');

    let (start, open, close) = match (obj_start, arr_start) {
        (Some(o), Some(a)) if a < o => (a, b'[', b']'),
        (Some(o), _) => (o, b'{', b'}'),
        (None, Some(a)) => (a, b'[', b']'),
        (None, None) => return s,
    };

    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes[start..].iter().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return &s[start..start + i + 1];
            }
        }
    }
    s
}

pub fn pseudo_embed(text: &str) -> Vec<f32> {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let hash = hasher.finalize();
    let seed = u64::from_le_bytes(hash[..8].try_into().unwrap());
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let v: Vec<f32> = (0..EMBEDDING_DIM)
        .map(|_| rng.gen_range(-1.0f32..1.0f32))
        .collect();
    normalize(v)
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// Bayesian "noisy-OR" confidence compounding for independent positive sources.
///
/// `combine(p, q) = 1 - (1 - p)*(1 - q)`, capped at 0.99 to keep room for
/// future contradictions. Inputs outside `[0.0, 1.0]` are clamped.
pub fn compound_confidence(old: f32, new: f32) -> f32 {
    let o = old.clamp(0.0, 1.0);
    let n = new.clamp(0.0, 1.0);
    let combined = 1.0 - (1.0 - o) * (1.0 - n);
    combined.min(0.99)
}

/// Exponential decay applied per stale day past `grace_days`.
///
/// Returns `confidence` unchanged if `days_stale <= grace_days`, otherwise
/// `confidence * factor.powi(days_stale - grace_days)`. Result is clamped
/// to `[0.0, 1.0]`. Negative `days_stale` is treated as zero.
pub fn decay_confidence(confidence: f32, days_stale: i32, grace_days: i32, factor: f32) -> f32 {
    let extra = days_stale.saturating_sub(grace_days).max(0);
    if extra == 0 {
        return confidence.clamp(0.0, 1.0);
    }
    let decayed = confidence * factor.powi(extra);
    decayed.clamp(0.0, 1.0)
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Maximal Marginal Relevance reranking.
///
/// Given a list of `(score, embedding_index)` pairs sorted by descending score,
/// iteratively selects items that balance relevance against diversity.
///
/// `lambda` controls the tradeoff: 1.0 = pure relevance, 0.0 = pure diversity.
/// Typical value: 0.7.
///
/// Returns the indices (into the original `items` slice) of the selected items,
/// in selection order, up to `k` items.
pub fn mmr_select<F>(items: &[(f32, usize)], k: usize, lambda: f32, similarity: F) -> Vec<usize>
where
    F: Fn(usize, usize) -> f32,
{
    if items.is_empty() || k == 0 {
        return vec![];
    }

    let mut selected: Vec<usize> = Vec::with_capacity(k);
    let mut remaining: Vec<usize> = (0..items.len()).collect();

    // Always pick the highest-scoring item first
    selected.push(remaining.remove(0));

    while selected.len() < k && !remaining.is_empty() {
        let mut best_idx_in_remaining = 0;
        let mut best_mmr = f32::NEG_INFINITY;

        for (ri, &candidate) in remaining.iter().enumerate() {
            let relevance = items[candidate].0;
            let max_sim = selected
                .iter()
                .map(|&s| similarity(items[candidate].1, items[s].1))
                .fold(f32::NEG_INFINITY, f32::max);

            let mmr = lambda * relevance - (1.0 - lambda) * max_sim;
            if mmr > best_mmr {
                best_mmr = mmr;
                best_idx_in_remaining = ri;
            }
        }

        selected.push(remaining.remove(best_idx_in_remaining));
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_return_one() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_return_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_return_negative_one() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_vectors_return_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn mismatched_lengths_return_zero() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn zero_vector_returns_zero() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn mmr_empty_returns_empty() {
        let result = mmr_select(&[], 5, 0.7, |_, _| 0.0);
        assert!(result.is_empty());
    }

    #[test]
    fn mmr_pure_relevance_preserves_order() {
        // lambda=1.0 means no diversity penalty, so order = pure score order
        let items = vec![(0.9, 0), (0.8, 1), (0.7, 2)];
        let result = mmr_select(&items, 3, 1.0, |_, _| 1.0);
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn mmr_diversity_reorders_similar_items() {
        // Three items: 0 and 1 are identical (similarity=1.0), 2 is different
        let items = vec![(0.9, 0), (0.85, 1), (0.8, 2)];
        let sim = |a: usize, b: usize| -> f32 {
            if a == b {
                return 1.0;
            }
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            match (lo, hi) {
                (0, 1) => 0.99, // 0 and 1 are near-duplicates
                _ => 0.1,       // 2 is different from both
            }
        };
        let result = mmr_select(&items, 3, 0.5, sim);
        // First pick: 0 (highest score)
        assert_eq!(result[0], 0);
        // Second pick: 2 should beat 1 because 1 is too similar to 0
        assert_eq!(result[1], 2);
        assert_eq!(result[2], 1);
    }

    #[test]
    fn mmr_respects_k_limit() {
        let items = vec![(0.9, 0), (0.8, 1), (0.7, 2)];
        let result = mmr_select(&items, 2, 0.7, |_, _| 0.0);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn mmr_k_zero_returns_empty() {
        let items = vec![(0.9, 0), (0.8, 1)];
        assert!(mmr_select(&items, 0, 0.7, |_, _| 0.0).is_empty());
    }

    #[test]
    fn normalize_zero_vector_is_unchanged() {
        let v = vec![0.0f32, 0.0, 0.0];
        let n = normalize(v.clone());
        assert_eq!(n, v);
    }

    #[test]
    fn normalize_produces_unit_vector() {
        let n = normalize(vec![3.0f32, 4.0]);
        let mag: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((mag - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_with_nan_does_not_panic() {
        // Documented behaviour: NaN propagates through f32 arithmetic to NaN.
        // What matters for callers is that this does not panic.
        let a = vec![f32::NAN, 1.0];
        let b = vec![1.0, 1.0];
        let _ = cosine_similarity(&a, &b);
    }

    // ---- compound_confidence ----

    #[test]
    fn compound_confidence_idempotent_with_zero() {
        assert!((compound_confidence(0.7, 0.0) - 0.7).abs() < 1e-6);
        assert!((compound_confidence(0.0, 0.7) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn compound_confidence_caps_at_0_99() {
        assert!((compound_confidence(0.99, 0.99) - 0.99).abs() < 1e-6);
        assert!((compound_confidence(1.0, 1.0) - 0.99).abs() < 1e-6);
    }

    #[test]
    fn compound_confidence_is_commutative() {
        let a = compound_confidence(0.3, 0.6);
        let b = compound_confidence(0.6, 0.3);
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn compound_confidence_clamps_negative_inputs() {
        // Negative or > 1.0 inputs are clamped, not propagated.
        let r = compound_confidence(-0.5, 0.5);
        assert!((r - 0.5).abs() < 1e-6);
    }

    // ---- decay_confidence ----

    #[test]
    fn decay_within_grace_period_is_noop() {
        assert!((decay_confidence(0.9, 5, 30, 0.995) - 0.9).abs() < 1e-6);
        assert!((decay_confidence(0.9, 30, 30, 0.995) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn decay_past_grace_reduces_confidence_monotonically() {
        let a = decay_confidence(0.9, 31, 30, 0.995);
        let b = decay_confidence(0.9, 60, 30, 0.995);
        assert!(a < 0.9);
        assert!(b < a);
    }

    #[test]
    fn decay_clamps_to_unit_interval() {
        // Pathological factor > 1.0 still clamps.
        assert!(decay_confidence(0.9, 100, 0, 1.5) <= 1.0);
        assert!(decay_confidence(-0.1, 0, 0, 0.5) >= 0.0);
    }

    #[test]
    fn decay_negative_days_treated_as_zero() {
        assert!((decay_confidence(0.7, -100, 30, 0.995) - 0.7).abs() < 1e-6);
    }

    // ---- proptests ----

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn cosine_self_similarity_is_one_for_nonzero_vectors(
            xs in proptest::collection::vec(-100.0f32..100.0, 1..16)
        ) {
            let norm: f32 = xs.iter().map(|x| x*x).sum::<f32>().sqrt();
            prop_assume!(norm > 1e-3);
            let sim = cosine_similarity(&xs, &xs);
            prop_assert!((sim - 1.0).abs() < 1e-3, "self-similarity {} != 1", sim);
        }

        #[test]
        fn cosine_is_in_minus_one_to_one(
            a in proptest::collection::vec(-10.0f32..10.0, 4..8),
            b in proptest::collection::vec(-10.0f32..10.0, 4..8),
        ) {
            // Force matching length.
            let n = a.len().min(b.len());
            let sim = cosine_similarity(&a[..n], &b[..n]);
            // NaN allowed (zero vector → 0.0 by contract; otherwise within range).
            if !sim.is_nan() {
                prop_assert!((-1.0 - 1e-3..=1.0 + 1e-3).contains(&sim));
            }
        }

        #[test]
        fn compound_confidence_monotonic_in_each_arg(
            a in 0.0f32..1.0,
            b in 0.0f32..1.0,
            delta in 0.0f32..0.5,
        ) {
            let base = compound_confidence(a, b);
            let bumped = compound_confidence((a + delta).min(1.0), b);
            prop_assert!(bumped + 1e-5 >= base);
        }

        #[test]
        fn compound_confidence_in_unit_interval(
            a in -1.0f32..2.0,
            b in -1.0f32..2.0,
        ) {
            let r = compound_confidence(a, b);
            prop_assert!((0.0..=0.99 + 1e-6).contains(&r));
        }

        #[test]
        fn decay_in_unit_interval(
            c in 0.0f32..1.0,
            days in 0i32..1000,
            grace in 0i32..100,
            factor in 0.0f32..1.0,
        ) {
            let r = decay_confidence(c, days, grace, factor);
            prop_assert!((0.0..=1.0 + 1e-6).contains(&r));
        }

        #[test]
        fn decay_monotonic_in_days(
            c in 0.01f32..1.0,
            d1 in 0i32..200,
            extra in 1i32..100,
        ) {
            let factor = 0.99f32;
            let grace = 30;
            let a = decay_confidence(c, d1, grace, factor);
            let b = decay_confidence(c, d1 + extra, grace, factor);
            prop_assert!(b <= a + 1e-5);
        }
    }
}
