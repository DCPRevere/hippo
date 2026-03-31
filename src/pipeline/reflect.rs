use std::sync::atomic::Ordering;

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

use crate::graph_backend::GraphBackend;
use crate::models::{ContextFact, MemoryStats, ReflectRequest, ReflectResponse};
use crate::state::AppState;

pub async fn reflect(state: &AppState, graph: &dyn GraphBackend, req: ReflectRequest) -> Result<ReflectResponse> {
    state.metrics.reflect_calls_total.fetch_add(1, Ordering::Relaxed);

    let suggest = req.suggest_questions.unwrap_or(true);

    match req.about {
        Some(ref entity_name) => reflect_entity(state, graph, entity_name, suggest).await,
        None => reflect_global(state, graph).await,
    }
}

async fn reflect_entity(
    state: &AppState,
    graph: &dyn GraphBackend,
    entity_name: &str,
    suggest: bool,
) -> Result<ReflectResponse> {
    let now = Utc::now();

    // Get all active facts for the entity
    let edges = graph.entity_facts(entity_name).await?;

    // Split into known (confidence >= 0.6) and uncertain (< 0.6)
    let mut known = Vec::new();
    let mut uncertain = Vec::new();
    let mut entity_relation_types: HashSet<String> = HashSet::new();

    for edge in &edges {
        entity_relation_types.insert(edge.relation_type.clone());
        let valid_at = edge.valid_at.parse::<DateTime<Utc>>().unwrap_or(now);
        let fact = ContextFact {
            fact: edge.fact.clone(),
            subject: edge.subject_name.clone(),
            relation_type: edge.relation_type.clone(),
            object: edge.object_name.clone(),
            confidence: edge.confidence,
            salience: edge.salience,
            valid_at,
            edge_id: edge.edge_id,
            hops: 1,
            source_agents: edge.source_agents
                .split('|')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
            memory_tier: edge.memory_tier.clone(),
        };
        if edge.decayed_confidence >= 0.6 {
            known.push(fact);
        } else {
            uncertain.push(fact);
        }
    }

    // Find gap types: relation types in graph but missing for this entity
    let all_types = graph.all_relation_types().await?;
    let gaps: Vec<String> = all_types
        .into_iter()
        .filter(|t| !entity_relation_types.contains(t))
        .collect();

    // Generate questions if requested and there are gaps
    let suggested_questions = if suggest && !gaps.is_empty() {
        let known_facts: Vec<String> = known.iter().map(|f| f.fact.clone()).collect();
        state
            .llm
            .generate_gap_questions(entity_name, &known_facts, &gaps)
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };

    Ok(ReflectResponse {
        entity: Some(entity_name.to_string()),
        known,
        uncertain,
        gaps,
        suggested_questions,
        stats: None,
    })
}

async fn reflect_global(state: &AppState, graph: &dyn GraphBackend) -> Result<ReflectResponse> {
    let (total_entities, total_facts, oldest_str, newest_str, avg_confidence) =
        graph.graph_stats().await?;

    let oldest_fact = oldest_str.and_then(|s| s.parse::<DateTime<Utc>>().ok());
    let newest_fact = newest_str.and_then(|s| s.parse::<DateTime<Utc>>().ok());

    let entities_by_type = graph.entity_type_counts().await?;

    let stats = MemoryStats {
        total_entities,
        total_facts,
        oldest_fact,
        newest_fact,
        avg_confidence,
        entities_by_type,
    };

    // Find under-documented entities (< 2 edges)
    let under_documented = graph.under_documented_entities(2).await?;
    let gaps: Vec<String> = under_documented
        .iter()
        .map(|(name, etype, count)| {
            format!("{name} ({etype}) — {count} fact(s)")
        })
        .collect();

    Ok(ReflectResponse {
        entity: None,
        known: vec![],
        uncertain: vec![],
        gaps,
        suggested_questions: vec![],
        stats: Some(stats),
    })
}
