//! Graph-data operations: `/seed`, `/admin/backup`, `/admin/restore`,
//! `/graphs`, `/graphs/drop/{name}`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::auth::Auth;
use crate::error::AppError;
use crate::http::{internal, json_ok, JsonOk};
use crate::models::{
    AdminSeedRequest, AdminSeedResponse, BackupEntity, BackupPayload, BackupRequest,
    RestoreRequest,
};
use crate::state::AppState;

pub(crate) async fn seed_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Json(req): Json<AdminSeedRequest>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    use crate::math::pseudo_embed;
    use crate::models::{Entity, MemoryTier, Relation};
    use chrono::Utc;

    let graph = state
        .resolve_graph_for_user(req.graph.as_deref(), &user)
        .await?;

    let mut entities_created = 0usize;
    let mut edges_created = 0usize;

    for e in &req.entities {
        let embedding = pseudo_embed(&e.name);
        let entity = Entity {
            id: e.id.clone(),
            name: e.name.clone(),
            entity_type: e.entity_type.clone(),
            resolved: e.resolved,
            hint: e.hint.clone(),
            content: None,
            created_at: Utc::now(),
            embedding,
        };
        graph
            .upsert_entity(&entity)
            .await
            .map_err(internal("seed entity"))?;
        entities_created += 1;
    }

    for edge in &req.edges {
        let embedding = pseudo_embed(&edge.fact);
        let valid_at = edge
            .valid_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let tier = match edge.memory_tier.as_str() {
            "working" => MemoryTier::Working,
            _ => MemoryTier::LongTerm,
        };
        let source_agents: Vec<String> = edge
            .source_agents
            .split('|')
            .map(|s| s.to_string())
            .collect();
        let relation = Relation {
            fact: edge.fact.clone(),
            relation_type: edge.relation_type.clone(),
            embedding,
            source_agents,
            valid_at,
            invalid_at: None,
            confidence: edge.confidence,
            salience: edge.salience,
            created_at: valid_at,
            memory_tier: tier,
            expires_at: None,
        };
        graph
            .create_edge(&edge.subject_id, &edge.object_id, &relation)
            .await
            .map_err(internal("seed edge"))?;
        edges_created += 1;
    }

    state.emit_audit(
        &user.user_id,
        "seed",
        format!("entities: {entities_created}, edges: {edges_created}"),
    );

    json_ok(AdminSeedResponse {
        entities_created,
        edges_created,
    })
}

// -- Admin backup/restore -----------------------------------------------------

pub(crate) async fn backup_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Json(req): Json<BackupRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph = state
        .resolve_graph_for_user(req.graph.as_deref(), &user)
        .await?;
    let entities = graph.dump_all_entities().await?;
    let edges = graph.dump_all_edges().await?;

    let backup_entities: Vec<BackupEntity> = entities
        .iter()
        .map(|e| BackupEntity {
            id: e.id.clone(),
            name: e.name.clone(),
            entity_type: e.entity_type.clone(),
            resolved: e.resolved,
            hint: e.hint.clone(),
        })
        .collect();

    let backup_edges: Vec<crate::models::AdminSeedEdge> = edges
        .iter()
        .map(|e| crate::models::AdminSeedEdge {
            subject_id: e.subject_id.clone(),
            object_id: e.object_id.clone(),
            fact: e.fact.clone(),
            relation_type: e.relation_type.clone(),
            confidence: e.confidence,
            salience: e.salience,
            valid_at: Some(e.valid_at.clone()),
            source_agents: e.source_agents.clone(),
            memory_tier: e.memory_tier.clone(),
        })
        .collect();

    let payload = BackupPayload {
        graph: graph.graph_name().to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        entities: backup_entities,
        edges: backup_edges,
    };

    let body = serde_json::to_string_pretty(&payload).map_err(|e| {
        tracing::error!("backup serialisation failed: {e}");
        AppError::internal("backup serialisation failed")
    })?;
    Ok((
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            (
                "content-disposition",
                "attachment; filename=\"backup.json\"",
            ),
        ],
        body,
    ))
}

pub(crate) async fn restore_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Json(req): Json<RestoreRequest>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    // Determine target graph: explicit target_graph, or the graph name from the backup
    let target = req.target_graph.as_deref().unwrap_or(&req.graph);
    let graph = state.resolve_graph_for_user(Some(target), &user).await?;

    use crate::math::pseudo_embed;
    use crate::models::{Entity, MemoryTier, Relation};
    use chrono::Utc;

    let mut entities_created = 0usize;
    let mut edges_created = 0usize;

    // Upsert entities
    for e in &req.entities {
        let embedding = pseudo_embed(&e.name);
        let entity = Entity {
            id: e.id.clone(),
            name: e.name.clone(),
            entity_type: e.entity_type.clone(),
            resolved: e.resolved,
            hint: e.hint.clone(),
            content: None,
            created_at: Utc::now(),
            embedding,
        };
        graph
            .upsert_entity(&entity)
            .await
            .map_err(internal("seed entity"))?;
        entities_created += 1;
    }

    // Create edges (same logic as seed_handler)
    for edge in &req.edges {
        let embedding = pseudo_embed(&edge.fact);
        let valid_at = edge
            .valid_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let tier = match edge.memory_tier.as_str() {
            "working" => MemoryTier::Working,
            _ => MemoryTier::LongTerm,
        };
        let source_agents: Vec<String> = edge
            .source_agents
            .split('|')
            .map(|s| s.to_string())
            .collect();
        let relation = Relation {
            fact: edge.fact.clone(),
            relation_type: edge.relation_type.clone(),
            embedding,
            source_agents,
            valid_at,
            invalid_at: None,
            confidence: edge.confidence,
            salience: edge.salience,
            created_at: valid_at,
            memory_tier: tier,
            expires_at: None,
        };
        graph
            .create_edge(&edge.subject_id, &edge.object_id, &relation)
            .await
            .map_err(internal("seed edge"))?;
        edges_created += 1;
    }

    json_ok(crate::models::AdminSeedResponse {
        entities_created,
        edges_created,
    })
}

pub(crate) async fn graphs_list_handler(
    State(state): State<Arc<AppState>>,
    Auth(_user): Auth,
) -> Result<JsonOk, AppError> {
    let graphs = state.graph_registry().list().await;
    json_ok(serde_json::json!({
        "default": state.graph_registry().default_graph_name(),
        "graphs": graphs,
    }))
}

pub(crate) async fn graphs_drop_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(name): Path<String>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }
    state.graph_registry().drop_graph(&name).await?;

    // Clear in-memory state when dropping the default graph
    if name == state.graph_registry().default_graph_name() {
        state.recent_node_ids.write().await.clear();
        state.checked_pairs.write().await.clear();
        state.credibility.write().await.clear();
        state.metrics.reset();
    }

    state.emit_audit(&user.user_id, "graph.drop", format!("graph: {name}"));

    json_ok(
        serde_json::json!({ "ok": true, "message": format!("Graph '{name}' dropped and reinitialised") }),
    )
}
