use std::sync::atomic::Ordering;

use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

use crate::graph_backend::GraphBackend;
use crate::math::{cosine_similarity, mmr_select};
use crate::models::{ContextFact, ContextProgress, ContextRequest, ContextResponse};
use crate::state::AppState;

pub async fn context(
    state: &AppState,
    graph: &dyn GraphBackend,
    req: ContextRequest,
    progress_tx: Option<tokio::sync::mpsc::Sender<ContextProgress>>,
) -> Result<ContextResponse> {
    state.metrics.context_calls_total.fetch_add(1, Ordering::Relaxed);

    let limit = req.limit.unwrap_or(state.config.default_context_limit);
    let max_hops = req.max_hops.unwrap_or(2).min(3);

    tracing::info!(query = %req.query, limit, max_hops, "context: query");

    macro_rules! send_progress {
        ($event:expr) => {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send($event).await;
            }
        };
    }

    // Step 1: Embed the query
    let embedding = state.llm.embed(&req.query).await?;
    send_progress!(ContextProgress::Embedding);

    let mut relevance_scores: HashMap<i64, f32> = HashMap::new();
    let mut hop_numbers: HashMap<i64, usize> = HashMap::new();
    let mut edges: Vec<crate::models::EdgeRow> = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

    // Step 2: Fulltext search on fact text — most direct relevance signal
    let ft_edges = graph.fulltext_search_edges(&req.query).await?;
    tracing::info!(count = ft_edges.len(), "context: fulltext edge results");
    send_progress!(ContextProgress::FulltextHits { count: ft_edges.len() });
    for edge in ft_edges {
        if seen_ids.insert(edge.edge_id) {
            tracing::debug!(edge_id = edge.edge_id, fact = %edge.fact, "context: fulltext edge hit");
            relevance_scores.insert(edge.edge_id, 0.95);
            hop_numbers.insert(edge.edge_id, 1);
            edges.push(edge);
        }
    }

    // Step 3: Fulltext search on entity names, N-hop walk — catches entity-centric queries
    let ft_entities = graph.fulltext_search_entities(&req.query).await?;
    tracing::info!(
        count = ft_entities.len(),
        names = ?ft_entities.iter().map(|e| &e.name).collect::<Vec<_>>(),
        "context: fulltext entity results"
    );
    if !ft_entities.is_empty() {
        let query_lower = req.query.to_lowercase();
        let query_tokens: std::collections::HashSet<&str> =
            query_lower.split_whitespace().collect();

        let (exact, partial): (Vec<_>, Vec<_>) = ft_entities.into_iter().partition(|e| {
            query_tokens.contains(e.name.to_lowercase().as_str())
        });

        if !exact.is_empty() {
            let ids: Vec<String> = exact.iter().map(|e| e.id.clone()).collect();
            let hop_results = graph.walk_n_hops(&ids, max_hops, 30).await?;
            tracing::info!(count = hop_results.len(), "context: exact entity n-hop edges");
            send_progress!(ContextProgress::HopResults { hop: 1, count: hop_results.len() });
            for (edge, hop) in hop_results {
                let hop_decay = 0.6f32.powi(hop as i32 - 1);
                let score = 0.9 * hop_decay;
                if seen_ids.insert(edge.edge_id) {
                    relevance_scores.insert(edge.edge_id, score);
                    hop_numbers.insert(edge.edge_id, hop);
                    edges.push(edge);
                } else {
                    relevance_scores.entry(edge.edge_id).and_modify(|s| *s = s.max(score));
                }
            }
        }

        if !partial.is_empty() {
            let ids: Vec<String> = partial.iter().map(|e| e.id.clone()).collect();
            let hop_results = graph.walk_n_hops(&ids, max_hops, 30).await?;
            tracing::info!(count = hop_results.len(), "context: partial entity n-hop edges");
            send_progress!(ContextProgress::HopResults { hop: 2, count: hop_results.len() });
            for (edge, hop) in hop_results {
                let hop_decay = 0.6f32.powi(hop as i32 - 1);
                let score = 0.6 * hop_decay;
                if seen_ids.insert(edge.edge_id) {
                    relevance_scores.insert(edge.edge_id, score);
                    hop_numbers.insert(edge.edge_id, hop);
                    edges.push(edge);
                }
            }
        }
    }

    // Step 4: Blend vector search into the pipeline (not just fallback)
    let vec_results = graph.vector_search_edges_scored(&embedding, 20).await?;
    tracing::info!(count = vec_results.len(), "context: vector search results");
    send_progress!(ContextProgress::VectorHits { count: vec_results.len() });
    for (edge, score) in vec_results {
        if seen_ids.insert(edge.edge_id) {
            relevance_scores.insert(edge.edge_id, score * 0.3);
            hop_numbers.insert(edge.edge_id, 1);
            edges.push(edge);
        } else {
            relevance_scores.entry(edge.edge_id).and_modify(|s| *s = (*s + score * 0.3).min(1.0));
        }
    }

    // Step 5: Filter out invalidated (belt-and-braces — queries already filter)
    edges.retain(|e| e.invalid_at.is_none());

    // Step 6: Score and sort
    // relevance (0..1) dominates; salience and recency break ties
    let now = Utc::now();
    let mut scored: Vec<(f32, crate::models::EdgeRow)> = edges
        .into_iter()
        .map(|edge| {
            let valid_at = chrono::DateTime::parse_from_rfc3339(&edge.valid_at)
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or(now);
            let days = (now - valid_at).num_seconds() as f32 / 86400.0;
            let recency = 1.0 / (1.0 + days);
            // Cap salience contribution to avoid drowning out relevance
            let salience_norm = ((edge.salience as f32).ln_1p() / 5.0).min(1.0);
            let relevance = relevance_scores.get(&edge.edge_id).copied().unwrap_or(0.5);
            let confidence_factor = edge.decayed_confidence;
            let score = relevance * 0.5 + confidence_factor * 0.1 + recency * 0.25 + salience_norm * 0.15;
            (score, edge)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (score, edge) in &scored {
        tracing::debug!(score, fact = %edge.fact, salience = edge.salience, "context: scored");
    }

    // Filter by memory tier if requested
    if let Some(ref tier_filter) = req.memory_tier_filter {
        scored.retain(|(_, edge)| edge.memory_tier == *tier_filter);
    }

    // Step 6b: MMR reranking for diversity
    // Build (score, index) pairs for mmr_select, using embeddings for similarity
    let items: Vec<(f32, usize)> = scored.iter().enumerate().map(|(i, (s, _))| (*s, i)).collect();
    let selected = mmr_select(&items, limit, 0.7, |a, b| {
        cosine_similarity(&scored[a].1.embedding, &scored[b].1.embedding)
    });
    scored = selected.into_iter().map(|i| scored[i].clone()).collect();
    send_progress!(ContextProgress::Ranked { total: scored.len() });

    // Step 7: Increment salience on returned edges (unique edge_ids only)
    let mut seen = std::collections::HashSet::new();
    let edge_ids: Vec<i64> = scored.iter()
        .map(|(_, e)| e.edge_id)
        .filter(|id| seen.insert(*id))
        .collect();
    if !edge_ids.is_empty() {
        graph.increment_salience(&edge_ids).await?;
    }

    // Step 8: Build response
    let facts: Vec<ContextFact> = scored
        .into_iter()
        .map(|(_, edge)| {
            let valid_at = chrono::DateTime::parse_from_rfc3339(&edge.valid_at)
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or(now);
            let source_agents: Vec<String> = edge.source_agents
                .split('|')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            ContextFact {
                hops: hop_numbers.get(&edge.edge_id).copied().unwrap_or(1),
                fact: edge.fact,
                subject: edge.subject_name,
                relation_type: edge.relation_type,
                object: edge.object_name,
                confidence: edge.confidence,
                salience: edge.salience,
                valid_at,
                edge_id: edge.edge_id,
                source_agents,
                memory_tier: edge.memory_tier,
            }
        })
        .collect();

    send_progress!(ContextProgress::Done { facts: facts.clone() });

    Ok(ContextResponse { facts })
}
