use std::sync::atomic::Ordering;

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use std::collections::{HashMap, HashSet};

use crate::events::GraphEvent;
use crate::graph_backend::GraphBackend;
use crate::math::cosine_similarity;
use crate::models::{
    Entity, GraphOp, LlmUsage, MemoryTier, OpExecutionTrace, Relation,
    RememberProgress, RememberRequest, RememberResponse, RememberTrace,
};
use crate::state::AppState;

// Re-export subgraph context types from models (canonical location).
pub use crate::models::{GraphContext, SubgraphEdge, SubgraphNode};

async fn send_progress(tx: &Option<tokio::sync::mpsc::Sender<RememberProgress>>, event: RememberProgress) {
    if let Some(tx) = tx {
        let _ = tx.send(event).await;
    }
}

pub async fn remember(
    state: &AppState,
    graph: &dyn GraphBackend,
    req: RememberRequest,
    progress_tx: Option<tokio::sync::mpsc::Sender<RememberProgress>>,
    user_id: Option<&str>,
) -> Result<RememberResponse> {
    state.metrics.remember_calls_total.fetch_add(1, Ordering::Relaxed);

    let source_agent = req.source_agent.as_deref().unwrap_or("unknown").to_string();
    let now = Utc::now();

    let ttl_secs = req.ttl_secs.or(state.config.pipeline.default_ttl_secs);
    let expires_at = ttl_secs.map(|s| now + chrono::Duration::seconds(s as i64));

    // Look up source credibility
    let recorded_cred = state.credibility.read().await.get(&source_agent);
    let src_cred = match req.source_credibility_hint {
        Some(h) => recorded_cred.max(h.min(1.0)),
        None => recorded_cred,
    };

    let mut usage = LlmUsage::default();

    // Step 1: Search graph for context
    tracing::info!(graph = %graph.graph_name(), statement = %req.statement, "remember: starting");
    let context = if state.config.pipeline.infer_pre_context {
        let ctx = gather_pre_extraction_context(state, graph, &req.statement, user_id).await?;
        usage.embed_calls += 1; // embed in gather_pre_extraction_context
        tracing::info!(
            graph = %graph.graph_name(),
            nodes = ctx.node_count(),
            edges = ctx.edge_count(),
            "remember: gathered context"
        );
        send_progress(&progress_tx, RememberProgress::ContextGathered {
            entities_found: ctx.node_count(),
            edges_found: ctx.edge_count(),
        }).await;
        ctx
    } else {
        GraphContext::empty()
    };

    // Step 2: Ask LLM for graph operations
    tracing::info!(statement = %req.statement, "remember: planning operations");
    send_progress(&progress_tx, RememberProgress::Planning).await;

    let mut ops_result = state.llm.extract_operations(&req.statement, &context).await?;
    usage.llm_calls += 1;
    tracing::info!(operations = ops_result.operations.len(), "remember: planned");
    send_progress(&progress_tx, RememberProgress::Planned {
        operations: ops_result.operations.len(),
    }).await;

    let original_operations = ops_result.operations.clone();

    // Build known IDs map from context before any moves
    let mut known_ids: HashMap<String, String> = HashMap::new();
    for node in &context.nodes {
        known_ids.insert(node.name.clone(), node.id.clone());
        known_ids.insert(node.id.clone(), node.id.clone());
    }

    // Step 3: If enrichment enabled, search graph by extracted entity names for missed context
    let mut was_revised = false;
    if state.config.pipeline.infer_enrichment {
        let entity_names: Vec<String> = ops_result.operations.iter().filter_map(|op| {
            match op {
                GraphOp::CreateNode { name, .. } => Some(name.clone()),
                _ => None,
            }
        }).collect();

        if !entity_names.is_empty() {
            let additional = gather_context_by_names(graph, &entity_names, &context).await?;
            if !additional.is_empty() {
                tracing::info!(
                    new_nodes = additional.node_count(),
                    "remember: found additional context, revising operations"
                );
                send_progress(&progress_tx, RememberProgress::Revising {
                    new_context_entities: additional.node_count(),
                }).await;

                // Merge original + additional context for revision
                let mut merged = context;
                let existing_ids: HashSet<String> = merged.nodes.iter().map(|n| n.id.clone()).collect();
                for node in additional.nodes {
                    if !existing_ids.contains(&node.id) {
                        known_ids.insert(node.name.clone(), node.id.clone());
                        known_ids.insert(node.id.clone(), node.id.clone());
                        merged.nodes.push(node);
                    }
                }
                merged.edges.extend(additional.edges);

                ops_result = state.llm.revise_operations(&ops_result, &merged).await?;
                usage.llm_calls += 1;
                was_revised = true;
                tracing::info!(operations = ops_result.operations.len(), "remember: revised");
            }
        }
    }

    // Step 4: Execute operations
    let mut entities_created = 0usize;
    let mut entities_updated = 0usize;
    let mut facts_written = 0usize;
    let mut facts_invalidated = 0usize;
    let mut execution_trace = Vec::new();

    // Map "new:<name>" references to actual IDs created in this batch
    let mut new_node_ids: HashMap<String, String> = HashMap::new();

    for op in &ops_result.operations {
        send_progress(&progress_tx, RememberProgress::Executing {
            op: format!("{:?}", op),
        }).await;

        match op {
            GraphOp::CreateNode { node_ref, name, node_type, properties } => {
                // Check if entity already exists by exact name match
                let existing = graph.fulltext_search_entities(name).await?;
                if let Some(found) = existing.iter().find(|e| e.name.to_lowercase() == name.to_lowercase()) {
                    tracing::info!(name = %name, "remember: node already exists, skipping create");
                    new_node_ids.insert(format!("new:{name}"), found.id.clone());
                    if let Some(r) = node_ref {
                        new_node_ids.insert(r.clone(), found.id.clone());
                    }
                    known_ids.insert(name.clone(), found.id.clone());

                    // Apply properties as update
                    if !properties.is_empty() {
                        for (key, value) in properties {
                            graph.set_entity_property(&found.id, key, value).await?;
                        }
                    }

                    execution_trace.push(OpExecutionTrace {
                        op: "create_node".to_string(),
                        outcome: "resolved_existing".to_string(),
                        details: Some(format!("{name} → {}", found.id)),
                    });
                    continue;
                }

                let embedding = state.llm.embed(name).await?;
                usage.embed_calls += 1;
                let id = Uuid::new_v4().to_string();
                let entity = Entity {
                    id: id.clone(),
                    name: name.clone(),
                    entity_type: node_type.clone(),
                    resolved: true,
                    hint: None,
                    content: None,
                    created_at: now,
                    embedding,
                };
                graph.upsert_entity(&entity).await?;

                // If the LLM created a "Principal" node or set user_id/is_principal,
                // tag the entity with the authenticated user's ID.
                let is_user_entity = name.to_lowercase() == "principal"
                    || properties.iter().any(|(k, v)| {
                        (k == "is_principal" && v == "true")
                            || k == "user_id"
                    });
                if is_user_entity {
                    if let Some(uid) = user_id {
                        graph.set_entity_property(&id, "user_id", uid).await?;
                    } else {
                        // Legacy fallback: no user_id available, use is_principal
                        graph.set_entity_property(&id, "is_principal", "true").await?;
                    }
                }

                for (key, value) in properties {
                    graph.set_entity_property(&id, key, value).await?;
                }

                new_node_ids.insert(format!("new:{name}"), id.clone());
                if let Some(r) = node_ref {
                    new_node_ids.insert(r.clone(), id.clone());
                }
                known_ids.insert(name.clone(), id.clone());
                let _ = state.recent_nodes_tx.try_send(id.clone());
                entities_created += 1;

                tracing::info!(name = %name, id = %id, "remember: created node");
                let _ = state.event_tx.send(GraphEvent::EntityCreated {
                    id: id.clone(),
                    name: name.clone(),
                    entity_type: node_type.clone(),
                    graph: graph.graph_name().to_string(),
                });
                execution_trace.push(OpExecutionTrace {
                    op: "create_node".to_string(),
                    outcome: "created".to_string(),
                    details: Some(format!("{name} → {id}")),
                });
            }

            GraphOp::UpdateNode { id, set } => {
                for (key, value) in set {
                    graph.set_entity_property(id, key, value).await?;
                }
                entities_updated += 1;
                let _ = state.recent_nodes_tx.try_send(id.clone());

                tracing::info!(id = %id, "remember: updated node");
                execution_trace.push(OpExecutionTrace {
                    op: "update_node".to_string(),
                    outcome: "updated".to_string(),
                    details: Some(format!("id={id}, set {:?}", set)),
                });
            }

            GraphOp::CreateEdge { from, to, relation, fact, confidence } => {
                let from_id = resolve_ref(from, &new_node_ids, &known_ids);
                let to_id = resolve_ref(to, &new_node_ids, &known_ids);

                let (from_id, to_id) = match (from_id, to_id) {
                    (Some(f), Some(t)) => (f, t),
                    _ => {
                        tracing::warn!(from = %from, to = %to, "remember: unresolved edge reference, skipping");
                        execution_trace.push(OpExecutionTrace {
                            op: "create_edge".to_string(),
                            outcome: "skipped".to_string(),
                            details: Some(format!("unresolved: {from} → {to}")),
                        });
                        continue;
                    }
                };

                // Embed and check for duplicate by cosine similarity
                let embedding = state.llm.embed(fact).await?;
                usage.embed_calls += 1;
                let existing = graph.find_all_active_edges_from(&from_id).await?;
                let is_dup = existing.iter().any(|e| {
                    cosine_similarity(&embedding, &e.embedding) > 0.9
                });

                if is_dup {
                    tracing::info!(fact = %fact, "remember: duplicate edge (by embedding), skipping");
                    execution_trace.push(OpExecutionTrace {
                        op: "create_edge".to_string(),
                        outcome: "duplicate".to_string(),
                        details: Some(fact.clone()),
                    });
                    continue;
                }

                let weighted_conf = confidence * src_cred;
                let rel = Relation {
                    fact: fact.clone(),
                    relation_type: relation.clone(),
                    embedding,
                    source_agents: vec![source_agent.clone()],
                    valid_at: now,
                    invalid_at: None,
                    confidence: weighted_conf,
                    salience: 0,
                    created_at: now,
                    memory_tier: MemoryTier::Working,
                    expires_at,
                };

                let edge_id = graph.create_edge(&from_id, &to_id, &rel).await?;
                facts_written += 1;

                tracing::info!(fact = %fact, relation = %relation, "remember: wrote edge");
                let _ = state.event_tx.send(GraphEvent::EdgeCreated {
                    edge_id,
                    from_name: from.clone(),
                    to_name: to.clone(),
                    fact: fact.clone(),
                    relation_type: relation.clone(),
                    graph: graph.graph_name().to_string(),
                });
                execution_trace.push(OpExecutionTrace {
                    op: "create_edge".to_string(),
                    outcome: "written".to_string(),
                    details: Some(fact.clone()),
                });
            }

            GraphOp::InvalidateEdge { edge_id, fact, reason } => {
                let mut invalidated = false;

                // Prefer direct edge_id if provided
                if let Some(eid) = edge_id {
                    graph.invalidate_edge(*eid, now).await?;
                    tracing::info!(edge_id = %eid, reason = %reason, "remember: invalidated edge by id");
                    let _ = state.event_tx.send(GraphEvent::EdgeInvalidated {
                        edge_id: *eid,
                        fact: fact.clone().unwrap_or_default(),
                        graph: graph.graph_name().to_string(),
                    });
                    invalidated = true;
                    facts_invalidated += 1;
                } else if let Some(fact_text) = fact {
                    // Fallback: find by fact text similarity
                    let embedding = state.llm.embed(fact_text).await?;
                    usage.embed_calls += 1;
                    let candidates = graph.vector_search_edges_scored(&embedding, 5, None).await?;
                    for (edge, score) in &candidates {
                        if *score > 0.85 && edge.invalid_at.is_none() {
                            graph.invalidate_edge(edge.edge_id, now).await?;
                            tracing::info!(fact = %edge.fact, reason = %reason, "remember: invalidated edge by similarity");
                            let _ = state.event_tx.send(GraphEvent::EdgeInvalidated {
                                edge_id: edge.edge_id,
                                fact: edge.fact.clone(),
                                graph: graph.graph_name().to_string(),
                            });
                            invalidated = true;
                            facts_invalidated += 1;
                            break;
                        }
                    }
                }

                execution_trace.push(OpExecutionTrace {
                    op: "invalidate_edge".to_string(),
                    outcome: if invalidated { "invalidated" } else { "not_found" }.to_string(),
                    details: Some(format!("{} — {reason}", edge_id.map(|id| format!("edge_id={id}")).or(fact.clone()).unwrap_or_default())),
                });
            }
        }
    }

    state.metrics.remember_facts_written.fetch_add(facts_written as u64, Ordering::Relaxed);
    state.metrics.remember_contradictions.fetch_add(facts_invalidated as u64, Ordering::Relaxed);

    let response = RememberResponse {
        entities_created,
        entities_resolved: entities_updated,
        facts_written,
        contradictions_invalidated: facts_invalidated,
        usage,
        trace: RememberTrace {
            operations: original_operations,
            revised_operations: if was_revised { Some(ops_result.operations.clone()) } else { None },
            execution: execution_trace,
        },
    };
    send_progress(&progress_tx, RememberProgress::Complete(response.clone())).await;
    let _ = state.event_tx.send(GraphEvent::RememberComplete {
        graph: graph.graph_name().to_string(),
        entities_created,
        facts_written,
        contradictions_invalidated: facts_invalidated,
    });
    Ok(response)
}

fn resolve_ref(reference: &str, ref_ids: &HashMap<String, String>, known_ids: &HashMap<String, String>) -> Option<String> {
    // Try ref lookup (e.g. "n1")
    if let Some(id) = ref_ids.get(reference) {
        return Some(id.clone());
    }
    // Try "new:<name>" for backward compat
    let new_key = format!("new:{reference}");
    if let Some(id) = ref_ids.get(&new_key) {
        return Some(id.clone());
    }
    // Try direct ID
    if let Some(id) = known_ids.get(reference) {
        return Some(id.clone());
    }
    // Try as a name (case-insensitive)
    let ref_lower = reference.to_lowercase();
    for (name, id) in known_ids {
        if name.to_lowercase() == ref_lower {
            return Some(id.clone());
        }
    }
    None
}


pub async fn gather_pre_extraction_context(
    state: &AppState,
    graph: &dyn GraphBackend,
    statement: &str,
    user_id: Option<&str>,
) -> Result<GraphContext> {
    gather_pre_extraction_context_at(state, graph, statement, None, user_id).await
}

pub async fn gather_pre_extraction_context_at(
    state: &AppState,
    graph: &dyn GraphBackend,
    statement: &str,
    at: Option<DateTime<Utc>>,
    user_id: Option<&str>,
) -> Result<GraphContext> {
    let mut seen_node_ids: HashSet<String> = HashSet::new();
    let mut nodes: Vec<SubgraphNode> = Vec::new();
    let mut edges: Vec<SubgraphEdge> = Vec::new();
    let mut seen_edge_ids: HashSet<i64> = HashSet::new();

    // Helper: add a node if not already seen
    let mut add_node = |id: &str, name: &str, etype: &str, node_user_id: Option<&str>,
                        seen: &mut HashSet<String>, nodes: &mut Vec<SubgraphNode>| {
        if seen.insert(id.to_string()) {
            nodes.push(SubgraphNode {
                id: id.to_string(),
                name: name.to_string(),
                node_type: etype.to_string(),
                properties: HashMap::new(),
                user_id: node_user_id.map(|s| s.to_string()),
            });
        }
    };

    // Helper: add an edge and its endpoint nodes
    let mut collect_edge = |edge: &crate::models::EdgeRow,
                            seen_nodes: &mut HashSet<String>,
                            nodes: &mut Vec<SubgraphNode>,
                            seen_edges: &mut HashSet<i64>,
                            edges: &mut Vec<SubgraphEdge>| {
        if !seen_edges.insert(edge.edge_id) {
            return;
        }
        add_node(&edge.subject_id, &edge.subject_name, "", None, seen_nodes, nodes);
        add_node(&edge.object_id, &edge.object_name, "", None, seen_nodes, nodes);
        edges.push(SubgraphEdge {
            id: edge.edge_id,
            from: edge.subject_id.clone(),
            to: edge.object_id.clone(),
            relation: edge.relation_type.clone(),
            fact: edge.fact.clone(),
            confidence: edge.confidence,
        });
    };

    // 0. Always include the user's entity if it exists (survives renames).
    //    Prefer user_id property; fall back to is_principal for legacy data.
    let mut principal_id: Option<String> = None;

    // Try user_id-based lookup first
    let user_entity = if let Some(uid) = user_id {
        graph.find_entity_by_property("user_id", uid).await
    } else {
        Ok(None)
    };

    match user_entity {
        Ok(Some(p)) => {
            tracing::debug!(name = %p.name, id = %p.id, "context: found user entity by user_id");
            add_node(&p.id, &p.name, &p.entity_type, user_id, &mut seen_node_ids, &mut nodes);
            principal_id = Some(p.id.clone());
        }
        _ => {
            // Fallback: legacy is_principal property
            match graph.find_entity_by_property("is_principal", "true").await {
                Ok(Some(p)) => {
                    tracing::debug!(name = %p.name, id = %p.id, "context: found principal (legacy)");
                    add_node(&p.id, &p.name, &p.entity_type, user_id, &mut seen_node_ids, &mut nodes);
                    principal_id = Some(p.id.clone());
                }
                Ok(None) => {
                    tracing::debug!("context: no principal by property, falling back to name search");
                    let principal_candidates = graph.fulltext_search_entities("Principal").await?;
                    for e in &principal_candidates {
                        if e.name.to_lowercase() == "principal" {
                            add_node(&e.id, &e.name, &e.entity_type, user_id, &mut seen_node_ids, &mut nodes);
                            principal_id = Some(e.id.clone());
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("context: find_entity_by_property failed: {e}");
                }
            }
        }
    }
    if principal_id.is_none() {
        tracing::warn!("context: no user entity found — first-person references will not resolve to an existing entity");
    }

    // 1. Fulltext search for entities mentioned in the statement
    let ft_entities = graph.fulltext_search_entities(statement).await?;
    tracing::debug!(count = ft_entities.len(), "context: fulltext search results");
    let mut matched_ids: Vec<String> = Vec::new();
    for e in &ft_entities {
        add_node(&e.id, &e.name, &e.entity_type, None, &mut seen_node_ids, &mut nodes);
        matched_ids.push(e.id.clone());
    }
    // Also walk from the user's entity
    if let Some(ref pid) = principal_id {
        if !matched_ids.contains(pid) {
            matched_ids.push(pid.clone());
        }
    }

    // 2. Vector search for semantically related edges
    let embedding = state.llm.embed(statement).await?;
    let vec_edges = graph.vector_search_edges_scored(&embedding, 10, at).await?;
    tracing::debug!(count = vec_edges.len(), "context: vector search edges");
    for (edge, _) in &vec_edges {
        collect_edge(edge, &mut seen_node_ids, &mut nodes, &mut seen_edge_ids, &mut edges);
    }

    // 3. Walk 1-hop from matched entities (excluding the principal to avoid pulling the entire graph)
    let non_principal_ids: Vec<String> = matched_ids.iter()
        .filter(|id| principal_id.as_ref() != Some(id))
        .cloned()
        .collect();
    if !non_principal_ids.is_empty() {
        let hop_results = graph.walk_n_hops(&non_principal_ids, 1, 30, at).await?;
        let hop_edges: Vec<_> = hop_results.into_iter().map(|(e, _)| e).collect();
        tracing::debug!(count = hop_edges.len(), from = non_principal_ids.len(), "context: 1-hop edges (non-principal)");
        for edge in &hop_edges {
            collect_edge(edge, &mut seen_node_ids, &mut nodes, &mut seen_edge_ids, &mut edges);
        }
    } else {
        tracing::debug!("context: no non-principal matched ids, skipping hop walks");
    }

    tracing::debug!(nodes = nodes.len(), edges = edges.len(), principal = ?principal_id, "context: final");
    Ok(GraphContext { nodes, edges, principal_id })
}

/// Search graph by entity names to find context that wasn't in the initial search.
async fn gather_context_by_names(
    graph: &dyn GraphBackend,
    names: &[String],
    existing: &GraphContext,
) -> Result<GraphContext> {
    let existing_ids: HashSet<String> = existing.nodes.iter().map(|n| n.id.clone()).collect();
    let mut seen_node_ids: HashSet<String> = existing_ids;
    let mut nodes: Vec<SubgraphNode> = Vec::new();
    let mut edges: Vec<SubgraphEdge> = Vec::new();
    let mut seen_edge_ids: HashSet<i64> = HashSet::new();

    for name in names {
        let entities = graph.fulltext_search_entities(name).await?;
        for e in &entities {
            if seen_node_ids.insert(e.id.clone()) {
                nodes.push(SubgraphNode {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    node_type: e.entity_type.clone(),
                    properties: HashMap::new(),
                    user_id: None,
                });
                let hop_results = graph.walk_n_hops(&[e.id.clone()], 1, 20, None).await?;
                let hop_edges: Vec<_> = hop_results.into_iter().map(|(e, _)| e).collect();
                for edge in &hop_edges {
                    if seen_edge_ids.insert(edge.edge_id) {
                        if seen_node_ids.insert(edge.subject_id.clone()) {
                            nodes.push(SubgraphNode {
                                id: edge.subject_id.clone(),
                                name: edge.subject_name.clone(),
                                node_type: String::new(),
                                properties: HashMap::new(),
                                user_id: None,
                            });
                        }
                        if seen_node_ids.insert(edge.object_id.clone()) {
                            nodes.push(SubgraphNode {
                                id: edge.object_id.clone(),
                                name: edge.object_name.clone(),
                                node_type: String::new(),
                                properties: HashMap::new(),
                                user_id: None,
                            });
                        }
                        edges.push(SubgraphEdge {
                            id: edge.edge_id,
                            from: edge.subject_id.clone(),
                            to: edge.object_id.clone(),
                            relation: edge.relation_type.clone(),
                            fact: edge.fact.clone(),
                            confidence: edge.confidence,
                        });
                    }
                }
            }
        }
    }

    Ok(GraphContext { nodes, edges, principal_id: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn resolve_ref_finds_by_ref_id() {
        let mut ref_ids = HashMap::new();
        ref_ids.insert("n1".to_string(), "uuid-123".to_string());
        let known = HashMap::new();
        assert_eq!(resolve_ref("n1", &ref_ids, &known), Some("uuid-123".into()));
    }

    #[test]
    fn resolve_ref_finds_by_new_prefix() {
        let mut ref_ids = HashMap::new();
        ref_ids.insert("new:Alice".to_string(), "uuid-alice".to_string());
        let known = HashMap::new();
        assert_eq!(resolve_ref("Alice", &ref_ids, &known), Some("uuid-alice".into()));
    }

    #[test]
    fn resolve_ref_finds_by_known_id() {
        let ref_ids = HashMap::new();
        let mut known = HashMap::new();
        known.insert("alice-uuid".to_string(), "alice-uuid".to_string());
        assert_eq!(resolve_ref("alice-uuid", &ref_ids, &known), Some("alice-uuid".into()));
    }

    #[test]
    fn resolve_ref_case_insensitive_name_match() {
        let ref_ids = HashMap::new();
        let mut known = HashMap::new();
        known.insert("Alice".to_string(), "uuid-alice".to_string());
        assert_eq!(resolve_ref("alice", &ref_ids, &known), Some("uuid-alice".into()));
        assert_eq!(resolve_ref("ALICE", &ref_ids, &known), Some("uuid-alice".into()));
    }

    #[test]
    fn resolve_ref_returns_none_for_unknown() {
        let ref_ids = HashMap::new();
        let known = HashMap::new();
        assert_eq!(resolve_ref("nobody", &ref_ids, &known), None);
    }
}
