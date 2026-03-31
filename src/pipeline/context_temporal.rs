use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

use crate::graph_backend::GraphBackend;
use crate::models::{ContextFact, ContextResponse, TemporalContextRequest};
use crate::state::AppState;

pub async fn context_temporal(state: &AppState, graph: &dyn GraphBackend, req: TemporalContextRequest) -> Result<ContextResponse> {
    let limit = req.limit.unwrap_or(state.config.default_context_limit);
    let at = req.at;

    tracing::info!(query = %req.query, %at, limit, "context_temporal: query");

    let embedding = state.llm.embed(&req.query).await?;

    let mut relevance_scores: HashMap<i64, f32> = HashMap::new();
    let mut edges: Vec<crate::models::EdgeRow> = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

    // Step 1: Fulltext search on fact text with temporal filter
    let ft_edges = graph.fulltext_search_edges_at(&req.query, at).await?;
    tracing::info!(count = ft_edges.len(), "context_temporal: fulltext edge results");
    for edge in ft_edges {
        if seen_ids.insert(edge.edge_id) {
            relevance_scores.insert(edge.edge_id, 0.95);
            edges.push(edge);
        }
    }

    // Step 2: Entity search + one-hop walk with temporal filter
    let ft_entities = graph.fulltext_search_entities(&req.query).await?;
    if !ft_entities.is_empty() {
        let query_lower = req.query.to_lowercase();
        let query_tokens: std::collections::HashSet<&str> =
            query_lower.split_whitespace().collect();

        let (exact, partial): (Vec<_>, Vec<_>) = ft_entities.into_iter().partition(|e| {
            query_tokens.contains(e.name.to_lowercase().as_str())
        });

        if !exact.is_empty() {
            let ids: Vec<String> = exact.iter().map(|e| e.id.clone()).collect();
            let hop_edges = graph.walk_one_hop_at(&ids, 50, at).await?;
            for edge in hop_edges {
                if seen_ids.insert(edge.edge_id) {
                    relevance_scores.insert(edge.edge_id, 0.9);
                    edges.push(edge);
                } else {
                    relevance_scores.entry(edge.edge_id).and_modify(|s| *s = s.max(0.9));
                }
            }
        }

        if !partial.is_empty() {
            let ids: Vec<String> = partial.iter().map(|e| e.id.clone()).collect();
            let hop_edges = graph.walk_one_hop_at(&ids, 20, at).await?;
            for edge in hop_edges {
                if seen_ids.insert(edge.edge_id) {
                    relevance_scores.insert(edge.edge_id, 0.6);
                    edges.push(edge);
                }
            }
        }
    }

    // Step 3: Vector search fallback with temporal filter
    if edges.is_empty() {
        tracing::info!("context_temporal: no fulltext results, falling back to vector search");
        let vec_results = graph.vector_search_edges_at(&embedding, 20, at).await?;
        for edge in vec_results {
            if seen_ids.insert(edge.edge_id) {
                relevance_scores.insert(edge.edge_id, 0.5);
                edges.push(edge);
            }
        }
    }

    // Step 4: Score and sort
    let now = Utc::now();
    let mut scored: Vec<(f32, crate::models::EdgeRow)> = edges
        .into_iter()
        .map(|edge| {
            let valid_at = chrono::DateTime::parse_from_rfc3339(&edge.valid_at)
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or(now);
            let days = (now - valid_at).num_seconds() as f32 / 86400.0;
            let recency = 1.0 / (1.0 + days);
            let salience_norm = ((edge.salience as f32).ln_1p() / 5.0).min(1.0);
            let relevance = relevance_scores.get(&edge.edge_id).copied().unwrap_or(0.5);
            let score = relevance * 0.6 + recency * 0.25 + salience_norm * 0.15;
            (score, edge)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    let mut seen = std::collections::HashSet::new();
    let edge_ids: Vec<i64> = scored.iter()
        .map(|(_, e)| e.edge_id)
        .filter(|id| seen.insert(*id))
        .collect();
    if !edge_ids.is_empty() {
        graph.increment_salience(&edge_ids).await?;
    }

    let facts = scored
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
                fact: edge.fact,
                subject: edge.subject_name,
                relation_type: edge.relation_type,
                object: edge.object_name,
                confidence: edge.confidence,
                salience: edge.salience,
                valid_at,
                edge_id: edge.edge_id,
                hops: 1,
                source_agents,
                memory_tier: edge.memory_tier,
            }
        })
        .collect();

    Ok(ContextResponse { facts })
}
