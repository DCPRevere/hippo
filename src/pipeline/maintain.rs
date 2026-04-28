use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::events::GraphEvent;
use crate::graph_backend::GraphBackend;
use crate::pipeline::dreamer::{
    Consolidator, DreamReport, Dreamer, DreamerConfig, Inferrer, Linker, Reconciler, WorkerPool,
};
use crate::state::AppState;
use anyhow::Result;
use chrono::Utc;

pub async fn run_maintenance_loop(
    state: std::sync::Arc<AppState>,
    mut shutdown: tokio::sync::watch::Receiver<()>,
) {
    let secs = state.config.pipeline.maintenance_interval_secs;
    if secs == 0 {
        tracing::info!("Maintenance loop disabled (MAINTENANCE_INTERVAL_SECS=0), use POST /maintain to trigger");
        let _ = shutdown.changed().await;
        return;
    }

    let interval = tokio::time::Duration::from_secs(secs);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.changed() => {
                tracing::info!("Maintenance loop stopping (shutdown signal)");
                return;
            }
        }
        let graph = state.graph_registry().get_default().await;
        match run_once_arc(state.clone(), graph).await {
            Ok(report) => {
                tracing::info!(
                    facts = report.facts_visited,
                    links = report.links_written,
                    inferences = report.inferences_written,
                    supersessions = report.supersessions_written,
                    consolidations = report.consolidations_written,
                    "dream complete"
                );
            }
            Err(e) => tracing::error!("Maintenance error: {e}"),
        }
    }
}

pub async fn run_housekeeping(state: &AppState, graph: &dyn GraphBackend) -> Result<()> {
    // Refresh entity/fact gauges
    if let Ok(stats) = graph.graph_stats().await {
        state
            .metrics
            .entity_count
            .store(stats.entity_count as u64, Ordering::Relaxed);
        state
            .metrics
            .fact_count
            .store(stats.edge_count as u64, Ordering::Relaxed);
    }

    // Promote facts from working to long-term memory
    let promoted = graph.promote_working_memory().await?;
    if promoted > 0 {
        tracing::info!(
            "maintenance: promoted {} facts to long-term memory",
            promoted
        );
    }

    // Expire edges that have passed their TTL
    let expired = graph.expire_ttl_edges(Utc::now()).await?;
    if expired > 0 {
        tracing::info!("maintenance: expired {expired} edges past TTL");
    }

    Ok(())
}

/// Run one maintenance cycle. Runs housekeeping (promote / expire / decay),
/// the legacy entity-deduplication batch, then drives the Dreamers via the
/// `WorkerPool`, finally resolving placeholders. Returns the aggregated
/// dream-report.
///
/// Two entry points:
///   - [`run_once`] takes borrowed `&AppState` + `&dyn GraphBackend`. It
///     legacy-shape (the maintenance internals do not need ownership), and
///     skips the Dreamer pool work — useful for tests and contexts that
///     don't want background-pool overhead.
///   - [`run_once_arc`] takes `Arc<AppState>` + `Arc<dyn GraphBackend>` and
///     runs the full Dreamer pipeline. Production callers (the HTTP handler
///     and the maintenance loop) use this.
pub async fn run_once(state: &AppState, graph: &dyn GraphBackend) -> Result<DreamReport> {
    tracing::info!("Running maintenance cycle (borrowed-state path, no Dreamers)");

    run_housekeeping(state, graph).await?;
    drain_recent_nodes(state).await;
    legacy_entity_dedup(state, graph).await?;
    placeholder_resolution(state, graph).await?;

    let _ = state.event_tx.send(GraphEvent::MaintenanceComplete {
        graph: graph.graph_name().to_string(),
    });
    Ok(DreamReport::default())
}

pub async fn run_once_arc(
    state: Arc<AppState>,
    graph: Arc<dyn GraphBackend>,
) -> Result<DreamReport> {
    tracing::info!("Running maintenance cycle (Dreamer pool)");

    run_housekeeping(&state, &*graph).await?;
    drain_recent_nodes(&state).await;
    legacy_entity_dedup(&state, &*graph).await?;

    let tuning = &state.config.pipeline.tuning;
    let dreamer_config = DreamerConfig::bounded(
        tuning.dreamer_worker_count,
        tuning.dreamer_max_units,
        tuning.dreamer_max_tokens,
    );

    let mut total = DreamReport::default();
    for dreamer in dreamers_for_cycle(state.clone(), &state.config) {
        let pool = WorkerPool::new(dreamer_config.clone());
        let report = pool.run_dream(dreamer, graph.clone()).await?;
        total.merge(&report);
    }

    placeholder_resolution(&state, &*graph).await?;

    let _ = state.event_tx.send(GraphEvent::MaintenanceComplete {
        graph: graph.graph_name().to_string(),
    });
    Ok(total)
}

/// Build the Dreamer chain that runs each maintenance cycle. Inferrer and
/// Consolidator are gated by config flags so deployments can opt in.
fn dreamers_for_cycle(
    state: Arc<AppState>,
    config: &crate::config::Config,
) -> Vec<Arc<dyn Dreamer>> {
    let mut out: Vec<Arc<dyn Dreamer>> = vec![
        Arc::new(Linker::new(state.clone())),
        Arc::new(Reconciler::new(state.clone())),
    ];
    if config.pipeline.infer_maintenance {
        out.push(Arc::new(Inferrer::new(state.clone())));
    }
    out.push(Arc::new(Consolidator::new(state)));
    out
}

/// The legacy entity-deduplication batch step. Stays here pending a
/// future port to a dedup-Dreamer.
async fn legacy_entity_dedup(state: &AppState, graph: &dyn GraphBackend) -> Result<()> {
    const BATCH_SIZE: usize = 20;
    let entities = graph.list_entities_by_recency(0, BATCH_SIZE).await?;
    if entities.is_empty() {
        return Ok(());
    }

    let mut duplicate_pairs: Vec<(String, String, String, String, Vec<String>)> = Vec::new();
    let mut pair_ids: Vec<(String, String)> = Vec::new();
    let mut seen_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    for entity in &entities {
        let candidates = graph.fulltext_search_entities(&entity.name).await?;
        for candidate in &candidates {
            if candidate.id == entity.id {
                continue;
            }
            let pair_key = if entity.id < candidate.id {
                (entity.id.clone(), candidate.id.clone())
            } else {
                (candidate.id.clone(), entity.id.clone())
            };
            if !seen_pairs.insert(pair_key) {
                continue;
            }

            if candidate.name.to_lowercase() == entity.name.to_lowercase() {
                let b_facts = graph
                    .get_entity_facts(&candidate.id)
                    .await
                    .unwrap_or_default();
                duplicate_pairs.push((
                    entity.name.clone(),
                    entity.entity_type.clone(),
                    candidate.name.clone(),
                    candidate.entity_type.clone(),
                    b_facts,
                ));
                pair_ids.push((entity.id.clone(), candidate.id.clone()));
            }
        }

        let link_threshold = state.config.pipeline.tuning.link_discovery_cosine_threshold;
        let similar = graph.vector_search_entities(&entity.embedding, 5).await?;
        for (candidate, score) in &similar {
            if candidate.id == entity.id || *score < link_threshold {
                continue;
            }
            let pair_key = if entity.id < candidate.id {
                (entity.id.clone(), candidate.id.clone())
            } else {
                (candidate.id.clone(), entity.id.clone())
            };
            if !seen_pairs.insert(pair_key) {
                continue;
            }
            let b_facts = graph
                .get_entity_facts(&candidate.id)
                .await
                .unwrap_or_default();
            duplicate_pairs.push((
                entity.name.clone(),
                entity.entity_type.clone(),
                candidate.name.clone(),
                candidate.entity_type.clone(),
                b_facts,
            ));
            pair_ids.push((entity.id.clone(), candidate.id.clone()));
        }
    }

    if !duplicate_pairs.is_empty() {
        let results = state.llm.resolve_entities_batch(&duplicate_pairs).await?;
        let cls_threshold = state.config.pipeline.tuning.classification_confidence_threshold;
        for (index, same, confidence) in &results {
            if *same && *confidence > cls_threshold {
                if let Some((from_id, to_id)) = pair_ids.get(*index) {
                    graph.merge_placeholder(from_id, to_id).await?;
                }
            }
        }
    }
    Ok(())
}

async fn drain_recent_nodes(state: &AppState) -> Vec<String> {
    let mut rx = state.recent_nodes_rx.lock().await;
    let mut ids = Vec::with_capacity(50);
    while ids.len() < 50 {
        match rx.try_recv() {
            Ok(id) => ids.push(id),
            Err(_) => break,
        }
    }
    // Update the snapshot buffer so consolidate can read recent IDs
    *state.recent_node_ids.write().await = ids.clone();
    ids
}


async fn placeholder_resolution(state: &AppState, graph: &dyn GraphBackend) -> Result<()> {
    let cutoff = Utc::now() - chrono::Duration::hours(24);
    let placeholders = graph.find_placeholder_nodes(cutoff).await?;

    for placeholder in placeholders {
        // Search for potential matches
        let ft_candidates = graph.fulltext_search_entities(&placeholder.name).await?;
        let vec_candidates = graph
            .vector_search_entities(&placeholder.embedding, 5)
            .await?;

        let all_candidates: Vec<_> = ft_candidates
            .into_iter()
            .chain(vec_candidates.into_iter().map(|(e, _)| e))
            .filter(|e| e.id != placeholder.id && e.resolved)
            .collect();

        for candidate in all_candidates {
            let extracted = crate::models::ExtractedEntity {
                name: placeholder.name.clone(),
                entity_type: placeholder.entity_type.clone(),
                resolved: false,
                hint: placeholder.hint.clone(),
                content: placeholder.content.clone(),
            };
            let candidate_facts = graph
                .get_entity_facts(&candidate.id)
                .await
                .unwrap_or_default();
            let (same, confidence) = state
                .llm
                .resolve_entities(&extracted, &candidate, &candidate_facts)
                .await?;
            if same
                && confidence > state.config.pipeline.tuning.classification_confidence_threshold
            {
                tracing::info!(
                    "Resolving placeholder '{}' -> '{}' (confidence: {:.2})",
                    placeholder.name,
                    candidate.name,
                    confidence
                );
                graph
                    .merge_placeholder(&placeholder.id, &candidate.id)
                    .await?;
                break;
            }
        }

        // Log warning if still unresolved after a week
        let created_at = chrono::DateTime::parse_from_rfc3339(&placeholder.created_at)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or(Utc::now());
        let age_days = (Utc::now() - created_at).num_days();
        if age_days > 7 {
            tracing::warn!(
                "Placeholder '{}' (hint: {:?}) unresolved after {} days",
                placeholder.name,
                placeholder.hint,
                age_days
            );
        }
    }
    Ok(())
}
