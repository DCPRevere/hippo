use axum::{
    extract::{rejection::JsonRejection, FromRequest, Path, Query, Request, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json, Router,
};
use futures::stream::Stream;
use tokio_stream::StreamExt;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::models::{
    AdminSeedRequest, AdminSeedResponse, AskRequest,
    BatchRememberRequest, BatchRememberResponse, BatchRememberResult,
    ConsolidateRequest, ContextProgress, ContextRequest, ErrorResponse, HealthResponse,
    MemoryTierStats, ReflectRequest, RememberProgress, RememberRequest,
    SmartQueryRequest, TemporalContextRequest,
};
use crate::pipeline::{ask, consolidate, context, context_temporal, diagnose, maintain, query, reflect, remember, timeline};
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
const MAX_HOPS: usize = 10;
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
        if let Some(limit) = self.limit {
            if limit == 0 || limit > MAX_LIMIT {
                return Err(format!("limit must be between 1 and {MAX_LIMIT}"));
            }
        }
        if let Some(hops) = self.max_hops {
            if hops > MAX_HOPS {
                return Err(format!("max_hops must be at most {MAX_HOPS}"));
            }
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

impl Validate for SmartQueryRequest {
    fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() {
            return Err("query must not be empty".into());
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

impl Validate for TemporalContextRequest {
    fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() {
            return Err("query must not be empty".into());
        }
        if let Some(limit) = self.limit {
            if limit == 0 || limit > MAX_LIMIT {
                return Err(format!("limit must be between 1 and {MAX_LIMIT}"));
            }
        }
        Ok(())
    }
}

impl Validate for ReflectRequest {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

impl Validate for ConsolidateRequest {
    fn validate(&self) -> Result<(), String> {
        if let Some(t) = self.prune_threshold {
            if !(0.0..=1.0).contains(&t) {
                return Err("prune_threshold must be between 0.0 and 1.0".into());
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
    let mut app = Router::new()
        .route("/ask", post(ask_handler))
        .route("/query", post(query_handler))
        .route("/remember", post(remember_handler))
        .route("/remember/stream", post(remember_stream_handler))
        .route("/remember/batch", post(remember_batch_handler))
        .route("/context", post(context_handler))
        .route("/context/stream", post(context_stream_handler))
        .route("/context/temporal", post(context_temporal_handler))
        .route("/timeline/{entity_name}", get(timeline_handler))
        .route("/reflect", post(reflect_handler))
        .route("/diagnose", post(diagnose_handler))
        .route("/graph", get(graph_handler))
        .route("/maintain", post(maintain_handler))
        .route("/consolidate", post(consolidate_handler))
        .route("/provenance/{edge_id}", get(provenance_handler))
        .route("/memory/stats", get(memory_stats_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/sources", get(sources_handler));

    if state.config.allow_admin {
        app = app
            .route("/admin/seed", post(admin_seed_handler))
            .route("/graphs", get(graphs_list_handler))
            .route("/graphs/drop/{name}", delete(graphs_drop_handler));
    }

    app.layer(TraceLayer::new_for_http())
        .with_state(state)
}

// -- Handlers -----------------------------------------------------------------

async fn ask_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<AskRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = ask::ask(&state, &*graph, req).await?;
    Ok(json_ok(resp))
}

async fn query_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<SmartQueryRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = query::smart_query(&state, &*graph, req).await?;
    Ok(json_ok(resp))
}

async fn remember_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<RememberRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = remember::remember(&state, &*graph, req, None).await?;
    Ok(json_ok(resp))
}

async fn remember_stream_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<RememberRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<RememberProgress>(32);
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;

    tokio::spawn(async move {
        match remember::remember(&state, &*graph, req, Some(tx.clone())).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Remember stream error: {e:#}");
                let _ = tx.send(RememberProgress::Error(e.to_string())).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|progress| {
        let data = serde_json::to_string(&progress).unwrap_or_default();
        Ok(Event::default().data(data))
    });

    Sse::new(stream)
}

async fn remember_batch_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<BatchRememberRequest>,
) -> Result<JsonOk, AppError> {
    let total = req.statements.len();
    let source_agent = req.source_agent.clone();
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;

    let results = if req.parallel {
        let futs: Vec<_> = req
            .statements
            .into_iter()
            .map(|statement| {
                let state = Arc::clone(&state);
                let source_agent = source_agent.clone();
                let graph = graph.clone();
                async move {
                    let remember_req = RememberRequest {
                        statement: statement.clone(),
                        source_agent,
                        source_credibility_hint: None,
                        graph: None,
                    };
                    match remember::remember(&state, &*graph, remember_req, None).await {
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
            };
            let result = match remember::remember(&state, &*graph, remember_req, None).await {
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
    ValidJson(req): ValidJson<ContextRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = context::context(&state, &*graph, req, None).await?;
    Ok(json_ok(resp))
}

async fn context_stream_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ContextRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<ContextProgress>(32);

    tokio::spawn(async move {
        let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
        match context::context(&state, &*graph, req, Some(tx.clone())).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Context stream error: {e:#}");
                let _ = tx.send(ContextProgress::Error(e.to_string())).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|progress| {
        let data = serde_json::to_string(&progress).unwrap_or_default();
        Ok(Event::default().data(data))
    });

    Sse::new(stream)
}

async fn context_temporal_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<TemporalContextRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = context_temporal::context_temporal(&state, &*graph, req).await?;
    Ok(json_ok(resp))
}

async fn reflect_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ReflectRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = reflect::reflect(&state, &*graph, req).await?;
    Ok(json_ok(resp))
}

async fn timeline_handler(
    State(state): State<Arc<AppState>>,
    Path(entity_name): Path<String>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    let resp = timeline::timeline(&state, &*graph, &entity_name).await?;
    Ok(json_ok(resp))
}

async fn diagnose_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ContextRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = diagnose::diagnose(&state, &*graph, req).await?;
    Ok(json_ok(resp))
}

async fn graph_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GraphQuery>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(params.graph.as_deref()).await;
    let resp = diagnose::graph_dump(&state, &*graph).await?;
    Ok(json_ok(resp))
}

async fn maintain_handler(
    State(state): State<Arc<AppState>>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    maintain::run_once(&state, &*graph).await?;
    Ok(json_ok(serde_json::json!({"status": "maintenance complete"})))
}

async fn consolidate_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ConsolidateRequest>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    let resp = consolidate::consolidate(&state, &*graph, req).await?;
    Ok(json_ok(resp))
}

async fn provenance_handler(
    State(state): State<Arc<AppState>>,
    Path(edge_id): Path<i64>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    let resp = graph.get_provenance(edge_id).await?;
    Ok(json_ok(resp))
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.to_prometheus(),
    )
}

async fn sources_handler(
    State(state): State<Arc<AppState>>,
) -> JsonOk {
    let sources = state.credibility.read().await.list();
    json_ok(serde_json::json!({ "sources": sources }))
}

async fn memory_stats_handler(
    State(state): State<Arc<AppState>>,
) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    let (working, long_term) = graph.memory_tier_stats().await?;
    Ok(json_ok(MemoryTierStats {
        working_count: working,
        long_term_count: long_term,
    }))
}

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

async fn admin_seed_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdminSeedRequest>,
) -> Result<JsonOk, AppError> {
    use chrono::Utc;
    use crate::llm::pseudo_embed;
    use crate::models::{Entity, MemoryTier, Relation};

    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;

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
) -> JsonOk {
    let graphs = state.graph_registry().list().await;
    json_ok(serde_json::json!({
        "default": state.graph_registry().default_graph_name(),
        "graphs": graphs,
    }))
}

async fn graphs_drop_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<JsonOk, AppError> {
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
