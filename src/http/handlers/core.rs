//! Core pipeline handlers: `/remember`, `/context`, `/ask`, REST resources,
//! and graph dump.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::Auth;
use crate::error::AppError;
use crate::http::{internal, json_ok, GraphQuery, JsonOk, ValidJson};
use crate::models::{
    AskRequest, BatchRememberRequest, BatchRememberResponse, BatchRememberResult, ContextRequest,
    RememberRequest,
};
use crate::pipeline::{ask, maintain, remember};
use crate::state::AppState;

pub(crate) async fn remember_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<RememberRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(req.graph.as_deref(), &user)
        .await?;
    state.emit_audit(
        &user.user_id,
        "remember",
        format!(
            "statement: {}",
            &req.statement[..80.min(req.statement.len())]
        ),
    );
    let resp = remember::remember(&state, &*graph, req, None, Some(&user.user_id)).await?;
    json_ok(resp)
}

pub(crate) async fn remember_batch_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<BatchRememberRequest>,
) -> Result<JsonOk, AppError> {
    let total = req.statements.len();
    let source_agent = req.source_agent.clone();
    let graph = state
        .resolve_graph_for_user(req.graph.as_deref(), &user)
        .await?;

    let user_id = user.user_id.clone();
    let results = if req.parallel {
        let futs: Vec<_> = req
            .statements
            .into_iter()
            .map(|statement| {
                let state = Arc::clone(&state);
                let source_agent = source_agent.clone();
                let graph = graph.clone();
                let uid = user_id.clone();
                async move {
                    let remember_req = RememberRequest {
                        statement: statement.clone(),
                        source_agent,
                        source_credibility_hint: None,
                        graph: None,
                        ttl_secs: req.ttl_secs,
                    };
                    match remember::remember(&state, &*graph, remember_req, None, Some(&uid)).await
                    {
                        Ok(resp) => BatchRememberResult {
                            statement,
                            ok: true,
                            facts_written: Some(resp.facts_written),
                            entities_created: Some(resp.entities_created),
                            error: None,
                        },
                        Err(e) => BatchRememberResult {
                            statement,
                            ok: false,
                            facts_written: None,
                            entities_created: None,
                            error: Some(e.to_string()),
                        },
                    }
                }
            })
            .collect();
        futures::future::join_all(futs).await
    } else {
        let mut results = Vec::with_capacity(total);
        for statement in req.statements {
            let remember_req = RememberRequest {
                statement: statement.clone(),
                source_agent: source_agent.clone(),
                source_credibility_hint: None,
                graph: None,
                ttl_secs: req.ttl_secs,
            };
            let result =
                match remember::remember(&state, &*graph, remember_req, None, Some(&user_id)).await
                {
                    Ok(resp) => BatchRememberResult {
                        statement,
                        ok: true,
                        facts_written: Some(resp.facts_written),
                        entities_created: Some(resp.entities_created),
                        error: None,
                    },
                    Err(e) => BatchRememberResult {
                        statement,
                        ok: false,
                        facts_written: None,
                        entities_created: None,
                        error: Some(e.to_string()),
                    },
                };
            results.push(result);
        }
        results
    };

    let succeeded = results.iter().filter(|r| r.ok).count();
    let failed = results.iter().filter(|r| !r.ok).count();

    json_ok(BatchRememberResponse {
        total,
        succeeded,
        failed,
        results,
    })
}

pub(crate) async fn context_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<ContextRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(req.graph.as_deref(), &user)
        .await?;
    let ctx = remember::gather_pre_extraction_context_at(
        &state,
        &*graph,
        &req.query,
        req.at,
        Some(&user.user_id),
    )
    .await?;
    json_ok(ctx)
}

pub(crate) async fn ask_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<AskRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(req.graph.as_deref(), &user)
        .await?;
    let resp = ask::ask(
        &state,
        &*graph,
        req,
        Some(&user.user_id),
        Some(&user.display_name),
    )
    .await?;
    json_ok(resp)
}

// -- REST resources -----------------------------------------------------------

pub(crate) async fn entity_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<String>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(params.graph.as_deref(), &user)
        .await?;
    match graph.get_entity_by_id(&id).await? {
        Some(entity) => json_ok(entity),
        None => Err(AppError::not_found(format!("entity '{id}' not found"))),
    }
}

pub(crate) async fn entity_edges_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<String>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(params.graph.as_deref(), &user)
        .await?;
    // Verify entity exists
    if graph.get_entity_by_id(&id).await?.is_none() {
        return Err(AppError::not_found(format!("entity '{id}' not found")));
    }
    let edges = graph.find_all_active_edges_from(&id).await?;
    json_ok(edges)
}

pub(crate) async fn entity_delete_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<String>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(params.graph.as_deref(), &user)
        .await?;
    let entity = graph
        .get_entity_by_id(&id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("entity '{id}' not found")))?;
    let edges_invalidated = graph
        .delete_entity(&id)
        .await
        .map_err(internal("delete entity"))?;

    let _ = state
        .event_tx
        .send(crate::events::GraphEvent::EntityDeleted {
            id: entity.id.clone(),
            name: entity.name.clone(),
            edges_invalidated,
            graph: graph.graph_name().to_string(),
        });

    json_ok(serde_json::json!({
        "id": entity.id,
        "name": entity.name,
        "edges_invalidated": edges_invalidated,
    }))
}

pub(crate) async fn edge_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<i64>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(params.graph.as_deref(), &user)
        .await?;
    // Walk all edges from both endpoints to find this edge by ID
    let all_edges = graph.dump_all_edges().await?;
    match all_edges.into_iter().find(|e| e.edge_id == id) {
        Some(edge) => json_ok(edge),
        None => Err(AppError::not_found(format!("edge {id} not found"))),
    }
}

pub(crate) async fn edge_provenance_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<i64>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state
        .resolve_graph_for_user(params.graph.as_deref(), &user)
        .await?;
    let resp = graph.get_provenance(id).await?;
    json_ok(resp)
}

// -- Operations ---------------------------------------------------------------

pub(crate) async fn maintain_handler(
    State(state): State<Arc<AppState>>,
    Auth(_user): Auth,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    maintain::run_once(&state, &*graph).await?;
    json_ok(
        serde_json::json!({"status": "maintenance complete"}),
    )
}

pub(crate) async fn graph_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Query(params): Query<GraphQuery>,
) -> Result<impl IntoResponse, AppError> {
    let graph = state
        .resolve_graph_for_user(params.graph.as_deref(), &user)
        .await?;
    let entities = graph.dump_all_entities().await?;
    let all_edges = graph.dump_all_edges().await?;

    let fmt = params.format.as_deref().unwrap_or("json");
    match fmt {
        "graphml" => {
            let body = crate::export::to_graphml(&entities, &all_edges);
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/xml"),
                    (
                        "content-disposition",
                        "attachment; filename=\"graph.graphml\"",
                    ),
                ],
                body,
            )
                .into_response())
        }
        "csv" => {
            let body = crate::export::to_csv(&entities, &all_edges);
            Ok((
                StatusCode::OK,
                [
                    ("content-type", "text/csv"),
                    ("content-disposition", "attachment; filename=\"graph.csv\""),
                ],
                body,
            )
                .into_response())
        }
        _ => {
            let (active, invalidated): (Vec<_>, Vec<_>) =
                all_edges.into_iter().partition(|e| e.invalid_at.is_none());
            Ok(json_ok(serde_json::json!({
                "graph": graph.graph_name(),
                "entities": entities,
                "edges": { "active": active, "invalidated": invalidated },
            }))?
            .into_response())
        }
    }
}

// -- Observability ------------------------------------------------------------
