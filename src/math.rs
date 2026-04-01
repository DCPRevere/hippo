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
    let v: Vec<f32> = (0..EMBEDDING_DIM).map(|_| rng.gen_range(-1.0f32..1.0f32)).collect();
    normalize(v)
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
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
pub fn mmr_select<F>(
    items: &[(f32, usize)],
    k: usize,
    lambda: f32,
    similarity: F,
) -> Vec<usize>
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
            if a == b { return 1.0; }
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            match (lo, hi) {
                (0, 1) => 0.99,  // 0 and 1 are near-duplicates
                _ => 0.1,        // 2 is different from both
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
}
