use std::collections::HashMap;

use anyhow::Result;
use chrono::Utc;

use crate::graph_backend::GraphBackend;
use crate::models::{AskRequest, AskResponse, ContextFact, GraphContext};
use crate::state::AppState;

/// Convert a [`GraphContext`] subgraph into a flat list of [`ContextFact`]s
/// that `synthesise_answer` understands.
fn graph_context_to_facts(ctx: &GraphContext) -> Vec<ContextFact> {
    let node_names: HashMap<&str, &str> = ctx
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.name.as_str()))
        .collect();

    let now = Utc::now();

    ctx.edges
        .iter()
        .map(|e| ContextFact {
            fact: e.fact.clone(),
            subject: node_names.get(e.from.as_str()).unwrap_or(&"?").to_string(),
            relation_type: e.relation.clone(),
            object: node_names.get(e.to.as_str()).unwrap_or(&"?").to_string(),
            confidence: e.confidence,
            salience: 0,
            valid_at: now,
            edge_id: e.id,
            hops: 0,
            source_agents: vec![],
            memory_tier: "working".to_string(),
        })
        .collect()
}

pub async fn ask(
    state: &AppState,
    graph: &dyn GraphBackend,
    req: AskRequest,
) -> Result<AskResponse> {
    let ctx = super::remember::gather_pre_extraction_context(state, graph, &req.question).await?;
    let facts = graph_context_to_facts(&ctx);

    let answer = state.llm.synthesise_answer(&req.question, &facts).await?;

    Ok(AskResponse {
        answer,
        facts: if req.verbose { Some(facts) } else { None },
    })
}
