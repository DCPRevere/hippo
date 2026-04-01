use anyhow::Result;
use chrono::Utc;
use std::time::Instant;

use crate::graph_backend::GraphBackend;
use crate::models::{
    ConsolidateReport, ConsolidateRequest, ConsolidateResponse, NewLinkReport, PrunedFactReport,
    Relation,
};
use crate::state::AppState;

pub async fn consolidate(state: &AppState, graph: &dyn GraphBackend, req: ConsolidateRequest) -> Result<ConsolidateResponse> {
    let start = Instant::now();

    let max_entity_pairs = req.max_entity_pairs.unwrap_or(30);
    let prune_threshold = req.prune_threshold.unwrap_or(0.05);
    let dry_run = req.dry_run.unwrap_or(false);

    // 1. Run standard housekeeping first (decay, promote, purge)
    if let Err(e) = super::maintain::run_housekeeping(state, graph).await {
        tracing::warn!("Housekeeping pass during consolidation had errors: {e}");
    }

    // 2. Link discovery with priority ordering
    let new_links = discover_links_prioritised(state, graph, max_entity_pairs, dry_run).await?;

    // 3. Prune low-confidence facts
    let pruned_edges = graph
        .archive_low_confidence_edges(prune_threshold, dry_run)
        .await?;

    let pruned_facts: Vec<PrunedFactReport> = pruned_edges
        .iter()
        .map(|e| PrunedFactReport {
            fact: e.fact.clone(),
            reason: format!(
                "confidence {:.2} below threshold {}",
                e.decayed_confidence, prune_threshold
            ),
        })
        .collect();

    // 4. Find entity clusters
    let clusters = graph.find_entity_clusters(2).await?;

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(ConsolidateResponse {
        links_created: new_links.len(),
        facts_pruned: pruned_facts.len(),
        clusters_found: clusters.len(),
        duration_ms,
        dry_run,
        report: ConsolidateReport {
            new_links,
            pruned_facts,
            clusters,
        },
    })
}

async fn discover_links_prioritised(
    state: &AppState,
    graph: &dyn GraphBackend,
    max_pairs: usize,
    dry_run: bool,
) -> Result<Vec<NewLinkReport>> {
    let mut new_links = Vec::new();
    let mut pairs_checked = 0usize;

    // Priority 1: 2-hop connected but no direct edge
    let two_hop_pairs = graph
        .find_two_hop_unlinked_pairs(max_pairs)
        .await?;

    for (a, b) in &two_hop_pairs {
        if pairs_checked >= max_pairs {
            break;
        }
        if let Some(link) = try_discover_link(state, graph,a, b, dry_run).await? {
            new_links.push(link);
        }
        pairs_checked += 1;
    }

    if pairs_checked >= max_pairs {
        return Ok(new_links);
    }

    // Priority 2: High cosine similarity but no direct edge (from recent node snapshot)
    let recent_ids: Vec<String> = {
        let recent = state.recent_node_ids.read().await;
        recent.iter().take(20).cloned().collect()
    };

    for node_id in &recent_ids {
        if pairs_checked >= max_pairs {
            break;
        }
        let node_entity = match graph.get_entity_by_id(node_id).await? {
            Some(e) => e,
            None => continue,
        };

        let close_nodes = graph
            .find_close_unlinked(node_id, &node_entity.embedding, 0.80)
            .await?;

        for (candidate, _score) in close_nodes {
            if pairs_checked >= max_pairs {
                break;
            }
            if let Some(link) =
                try_discover_link(state, graph,&node_entity, &candidate, dry_run).await?
            {
                new_links.push(link);
            }
            pairs_checked += 1;
        }
    }

    Ok(new_links)
}

async fn try_discover_link(
    state: &AppState,
    graph: &dyn GraphBackend,
    a: &crate::models::EntityRow,
    b: &crate::models::EntityRow,
    dry_run: bool,
) -> Result<Option<NewLinkReport>> {
    // Skip if already checked this pair
    let pair = if a.id < b.id {
        (a.id.clone(), b.id.clone())
    } else {
        (b.id.clone(), a.id.clone())
    };

    {
        let checked = state.checked_pairs.read().await;
        if checked.contains(&pair) {
            return Ok(None);
        }
    }

    let a_facts = graph.get_entity_facts(&a.id).await?;
    let b_facts = graph.get_entity_facts(&b.id).await?;

    let result = state
        .llm
        .discover_link(a, b, &a_facts, &b_facts)
        .await?;

    // Mark pair as checked
    {
        let mut checked = state.checked_pairs.write().await;
        checked.insert(pair);
    }

    match result {
        Some((rel_type, fact, confidence)) => {
            tracing::info!(
                "Consolidation discovered link: '{}' --[{}]--> '{}'",
                a.name,
                rel_type,
                b.name
            );

            if !dry_run {
                let embedding = state.llm.embed(&fact).await?;
                let now = Utc::now();
                let relation = Relation {
                    fact: fact.clone(),
                    relation_type: rel_type.clone(),
                    embedding,
                    source_agents: vec!["hippo".to_string()],
                    valid_at: now,
                    invalid_at: None,
                    confidence,
                    salience: 0,
                    created_at: now,
                    memory_tier: crate::models::MemoryTier::Working,
                    expires_at: None,
                };
                graph.create_edge(&a.id, &b.id, &relation).await?;
            }

            Ok(Some(NewLinkReport {
                entity_a: a.name.clone(),
                entity_b: b.name.clone(),
                relation: rel_type,
                confidence,
            }))
        }
        None => Ok(None),
    }
}
