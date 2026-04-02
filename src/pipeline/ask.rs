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

/// Merge additional context into an existing GraphContext, deduplicating nodes and edges.
fn merge_context(base: &mut GraphContext, extra: GraphContext) {
    let existing_node_ids: std::collections::HashSet<String> =
        base.nodes.iter().map(|n| n.id.clone()).collect();
    let existing_edge_ids: std::collections::HashSet<i64> =
        base.edges.iter().map(|e| e.id).collect();

    for node in extra.nodes {
        if !existing_node_ids.contains(&node.id) {
            base.nodes.push(node);
        }
    }
    for edge in extra.edges {
        if !existing_edge_ids.contains(&edge.id) {
            base.edges.push(edge);
        }
    }
}

pub async fn ask(
    state: &AppState,
    graph: &dyn GraphBackend,
    req: AskRequest,
    user_id: Option<&str>,
    user_display_name: Option<&str>,
) -> Result<AskResponse> {
    let max_iterations = req.max_iterations.max(1);

    let mut ctx = super::remember::gather_pre_extraction_context(state, graph, &req.question, user_id).await?;
    let mut facts = graph_context_to_facts(&ctx);
    let mut iterations_used = 0;

    for i in 0..max_iterations {
        iterations_used = i + 1;

        let missing = state.llm.identify_missing_context(&req.question, &facts).await?;
        if missing.is_empty() {
            tracing::debug!(iteration = i + 1, "ask: LLM has sufficient context");
            break;
        }

        tracing::debug!(iteration = i + 1, entities = ?missing, "ask: LLM requested additional context");
        let extra = super::remember::gather_context_by_names(graph, &missing, &ctx).await?;

        let before = ctx.edge_count();
        merge_context(&mut ctx, extra);
        let after = ctx.edge_count();

        // If no new edges were added, further iterations won't help
        if after == before {
            tracing::debug!(iteration = i + 1, "ask: no new context found, stopping");
            break;
        }

        facts = graph_context_to_facts(&ctx);
    }

    let answer = state.llm.synthesise_answer(&req.question, &facts, user_display_name).await?;

    Ok(AskResponse {
        answer,
        facts: if req.verbose { Some(facts) } else { None },
        iterations: iterations_used,
    })
}
