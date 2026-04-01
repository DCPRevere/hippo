use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

use crate::graph_backend::GraphBackend;
use crate::models::{
    ContextFact, ContextRequest, DiagnoseResponse, DiagnoseStep, GraphDumpResponse, GraphEdge,
    GraphEntity,
};
use crate::state::AppState;

pub async fn diagnose(state: &AppState, graph: &dyn GraphBackend, req: ContextRequest) -> Result<DiagnoseResponse> {
    let limit = req.limit.unwrap_or(state.config.pipeline.default_context_limit);
    let mut steps = Vec::new();

    // Step 1: Fulltext search on fact text
    let ft_edges = graph.fulltext_search_edges(&req.query).await?;
    steps.push(DiagnoseStep {
        step: "fulltext_facts".to_string(),
        description: format!(
            "CONTAINS search on r.fact for tokens in {:?} — relevance 0.95",
            req.query
        ),
        results: ft_edges
            .iter()
            .map(|e| {
                serde_json::json!({
                    "edge_id": e.edge_id,
                    "fact": e.fact,
                    "subject": e.subject_name,
                    "relation_type": e.relation_type,
                    "object": e.object_name,
                    "assigned_relevance": 0.95,
                })
            })
            .collect(),
    });

    let mut relevance_scores: HashMap<i64, f32> = HashMap::new();
    let mut edges: Vec<crate::models::EdgeRow> = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for edge in ft_edges {
        if seen_ids.insert(edge.edge_id) {
            relevance_scores.insert(edge.edge_id, 0.95);
            edges.push(edge);
        }
    }

    // Step 2: Entity fulltext + hop
    let ft_entities = graph.fulltext_search_entities(&req.query).await?;
    let query_lower = req.query.to_lowercase();
    let query_tokens: std::collections::HashSet<&str> = query_lower.split_whitespace().collect();
    let (exact, partial): (Vec<_>, Vec<_>) = ft_entities
        .iter()
        .partition(|e| query_tokens.contains(e.name.to_lowercase().as_str()));

    steps.push(DiagnoseStep {
        step: "fulltext_entities".to_string(),
        description: "Fulltext search on entity names".to_string(),
        results: ft_entities
            .iter()
            .map(|e| {
                serde_json::json!({
                    "name": e.name,
                    "entity_type": e.entity_type,
                    "match_type": if exact.iter().any(|x| x.id == e.id) { "exact" } else { "partial" },
                })
            })
            .collect(),
    });

    if !exact.is_empty() {
        let ids: Vec<String> = exact.iter().map(|e| e.id.clone()).collect();
        let hop_edges = graph.walk_one_hop(&ids, 50).await?;
        steps.push(DiagnoseStep {
            step: "exact_entity_hop".to_string(),
            description: format!("One-hop from exact entity matches {:?} — relevance 0.9", exact.iter().map(|e| &e.name).collect::<Vec<_>>()),
            results: hop_edges
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "edge_id": e.edge_id,
                        "fact": e.fact,
                        "subject": e.subject_name,
                        "object": e.object_name,
                        "new": seen_ids.contains(&e.edge_id) == false,
                        "assigned_relevance": 0.9,
                    })
                })
                .collect(),
        });
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
        let hop_edges = graph.walk_one_hop(&ids, 20).await?;
        steps.push(DiagnoseStep {
            step: "partial_entity_hop".to_string(),
            description: format!("One-hop from partial entity matches {:?} — relevance 0.6", partial.iter().map(|e| &e.name).collect::<Vec<_>>()),
            results: hop_edges
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "edge_id": e.edge_id,
                        "fact": e.fact,
                        "subject": e.subject_name,
                        "object": e.object_name,
                        "new": seen_ids.contains(&e.edge_id) == false,
                        "assigned_relevance": 0.6,
                    })
                })
                .collect(),
        });
        for edge in hop_edges {
            if seen_ids.insert(edge.edge_id) {
                relevance_scores.insert(edge.edge_id, 0.6);
                edges.push(edge);
            }
        }
    }

    // Step 3: Vector fallback
    if edges.is_empty() {
        let embedding = state.llm.embed(&req.query).await?;
        let vec_results = graph.vector_search_edges_scored(&embedding, 20).await?;
        steps.push(DiagnoseStep {
            step: "vector_fallback".to_string(),
            description: "No fulltext results — falling back to vector search on edge embeddings".to_string(),
            results: vec_results
                .iter()
                .map(|(e, score)| {
                    serde_json::json!({
                        "edge_id": e.edge_id,
                        "fact": e.fact,
                        "subject": e.subject_name,
                        "object": e.object_name,
                        "vector_score": score,
                    })
                })
                .collect(),
        });
        for (edge, score) in vec_results {
            if seen_ids.insert(edge.edge_id) {
                relevance_scores.insert(edge.edge_id, score);
                edges.push(edge);
            }
        }
    } else {
        steps.push(DiagnoseStep {
            step: "vector_fallback".to_string(),
            description: "Skipped — fulltext found results".to_string(),
            results: vec![],
        });
    }

    // Invalidation filter
    let before_filter = edges.len();
    edges.retain(|e| e.invalid_at.is_none());
    steps.push(DiagnoseStep {
        step: "invalidation_filter".to_string(),
        description: format!("Removed {} invalidated edges", before_filter - edges.len()),
        results: vec![],
    });

    // Scoring
    let now = Utc::now();
    let mut scored: Vec<(f32, f32, f32, f32, crate::models::EdgeRow)> = edges
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
            (score, relevance, recency, salience_norm, edge)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    steps.push(DiagnoseStep {
        step: "scoring".to_string(),
        description: "score = relevance*0.6 + recency*0.25 + salience_norm*0.15".to_string(),
        results: scored
            .iter()
            .map(|(score, relevance, recency, salience_norm, edge)| {
                serde_json::json!({
                    "fact": edge.fact,
                    "subject": edge.subject_name,
                    "object": edge.object_name,
                    "score": score,
                    "relevance": relevance,
                    "recency": recency,
                    "salience_norm": salience_norm,
                    "salience_raw": edge.salience,
                    "kept": scored.iter().take(limit).any(|(_, _, _, _, e)| e.edge_id == edge.edge_id),
                })
            })
            .collect(),
    });

    scored.truncate(limit);

    let final_facts = scored
        .into_iter()
        .map(|(_, _, _, _, edge)| {
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

    Ok(DiagnoseResponse {
        query: req.query,
        steps,
        final_facts,
    })
}

pub async fn graph_dump(state: &AppState, graph: &dyn GraphBackend) -> Result<GraphDumpResponse> {
    let entities = graph
        .dump_all_entities()
        .await?
        .into_iter()
        .map(|e| GraphEntity {
            name: e.name,
            entity_type: e.entity_type,
            resolved: e.resolved,
        })
        .collect();

    let all_edges = graph.dump_all_edges().await?;
    let mut active_edges = Vec::new();
    let mut invalidated_edges = Vec::new();
    for e in all_edges {
        let ge = GraphEdge {
            subject: e.subject_name,
            relation_type: e.relation_type,
            object: e.object_name,
            fact: e.fact,
            salience: e.salience,
            confidence: e.confidence,
            valid_at: e.valid_at,
            invalid_at: e.invalid_at,
        };
        if ge.invalid_at.is_none() {
            active_edges.push(ge);
        } else {
            invalidated_edges.push(ge);
        }
    }

    Ok(GraphDumpResponse {
        entities,
        active_edges,
        invalidated_edges,
    })
}
