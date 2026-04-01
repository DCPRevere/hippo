use axum::{
    extract::{rejection::JsonRejection, FromRequest, Path, Query, Request, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json, Router,
};
use futures::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt as _;
use tower_http::trace::TraceLayer;

use crate::auth::Auth;
use crate::error::AppError;
use crate::models::{
    AdminSeedRequest, AdminSeedResponse, AskRequest,
    BatchRememberRequest, BatchRememberResponse, BatchRememberResult,
    ContextRequest, HealthResponse, RememberRequest,
};
use crate::pipeline::{ask, maintain, remember};
use crate::state::AppState;

// -- JSON response helper -----------------------------------------------------

struct JsonOk(String);

impl IntoResponse for JsonOk {
    fn into_response(self) -> axum::response::Response {
        let mut resp = self.0.into_response();
        *resp.status_mut() = StatusCode::OK;
        resp.headers_mut().insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
        resp
    }
}

fn json_ok(value: impl serde::Serialize) -> JsonOk {
    JsonOk(serde_json::to_string_pretty(&value).unwrap_or_default())
}

// -- Request validation -------------------------------------------------------

trait Validate {
    fn validate(&self) -> Result<(), String>;
}

/// An axum extractor that deserialises JSON then runs `Validate::validate`,
/// returning 400 on parse or validation failure.
struct ValidJson<T>(T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned + Validate + Send,
{
    type Rejection = AppError;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async {
            let Json(value) = Json::<T>::from_request(req, state)
                .await
                .map_err(|e: JsonRejection| AppError::bad_request(e.body_text()))?;

            value.validate().map_err(AppError::bad_request)?;
            Ok(ValidJson(value))
        }
    }
}

const MAX_LIMIT: usize = 500;
const MAX_BATCH: usize = 100;

impl Validate for RememberRequest {
    fn validate(&self) -> Result<(), String> {
        if self.statement.trim().is_empty() {
            return Err("statement must not be empty".into());
        }
        if let Some(h) = self.source_credibility_hint {
            if !(0.0..=1.0).contains(&h) {
                return Err("source_credibility_hint must be between 0.0 and 1.0".into());
            }
        }
        Ok(())
    }
}

impl Validate for ContextRequest {
    fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() {
            return Err("query must not be empty".into());
        }
        Ok(())
    }
}

impl Validate for AskRequest {
    fn validate(&self) -> Result<(), String> {
        if self.question.trim().is_empty() {
            return Err("question must not be empty".into());
        }
        if let Some(limit) = self.limit {
            if limit == 0 || limit > MAX_LIMIT {
                return Err(format!("limit must be between 1 and {MAX_LIMIT}"));
            }
        }
        Ok(())
    }
}

impl Validate for BatchRememberRequest {
    fn validate(&self) -> Result<(), String> {
        if self.statements.is_empty() {
            return Err("statements must not be empty".into());
        }
        if self.statements.len() > MAX_BATCH {
            return Err(format!("at most {MAX_BATCH} statements per batch"));
        }
        for (i, s) in self.statements.iter().enumerate() {
            if s.trim().is_empty() {
                return Err(format!("statements[{i}] must not be empty"));
            }
        }
        Ok(())
    }
}

// -- Router -------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct GraphQuery {
    graph: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    let app = Router::new()
        // Core endpoints
        .route("/remember", post(remember_handler))
        .route("/remember/batch", post(remember_batch_handler))
        .route("/context", post(context_handler))
        .route("/ask", post(ask_handler))
        // REST resources
        .route("/entities/{id}", get(entity_handler).delete(entity_delete_handler))
        .route("/entities/{id}/edges", get(entity_edges_handler))
        .route("/edges/{id}", get(edge_handler))
        .route("/edges/{id}/provenance", get(edge_provenance_handler))
        // Operations
        .route("/maintain", post(maintain_handler))
        .route("/graph", get(graph_handler))
        // SSE
        .route("/events", get(events_handler))
        // Observability
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        // Graphs
        .route("/graphs", get(graphs_list_handler))
        .route("/graphs/drop/{name}", delete(graphs_drop_handler))
        // Seed
        .route("/seed", post(seed_handler))
        // Admin user management
        .route("/admin/users", post(admin_create_user_handler))
        .route("/admin/users", get(admin_list_users_handler))
        .route("/admin/users/{user_id}", delete(admin_delete_user_handler))
        // API key management
        .route("/admin/users/{user_id}/keys", post(admin_create_key_handler))
        .route("/admin/users/{user_id}/keys", get(admin_list_keys_handler))
        .route("/admin/users/{user_id}/keys/{label}", delete(admin_revoke_key_handler));

    app.layer(TraceLayer::new_for_http())
        .with_state(state)
}

// -- Handlers -----------------------------------------------------------------

async fn remember_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<RememberRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(req.graph.as_deref(), &user).await?;
    let resp = remember::remember(&state, &*graph, req, None, Some(&user.user_id)).await?;
    Ok(json_ok(resp))
}

async fn remember_batch_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<BatchRememberRequest>,
) -> Result<JsonOk, AppError> {
    let total = req.statements.len();
    let source_agent = req.source_agent.clone();
    let graph = state.resolve_graph_for_user(req.graph.as_deref(), &user).await?;

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
                    match remember::remember(&state, &*graph, remember_req, None, Some(&uid)).await {
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
            let result = match remember::remember(&state, &*graph, remember_req, None, Some(&user_id)).await {
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

    Ok(json_ok(BatchRememberResponse {
        total,
        succeeded,
        failed,
        results,
    }))
}

async fn context_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<ContextRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(req.graph.as_deref(), &user).await?;
    let ctx = remember::gather_pre_extraction_context_at(&state, &*graph, &req.query, req.at, Some(&user.user_id)).await?;
    Ok(json_ok(ctx))
}

async fn ask_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<AskRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(req.graph.as_deref(), &user).await?;
    let resp = ask::ask(&state, &*graph, req, Some(&user.user_id), Some(&user.display_name)).await?;
    Ok(json_ok(resp))
}

// -- REST resources -----------------------------------------------------------

async fn entity_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<String>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(params.graph.as_deref(), &user).await?;
    match graph.get_entity_by_id(&id).await? {
        Some(entity) => Ok(json_ok(entity)),
        None => Err(AppError::not_found(format!("entity '{id}' not found"))),
    }
}

async fn entity_edges_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<String>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(params.graph.as_deref(), &user).await?;
    // Verify entity exists
    if graph.get_entity_by_id(&id).await?.is_none() {
        return Err(AppError::not_found(format!("entity '{id}' not found")));
    }
    let edges = graph.find_all_active_edges_from(&id).await?;
    Ok(json_ok(edges))
}

async fn entity_delete_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<String>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(params.graph.as_deref(), &user).await?;
    let entity = graph.get_entity_by_id(&id).await?
        .ok_or_else(|| AppError::not_found(format!("entity '{id}' not found")))?;
    let edges_invalidated = graph.delete_entity(&id).await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let _ = state.event_tx.send(crate::events::GraphEvent::EntityDeleted {
        id: entity.id.clone(),
        name: entity.name.clone(),
        edges_invalidated,
        graph: graph.graph_name().to_string(),
    });

    Ok(json_ok(serde_json::json!({
        "id": entity.id,
        "name": entity.name,
        "edges_invalidated": edges_invalidated,
    })))
}

async fn edge_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<i64>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(params.graph.as_deref(), &user).await?;
    // Walk all edges from both endpoints to find this edge by ID
    let all_edges = graph.dump_all_edges().await?;
    match all_edges.into_iter().find(|e| e.edge_id == id) {
        Some(edge) => Ok(json_ok(edge)),
        None => Err(AppError::not_found(format!("edge {id} not found"))),
    }
}

async fn edge_provenance_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(id): Path<i64>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(params.graph.as_deref(), &user).await?;
    let resp = graph.get_provenance(id).await?;
    Ok(json_ok(resp))
}

// -- Operations ---------------------------------------------------------------

async fn maintain_handler(
    State(state): State<Arc<AppState>>,
    Auth(_user): Auth,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    maintain::run_once(&state, &*graph).await?;
    Ok(json_ok(serde_json::json!({"status": "maintenance complete"})))
}

async fn graph_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.resolve_graph_for_user(params.graph.as_deref(), &user).await?;
    let entities = graph.dump_all_entities().await?;
    let all_edges = graph.dump_all_edges().await?;
    let (active, invalidated): (Vec<_>, Vec<_>) = all_edges
        .into_iter()
        .partition(|e| e.invalid_at.is_none());
    Ok(json_ok(serde_json::json!({
        "graph": graph.graph_name(),
        "entities": entities,
        "edges": { "active": active, "invalidated": invalidated },
    })))
}

// -- Observability ------------------------------------------------------------

async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    graph.ping().await.map_err(|e| {
        AppError::unavailable(format!("graph backend unavailable: {e}"))
    })?;
    Ok(json_ok(HealthResponse {
        status: "ok".to_string(),
        graph: state.graph_registry().default_graph_name().to_string(),
    }))
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.to_prometheus(),
    )
}



// -- SSE ----------------------------------------------------------------------

async fn events_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GraphQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let graph_filter = params.graph;

    let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(move |result| {
            match result {
                Ok(event) => {
                    // Apply optional graph filter
                    if let Some(ref g) = graph_filter {
                        if event.graph() != g {
                            return None;
                        }
                    }
                    let event_name = event.event_name().to_string();
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    Some(Ok(Event::default().event(event_name).data(data)))
                }
                // Skip lagged messages
                Err(_) => None,
            }
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// -- Admin --------------------------------------------------------------------

async fn seed_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Json(req): Json<AdminSeedRequest>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    use chrono::Utc;
    use crate::llm::pseudo_embed;
    use crate::models::{Entity, MemoryTier, Relation};

    let graph = state.resolve_graph_for_user(req.graph.as_deref(), &user).await?;

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
        graph.upsert_entity(&entity).await.map_err(|err| {
            AppError::internal(format!("entity '{}': {err}", e.name))
        })?;
        entities_created += 1;
    }

    for edge in &req.edges {
        let embedding = pseudo_embed(&edge.fact);
        let valid_at = edge.valid_at.as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let tier = match edge.memory_tier.as_str() {
            "working" => MemoryTier::Working,
            _ => MemoryTier::LongTerm,
        };
        let source_agents: Vec<String> = edge.source_agents
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
        graph.create_edge(&edge.subject_id, &edge.object_id, &relation).await.map_err(|err| {
            AppError::internal(format!("edge '{}': {err}", edge.fact))
        })?;
        edges_created += 1;
    }

    Ok(json_ok(AdminSeedResponse {
        entities_created,
        edges_created,
    }))
}

async fn graphs_list_handler(
    State(state): State<Arc<AppState>>,
    Auth(_user): Auth,
) -> JsonOk {
    let graphs = state.graph_registry().list().await;
    json_ok(serde_json::json!({
        "default": state.graph_registry().default_graph_name(),
        "graphs": graphs,
    }))
}

async fn graphs_drop_handler(
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

    Ok(json_ok(serde_json::json!({ "ok": true, "message": format!("Graph '{name}' dropped and reinitialised") })))
}

// -- Admin user management ----------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct CreateUserRequest {
    user_id: String,
    display_name: String,
    #[serde(default = "default_user_role")]
    role: String,
    #[serde(default)]
    graphs: Vec<String>,
}

fn default_user_role() -> String {
    "user".to_string()
}

async fn admin_create_user_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Json(req): Json<CreateUserRequest>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph_store = get_graph_store(&state)?;

    let raw_key = graph_store
        .create_user(&req.user_id, &req.display_name, &req.role, &req.graphs)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(json_ok(serde_json::json!({
        "user_id": req.user_id,
        "api_key": raw_key,
    })))
}

async fn admin_list_users_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph_store = get_graph_store(&state)?;

    let users = graph_store
        .list_users()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(json_ok(serde_json::json!({ "users": users })))
}

async fn admin_delete_user_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(user_id): Path<String>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph_store = get_graph_store(&state)?;

    graph_store
        .delete_user(&user_id)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(json_ok(serde_json::json!({ "ok": true })))
}

// -- Admin API key management -------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct CreateKeyRequest {
    label: String,
}

fn get_graph_store(state: &AppState) -> Result<&crate::auth::GraphUserStore, AppError> {
    let store = state
        .user_store
        .as_ref()
        .ok_or_else(|| AppError::bad_request("auth is not enabled"))?;
    store
        .as_any()
        .downcast_ref::<crate::auth::GraphUserStore>()
        .ok_or_else(|| AppError::internal("user store does not support management"))
}

async fn admin_create_key_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(user_id): Path<String>,
    Json(req): Json<CreateKeyRequest>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph_store = get_graph_store(&state)?;
    let raw_key = graph_store
        .create_api_key(&user_id, &req.label)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(json_ok(serde_json::json!({
        "user_id": user_id,
        "label": req.label,
        "api_key": raw_key,
    })))
}

async fn admin_list_keys_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path(user_id): Path<String>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph_store = get_graph_store(&state)?;
    let keys = graph_store
        .list_api_keys(&user_id)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(json_ok(serde_json::json!({ "keys": keys })))
}

async fn admin_revoke_key_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Path((user_id, label)): Path<(String, String)>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let graph_store = get_graph_store(&state)?;
    graph_store
        .revoke_api_key(&user_id, &label)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;

    Ok(json_ok(serde_json::json!({ "ok": true })))
}
