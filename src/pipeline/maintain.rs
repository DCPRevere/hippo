use std::sync::atomic::Ordering;

use anyhow::Result;
use chrono::Utc;
use crate::graph_backend::GraphBackend;
use crate::math::cosine_similarity;
use crate::models::Relation;
use crate::state::AppState;

pub async fn run_maintenance_loop(
    state: std::sync::Arc<AppState>,
    mut shutdown: tokio::sync::watch::Receiver<()>,
) {
    let secs = state.config.maintenance_interval_secs;
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
        if let Err(e) = run_once(&state, &*graph).await {
            tracing::error!("Maintenance error: {e}");
        }
    }
}

pub async fn run_housekeeping(state: &AppState, graph: &dyn GraphBackend) -> Result<()> {
    // Refresh entity/fact gauges
    if let Ok((entities, facts, _, _, _)) = graph.graph_stats().await {
        state.metrics.entity_count.store(entities as u64, Ordering::Relaxed);
        state.metrics.fact_count.store(facts as u64, Ordering::Relaxed);
    }

    // Promote facts from working to long-term memory
    let promoted = graph.promote_working_memory().await?;
    if promoted > 0 {
        tracing::info!("maintenance: promoted {} facts to long-term memory", promoted);
    }

    // Purge stale working memory (older than 24h, low salience)
    let cutoff = Utc::now() - chrono::Duration::hours(24);
    let purged = graph.purge_stale_working_memory(cutoff).await?;
    if purged > 0 {
        tracing::info!("maintenance: purged {} stale working memory edges", purged);
    }

    Ok(())
}

pub async fn run_once(
    state: &AppState,
    graph: &dyn GraphBackend,
) -> Result<()> {
    tracing::info!("Running maintenance cycle");

    run_housekeeping(state, graph).await?;

    // Drain any recently-pushed node IDs (not used for gating anymore, just keep channel clear)
    drain_recent_nodes(state).await;

    // Walk all nodes from most recently created
    const BATCH_SIZE: usize = 20;
    let entities = graph.list_entities_by_recency(0, BATCH_SIZE).await?;

    if entities.is_empty() {
        tracing::debug!("No entities in graph");
        return Ok(());
    }

    tracing::info!(count = entities.len(), "maintenance: processing entities");

    // Phase 1: Collect duplicate candidate pairs with IDs (graph queries only, no LLM)
    let mut duplicate_pairs: Vec<(String, String, String, String, Vec<String>)> = Vec::new();
    let mut pair_ids: Vec<(String, String)> = Vec::new();
    let mut seen_pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    for entity in &entities {
        // Search by name
        let candidates = graph.fulltext_search_entities(&entity.name).await?;
        for candidate in &candidates {
            if candidate.id == entity.id { continue; }
            let pair_key = if entity.id < candidate.id {
                (entity.id.clone(), candidate.id.clone())
            } else {
                (candidate.id.clone(), entity.id.clone())
            };
            if !seen_pairs.insert(pair_key) { continue; }

            if candidate.name.to_lowercase() == entity.name.to_lowercase() {
                let b_facts = graph.get_entity_facts(&candidate.id).await.unwrap_or_default();
                duplicate_pairs.push((
                    entity.name.clone(), entity.entity_type.clone(),
                    candidate.name.clone(), candidate.entity_type.clone(),
                    b_facts,
                ));
                pair_ids.push((entity.id.clone(), candidate.id.clone()));
            }
        }

        // Search by embedding similarity
        let similar = graph.vector_search_entities(&entity.embedding, 5).await?;
        for (candidate, score) in &similar {
            if candidate.id == entity.id || *score < 0.85 { continue; }
            let pair_key = if entity.id < candidate.id {
                (entity.id.clone(), candidate.id.clone())
            } else {
                (candidate.id.clone(), entity.id.clone())
            };
            if !seen_pairs.insert(pair_key) { continue; }

            let b_facts = graph.get_entity_facts(&candidate.id).await.unwrap_or_default();
            duplicate_pairs.push((
                entity.name.clone(), entity.entity_type.clone(),
                candidate.name.clone(), candidate.entity_type.clone(),
                b_facts,
            ));
            pair_ids.push((entity.id.clone(), candidate.id.clone()));
        }
    }

    // Phase 2: Single batch LLM call for all duplicate candidates
    if !duplicate_pairs.is_empty() {
        tracing::info!(pairs = duplicate_pairs.len(), "maintenance: checking entity pairs for duplicates");

        let results = state.llm.resolve_entities_batch(&duplicate_pairs).await?;

        for (index, same, confidence) in &results {
            if *same && *confidence > 0.85 {
                if let Some((from_id, to_id)) = pair_ids.get(*index) {
                    let (a_name, _, b_name, _, _) = &duplicate_pairs[*index];
                    tracing::info!(
                        from = %a_name,
                        to = %b_name,
                        confidence,
                        "maintenance: merging duplicate entities"
                    );
                    graph.merge_placeholder(from_id, to_id).await?;
                }
            }
        }
    }

    // Phase 3: Link discovery, contradiction scan, inference, placeholder resolution
    let node_ids: Vec<String> = entities.iter().map(|e| e.id.clone()).collect();
    link_discovery(state, graph, &node_ids).await?;
    contradiction_scan(state, graph, &node_ids).await?;
    if state.config.infer_maintenance {
        inference_scan(state, graph, &node_ids).await?;
    }
    placeholder_resolution(state, graph).await?;

    Ok(())
}

async fn drain_recent_nodes(
    state: &AppState,
) -> Vec<String> {
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

async fn link_discovery(state: &AppState, graph: &dyn GraphBackend, node_ids: &[String]) -> Result<()> {
    for node_id in node_ids {
        let node_entity = match graph.get_entity_by_id(node_id).await? {
            Some(e) => e,
            None => continue,
        };

        let close_nodes = graph
            .find_close_unlinked(node_id, &node_entity.embedding, 0.85)
            .await?;

        for (candidate, _score) in close_nodes {
            // Check if we've already evaluated this pair
            let pair = if node_id < &candidate.id {
                (node_id.clone(), candidate.id.clone())
            } else {
                (candidate.id.clone(), node_id.clone())
            };

            {
                let checked = state.checked_pairs.read().await;
                if checked.contains(&pair) {
                    continue;
                }
            }

            // Get facts for both nodes
            let a_facts = graph.get_entity_facts(node_id).await?;
            let b_facts = graph.get_entity_facts(&candidate.id).await?;

            if let Some((rel_type, fact, confidence)) = state
                .llm
                .discover_link(&node_entity, &candidate, &a_facts, &b_facts)
                .await?
            {
                tracing::info!(
                    "Discovered link: '{}' --[{}]--> '{}'",
                    node_entity.name,
                    rel_type,
                    candidate.name
                );

                let embedding = state.llm.embed(&fact).await?;
                let now = Utc::now();
                let relation = Relation {
                    fact,
                    relation_type: rel_type,
                    embedding,
                    source_agents: vec!["hippo".to_string()],
                    valid_at: now,
                    invalid_at: None,
                    confidence,
                    salience: 0,
                    created_at: now,
                    memory_tier: crate::models::MemoryTier::Working,
                };
                graph.create_edge(node_id, &candidate.id, &relation).await?;
            }

            // Mark pair as checked, prune if too large
            let mut checked = state.checked_pairs.write().await;
            checked.insert(pair);
            if checked.len() > 10_000 {
                let to_remove: Vec<_> = checked.iter().take(5_000).cloned().collect();
                for pair in to_remove {
                    checked.remove(&pair);
                }
            }
        }
    }
    Ok(())
}

async fn contradiction_scan(state: &AppState, graph: &dyn GraphBackend, node_ids: &[String]) -> Result<()> {
    for node_id in node_ids {
        let edges = graph.find_all_active_edges_from(node_id).await?;

        // Group by (to_id, relation_type)
        let mut groups: std::collections::HashMap<(String, String), Vec<_>> =
            std::collections::HashMap::new();
        for edge in edges {
            groups
                .entry((edge.object_id.clone(), edge.relation_type.clone()))
                .or_default()
                .push(edge);
        }

        for ((_, rel_type), group) in groups {
            if group.len() < 2 {
                continue;
            }
            // Compare each pair
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let (classification, _) = state
                        .llm
                        .classify_edge(&group[i].fact, &group[j].fact, &rel_type)
                        .await?;

                    if classification == crate::models::EdgeClassification::Contradiction {
                        // Invalidate the older one
                        let older = if group[i].valid_at < group[j].valid_at {
                            group[i].edge_id
                        } else {
                            group[j].edge_id
                        };
                        tracing::info!("Contradiction scan: invalidating edge {older}");
                        graph.invalidate_edge(older, Utc::now()).await?;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn inference_scan(state: &AppState, graph: &dyn GraphBackend, node_ids: &[String]) -> Result<()> {
    const MAX_PER_CYCLE: usize = 5;

    for node_id in node_ids.iter().take(MAX_PER_CYCLE) {
        let entity = match graph.get_entity_by_id(node_id).await? {
            Some(e) => e,
            None => continue,
        };

        let entity_facts = graph.get_entity_facts(node_id).await?;
        let hop_edges = graph.walk_one_hop(&[node_id.clone()], 20).await?;

        // Build neighbour context — group facts by neighbour name
        let mut neighbour_map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for edge in &hop_edges {
            let neighbour_name = if edge.subject_id == *node_id {
                &edge.object_name
            } else {
                &edge.subject_name
            };
            neighbour_map
                .entry(neighbour_name.clone())
                .or_default()
                .push(edge.fact.clone());
        }
        let neighbor_facts: Vec<(String, Vec<String>)> = neighbour_map.into_iter().collect();

        let inferences = state
            .llm
            .find_missing_inferences(&entity.name, &entity_facts, &neighbor_facts)
            .await?;

        for (rel_type, object_name, fact_text, confidence) in inferences {
            // Resolve the object entity
            let object_entities = graph.fulltext_search_entities(&object_name).await?;
            let object_id = object_entities
                .iter()
                .find(|e| e.name.to_lowercase() == object_name.to_lowercase())
                .map(|e| e.id.clone());

            let object_id = match object_id {
                Some(id) => id,
                None => {
                    tracing::debug!(
                        "inference_scan: object '{}' not found, skipping",
                        object_name
                    );
                    continue;
                }
            };

            // Check for duplicate via embedding similarity
            let embedding = state.llm.embed(&fact_text).await?;
            let existing = graph.find_all_active_edges_from(node_id).await?;
            let is_duplicate = existing.iter().any(|e| {
                if e.embedding.is_empty() {
                    return false;
                }
                cosine_similarity(&embedding, &e.embedding) > 0.9
            });
            if is_duplicate {
                tracing::debug!("inference_scan: duplicate edge, skipping: {}", fact_text);
                continue;
            }

            let now = Utc::now();
            let relation = Relation {
                fact: fact_text.clone(),
                relation_type: rel_type,
                embedding,
                source_agents: vec!["maintenance/inference".to_string()],
                valid_at: now,
                invalid_at: None,
                confidence: confidence * 0.8, // Discount for being inferred
                salience: 0,
                created_at: now,
                memory_tier: crate::models::MemoryTier::Working,
            };
            graph.create_edge(node_id, &object_id, &relation).await?;
            tracing::info!(
                "inference_scan: inferred '{}' for '{}'",
                fact_text,
                entity.name
            );
        }
    }
    Ok(())
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
            let candidate_facts = graph.get_entity_facts(&candidate.id).await.unwrap_or_default();
            let (same, confidence) = state.llm.resolve_entities(&extracted, &candidate, &candidate_facts).await?;
            if same && confidence > 0.85 {
                tracing::info!(
                    "Resolving placeholder '{}' -> '{}' (confidence: {:.2})",
                    placeholder.name,
                    candidate.name,
                    confidence
                );
                graph.merge_placeholder(&placeholder.id, &candidate.id).await?;
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
